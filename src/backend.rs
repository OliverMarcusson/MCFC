use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{BinaryOp, ContextKind, PathSegment, SleepUnit, Type, UnaryOp};
use crate::ir::{
    IrAssignTarget, IrCapture, IrExpr, IrExprKind, IrFunction, IrMacroPlaceholder, IrPathExpr,
    IrProgram, IrStmt,
};
use crate::project::HelperConfig;
use crate::types::{CastKind, RefKind};

#[derive(Debug, Clone)]
pub struct BuildArtifacts {
    pub files: BTreeMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct BackendOptions {
    pub namespace: String,
    pub load_tag_values: Option<Vec<String>>,
    pub tick_tag_values: Option<Vec<String>>,
    pub exports: Vec<ExportedFunction>,
    pub helper: Option<HelperConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedFunction {
    pub path: String,
    pub function: String,
}

/// Merge sort steps per tick. Each moves one element and costs about 10
/// commands, so a slice is about 10,000 commands: roughly 10 ms of the
/// 50 ms tick, enough to finish fast without dropping ticks.
const SORT_STEPS_PER_TICK: usize = 1000;

pub fn generate(program: &IrProgram, options: &BackendOptions) -> BuildArtifacts {
    let mut backend = Backend::new(program, options.namespace.clone());
    backend.generate(program, options);
    BuildArtifacts {
        files: backend.files,
    }
}

struct Backend {
    namespace: String,
    files: BTreeMap<String, String>,
    functions: BTreeMap<String, FunctionInfo>,
    max_depth: usize,
    block_counter: usize,
    temp_counter: usize,
    macro_counter: usize,
    state_objectives: Vec<ManagedObjective>,
    state_storage_paths: BTreeSet<(bool, String)>,
    world_states: Vec<crate::ast::PlayerStateDef>,
    block_builder_state_fields: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    helper: Option<HelperConfig>,
    uses_rpc: bool,
    uses_food: bool,
    uses_impulse: bool,
    uses_health: bool,
    uses_raycast: bool,
    uses_ownership: bool,
    uses_sidebar: bool,
    uses_bitwise: bool,
    uses_log: bool,
    uses_actionbar: bool,
    uses_escape: bool,
    bukkit: BukkitRuntime,
    /// Per-site RPC waiter functions to register on the tick tag (reload-safe).
    rpc_tick_functions: Vec<String>,
    /// Functions that can pause, see `suspending_functions`.
    suspending: BTreeSet<String>,
    /// Recursive functions to their group, see `types::analyze_calls`.
    recursion_groups: BTreeMap<String, usize>,
    /// `(depth, function)` frames that recursive calls save and restore.
    frame_saves: BTreeSet<(usize, String)>,
}

#[derive(Debug, Clone)]
struct FunctionInfo {
    params: Vec<(String, Type)>,
    return_type: Type,
    locals: BTreeMap<String, Type>,
}

#[derive(Debug, Clone)]
struct ManagedObjective {
    objective: String,
    display_name: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct BukkitRuntime {
    join_handlers: Vec<String>,
    death_handlers: Vec<String>,
    /// `(event kind, handler)` for events raised by an advancement trigger.
    advancement_handlers: Vec<(String, String)>,
    agent_handlers: Vec<AgentEventHandler>,
    commands: Vec<BukkitCommand>,
    every_tasks: Vec<(String, u32)>,
    after_tasks: Vec<(String, u32)>,
    /// `(button label, trigger objective)` for `@Menu` commands.
    menu_buttons: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
struct BukkitCommand {
    command: String,
    handler: String,
    objective: String,
}

#[derive(Debug, Clone)]
struct AgentEventHandler {
    event: String,
    handler: String,
    parameter: String,
    decision: bool,
}

#[derive(Debug, Clone)]
struct Guard {
    ctrl_slot: String,
    break_slot: Option<String>,
    continue_slot: Option<String>,
}

#[derive(Debug, Clone)]
struct LoopContext {
    break_slot: String,
    continue_slot: String,
    continue_target: String,
}

#[derive(Debug, Clone)]
struct RenderedStoragePath {
    path: String,
    macro_storage: Option<String>,
}

#[derive(Debug, Clone)]
enum ContinuationItem {
    Stmt(Box<IrStmt>),
    Call {
        function_name: String,
        allow_continue: bool,
    },
    ClearScore(String),
    ExitContext,
}

#[derive(Debug, Clone)]
struct ContextResume {
    kind: ContextKind,
    anchor_slot: SlotRef,
}

impl Guard {
    fn for_function(depth: usize, function: &str) -> Self {
        Self {
            ctrl_slot: control_slot(depth, function),
            break_slot: None,
            continue_slot: None,
        }
    }

    fn within_loop(&self, loop_ctx: &LoopContext) -> Self {
        Self {
            ctrl_slot: self.ctrl_slot.clone(),
            break_slot: Some(loop_ctx.break_slot.clone()),
            continue_slot: Some(loop_ctx.continue_slot.clone()),
        }
    }

    fn wrap(&self, command: impl Into<String>) -> String {
        self.wrap_with_options(command, true, true)
    }

    fn wrap_allow_continue(&self, command: impl Into<String>) -> String {
        self.wrap_with_options(command, true, false)
    }

    fn wrap_with_options(
        &self,
        command: impl Into<String>,
        check_break: bool,
        check_continue: bool,
    ) -> String {
        let mut prefix = format!("execute if score {} mcfc matches 0", self.ctrl_slot);
        if check_break && let Some(slot) = &self.break_slot {
            prefix.push_str(&format!(" if score {} mcfc matches 0", slot));
        }
        if check_continue && let Some(slot) = &self.continue_slot {
            prefix.push_str(&format!(" if score {} mcfc matches 0", slot));
        }
        format!("{} run {}", prefix, command.into())
    }
}

impl Backend {
    fn new(program: &IrProgram, namespace: String) -> Self {
        let functions = program
            .functions
            .iter()
            .map(|function| {
                (
                    function.name.clone(),
                    FunctionInfo {
                        params: function
                            .params
                            .iter()
                            .map(|param| (param.name.clone(), param.ty.clone()))
                            .collect(),
                        return_type: function.return_type.clone(),
                        locals: function.locals.clone(),
                    },
                )
            })
            .collect();
        Self {
            namespace,
            files: BTreeMap::new(),
            functions,
            max_depth: program.call_depths.values().copied().max().unwrap_or(0) + 1,
            block_counter: 0,
            temp_counter: 0,
            macro_counter: 0,
            state_objectives: collect_state_objectives(program),
            world_states: program.world_states.clone(),
            state_storage_paths: program
                .player_states
                .iter()
                .filter(|state| !matches!(state.ty, Type::Int | Type::Bool))
                .map(|state| {
                    (
                        state.owner == crate::ast::StateOwner::Player,
                        state.path.join("."),
                    )
                })
                .collect(),
            block_builder_state_fields: collect_block_builder_state_fields(program),
            helper: None,
            uses_rpc: program_uses_rpc(program),
            uses_food: false,
            uses_impulse: false,
            uses_health: false,
            uses_raycast: false,
            uses_ownership: false,
            uses_sidebar: false,
            uses_bitwise: false,
            uses_log: false,
            uses_actionbar: false,
            uses_escape: false,
            bukkit: discover_bukkit_runtime(program),
            rpc_tick_functions: Vec::new(),
            suspending: suspending_functions(program),
            recursion_groups: program.recursion_groups.clone(),
            frame_saves: BTreeSet::new(),
        }
    }

    fn helper_backend(&self) -> crate::project::HelperBackend {
        self.helper
            .as_ref()
            .map(|config| config.backend)
            .unwrap_or_default()
    }

    fn agent_enabled(&self) -> bool {
        self.helper
            .as_ref()
            .and_then(|config| config.agent.as_ref())
            .is_some_and(|agent| agent.enabled)
    }

    fn generate(&mut self, program: &IrProgram, options: &BackendOptions) {
        self.helper = options.helper.clone();
        self.emit_pack_mcmeta();
        self.emit_load_tag(program, options.load_tag_values.as_deref());
        self.emit_setup();
        self.emit_main_entry();
        self.emit_tick_entry();
        for function in &program.functions {
            for depth in 0..=self.max_depth {
                self.emit_function_variant(function, depth);
            }
        }
        if self.uses_food {
            self.emit_food_runtime();
        }
        if self.uses_impulse {
            self.emit_impulse_runtime();
        }
        if self.uses_health {
            self.emit_health_runtime();
        }
        if self.uses_raycast {
            self.emit_raycast_runtime();
        }
        if self.uses_bitwise {
            self.emit_bitwise_runtime();
        }
        if self.uses_actionbar {
            self.emit_actionbar_runtime();
        }
        if self.uses_escape {
            self.emit_escape_runtime();
        }
        self.emit_world_state_defaults();
        if self.uses_log {
            // Default level info; `Log.setLevel` persists across reloads.
            let ns = self.namespace.clone();
            let setup = format!("data/{ns}/function/generated/setup.mcfunction");
            if let Some(body) = self.files.get_mut(&setup) {
                body.push_str(&format!(
                    "execute unless score #log.{ns} mcfc matches -2147483648.. run scoreboard players set #log.{ns} mcfc 1
"
                ));
            }
        }
        if self.uses_sidebar {
            let setup = format!(
                "data/{}/function/generated/setup.mcfunction",
                self.namespace
            );
            if let Some(body) = self.files.get_mut(&setup) {
                body.push_str(
                    "scoreboard objectives add mcfc_sidebar dummy
scoreboard objectives modify mcfc_sidebar numberformat blank
scoreboard objectives setdisplay sidebar mcfc_sidebar
",
                );
            }
        }
        if self.uses_ownership {
            let ns = &self.namespace;
            let setup = format!("data/{ns}/function/generated/setup.mcfunction");
            if let Some(body) = self.files.get_mut(&setup) {
                body.push_str(
                    "scoreboard objectives add mcfc_id dummy
scoreboard objectives add mcfc_owner dummy
",
                );
            }
            self.files.insert(
                format!("data/{ns}/function/generated/assign_id.mcfunction"),
                "scoreboard players add #next mcfc_id 1
scoreboard players operation @s mcfc_id = #next mcfc_id
"
                .to_string(),
            );
        }
        self.emit_bukkit_runtime();
        // Agent-only packs do not otherwise need the RPC runtime, but mcfd must
        // still discover their descriptor in order to attach and route events.
        if !self.uses_rpc && self.agent_enabled() {
            self.emit_mcfd_descriptor();
        }
        self.emit_auto_export_wrappers(program, &options.exports);
        self.emit_export_wrappers(&options.exports);
        self.emit_test_runner(program);
        self.emit_pack_menu();
        if self.uses_rpc {
            self.emit_rpc_runtime();
        }
        self.emit_frame_stack();
        // Emit the tick tag last so it can include the per-site RPC waiters
        // collected while emitting function bodies.
        self.emit_tick_tag(program, options.tick_tag_values.as_deref());
    }

    fn emit_pack_mcmeta(&mut self) {
        self.files.insert(
            "pack.mcmeta".to_string(),
            "{\n  \"pack\": {\n    \"min_format\": [121, 0],\n    \"max_format\": [121, 0],\n    \"description\": \"Generated by mcfc for Minecraft 26.3\"\n  }\n}\n"
                .to_string(),
        );
    }

    /// Lantern Load: `#minecraft:load` runs `#load:_private/load`, which resets
    /// `load.status` and then runs `#load:load` in the same order every reload,
    /// so this pack coexists with other packs using the convention.
    fn emit_load_tag(&mut self, program: &IrProgram, override_values: Option<&[String]>) {
        let ns = &self.namespace;
        let mut values = vec![format!("{ns}:generated/load_status")];
        // Static field initializers run before the pack's own load functions.
        for function in &program.functions {
            if function.name.ends_with("__clinit") {
                let path = crate::parser::resource_name(&function.name).replace("::", "/");
                values.push(format!("{ns}:{path}"));
            }
        }
        values.extend(
            override_values
                .map(|items| items.to_vec())
                .unwrap_or_else(|| vec![format!("{ns}:main")]),
        );
        for (path, contents) in [
            (
                "data/minecraft/tags/function/load.json",
                render_tag_file(&["#load:_private/load".to_string()]),
            ),
            (
                "data/load/tags/function/_private/load.json",
                "{\n  \"values\": [\n    \"#load:_private/init\",\n    {\"id\": \"#load:pre_load\", \"required\": false},\n    {\"id\": \"#load:load\", \"required\": false},\n    {\"id\": \"#load:post_load\", \"required\": false}\n  ]\n}\n".to_string(),
            ),
            (
                "data/load/tags/function/_private/init.json",
                render_tag_file(&["load:_private/init".to_string()]),
            ),
            (
                "data/load/function/_private/init.mcfunction",
                "scoreboard objectives add load.status dummy\nscoreboard players reset * load.status\n".to_string(),
            ),
            ("data/load/tags/function/load.json", render_tag_file(&values)),
            (
                &format!("data/{ns}/function/generated/load_status.mcfunction"),
                format!("scoreboard players set {ns} load.status 1\n"),
            ),
        ] {
            self.files.insert(path.to_string(), contents);
        }
    }

    fn emit_tick_tag(&mut self, program: &IrProgram, override_values: Option<&[String]>) {
        let mut values = override_values
            .map(|items| items.to_vec())
            .unwrap_or_default();
        if has_special_tick(program) {
            let tick = format!("{}:tick", self.namespace);
            if !values.contains(&tick) {
                values.push(tick);
            }
        }
        if self.uses_food {
            values.push(format!("{}:generated/food_tick", self.namespace));
        }
        if self.uses_health {
            values.push(format!("{}:generated/health_tick", self.namespace));
        }
        if self.uses_actionbar {
            values.push(format!("{}:generated/actionbar/tick", self.namespace));
        }
        if !self.bukkit.join_handlers.is_empty()
            || !self.bukkit.death_handlers.is_empty()
            || !self.bukkit.agent_handlers.is_empty()
            || !self.bukkit.commands.is_empty()
            || !self.bukkit.every_tasks.is_empty()
        {
            let runtime_tick = format!("{}:generated/bukkit/tick", self.namespace);
            if !values.contains(&runtime_tick) {
                values.push(runtime_tick);
            }
        }
        // The mcfd transport needs a global tick driver to throttle `/reload`
        // and apply the helper inbox while any request is in flight.
        if self.uses_rpc && self.helper_backend() == crate::project::HelperBackend::Mcfd {
            let pump = format!("{}:rpc/pump", self.namespace);
            if !values.contains(&pump) {
                values.push(pump);
            }
        }
        // Per-site RPC waiters run from the tick tag so they survive `/reload`.
        if self.uses_rpc {
            for tick_function in self.rpc_tick_functions.clone() {
                let value = format!("{}:{}", self.namespace, tick_function);
                if !values.contains(&value) {
                    values.push(value);
                }
            }
        }
        // Garbage collection runs last, when only paused functions are mid-call.
        // An idle tick costs the one compare.
        if program
            .functions
            .iter()
            .any(|function| function.name == "__mcfc_gc")
        {
            let ns = self.namespace.clone();
            self.files.insert(
                format!("data/{ns}/function/generated/gc.mcfunction"),
                format!(
                    "execute if score {} mcfc > {} mcfc run function {ns}:generated/gc_run
",
                    numeric_slot(0, "", "@world.mcfcAllocated"),
                    numeric_slot(0, "", "@world.mcfcThreshold"),
                ),
            );
            self.files.insert(
                format!("data/{ns}/function/generated/gc_run.mcfunction"),
                format!(
                    "scoreboard players set {} mcfc 0
function {ns}:{}
",
                    control_slot(0, "__mcfc_gc"),
                    self.function_entry_name("__mcfc_gc", 0)
                ),
            );
            values.push(format!("{ns}:generated/gc"));
        }
        if !values.is_empty() {
            self.files.insert(
                "data/minecraft/tags/function/tick.json".to_string(),
                render_tag_file(&values),
            );
        }
    }

    /// World state keeps its value across reloads; a missing one starts empty.
    /// Only states some function reads or writes get a line, so an unused
    /// `@WorldState` in `std` costs nothing.
    fn emit_world_state_defaults(&mut self) {
        let mut lines = String::new();
        for state in &self.world_states {
            let name = format!(
                "{}{}",
                crate::types::WORLD_STATE_PREFIX,
                state.path.join(".")
            );
            let slot = if matches!(
                state.ty,
                Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_)
            ) {
                numeric_slot(0, "", &name)
            } else {
                string_slot(0, "", &name)
            };
            let used = self.files.values().any(|body| {
                body.match_indices(&slot).any(|(at, _)| {
                    !body[at + slot.len()..]
                        .starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
                })
            });
            if !used {
                continue;
            }
            let default = match &state.ty {
                Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => {
                    lines.push_str(&format!(
                        "execute unless score {slot} mcfc matches -2147483648.. run scoreboard players set {slot} mcfc 0
"
                    ));
                    continue;
                }
                Type::String => "\"\"",
                Type::Float => "0.0f",
                Type::Array(_) => "[]",
                Type::EntityRef | Type::PlayerRef => Self::NO_ENTITY_HANDLE,
                _ => "{}",
            };
            lines.push_str(&format!(
                "execute unless data storage {ns}:runtime {slot} run data modify storage {ns}:runtime {slot} set value {default}
",
                ns = self.namespace
            ));
        }
        let setup = format!(
            "data/{}/function/generated/setup.mcfunction",
            self.namespace
        );
        if let Some(body) = self.files.get_mut(&setup) {
            body.push_str(&lines);
        }
    }

    fn emit_setup(&mut self) {
        let mut lines = vec![
            "scoreboard objectives add mcfc dummy".to_string(),
            format!(
                "data modify storage {}:runtime frames set value {{}}",
                self.namespace
            ),
            // A call chain cut off by the command limit never pops its
            // recursion frames; left in the world, they pile up every tick.
            format!("data remove storage {}:runtime stack", self.namespace),
        ];
        for objective in &self.state_objectives {
            if let Some(display_name) = &objective.display_name {
                lines.push(format!(
                    "scoreboard objectives add {} dummy {}",
                    objective.objective,
                    quoted(display_name)
                ));
            } else {
                lines.push(format!(
                    "scoreboard objectives add {} dummy",
                    objective.objective
                ));
            }
        }

        for depth in 0..=self.max_depth {
            for (function, info) in &self.functions {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    control_slot(depth, function)
                ));
                for (name, ty) in &info.locals {
                    if matches!(ty, Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_)) {
                        lines.push(format!(
                            "scoreboard players set {} mcfc 0",
                            numeric_slot(depth, function, name)
                        ));
                    }
                }
                if matches!(
                    info.return_type,
                    Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_)
                ) {
                    lines.push(format!(
                        "scoreboard players set {} mcfc 0",
                        numeric_return_slot(depth, function)
                    ));
                }
            }
        }

        if self.uses_rpc {
            // Shared host-bridge state. `out` carries requests for the mod/agent
            // transports; mcfd reads requests from the log marker instead.
            lines.push("data modify storage mcfc:rpc out set value []".to_string());
            lines.push("data modify storage mcfc:rpc sites set value {}".to_string());
            lines.push("data modify storage mcfc:rpc results set value {}".to_string());
            lines.push("scoreboard players set rpc_next mcfc 0".to_string());
            lines.push("scoreboard players set rpc_active mcfc 0".to_string());
            lines.push("scoreboard players set rpc_reload_timer mcfc 0".to_string());
        }
        if !self.bukkit.death_handlers.is_empty() {
            lines.push("scoreboard objectives add mcfc_deaths deathCount".to_string());
            lines.push("scoreboard objectives add mcfc_deaths_seen dummy".to_string());
        }
        for command in &self.bukkit.commands {
            lines.push(format!(
                "scoreboard objectives add {} trigger",
                command.objective
            ));
        }

        self.files.insert(
            format!(
                "data/{}/function/generated/setup.mcfunction",
                self.namespace
            ),
            lines.join("\n") + "\n",
        );
    }

    /// `escape.c.v` to `escape.s`, escaped for a "..." string. The game prints
    /// `escape.c` as `{v:"..."}` with the value escaped, unless the value's
    /// first quote is `"`: then it uses '...', which leaves `"` bare. Then a
    /// `'` is put in front, which makes the game pick "..." again.
    fn emit_escape_runtime(&mut self) {
        let ns = &self.namespace;
        let runtime = format!("storage {ns}:runtime");
        self.files.insert(
            format!("data/{ns}/function/generated/escape_string.mcfunction"),
            format!(
                "data modify {runtime} escape.q set string {runtime} escape.c 3 4
data modify {runtime} escape.s set string {runtime} escape.c 4 -2
execute if data {runtime} escape{{q:\"'\"}} run function {ns}:generated/escape_string_single with {runtime} escape
"
            ),
        );
        self.files.insert(
            format!("data/{ns}/function/generated/escape_string_single.mcfunction"),
            format!(
                "$data modify {runtime} escape.c.v set value '\\'$(s)'
data modify {runtime} escape.s set string {runtime} escape.c 5 -2
"
            ),
        );
    }

    fn emit_food_runtime(&mut self) {
        let ns = &self.namespace;
        let setup = format!("data/{ns}/function/generated/setup.mcfunction");
        if let Some(body) = self.files.get_mut(&setup) {
            body.push_str("scoreboard objectives add mcfc_food_goal dummy\n");
            body.push_str("scoreboard objectives add mcfc_food_active dummy\n");
            body.push_str("scoreboard objectives add mcfc_food_now dummy\n");
        }
        self.files.insert(
            format!("data/{ns}/function/generated/food_tick.mcfunction"),
            format!("execute as @a[scores={{mcfc_food_active=1..}}] run function {ns}:generated/food_player\n"),
        );
        self.files.insert(
            format!("data/{ns}/function/generated/food_player.mcfunction"),
            "execute store result score @s mcfc_food_now run data get entity @s foodLevel 1\n\
execute if score @s mcfc_food_now < @s mcfc_food_goal run effect give @s minecraft:saturation 1 0 true\n\
execute if score @s mcfc_food_now > @s mcfc_food_goal run effect give @s minecraft:hunger 1 255 true\n\
execute if score @s mcfc_food_now = @s mcfc_food_goal run scoreboard players set @s mcfc_food_active 0\n".to_string(),
        );
    }

    /// Players ignore `Motion` writes, so `addVelocity` equips a saddle whose
    /// enchantment fires `apply_impulse` once. The magnitude must be a constant,
    /// so each local axis is split into 32 bits with one gated effect per bit.
    fn emit_impulse_runtime(&mut self) {
        let ns = self.namespace.clone();
        let storage = |path: &str| {
            format!("{{type:\"storage\",storage:\"{ns}:runtime\",path:\"mcfc_impulse.{path}\"}}")
        };
        let single = |kind: &str, input: &str| format!("{{type:\"{kind}\",input:{input}}}");
        let many = |kind: &str, inputs: &[&str]| {
            format!("{{type:\"{kind}\",inputs:[{}]}}", inputs.join(","))
        };
        let radians = |index: usize| {
            many(
                "mul",
                &[&storage(&format!("rot[{index}]")), "0.017453292519943295"],
            )
        };
        let (sin_yaw, cos_yaw) = (single("sin", &radians(0)), single("cos", &radians(0)));
        let (sin_pitch, cos_pitch) = (single("sin", &radians(1)), single("cos", &radians(1)));
        let (x, y, z) = (storage("x"), storage("y"), storage("z"));
        let neg_x = single("negate", &x);
        // World velocity projected onto the player's left, up and forward axes,
        // the frame `apply_impulse` reads its direction in.
        let local = [
            many(
                "add",
                &[&many("mul", &[&x, &cos_yaw]), &many("mul", &[&z, &sin_yaw])],
            ),
            many(
                "add",
                &[
                    &many("mul", &[&neg_x, &sin_yaw, &sin_pitch]),
                    &many("mul", &[&y, &cos_pitch]),
                    &many("mul", &[&z, &cos_yaw, &sin_pitch]),
                ],
            ),
            many(
                "add",
                &[
                    &many("mul", &[&neg_x, &sin_yaw, &cos_pitch]),
                    &single("negate", &many("mul", &[&y, &sin_pitch])),
                    &many("mul", &[&z, &cos_yaw, &cos_pitch]),
                ],
            ),
        ];
        let mut body = vec![format!(
            "data modify storage {ns}:runtime mcfc_impulse.rot set from entity @s Rotation"
        )];
        let mut effects = vec![format!(
            "{{\"effect\":{{\"type\":\"minecraft:run_function\",\"function\":\"{ns}:generated/impulse_reset\"}}}}"
        )];
        for (axis, provider) in local.iter().enumerate() {
            let score = format!("#impulse_{axis}");
            body.push(format!(
                "data modify storage {ns}:runtime mcfc_impulse.local set compute default float {provider}"
            ));
            body.push(format!(
                "execute store result score {score} mcfc run data get storage {ns}:runtime mcfc_impulse.local 10000"
            ));
            // Two's complement: bit 31 carries -2^31 and the rest hold value + 2^31.
            body.push(format!("scoreboard players set {score}_31 mcfc 0"));
            body.push(format!(
                "execute if score {score} mcfc matches ..-1 run scoreboard players set {score}_31 mcfc 1"
            ));
            body.push(format!(
                "execute if score {score}_31 mcfc matches 1 run scoreboard players add {score} mcfc 2147483647"
            ));
            body.push(format!(
                "execute if score {score}_31 mcfc matches 1 run scoreboard players add {score} mcfc 1"
            ));
            for bit in (0..31).rev() {
                let value = 1i64 << bit;
                body.push(format!("scoreboard players set {score}_{bit} mcfc 0"));
                body.push(format!(
                    "execute if score {score} mcfc matches {value}.. store success score {score}_{bit} mcfc run scoreboard players remove {score} mcfc {value}"
                ));
            }
            let mut direction = [0; 3];
            direction[axis] = 1;
            let direction = format!("[{},{},{}]", direction[0], direction[1], direction[2]);
            for bit in 0..32 {
                let step = if bit == 31 {
                    -(1i64 << 31)
                } else {
                    1i64 << bit
                };
                let magnitude = step as f64 / 10000.0;
                effects.push(format!(
                    "{{\"requirements\":{{\"type\":\"minecraft:value_check\",\"value\":{{\"type\":\"minecraft:score\",\"target\":{{\"type\":\"minecraft:fixed\",\"name\":\"{score}_{bit}\"}},\"score\":\"mcfc\"}},\"range\":1}},\"effect\":{{\"type\":\"minecraft:apply_impulse\",\"direction\":{direction},\"coordinate_scale\":[1,1,1],\"magnitude\":{magnitude}}}}}"
                ));
            }
        }
        body.push(format!(
            "item replace entity @s saddle with minecraft:saddle[minecraft:equippable={{slot:\"saddle\",equip_sound:\"minecraft:intentionally_empty\"}},minecraft:enchantments={{\"{ns}:impulse\":1}}]"
        ));
        // Changing game mode re-evaluates equipment, which fires `location_changed`.
        // Falling creative players go through adventure so they don't start flying.
        body.extend(
            [
                "scoreboard players set #impulse_mode mcfc 0",
                "execute if entity @s[gamemode=survival] run scoreboard players set #impulse_mode mcfc 1",
                "execute if entity @s[gamemode=adventure] run scoreboard players set #impulse_mode mcfc 2",
                "execute if entity @s[gamemode=creative] run scoreboard players set #impulse_mode mcfc 3",
                "execute if score #impulse_mode mcfc matches 3 if predicate {type:\"minecraft:entity_properties\",entity:\"this\",predicate:{flags:{is_on_ground:false,is_flying:false}}} run scoreboard players set #impulse_mode mcfc 4",
                "execute if score #impulse_mode mcfc matches 1..3 run gamemode spectator",
                "execute if score #impulse_mode mcfc matches 4 run gamemode adventure",
                "execute if score #impulse_mode mcfc matches 1 run gamemode survival",
                "execute if score #impulse_mode mcfc matches 2 run gamemode adventure",
                "execute if score #impulse_mode mcfc matches 3..4 run gamemode creative",
                "item replace entity @s saddle with minecraft:air",
            ]
            .map(String::from),
        );
        self.files.insert(
            format!("data/{ns}/function/generated/impulse.mcfunction"),
            body.join("\n") + "\n",
        );
        self.files.insert(
            format!("data/{ns}/function/generated/impulse_reset.mcfunction"),
            "item replace entity @s saddle with minecraft:air\n".to_string(),
        );
        self.files.insert(
            format!("data/{ns}/enchantment/impulse.json"),
            format!(
                "{{\"description\":\"\",\"supported_items\":\"minecraft:saddle\",\"weight\":1,\"max_level\":1,\"min_cost\":{{\"base\":0,\"per_level_above_first\":0}},\"max_cost\":{{\"base\":0,\"per_level_above_first\":0}},\"anvil_cost\":0,\"slots\":[\"saddle\"],\"effects\":{{\"minecraft:location_changed\":[{}]}}}}\n",
                effects.join(",")
            ),
        );
    }

    /// Steps 0.1 blocks from the eyes until a non-replaceable block or, in entity
    /// mode, a hitbox other than the caster's. `#ray_hit` is 1 for a block, 2 for an entity.
    fn emit_raycast_runtime(&mut self) {
        let ns = &self.namespace;
        let files = [
            (
                "start",
                format!(
                    "tag @s add mcfc_ray_source\nscoreboard players set #ray_hit mcfc 0\nfunction {ns}:generated/raycast/step\ntag @s remove mcfc_ray_source"
                ),
            ),
            (
                "step",
                format!(
                    "execute unless block ~ ~ ~ #minecraft:replaceable run return run function {ns}:generated/raycast/block\n\
execute if score #ray_mode mcfc matches 1 as @e[dx=0,dy=0,dz=0,tag=!mcfc_ray_source] positioned ~-0.99 ~-0.99 ~-0.99 if entity @s[dx=0,dy=0,dz=0] run return run function {ns}:generated/raycast/entity\n\
scoreboard players remove #ray_steps mcfc 1\n\
execute if score #ray_steps mcfc matches 1.. positioned ^ ^ ^0.1 run function {ns}:generated/raycast/step"
                ),
            ),
            (
                "entity",
                "tag @s add mcfc_ray_hit\nscoreboard players set #ray_hit mcfc 2".to_string(),
            ),
            (
                "block",
                format!(
                    "scoreboard players set #ray_hit mcfc 1\n\
execute align xyz run summon minecraft:marker ~ ~ ~ {{Tags:[\"mcfc_ray_marker\"]}}\n\
execute store result storage {ns}:runtime mcfc_ray.x int 1 run data get entity @e[type=minecraft:marker,tag=mcfc_ray_marker,limit=1] Pos[0]\n\
execute store result storage {ns}:runtime mcfc_ray.y int 1 run data get entity @e[type=minecraft:marker,tag=mcfc_ray_marker,limit=1] Pos[1]\n\
execute store result storage {ns}:runtime mcfc_ray.z int 1 run data get entity @e[type=minecraft:marker,tag=mcfc_ray_marker,limit=1] Pos[2]\n\
kill @e[type=minecraft:marker,tag=mcfc_ray_marker]\n\
function {ns}:generated/raycast/pos with storage {ns}:runtime mcfc_ray"
                ),
            ),
            (
                "pos",
                format!(
                    "$data modify storage {ns}:runtime mcfc_ray.pos set value \"$(x) $(y) $(z)\""
                ),
            ),
        ];
        for (name, body) in files {
            self.files.insert(
                format!("data/{ns}/function/generated/raycast/{name}.mcfunction"),
                body + "\n",
            );
        }
    }

    /// `block.getType()`: a binary search over every block id. Each node tests
    /// a block tag holding the lower half of its ids; leaves test up to 8 ids
    /// one by one, so a lookup is about 16 commands.
    fn emit_block_type_probe(&mut self) {
        let ns = self.namespace.clone();
        if self.files.contains_key(&format!(
            "data/{ns}/function/generated/block_type/0.mcfunction"
        )) {
            return;
        }
        fn node(
            files: &mut BTreeMap<String, String>,
            ns: &str,
            ids: &[&str],
            next: &mut usize,
        ) -> usize {
            let index = *next;
            *next += 1;
            let body = if ids.len() <= 8 {
                ids.iter()
                    .map(|id| format!("execute if block ~ ~ ~ {id} run return run data modify storage {ns}:runtime block_type set value \"{id}\"
"))
                    .collect::<String>()
            } else {
                let (low, high) = ids.split_at(ids.len() / 2);
                let low_node = node(files, ns, low, next);
                let high_node = node(files, ns, high, next);
                let values: Vec<String> = low.iter().map(|id| id.to_string()).collect();
                files.insert(
                    format!("data/{ns}/tags/block/mcfc_block_type/{index}.json"),
                    render_tag_file(&values),
                );
                format!(
                    "execute if block ~ ~ ~ #{ns}:mcfc_block_type/{index} run return run function {ns}:generated/block_type/{low_node}
function {ns}:generated/block_type/{high_node}
"
                )
            };
            files.insert(
                format!("data/{ns}/function/generated/block_type/{index}.mcfunction"),
                body,
            );
            index
        }
        node(
            &mut self.files,
            &ns,
            crate::minecraft_ids::BLOCK_IDS,
            &mut 0,
        );
    }

    /// Bitwise ops on scores `#bit_a`, `#bit_b` into `#bit_r`. `bitwise` walks
    /// the bits low to high with floored `%` and `/`, which keep the sign; `shift` multiplies or floor-divides by a power of two.
    fn emit_bitwise_runtime(&mut self) {
        let ns = self.namespace.clone();
        let bitwise = "scoreboard players set #bit_r mcfc 0
scoreboard players set #bit_p mcfc 1
scoreboard players set #bit_two mcfc 2
function NS:generated/bitwise/step
";
        // Once both inputs are 0 or -1, every higher bit is that sign bit, so the
        // rest of the result is all ones (-p) or all zeros.
        let step = "execute if score #bit_a mcfc matches -1..0 if score #bit_b mcfc matches -1..0 run return run function NS:generated/bitwise/tail
scoreboard players operation #bit_x mcfc = #bit_a mcfc
scoreboard players operation #bit_x mcfc %= #bit_two mcfc
scoreboard players operation #bit_y mcfc = #bit_b mcfc
scoreboard players operation #bit_y mcfc %= #bit_two mcfc
scoreboard players operation #bit_x mcfc += #bit_y mcfc
execute if score #bit_op mcfc matches 0 if score #bit_x mcfc matches 2 run scoreboard players operation #bit_r mcfc += #bit_p mcfc
execute if score #bit_op mcfc matches 1 if score #bit_x mcfc matches 1.. run scoreboard players operation #bit_r mcfc += #bit_p mcfc
execute if score #bit_op mcfc matches 2 if score #bit_x mcfc matches 1 run scoreboard players operation #bit_r mcfc += #bit_p mcfc
scoreboard players operation #bit_a mcfc /= #bit_two mcfc
scoreboard players operation #bit_b mcfc /= #bit_two mcfc
scoreboard players operation #bit_p mcfc += #bit_p mcfc
function NS:generated/bitwise/step
";
        let tail = "scoreboard players operation #bit_x mcfc = #bit_a mcfc
scoreboard players operation #bit_x mcfc += #bit_b mcfc
execute if score #bit_op mcfc matches 0 if score #bit_x mcfc matches -2 run scoreboard players operation #bit_r mcfc -= #bit_p mcfc
execute if score #bit_op mcfc matches 1 if score #bit_x mcfc matches ..-1 run scoreboard players operation #bit_r mcfc -= #bit_p mcfc
execute if score #bit_op mcfc matches 2 if score #bit_x mcfc matches -1 run scoreboard players operation #bit_r mcfc -= #bit_p mcfc
";
        // Java masks the count to 0..31; `%=` is floorMod, so -1 becomes 31.
        let mut shift = "scoreboard players set #bit_two mcfc 32
scoreboard players operation #bit_b mcfc %= #bit_two mcfc
"
        .to_string();
        for bit in 0..32 {
            shift.push_str(&format!(
                "execute if score #bit_b mcfc matches {bit} run scoreboard players set #bit_p mcfc {}
",
                1i32.wrapping_shl(bit)
            ));
        }
        shift.push_str("scoreboard players operation #bit_r mcfc = #bit_a mcfc
execute if score #bit_op mcfc matches 0 run scoreboard players operation #bit_r mcfc *= #bit_p mcfc
execute if score #bit_op mcfc matches 1 unless score #bit_b mcfc matches 31 run scoreboard players operation #bit_r mcfc /= #bit_p mcfc
execute if score #bit_op mcfc matches 1 if score #bit_b mcfc matches 31 run scoreboard players set #bit_r mcfc 0
execute if score #bit_op mcfc matches 1 if score #bit_b mcfc matches 31 if score #bit_a mcfc matches ..-1 run scoreboard players set #bit_r mcfc -1
");
        for (name, body) in [
            ("bitwise", bitwise.to_string()),
            ("step", step.to_string()),
            ("tail", tail.to_string()),
            ("shift", shift),
        ] {
            self.files.insert(
                format!("data/{ns}/function/generated/bitwise/{name}.mcfunction"),
                body.replace("NS", &ns),
            );
        }
    }

    /// Each advancement event gets an advancement whose reward runs as the player,
    /// revokes itself so it fires again, and calls the handler.
    fn emit_advancement_events(&mut self) {
        let ns = self.namespace.clone();
        for (kind, handler) in self.bukkit.advancement_handlers.clone() {
            let (_, trigger) = crate::language_catalog::ADVANCEMENT_EVENTS
                .iter()
                .find(|(event, _)| *event == kind)
                .unwrap();
            // Only hits from an entity: falls and fire have nothing for entity().
            let conditions = if kind == "entity_hurt_player" {
                ",\"conditions\":{\"damage\":{\"source_entity\":{}}}"
            } else {
                ""
            };
            self.files.insert(
                format!("data/{ns}/advancement/mcfc_event/{kind}.json"),
                format!(
                    "{{\"criteria\":{{\"event\":{{\"trigger\":\"minecraft:{trigger}\"{conditions}}}}},\"rewards\":{{\"function\":\"{ns}:generated/bukkit/{kind}\"}}}}
"
                ),
            );
            let mut lines = vec![format!("advancement revoke @s only {ns}:mcfc_event/{kind}")];
            let has_entity = crate::language_catalog::vanilla_event_has_entity(&kind);
            if kind == "player_hurt_entity" {
                // The victim was hurt this tick (HurtTime 10) by this player. That
                // misses interaction entities, which the look ray below finds.
                lines.push("tag @s add mcfc_event_self".to_string());
                lines.push(format!(
                    "execute as @e[nbt={{HurtTime:10s}}] if function {ns}:generated/bukkit/attacked_by_self run tag @s add mcfc_event_target"
                ));
                lines.push("tag @s remove mcfc_event_self".to_string());
                self.files.insert(
                    format!("data/{ns}/function/generated/bukkit/attacked_by_self.mcfunction"),
                    "return run execute on attacker if entity @s[tag=mcfc_event_self]
"
                    .to_string(),
                );
            }
            if crate::language_catalog::vanilla_event_has_block(&kind) {
                self.emit_raycast_runtime();
                let out = format!("storage {ns}:runtime mcfc_event.block");
                lines.extend([
                    ray_reach_steps("block"),
                    "scoreboard players set #ray_mode mcfc 0".to_string(),
                    format!("execute anchored eyes positioned ^ ^ ^ run function {ns}:generated/raycast/start"),
                    format!("data modify {out} set value {{present:0b}}"),
                    format!("execute if score #ray_hit mcfc matches 1 run data modify {out} set value {{present:1b,value:{{prefix:\"\"}}}}"),
                    format!("execute if score #ray_hit mcfc matches 1 run data modify {out}.value.pos set from storage {ns}:runtime mcfc_ray.pos"),
                ]);
            }
            if kind == "entity_hurt_player" {
                lines.push("execute on attacker run tag @s add mcfc_event_target".to_string());
            } else if has_entity {
                self.emit_raycast_runtime();
                lines.extend([
                    ray_reach_steps("entity"),
                    "scoreboard players set #ray_mode mcfc 1".to_string(),
                    format!(
                        "execute unless entity @e[tag=mcfc_event_target] anchored eyes positioned ^ ^ ^ run function {ns}:generated/raycast/start"
                    ),
                    "execute if score #ray_hit mcfc matches 2 run tag @e[tag=mcfc_ray_hit] add mcfc_event_target".to_string(),
                    "tag @e[tag=mcfc_ray_hit] remove mcfc_ray_hit".to_string(),
                ]);
            }
            lines.push(format!(
                "scoreboard players set {} mcfc 0",
                control_slot(0, &handler)
            ));
            lines.push(format!(
                "function {ns}:{}",
                self.function_entry_name(&handler, 0)
            ));
            if has_entity {
                lines.push("tag @e[tag=mcfc_event_target] remove mcfc_event_target".to_string());
            }
            self.files.insert(
                format!("data/{ns}/function/generated/bukkit/{kind}.mcfunction"),
                lines.join(
                    "
",
                ) + "
",
            );
        }
    }

    /// Caps max health at the target with a modifier, then heals to the cap.
    /// The heal lands on a later player tick, so the cap stays for two ticks.
    fn emit_health_runtime(&mut self) {
        let ns = self.namespace.clone();
        let setup = format!("data/{ns}/function/generated/setup.mcfunction");
        if let Some(body) = self.files.get_mut(&setup) {
            body.push_str("scoreboard objectives add mcfc_health_wait dummy\n");
        }
        let storage = |path: &str| {
            format!("{{type:\"storage\",storage:\"{ns}:runtime\",path:\"mcfc_health.{path}\"}}")
        };
        let files = [
            (
                "set_health",
                [
                    format!("execute store result score #health mcfc run data get storage {ns}:runtime mcfc_health.target 1000"),
                    "execute if score #health mcfc matches ..0 unless entity @s[gamemode=creative] unless entity @s[gamemode=spectator] run return run kill @s".to_string(),
                    "execute if score #health mcfc matches ..0 run return 0".to_string(),
                    format!("execute store result storage {ns}:runtime mcfc_health.max double 0.001 run attribute @s minecraft:max_health get 1000"),
                    format!(
                        "data modify storage {ns}:runtime mcfc_health.cap set compute default float {{type:\"sub\",left:{{type:\"div\",left:{{type:\"min\",inputs:[{},{}]}},right:{}}},right:1}}",
                        storage("target"),
                        storage("max"),
                        storage("max")
                    ),
                    format!("attribute @s minecraft:max_health modifier remove {ns}:health_cap"),
                    format!("function {ns}:generated/health_cap with storage {ns}:runtime mcfc_health"),
                    "effect clear @s minecraft:instant_health".to_string(),
                    "effect give @s minecraft:instant_health 1 28 true".to_string(),
                    "scoreboard players set @s mcfc_health_wait 2".to_string(),
                ]
                .join("\n"),
            ),
            (
                "health_cap",
                format!("$attribute @s minecraft:max_health modifier add {ns}:health_cap $(cap) add_multiplied_total"),
            ),
            (
                "health_tick",
                format!("execute as @a[scores={{mcfc_health_wait=1..}}] run function {ns}:generated/health_wait"),
            ),
            (
                "health_wait",
                format!(
                    "scoreboard players remove @s mcfc_health_wait 1\nexecute if score @s mcfc_health_wait matches 0 run attribute @s minecraft:max_health modifier remove {ns}:health_cap"
                ),
            ),
        ];
        for (name, body) in files {
            self.files.insert(
                format!("data/{ns}/function/generated/{name}.mcfunction"),
                body + "\n",
            );
        }
    }

    fn emit_main_entry(&mut self) {
        let mut body = vec![format!("function {}:generated/setup", self.namespace)];
        if !self.bukkit.after_tasks.is_empty() {
            body.push(format!("function {}:generated/bukkit/load", self.namespace));
        }
        if self.functions.contains_key("main") {
            body.push(format!(
                "scoreboard players set {} mcfc 0",
                control_slot(0, "main")
            ));
            body.push(format!(
                "function {}:{}",
                self.namespace,
                self.function_entry_name("main", 0)
            ));
        }

        let lines = if self.uses_rpc {
            // The mcfd transport delivers results with `/reload`, which re-runs the
            // load tag. Guard setup + main so they run once per world instead of on
            // every reload — re-running would restart the program and wipe in-flight
            // RPC state. Scheduled continuations persist across reload, so the
            // program keeps progressing without re-initialising. Reset the
            // `#mcfc_init` fake-player score to re-arm a fresh start.
            let mut guarded = vec![
                "scoreboard objectives add mcfc dummy".to_string(),
                "execute if score #mcfc_init mcfc matches 1 run return 0".to_string(),
                "scoreboard players set #mcfc_init mcfc 1".to_string(),
            ];
            guarded.extend(body);
            guarded
        } else {
            body
        };

        self.files.insert(
            format!("data/{}/function/main.mcfunction", self.namespace),
            lines.join("\n") + "\n",
        );
    }

    fn emit_tick_entry(&mut self) {
        let Some(info) = self.functions.get("tick") else {
            return;
        };
        if !info.params.is_empty() || info.return_type != Type::Void {
            return;
        }
        let contents = format!(
            "scoreboard players set {} mcfc 0\nfunction {}:{}\n",
            control_slot(0, "tick"),
            self.namespace,
            self.function_entry_name("tick", 0)
        );
        self.files.insert(
            format!("data/{}/function/tick.mcfunction", self.namespace),
            contents,
        );
    }

    fn emit_auto_export_wrappers(&mut self, program: &IrProgram, exports: &[ExportedFunction]) {
        for function in &program.functions {
            if function.generated
                || function.name == "main"
                || function.name == "tick"
                || function.name.starts_with("std::")
                || is_bukkit_generated_function(&function.name)
                || !function.params.is_empty()
                || function.return_type != Type::Void
            {
                continue;
            }
            // Module functions (`util::greet`) get nested paths (`util/greet`).
            let public_path = crate::parser::resource_name(&function.name).replace("::", "/");
            let relative = format!(
                "data/{}/function/{}.mcfunction",
                self.namespace, public_path
            );
            if exports
                .iter()
                .any(|export| export.path.trim_matches('/') == public_path)
            {
                continue;
            }
            if !self.files.contains_key(&relative) {
                let contents = format!(
                    "scoreboard players set {} mcfc 0\nfunction {}:{}\n",
                    control_slot(0, &function.name),
                    self.namespace,
                    self.function_entry_name(&function.name, 0)
                );
                self.files.insert(relative, contents);
            }
        }
    }

    fn emit_bukkit_runtime(&mut self) {
        let has_tick_runtime = !self.bukkit.join_handlers.is_empty()
            || !self.bukkit.death_handlers.is_empty()
            || !self.bukkit.agent_handlers.is_empty()
            || !self.bukkit.commands.is_empty()
            || !self.bukkit.every_tasks.is_empty();
        if has_tick_runtime {
            let mut tick = Vec::new();
            if !self.bukkit.join_handlers.is_empty() {
                let tag = bukkit_join_tag(&self.namespace);
                tick.push(format!(
                    "execute as @a[tag=!{}] run function {}:generated/bukkit/player_join",
                    tag, self.namespace
                ));
                tick.push(format!("tag @a[tag=!{}] add {}", tag, tag));
            }
            if !self.bukkit.death_handlers.is_empty() {
                tick.push(format!(
                    "execute as @a if score @s mcfc_deaths matches 1.. unless score @s mcfc_deaths_seen = @s mcfc_deaths run function {}:generated/bukkit/player_death",
                    self.namespace
                ));
                tick.push("execute as @a if score @s mcfc_deaths matches 1.. run scoreboard players operation @s mcfc_deaths_seen = @s mcfc_deaths".to_string());
            }
            for command in &self.bukkit.commands {
                let objective = &command.objective;
                tick.push(format!("scoreboard players enable @a {}", objective));
                tick.push(format!(
                    "execute as @a[scores={{{}=1..}}] run function {}:generated/bukkit/command/{}",
                    objective, self.namespace, command.command
                ));
                tick.push(format!(
                    "scoreboard players set @a[scores={{{}=1..}}] {} 0",
                    objective, objective
                ));
            }
            for (task, interval) in &self.bukkit.every_tasks {
                let clock = format!("#mcfct_{}", sanitize(task));
                tick.push(format!("scoreboard players add {} mcfc 1", clock));
                tick.push(format!(
                    "execute if score {} mcfc matches {}.. run scoreboard players set {} mcfc 0",
                    clock, interval, clock
                ));
                tick.push(format!(
                    "execute if score {} mcfc matches 0 run function {}:generated/bukkit/task/{}",
                    clock, self.namespace, task
                ));
            }
            self.files.insert(
                format!(
                    "data/{}/function/generated/bukkit/tick.mcfunction",
                    self.namespace
                ),
                tick.join("\n") + "\n",
            );
        }

        for (path, handlers) in [
            ("player_join", self.bukkit.join_handlers.clone()),
            ("player_death", self.bukkit.death_handlers.clone()),
        ] {
            if handlers.is_empty() {
                continue;
            }
            let mut lines = Vec::new();
            for handler in handlers {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    control_slot(0, &handler)
                ));
                lines.push(format!(
                    "function {}:{}",
                    self.namespace,
                    self.function_entry_name(&handler, 0)
                ));
            }
            self.files.insert(
                format!(
                    "data/{}/function/generated/bukkit/{}.mcfunction",
                    self.namespace, path
                ),
                lines.join("\n") + "\n",
            );
        }

        self.emit_advancement_events();

        // The JVM agent writes a typed event compound into `<ns>:agent current`
        // and invokes this wrapper as the affected player. Copying the compound
        // into the ordinary function frame makes agent handlers indistinguishable
        // from regular MCFC functions to the rest of the backend.
        for agent in self.bukkit.agent_handlers.clone() {
            let parameter_type = self.functions[&agent.handler].params[0].1.clone();
            let parameter_slot = local_slot(0, &agent.handler, &agent.parameter, &parameter_type);
            let contents = format!(
                "data modify storage {}:runtime {} set from storage {}:agent current\nscoreboard players set {} mcfc 0\nfunction {}:{}\n",
                self.namespace,
                parameter_slot.storage_path(),
                self.namespace,
                control_slot(0, &agent.handler),
                self.namespace,
                self.function_entry_name(&agent.handler, 0),
            );
            self.files.insert(
                format!(
                    "data/{}/function/agent/event/{}.mcfunction",
                    self.namespace, agent.event
                ),
                contents,
            );
        }

        for command in self.bukkit.commands.clone() {
            let mut contents = String::new();
            if let Some(info) = self.functions.get(&command.handler)
                && info.params.len() == 2
                && info.params[0].1 == Type::Struct("CommandSender".to_string())
                && info.params[1].1 == Type::Array(Box::new(Type::String))
            {
                let sender = local_slot(0, &command.handler, &info.params[0].0, &info.params[0].1);
                let args = local_slot(0, &command.handler, &info.params[1].0, &info.params[1].1);
                contents.push_str(&format!(
                        "data modify storage {}:runtime {} set from storage {}:agent command.sender\ndata modify storage {}:runtime {} set from storage {}:agent command.args\n",
                        self.namespace, sender.storage_path(), self.namespace,
                        self.namespace, args.storage_path(), self.namespace,
                    ));
            }
            contents.push_str(&format!(
                "scoreboard players set {} mcfc 0\nfunction {}:{}\n",
                control_slot(0, &command.handler),
                self.namespace,
                self.function_entry_name(&command.handler, 0)
            ));
            self.files.insert(
                format!(
                    "data/{}/function/generated/bukkit/command/{}.mcfunction",
                    self.namespace, command.command
                ),
                contents.clone(),
            );
            self.files.insert(
                format!(
                    "data/{}/function/agent/command/{}.mcfunction",
                    self.namespace, command.command
                ),
                contents,
            );
        }

        for (task, ticks) in self.bukkit.every_tasks.clone() {
            let Some(function) = self.bukkit_function_for_task(&task, ticks, true) else {
                continue;
            };
            self.emit_bukkit_task_wrapper(&task, &function);
        }

        if !self.bukkit.after_tasks.is_empty() {
            let mut load = Vec::new();
            for (task, ticks) in self.bukkit.after_tasks.clone() {
                let function = self
                    .bukkit_function_for_task(&task, ticks, false)
                    .expect("discovered task has a handler");
                self.emit_bukkit_task_wrapper(&task, &function);
                load.push(format!(
                    "schedule function {}:generated/bukkit/task/{} {}t replace",
                    self.namespace, task, ticks
                ));
            }
            self.files.insert(
                format!(
                    "data/{}/function/generated/bukkit/load.mcfunction",
                    self.namespace
                ),
                load.join("\n") + "\n",
            );
        }
    }

    fn bukkit_function_for_task(&self, task: &str, ticks: u32, repeating: bool) -> Option<String> {
        let suffix = if repeating {
            "every_ticks"
        } else {
            "after_ticks"
        };
        let expected = format!("__mcfc_task_{}_{}_{}", task, suffix, ticks);
        self.functions.contains_key(&expected).then_some(expected)
    }

    fn emit_bukkit_task_wrapper(&mut self, task: &str, handler: &str) {
        let contents = format!(
            "scoreboard players set {} mcfc 0\nfunction {}:{}\n",
            control_slot(0, handler),
            self.namespace,
            self.function_entry_name(handler, 0)
        );
        self.files.insert(
            format!(
                "data/{}/function/generated/bukkit/task/{}.mcfunction",
                self.namespace, task
            ),
            contents,
        );
    }

    /// `@Menu` buttons live in `ns:about`, listed in the Smithed data pack menu
    /// on the pause screen (docs.smithed.dev/conventions/data-pack-menu).
    fn emit_pack_menu(&mut self) {
        if self.bukkit.menu_buttons.is_empty() {
            return;
        }
        let ns = self.namespace.clone();
        let entry = |id: &str| {
            format!(
                "{{
  \"values\": [
    {{
      \"id\": \"{id}\",
      \"required\": false
    }}
  ]
}}
"
            )
        };
        let back = r#"{"action":{"type":"show_dialog","dialog":"smithed:data_packs"},"label":{"translate":"gui.back"},"width":200}"#;
        let actions = self
            .bukkit
            .menu_buttons
            .iter()
            .map(|(label, objective)| {
                format!(
                    r#"{{"label":{},"action":{{"type":"run_command","command":"/trigger {objective}"}}}}"#,
                    quoted(label)
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let about = format!(
            r#"{{"type":"minecraft:multi_action","title":{},"actions":[{actions}],"exit_action":{back}}}"#,
            quoted(&ns)
        );
        let root = r##"{"type":"minecraft:dialog_list","external_title":{"translate":"menu.smithed.data_packs","fallback":"%s...","with":[{"translate":"selectWorld.dataPacks"}]},"title":{"translate":"menu.smithed.data_packs.title","fallback":"%s","with":[{"translate":"selectWorld.dataPacks"}]},"dialogs":"#smithed:data_packs","exit_action":{"label":{"translate":"gui.back"},"width":200}}"##;
        self.files.insert(
            format!("data/{ns}/dialog/about.json"),
            about
                + "
",
        );
        self.files.insert(
            "data/smithed/dialog/data_packs.json".to_string(),
            root.to_string()
                + "
",
        );
        self.files.insert(
            "data/smithed/tags/dialog/data_packs.json".to_string(),
            entry(&format!("{ns}:about")),
        );
        self.files.insert(
            "data/minecraft/tags/dialog/pause_screen_additions.json".to_string(),
            entry("smithed:data_packs"),
        );
    }

    /// `/function ns:test` runs every `@Test` function and prints a summary.
    /// ponytail: tests must finish in one tick; suspending tests are not awaited.
    fn emit_test_runner(&mut self, program: &IrProgram) {
        let ns = self.namespace.clone();
        let mut lines = vec![
            "scoreboard players set #test_pass mcfc 0".to_string(),
            "scoreboard players set #test_fail mcfc 0".to_string(),
        ];
        for function in &program.functions {
            let Some(test) = function.name.strip_prefix("__mcfc_test_") else {
                continue;
            };
            if function.generated {
                continue;
            }
            lines.push("scoreboard players set #test_failed mcfc 0".to_string());
            lines.push(format!(
                "scoreboard players set {} mcfc 0",
                control_slot(0, &function.name)
            ));
            lines.push(format!(
                "function {ns}:{}",
                self.function_entry_name(&function.name, 0)
            ));
            lines.push("execute if score #test_failed mcfc matches 0 run scoreboard players add #test_pass mcfc 1".to_string());
            lines.push("execute if score #test_failed mcfc matches 1 run scoreboard players add #test_fail mcfc 1".to_string());
            lines.push(format!(
                "execute if score #test_failed mcfc matches 1 run tellraw @a {{\"text\":\"[{ns} TEST] FAIL {test}\",\"color\":\"red\"}}"
            ));
        }
        if lines.len() == 2 {
            return;
        }
        lines.push(format!(
            "tellraw @a [{{\"text\":\"[{ns} TEST] \"}},{{\"score\":{{\"name\":\"#test_pass\",\"objective\":\"mcfc\"}},\"color\":\"green\"}},{{\"text\":\" passed, \"}},{{\"score\":{{\"name\":\"#test_fail\",\"objective\":\"mcfc\"}},\"color\":\"red\"}},{{\"text\":\" failed\"}}]"
        ));
        self.files.insert(
            format!("data/{ns}/function/test.mcfunction"),
            lines.join(
                "
",
            ) + "
",
        );
    }

    fn emit_export_wrappers(&mut self, exports: &[ExportedFunction]) {
        for export in exports {
            let path = export.path.trim_matches('/');
            if path.is_empty() {
                continue;
            }
            let relative = format!("data/{}/function/{}.mcfunction", self.namespace, path);
            let contents = format!(
                "scoreboard players set {} mcfc 0\nfunction {}:{}\n",
                control_slot(0, &export.function),
                self.namespace,
                self.function_entry_name(&export.function, 0)
            );
            self.files.insert(relative, contents);
        }
    }

    fn emit_function_variant(&mut self, function: &IrFunction, depth: usize) {
        let path = self.function_entry_path(&function.name, depth);
        let mut lines = Vec::new();
        if function.name == crate::types::GC_LOCALS {
            lines = self.gc_local_marks(depth);
        }
        let guard = Guard::for_function(depth, &function.name);
        self.emit_stmt_list(
            function,
            depth,
            &function.body,
            &guard,
            None,
            None,
            &[],
            &mut lines,
        );
        self.files.insert(
            path,
            if lines.is_empty() {
                "# empty function\n".to_string()
            } else {
                lines.join("\n") + "\n"
            },
        );
    }

    /// The garbage collector's roots in locals: every object-typed local of
    /// every function, at every depth. Finished calls leave stale values, which
    /// only keep an object alive longer; paused ones need them. A local holding
    /// objects in storage is copied to world state for its scan function.
    fn gc_local_marks(&self, depth: usize) -> Vec<String> {
        let callee_depth = depth + 1;
        if callee_depth > self.max_depth {
            return Vec::new();
        }
        let ns = &self.namespace;
        let call = |callee: &str| {
            [
                format!(
                    "scoreboard players set {} mcfc 0",
                    control_slot(callee_depth, callee)
                ),
                format!(
                    "function {ns}:{}",
                    self.function_entry_name(callee, callee_depth)
                ),
            ]
        };
        let mut lines = Vec::new();
        for (name, info) in &self.functions {
            if name.starts_with("std::heap::") || name.starts_with("__mcfc_gc") {
                continue;
            }
            for (local, ty) in &info.locals {
                let (scratch, scan) = crate::types::gc_scan_names(ty);
                let is_object = matches!(ty, Type::Class(_));
                if !is_object && !self.functions.contains_key(&scan) {
                    continue;
                }
                for local_depth in 0..=self.max_depth {
                    let slot = local_slot(local_depth, name, local, ty);
                    if is_object {
                        lines.push(format!(
                            "scoreboard players operation {} mcfc = {} mcfc",
                            numeric_slot(callee_depth, "std::heap::mark", "id"),
                            slot.numeric_name()
                        ));
                        lines.extend(call("std::heap::mark"));
                    } else {
                        let world = format!("{}{scratch}", crate::types::WORLD_STATE_PREFIX);
                        lines.push(format!(
                            "data modify storage {ns}:runtime {} set from storage {ns}:runtime {}",
                            string_slot(0, "", &world),
                            slot.storage_path()
                        ));
                        lines.extend(call(&scan));
                    }
                }
            }
        }
        for owner in ["players", "entities"] {
            let scan = format!("__mcfc_gc_scan_state_{owner}");
            if self.functions.contains_key(&scan) {
                let world = format!(
                    "{}mcfcScratch_state_{owner}",
                    crate::types::WORLD_STATE_PREFIX
                );
                lines.push(format!(
                    "data modify storage {ns}:runtime {} set from storage {ns}:state {owner}",
                    string_slot(0, "", &world)
                ));
                lines.extend(call(&scan));
            }
        }
        lines
    }

    fn emit_stmt_list(
        &mut self,
        function: &IrFunction,
        depth: usize,
        stmts: &[IrStmt],
        guard: &Guard,
        loop_ctx: Option<&LoopContext>,
        resume_context: Option<&ContextResume>,
        sleep_tail: &[ContinuationItem],
        lines: &mut Vec<String>,
    ) -> bool {
        for (index, stmt) in stmts.iter().enumerate() {
            let tail = continuation_after_stmts(&stmts[index + 1..], sleep_tail);
            if let Some((call, callee)) = suspending_call(stmt, &self.suspending) {
                self.emit_suspending_call(
                    function,
                    depth,
                    stmt,
                    call,
                    callee,
                    &tail,
                    guard,
                    loop_ctx,
                    resume_context,
                    lines,
                );
                return true;
            }
            if let IrStmt::Expr(expr) = stmt
                && let Some(receiver) = sort_receiver(expr)
            {
                self.emit_sort(
                    function,
                    depth,
                    receiver,
                    &tail,
                    guard,
                    loop_ctx,
                    resume_context,
                    lines,
                );
                return true;
            }
            match stmt {
                IrStmt::Let { name, value, .. } => {
                    let mut stmt_lines = Vec::new();
                    self.compile_expr_into_named_slot(
                        function,
                        depth,
                        value,
                        name,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);
                }
                IrStmt::Assign { target, value } => {
                    let mut stmt_lines = Vec::new();
                    match target {
                        IrAssignTarget::Variable(name)
                            if name.starts_with(crate::types::WORLD_STATE_PREFIX)
                                && matches!(value.ty, Type::EntityRef | Type::PlayerRef) =>
                        {
                            self.compile_world_entity_assign(
                                function,
                                depth,
                                name,
                                value,
                                &mut stmt_lines,
                            );
                        }
                        IrAssignTarget::Variable(name) => {
                            self.compile_expr_into_named_slot(
                                function,
                                depth,
                                value,
                                name,
                                &mut stmt_lines,
                            );
                        }
                        IrAssignTarget::Path(path) => {
                            self.compile_path_assign(function, depth, path, value, &mut stmt_lines);
                        }
                    }
                    self.extend_guarded(lines, guard, stmt_lines);
                }
                IrStmt::RawCommand(raw) => lines.push(guard.wrap(expand_display_text_sugar(raw))),
                IrStmt::MacroCommand {
                    template,
                    placeholders,
                } => {
                    let mut stmt_lines = Vec::new();
                    self.emit_macro_command(
                        function,
                        depth,
                        template,
                        placeholders,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);
                }
                IrStmt::Sleep { duration, unit } => {
                    let continuation_name = self.emit_sleep_continuation(
                        function,
                        depth,
                        &tail,
                        guard,
                        loop_ctx,
                        resume_context,
                        Vec::new(),
                    );
                    let duration_name = self.new_temp();
                    let duration_slot =
                        local_slot(depth, &function.name, &duration_name, &Type::Int);
                    let macro_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                    let mut stmt_lines = Vec::new();
                    self.compile_expr_into_slot(
                        function,
                        depth,
                        duration,
                        &duration_slot,
                        &mut stmt_lines,
                    );
                    let duration_key = match unit {
                        SleepUnit::Seconds => "seconds",
                        SleepUnit::Ticks => "ticks",
                    };
                    stmt_lines.push(format!(
                        "execute store result storage {}:runtime {}.{} int 1 run scoreboard players get {} mcfc",
                        self.namespace,
                        macro_slot.storage_path(),
                        duration_key,
                        duration_slot.numeric_name()
                    ));
                    let placeholder = match unit {
                        SleepUnit::Seconds => "$(seconds)s",
                        SleepUnit::Ticks => "$(ticks)t",
                    };
                    stmt_lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        format!(
                            "schedule function {}:{} {}",
                            self.namespace, continuation_name, placeholder
                        ),
                    ));
                    stmt_lines.push(format!(
                        "scoreboard players set {} mcfc 1",
                        susp_slot(depth, &function.name)
                    ));
                    stmt_lines.push(format!("scoreboard players set {} mcfc 1", guard.ctrl_slot));
                    self.extend_guarded(lines, guard, stmt_lines);
                    return true;
                }
                IrStmt::HostCall {
                    module,
                    function: host_fn,
                    args,
                    dest,
                    return_type,
                } => {
                    let continuation_name = self.emit_sleep_continuation(
                        function,
                        depth,
                        &tail,
                        guard,
                        loop_ctx,
                        resume_context,
                        Vec::new(),
                    );
                    let mut stmt_lines = vec![format!(
                        "scoreboard players set {} mcfc 1",
                        susp_slot(depth, &function.name)
                    )];
                    self.emit_host_call_dispatch(
                        function,
                        depth,
                        module,
                        host_fn,
                        args,
                        dest.as_deref(),
                        return_type,
                        &continuation_name,
                        guard,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);
                    return true;
                }
                IrStmt::Context { kind, anchor, body } => {
                    let anchor_name = self.new_temp();
                    let anchor_slot = local_slot(depth, &function.name, &anchor_name, &anchor.ty);
                    let mut stmt_lines = Vec::new();
                    self.compile_expr_into_slot(
                        function,
                        depth,
                        anchor,
                        &anchor_slot,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);

                    let mut context_tail = vec![ContinuationItem::ExitContext];
                    context_tail.extend(tail.clone());
                    let context_resume = ContextResume {
                        kind: *kind,
                        anchor_slot: anchor_slot.clone(),
                    };
                    let (body_path, body_name) = self.new_block(
                        function,
                        depth,
                        &format!("context_{}", context_execute_keyword(*kind)),
                    );
                    let mut body_lines = Vec::new();
                    self.emit_stmt_list(
                        function,
                        depth,
                        body,
                        guard,
                        loop_ctx,
                        Some(&context_resume),
                        &context_tail,
                        &mut body_lines,
                    );
                    self.files.insert(
                        body_path,
                        if body_lines.is_empty() {
                            "# empty context block\n".to_string()
                        } else {
                            body_lines.join("\n") + "\n"
                        },
                    );

                    lines.push(guard.wrap(self.query_command(
                        &anchor_slot,
                        format!(
                            "execute {} $(selector) run function {}:{}",
                            context_execute_keyword(*kind),
                            self.namespace,
                            body_name
                        ),
                        true,
                    )));
                }
                IrStmt::Async {
                    function: async_function,
                    captures,
                } => {
                    let mut stmt_lines = Vec::new();
                    self.emit_async_launch(
                        function,
                        depth,
                        async_function,
                        captures,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);
                }
                IrStmt::Expr(expr) => {
                    let scratch = self.new_temp();
                    let mut stmt_lines = Vec::new();
                    self.compile_expr_into_named_slot(
                        function,
                        depth,
                        expr,
                        &scratch,
                        &mut stmt_lines,
                    );
                    self.extend_guarded(lines, guard, stmt_lines);
                }
                IrStmt::Return(expr) => {
                    let mut stmt_lines = Vec::new();
                    if let Some(expr) = expr {
                        let slot = return_slot(depth, &function.name, &expr.ty);
                        self.compile_expr_into_slot(function, depth, expr, &slot, &mut stmt_lines);
                    }
                    stmt_lines.push(format!(
                        "scoreboard players set {} mcfc 1",
                        control_slot(depth, &function.name)
                    ));
                    self.extend_guarded(lines, guard, stmt_lines);
                    return true;
                }
                IrStmt::Break => {
                    let Some(loop_ctx) = loop_ctx else {
                        continue;
                    };
                    lines.push(guard.wrap(format!(
                        "scoreboard players set {} mcfc 1",
                        loop_ctx.break_slot
                    )));
                    return true;
                }
                IrStmt::Continue => {
                    let Some(loop_ctx) = loop_ctx else {
                        continue;
                    };
                    lines.push(guard.wrap(format!(
                        "scoreboard players set {} mcfc 1",
                        loop_ctx.continue_slot
                    )));
                    return true;
                }
                IrStmt::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    let mut stmt_lines = Vec::new();
                    let clauses =
                        self.compile_condition(function, depth, condition, &mut stmt_lines);
                    // With an else branch, test the condition once: the then
                    // branch may change what it reads.
                    let reads_fresh_temp = matches!(clauses.as_slice(), [single]
                        if single.strip_prefix("if score ")
                            .and_then(|rest| rest.strip_suffix(" mcfc matches 1"))
                            .is_some_and(|slot| slot.contains("___tmp")));
                    let (then_test, else_test) = match clauses.as_slice() {
                        _ if else_body.is_empty() => (clauses.join(" "), None),
                        [single] if reads_fresh_temp => {
                            (single.clone(), Some(negate_clause(single)))
                        }
                        _ => {
                            let slot = numeric_slot(depth, &function.name, &self.new_temp());
                            stmt_lines.push(format!(
                                "execute store success score {slot} mcfc {}",
                                clauses.join(" ")
                            ));
                            (
                                format!("if score {slot} mcfc matches 1"),
                                Some(format!("unless score {slot} mcfc matches 1")),
                            )
                        }
                    };
                    self.extend_guarded(lines, guard, stmt_lines);

                    let (then_path, then_name) = self.new_block(function, depth, "if_then");
                    let mut then_lines = Vec::new();
                    self.emit_stmt_list(
                        function,
                        depth,
                        then_body,
                        guard,
                        loop_ctx,
                        resume_context,
                        &tail,
                        &mut then_lines,
                    );
                    self.files.insert(
                        then_path,
                        if then_lines.is_empty() {
                            "# empty if block\n".to_string()
                        } else {
                            then_lines.join("\n") + "\n"
                        },
                    );
                    lines.push(guard.wrap(format!(
                        "execute {then_test} run function {}:{}",
                        self.namespace, then_name
                    )));

                    if !else_body.is_empty() {
                        let (else_path, else_name) = self.new_block(function, depth, "if_else");
                        let mut else_lines = Vec::new();
                        self.emit_stmt_list(
                            function,
                            depth,
                            else_body,
                            guard,
                            loop_ctx,
                            resume_context,
                            &tail,
                            &mut else_lines,
                        );
                        self.files.insert(
                            else_path,
                            if else_lines.is_empty() {
                                "# empty else block\n".to_string()
                            } else {
                                else_lines.join("\n") + "\n"
                            },
                        );
                        lines.push(guard.wrap(format!(
                            "execute {} run function {}:{}",
                            else_test.as_deref().expect("else branches test one clause"),
                            self.namespace,
                            else_name
                        )));
                    }
                }
                IrStmt::While {
                    condition,
                    body,
                    step,
                } => {
                    let break_slot = numeric_slot(depth, &function.name, &self.new_temp());
                    let continue_slot = numeric_slot(depth, &function.name, &self.new_temp());
                    let (cond_path, cond_name) = self.new_block(function, depth, "while_cond");
                    let (body_path, body_name) = self.new_block(function, depth, "while_body");
                    let step_block =
                        (!step.is_empty()).then(|| self.new_block(function, depth, "while_step"));

                    let loop_ctx = LoopContext {
                        break_slot: break_slot.clone(),
                        continue_slot: continue_slot.clone(),
                        continue_target: step_block
                            .as_ref()
                            .map_or(&cond_name, |(_, name)| name)
                            .clone(),
                    };
                    let loop_guard = guard.within_loop(&loop_ctx);
                    let cond_tail = |target: &str| {
                        let mut items = vec![
                            ContinuationItem::Call {
                                function_name: target.to_string(),
                                allow_continue: true,
                            },
                            ContinuationItem::ClearScore(break_slot.clone()),
                            ContinuationItem::ClearScore(continue_slot.clone()),
                        ];
                        items.extend(tail.clone());
                        items
                    };
                    let body_sleep_tail = cond_tail(&loop_ctx.continue_target);
                    if let Some((step_path, _)) = &step_block {
                        let mut step_lines = vec![loop_guard.wrap_allow_continue(format!(
                            "scoreboard players set {} mcfc 0",
                            continue_slot
                        ))];
                        self.emit_stmt_list(
                            function,
                            depth,
                            step,
                            &loop_guard,
                            None,
                            resume_context,
                            &cond_tail(&cond_name),
                            &mut step_lines,
                        );
                        step_lines.push(loop_guard.wrap_allow_continue(format!(
                            "function {}:{}",
                            self.namespace, cond_name
                        )));
                        self.files
                            .insert(step_path.clone(), step_lines.join("\n") + "\n");
                    }

                    // With a step block, the step clears the continue flag instead.
                    let mut cond_lines = Vec::new();
                    if step_block.is_none() {
                        cond_lines.push(loop_guard.wrap_allow_continue(format!(
                            "scoreboard players set {} mcfc 0",
                            continue_slot
                        )));
                    }
                    let mut cond_eval = Vec::new();
                    let run_body = format!("function {}:{}", self.namespace, body_name);
                    let run_body = if matches!(condition.kind, IrExprKind::Bool(true)) {
                        run_body
                    } else {
                        let clauses =
                            self.compile_condition(function, depth, condition, &mut cond_eval);
                        format!("execute {} run {run_body}", clauses.join(" "))
                    };
                    self.extend_guarded_allow_continue(&mut cond_lines, &loop_guard, cond_eval);
                    cond_lines.push(loop_guard.wrap_allow_continue(run_body));

                    let mut body_lines = Vec::new();
                    self.emit_stmt_list(
                        function,
                        depth,
                        body,
                        &loop_guard,
                        Some(&loop_ctx),
                        resume_context,
                        &body_sleep_tail,
                        &mut body_lines,
                    );
                    body_lines.push(loop_guard.wrap_allow_continue(format!(
                        "function {}:{}",
                        self.namespace, loop_ctx.continue_target
                    )));

                    self.files.insert(cond_path, cond_lines.join("\n") + "\n");
                    self.files.insert(
                        body_path,
                        if body_lines.is_empty() {
                            "# empty while body\n".to_string()
                        } else {
                            body_lines.join("\n") + "\n"
                        },
                    );

                    lines.push(guard.wrap(format!("scoreboard players set {} mcfc 0", break_slot)));
                    lines.push(
                        guard.wrap(format!("scoreboard players set {} mcfc 0", continue_slot)),
                    );
                    lines.push(guard.wrap(format!("function {}:{}", self.namespace, cond_name)));
                }
                IrStmt::For {
                    name,
                    iterable,
                    body,
                } => match &iterable.ty {
                    Type::EntitySet => {
                        let query_name = self.new_temp();
                        let mut init_lines = Vec::new();
                        self.compile_expr_into_named_slot(
                            function,
                            depth,
                            iterable,
                            &query_name,
                            &mut init_lines,
                        );
                        self.extend_guarded(lines, guard, init_lines);

                        let (body_path, body_name) = self.new_block(function, depth, "for_each");
                        let mut body_lines = vec![
                            format!(
                                "data modify storage {}:runtime {}.prefix set value \"\"",
                                self.namespace,
                                string_slot(depth, &function.name, name)
                            ),
                            format!(
                                "data modify storage {}:runtime {}.selector set value \"@s\"",
                                self.namespace,
                                string_slot(depth, &function.name, name)
                            ),
                        ];
                        self.emit_stmt_list(
                            function,
                            depth,
                            body,
                            guard,
                            loop_ctx,
                            resume_context,
                            &tail,
                            &mut body_lines,
                        );
                        self.files.insert(body_path, body_lines.join("\n") + "\n");
                        lines.push(guard.wrap(self.query_command(
                            &local_slot(depth, &function.name, &query_name, &Type::EntitySet),
                            format!(
                                "execute as $(selector) run function {}:{}",
                                self.namespace, body_name
                            ),
                            true,
                        )));
                    }
                    Type::Array(element) => {
                        let snapshot_name = self.new_temp();
                        let index_name = self.new_temp();
                        let len_name = self.new_temp();
                        let break_slot = numeric_slot(depth, &function.name, &self.new_temp());
                        let continue_slot = numeric_slot(depth, &function.name, &self.new_temp());
                        let (cond_path, cond_name) =
                            self.new_block(function, depth, "for_each_cond");
                        let (body_path, body_name) =
                            self.new_block(function, depth, "for_each_body");
                        let (step_path, step_name) =
                            self.new_block(function, depth, "for_each_step");
                        let loop_ctx = LoopContext {
                            break_slot: break_slot.clone(),
                            continue_slot: continue_slot.clone(),
                            continue_target: step_name.clone(),
                        };
                        let loop_guard = guard.within_loop(&loop_ctx);
                        let mut body_sleep_tail = vec![
                            ContinuationItem::Call {
                                function_name: step_name.clone(),
                                allow_continue: true,
                            },
                            ContinuationItem::ClearScore(break_slot.clone()),
                            ContinuationItem::ClearScore(continue_slot.clone()),
                        ];
                        body_sleep_tail.extend(tail.clone());

                        let mut init_lines = Vec::new();
                        self.compile_expr_into_named_slot(
                            function,
                            depth,
                            iterable,
                            &snapshot_name,
                            &mut init_lines,
                        );
                        init_lines.push(format!(
                            "scoreboard players set {} mcfc 0",
                            numeric_slot(depth, &function.name, &index_name)
                        ));
                        init_lines.push(format!(
                            "execute store result score {} mcfc run data get storage {}:runtime {}",
                            numeric_slot(depth, &function.name, &len_name),
                            self.namespace,
                            local_slot(
                                depth,
                                &function.name,
                                &snapshot_name,
                                &Type::Array(element.clone())
                            )
                            .storage_path()
                        ));
                        self.extend_guarded(lines, guard, init_lines);
                        lines.push(
                            guard.wrap(format!("scoreboard players set {} mcfc 0", break_slot)),
                        );
                        lines.push(
                            guard.wrap(format!("scoreboard players set {} mcfc 0", continue_slot)),
                        );

                        let cond_expr = IrExpr {
                            ty: Type::Bool,
                            ref_kind: RefKind::Unknown,
                            kind: IrExprKind::Binary {
                                op: BinaryOp::Lt,
                                left: Box::new(IrExpr {
                                    ty: Type::Int,
                                    ref_kind: RefKind::Unknown,
                                    kind: IrExprKind::Variable(index_name.clone()),
                                }),
                                right: Box::new(IrExpr {
                                    ty: Type::Int,
                                    ref_kind: RefKind::Unknown,
                                    kind: IrExprKind::Variable(len_name.clone()),
                                }),
                            },
                        };
                        let mut cond_lines = Vec::new();
                        let mut cond_eval = Vec::new();
                        let clauses =
                            self.compile_condition(function, depth, &cond_expr, &mut cond_eval);
                        self.extend_guarded(&mut cond_lines, &loop_guard, cond_eval);
                        cond_lines.push(loop_guard.wrap(format!(
                            "execute {} run function {}:{}",
                            clauses.join(" "),
                            self.namespace,
                            body_name
                        )));

                        let mut body_lines = Vec::new();
                        let macro_storage = format!(
                            "frames.d{}.{}.__for_each{}",
                            depth,
                            sanitize(&function.name),
                            self.new_temp()
                        );
                        body_lines.push(format!(
                                "execute store result storage {}:runtime {}.index int 1 run scoreboard players get {} mcfc",
                                self.namespace,
                                macro_storage,
                                numeric_slot(depth, &function.name, &index_name)
                            ));
                        let loop_slot = local_slot(depth, &function.name, name, element.as_ref());
                        let command = match element.as_ref() {
                            Type::Int
                            | Type::Bool
                            | Type::Enum(_)
                            | Type::Class(_)
                            | Type::Generic(..) => format!(
                                "execute store result score {} mcfc run data get storage {}:runtime {}[$(index)] 1",
                                loop_slot.numeric_name(),
                                self.namespace,
                                local_slot(
                                    depth,
                                    &function.name,
                                    &snapshot_name,
                                    &Type::Array(element.clone())
                                )
                                .storage_path()
                            ),
                            _ => format!(
                                "data modify storage {}:runtime {} set from storage {}:runtime {}[$(index)]",
                                self.namespace,
                                loop_slot.storage_path(),
                                self.namespace,
                                local_slot(
                                    depth,
                                    &function.name,
                                    &snapshot_name,
                                    &Type::Array(element.clone())
                                )
                                .storage_path()
                            ),
                        };
                        body_lines.push(self.storage_path_command(command, Some(macro_storage)));
                        self.emit_stmt_list(
                            function,
                            depth,
                            body,
                            &loop_guard,
                            Some(&loop_ctx),
                            resume_context,
                            &body_sleep_tail,
                            &mut body_lines,
                        );
                        body_lines.push(loop_guard.wrap_allow_continue(format!(
                            "function {}:{}",
                            self.namespace, loop_ctx.continue_target
                        )));

                        let mut step_lines = vec![loop_guard.wrap_allow_continue(format!(
                            "scoreboard players set {} mcfc 0",
                            continue_slot
                        ))];
                        step_lines.push(loop_guard.wrap_allow_continue(format!(
                            "scoreboard players add {} mcfc 1",
                            numeric_slot(depth, &function.name, &index_name)
                        )));
                        step_lines.push(loop_guard.wrap_allow_continue(format!(
                            "function {}:{}",
                            self.namespace, cond_name
                        )));

                        self.files.insert(cond_path, cond_lines.join("\n") + "\n");
                        self.files.insert(body_path, body_lines.join("\n") + "\n");
                        self.files.insert(step_path, step_lines.join("\n") + "\n");
                        lines
                            .push(guard.wrap(format!("function {}:{}", self.namespace, cond_name)));
                    }
                    _ => {}
                },
            }
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_sleep_continuation(
        &mut self,
        function: &IrFunction,
        depth: usize,
        continuation: &[ContinuationItem],
        guard: &Guard,
        loop_ctx: Option<&LoopContext>,
        resume_context: Option<&ContextResume>,
        prefix: Vec<String>,
    ) -> String {
        let (path, name) = self.new_block(function, depth, "sleep_resume");
        let mut lines = vec![
            format!("scoreboard players set {} mcfc 0", guard.ctrl_slot),
            format!(
                "scoreboard players set {} mcfc 0",
                susp_slot(depth, &function.name)
            ),
        ];
        // The pause ran under this guard, so the loop's break and continue
        // flags were 0 and still are. Saying so lets the pack optimizer drop
        // their checks from the rest of the loop body.
        for slot in [&guard.break_slot, &guard.continue_slot]
            .into_iter()
            .flatten()
        {
            lines.push(format!("scoreboard players set {slot} mcfc 0"));
        }
        lines.extend(prefix);
        self.emit_contextual_continuation_items(
            function,
            depth,
            continuation,
            guard,
            loop_ctx,
            resume_context,
            &mut lines,
        );
        lines.push(self.finish_line(function, depth));
        self.files.insert(
            path,
            if lines.is_empty() {
                "# empty sleep continuation\n".to_string()
            } else {
                lines.join("\n") + "\n"
            },
        );
        name
    }

    /// Emit the dispatch code for a host call (`module.fn(args)`): allocate a
    /// request id, marshal the request into `mcfc:rpc`, suspend the coroutine, and
    /// schedule a per-site waiter that resumes `continuation` once the result lands
    /// in `mcfc:rpc results.<id>`. Mirrors the `Sleep` lowering but polls instead of
    /// waiting a fixed duration.
    #[allow(clippy::too_many_arguments)]
    fn emit_host_call_dispatch(
        &mut self,
        function: &IrFunction,
        depth: usize,
        module: &str,
        host_fn: &str,
        args: &[IrExpr],
        dest: Option<&str>,
        return_type: &Type,
        continuation: &str,
        guard: &Guard,
        lines: &mut Vec<String>,
    ) {
        let (_, base) = self.new_block(function, depth, "rpc");
        let site = sanitize(&base);
        let deadline_slot = format!("rpc_deadline_{}", site);
        let wait_slot = format!("rpc_wait_{}", site);
        let tick_name = format!("{}_tick", base);
        let check_name = format!("{}_check", base);
        let timeout_name = format!("{}_timeout", base);

        // Build the request envelope at `mcfc:rpc sites.<site>`.
        lines.push(format!(
            "data modify storage mcfc:rpc sites.{} set value {{}}",
            site
        ));
        lines.push(format!(
            "data modify storage mcfc:rpc sites.{}.req set value {{mcpipe:1,protocol:2,pack:{},namespace:{},v:1,mod:{},fn:{},args:[]}}",
            site,
            quoted(&self.namespace),
            quoted(&self.namespace),
            quoted(module),
            quoted(host_fn)
        ));
        lines.push("scoreboard players add rpc_next mcfc 1".to_string());
        lines.push(format!(
            "execute store result storage mcfc:rpc sites.{}.id int 1 run scoreboard players get rpc_next mcfc",
            site
        ));
        lines.push(format!(
            "execute store result storage mcfc:rpc sites.{}.req.id int 1 run scoreboard players get rpc_next mcfc",
            site
        ));

        for arg in args {
            let arg_slot = local_slot(depth, &function.name, &self.new_temp(), &arg.ty);
            self.compile_expr_into_slot(function, depth, arg, &arg_slot, lines);
            if matches!(
                arg.ty,
                Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_)
            ) {
                lines.push(format!(
                    "data modify storage mcfc:rpc sites.{}.req.args append value 0",
                    site
                ));
                lines.push(format!(
                    "execute store result storage mcfc:rpc sites.{}.req.args[-1] int 1 run scoreboard players get {} mcfc",
                    site,
                    arg_slot.numeric_name()
                ));
            } else {
                lines.push(format!(
                    "data modify storage mcfc:rpc sites.{}.req.args append from storage {}:runtime {}",
                    site,
                    self.namespace,
                    arg_slot.storage_path()
                ));
            }
        }

        // Transport: mcfd emits the request through an off-map entity death to
        // latest.log;
        // the mod/agent transports read the shared `out` queue. We populate both.
        if self.helper_backend() == crate::project::HelperBackend::Mcfd {
            // Stage the complete request compound under `emit.req`. The entity's
            // macro writes this serialized SNBT into the CustomName string that
            // Minecraft includes in the logged death message.
            lines.push("data modify storage mcfc:rpc emit set value {}".to_string());
            lines.push(format!(
                "data modify storage mcfc:rpc emit.req set from storage mcfc:rpc sites.{}.req",
                site
            ));
            lines.push(format!("function {}:rpc/emit", self.namespace));
        }
        lines.push(format!(
            "data modify storage mcfc:rpc out append from storage mcfc:rpc sites.{}.req",
            site
        ));

        // Suspend and arm the waiter. The waiter runs from the tick tag (not
        // `schedule`) so it survives the `/reload`s the mcfd transport performs.
        lines.push("scoreboard players add rpc_active mcfc 1".to_string());
        lines.push(format!("scoreboard players set {} mcfc 200", deadline_slot));
        lines.push(format!("scoreboard players set {} mcfc 1", wait_slot));
        lines.push(format!("scoreboard players set {} mcfc 1", guard.ctrl_slot));

        let dest_frame = dest.map(|name| {
            local_slot(depth, &function.name, name, return_type)
                .storage_path()
                .to_string()
        });

        // Waiter clock (registered on the tick tag, active only while waiting):
        // poll the result, else count down to a timeout. Reload-safe.
        let tick_lines = [
            format!(
                "execute unless score {} mcfc matches 1 run return 0",
                wait_slot
            ),
            format!("scoreboard players remove {} mcfc 1", deadline_slot),
            "scoreboard players set rpc_hit mcfc 0".to_string(),
            format!(
                "function {}:{} with storage mcfc:rpc sites.{}",
                self.namespace, check_name, site
            ),
            "execute if score rpc_hit mcfc matches 1 run return 0".to_string(),
            format!(
                "execute if score {} mcfc matches ..0 run function {}:{}",
                deadline_slot, self.namespace, timeout_name
            ),
        ];
        self.files.insert(
            format!("data/{}/function/{}.mcfunction", self.namespace, tick_name),
            tick_lines.join("\n") + "\n",
        );
        self.rpc_tick_functions.push(tick_name.clone());

        // Result applier (macro over `$(id)`): bind the result, GC it, resume.
        let mut check_lines =
            vec!["$execute unless data storage mcfc:rpc results.$(id) run return 0".to_string()];
        if let Some(frame) = &dest_frame {
            check_lines.push(format!(
                "$data modify storage {}:runtime {} set from storage mcfc:rpc results.$(id)",
                self.namespace, frame
            ));
        }
        check_lines.push("$data remove storage mcfc:rpc results.$(id)".to_string());
        check_lines.push(format!("scoreboard players set {} mcfc 0", wait_slot));
        check_lines.push("scoreboard players remove rpc_active mcfc 1".to_string());
        check_lines.push("scoreboard players set rpc_hit mcfc 1".to_string());
        check_lines.push(format!("function {}:{}", self.namespace, continuation));
        self.files.insert(
            format!("data/{}/function/{}.mcfunction", self.namespace, check_name),
            check_lines.join("\n") + "\n",
        );

        // Timeout: resume with a failure result so the world never hangs.
        let mut timeout_lines = Vec::new();
        if let Some(frame) = &dest_frame {
            timeout_lines.push(format!(
                "data modify storage {}:runtime {} set value {{ok:0b,err:\"timeout\"}}",
                self.namespace, frame
            ));
        }
        timeout_lines.push(format!("scoreboard players set {} mcfc 0", wait_slot));
        timeout_lines.push("scoreboard players remove rpc_active mcfc 1".to_string());
        timeout_lines.push("scoreboard players set rpc_hit mcfc 1".to_string());
        timeout_lines.push(format!("function {}:{}", self.namespace, continuation));
        self.files.insert(
            format!(
                "data/{}/function/{}.mcfunction",
                self.namespace, timeout_name
            ),
            timeout_lines.join("\n") + "\n",
        );
    }

    /// Emit the global RPC driver (mcfd transport only): a tick pump that throttles
    /// `/reload` and applies the helper inbox while any request is in flight.
    fn emit_rpc_runtime(&mut self) {
        if self.helper_backend() != crate::project::HelperBackend::Mcfd {
            return;
        }
        let ns = self.namespace.clone();
        self.files.insert(
            format!("data/{}/function/rpc/pump.mcfunction", ns),
            format!(
                "execute if score rpc_active mcfc matches 1.. run function {}:rpc/pump_active\n",
                ns
            ),
        );
        self.files.insert(
            format!("data/{}/function/rpc/pump_active.mcfunction", ns),
            [
                format!("function {}:rpc/inbox", ns),
                "scoreboard players add rpc_reload_timer mcfc 1".to_string(),
                "execute if score rpc_reload_timer mcfc matches 20.. run scoreboard players set rpc_reload_timer mcfc 0".to_string(),
                "execute if score rpc_reload_timer mcfc matches 0 run reload".to_string(),
            ]
            .join("\n")
                + "\n",
        );
        self.files.insert(
            format!("data/{}/function/rpc/inbox.mcfunction", ns),
            "# mcfd writes `data modify storage mcfc:rpc results.<id> set value {...}` lines here\n"
                .to_string(),
        );
        // Vanilla logs named entity deaths reliably. This mirrors VanilLog's
        // approach without changing gamerules or sending request data to chat.
        // The pig exists only for these three commands, 1000 blocks above the
        // executor, so it has no gameplay-visible effect.
        self.files.insert(
            format!("data/{}/function/rpc/emit.mcfunction", ns),
            [
                "summon minecraft:pig ~ ~1000 ~ {Tags:[\"mcfc_rpc_emit\"],Age:-24000,Health:1f,NoAI:1b,Silent:1b}".to_string(),
                format!(
                    "function {}:rpc/emit_name with storage mcfc:rpc emit",
                    self.namespace
                ),
                "damage @e[type=minecraft:pig,tag=mcfc_rpc_emit,sort=nearest,limit=1] 1 minecraft:generic_kill".to_string(),
            ]
            .join("\n")
                + "\n",
        );
        self.files.insert(
            format!("data/{}/function/rpc/emit_name.mcfunction", ns),
            "$data modify entity @e[type=minecraft:pig,tag=mcfc_rpc_emit,sort=nearest,limit=1] CustomName set value '[mcfc_rpc] $(req)'\n".to_string(),
        );
        self.emit_mcfd_descriptor();
    }

    fn emit_mcfd_descriptor(&mut self) {
        self.files
            .insert("mcfd.pack.toml".to_string(), self.render_mcfd_toml());
    }

    /// Render the `mcfd.pack.toml` descriptor the global service discovers.
    /// Capabilities mirror the `[helper.capabilities]` manifest section.
    fn render_mcfd_toml(&self) -> String {
        let capabilities = self.helper.as_ref().map(|config| &config.capabilities);
        let mut out = String::new();
        out.push_str("# Generated by mcfc. Discovered automatically by the global mcfd service.\n");
        out.push_str("protocol = 2\n");
        out.push_str(&format!("pack_id = {}\n", toml_string(&self.namespace)));
        out.push_str(&format!("namespace = {}\n", toml_string(&self.namespace)));
        out.push_str("datapack = \".\"\n");
        out.push_str(
            "# Override only if auto-detection fails (Windows: forward slashes or single quotes):\n",
        );
        out.push_str("# log = 'C:/path/to/.minecraft/logs/latest.log'\n");
        out.push_str("result_ttl_secs = 300\n\n");
        if self.agent_enabled() {
            out.push_str("[agent]\n");
            out.push_str("enabled = true\n\n");
            if let Some(agent) = self
                .helper
                .as_ref()
                .and_then(|config| config.agent.as_ref())
            {
                let mut events = agent.events.iter().cloned().collect::<BTreeSet<_>>();
                events.extend(
                    self.bukkit
                        .agent_handlers
                        .iter()
                        .map(|handler| handler.event.clone()),
                );
                if !events.is_empty() {
                    let events = events
                        .iter()
                        .map(|event| toml_string(event))
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!("events = [{}]\n", events));
                }
                let mut commands = agent.commands.iter().cloned().collect::<BTreeSet<_>>();
                commands.extend(
                    self.bukkit
                        .commands
                        .iter()
                        .map(|command| command.command.clone()),
                );
                if !commands.is_empty() {
                    let commands = commands
                        .iter()
                        .map(|command| toml_string(command))
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!("commands = [{}]\n", commands));
                }
                let deciders = self
                    .bukkit
                    .agent_handlers
                    .iter()
                    .filter(|handler| handler.decision)
                    .map(|handler| handler.event.clone())
                    .collect::<BTreeSet<_>>();
                if !deciders.is_empty() {
                    let events = deciders
                        .iter()
                        .map(|event| toml_string(event))
                        .collect::<Vec<_>>()
                        .join(", ");
                    out.push_str(&format!("deciders = [{}]\n\n", events));
                }
            }
        }
        out.push_str("[capabilities]\n");
        if let Some(capabilities) = capabilities {
            if let Some(http) = &capabilities.http {
                let domains = http
                    .allow_domains
                    .iter()
                    .map(|domain| toml_string(domain))
                    .collect::<Vec<_>>()
                    .join(", ");
                out.push_str(&format!("http = {{ allow_domains = [{}]", domains));
                if let Some(env) = &http.bearer_token_env {
                    out.push_str(&format!(", bearer_token_env = {}", toml_string(env)));
                }
                out.push_str(" }\n");
            }
            if let Some(file) = &capabilities.file {
                out.push_str(&format!(
                    "file = {{ root = {} }}\n",
                    toml_string(&file.root)
                ));
            }
            if let Some(kv) = &capabilities.kv {
                out.push_str(&format!("kv = {{ root = {} }}\n", toml_string(&kv.root)));
            }
            if let Some(db) = &capabilities.db {
                out.push_str(&format!("db = {{ path = {} }}\n", toml_string(&db.path)));
            }
            if capabilities.time {
                out.push_str("time = true\n");
            }
            if capabilities.rand {
                out.push_str("rand = true\n");
            }
        }
        out
    }

    fn emit_continuation_items(
        &mut self,
        function: &IrFunction,
        depth: usize,
        items: &[ContinuationItem],
        guard: &Guard,
        loop_ctx: Option<&LoopContext>,
        resume_context: Option<&ContextResume>,
        lines: &mut Vec<String>,
    ) -> bool {
        for (index, item) in items.iter().enumerate() {
            match item {
                ContinuationItem::Stmt(stmt) => {
                    if self.emit_stmt_list(
                        function,
                        depth,
                        std::slice::from_ref(stmt),
                        guard,
                        loop_ctx,
                        resume_context,
                        &items[index + 1..],
                        lines,
                    ) {
                        return true;
                    }
                }
                ContinuationItem::Call {
                    function_name,
                    allow_continue,
                } => {
                    let command = format!("function {}:{}", self.namespace, function_name);
                    lines.push(if *allow_continue {
                        guard.wrap_allow_continue(command)
                    } else {
                        guard.wrap(command)
                    });
                }
                ContinuationItem::ClearScore(slot) => {
                    lines.push(guard.wrap_with_options(
                        format!("scoreboard players set {} mcfc 0", slot),
                        false,
                        false,
                    ));
                }
                ContinuationItem::ExitContext => return false,
            }
        }
        false
    }

    fn emit_contextual_continuation_items(
        &mut self,
        function: &IrFunction,
        depth: usize,
        items: &[ContinuationItem],
        guard: &Guard,
        loop_ctx: Option<&LoopContext>,
        resume_context: Option<&ContextResume>,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(context) = resume_context else {
            return self
                .emit_continuation_items(function, depth, items, guard, loop_ctx, None, lines);
        };
        let split = items
            .iter()
            .position(|item| matches!(item, ContinuationItem::ExitContext))
            .unwrap_or(items.len());
        let inside = &items[..split];
        let outside = if split < items.len() {
            &items[split + 1..]
        } else {
            &[]
        };

        if !inside.is_empty() {
            let (path, name) = self.new_block(function, depth, "sleep_context");
            let mut inner_lines = Vec::new();
            self.emit_continuation_items(
                function,
                depth,
                inside,
                guard,
                loop_ctx,
                resume_context,
                &mut inner_lines,
            );
            self.files.insert(
                path,
                if inner_lines.is_empty() {
                    "# empty sleep context continuation\n".to_string()
                } else {
                    inner_lines.join("\n") + "\n"
                },
            );
            lines.push(guard.wrap(self.query_command(
                &context.anchor_slot,
                format!(
                    "execute {} $(selector) run function {}:{}",
                    context_execute_keyword(context.kind),
                    self.namespace,
                    name
                ),
                true,
            )));
        }

        self.emit_continuation_items(function, depth, outside, guard, loop_ctx, None, lines)
    }

    fn emit_async_launch(
        &mut self,
        parent: &IrFunction,
        parent_depth: usize,
        async_function: &IrFunction,
        captures: &[IrCapture],
        lines: &mut Vec<String>,
    ) {
        for capture in captures {
            let source = local_slot(parent_depth, &parent.name, &capture.name, &capture.ty);
            let target = local_slot(0, &async_function.name, &capture.name, &capture.ty);
            match capture.ty {
                Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => {
                    lines.push(format!(
                        "scoreboard players operation {} mcfc = {} mcfc",
                        target.numeric_name(),
                        source.numeric_name()
                    ))
                }
                Type::Void => {}
                _ => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    source.storage_path()
                )),
            }
        }
        lines.push(format!(
            "scoreboard players set {} mcfc 0",
            control_slot(0, &async_function.name)
        ));
        lines.push(format!(
            "function {}:{}",
            self.namespace,
            self.function_entry_name(&async_function.name, 0)
        ));
    }

    fn extend_guarded(&self, target: &mut Vec<String>, guard: &Guard, lines: Vec<String>) {
        target.extend(lines.into_iter().map(|line| guard.wrap(line)));
    }

    fn extend_guarded_allow_continue(
        &self,
        target: &mut Vec<String>,
        guard: &Guard,
        lines: Vec<String>,
    ) {
        target.extend(
            lines
                .into_iter()
                .map(|line| guard.wrap_allow_continue(line)),
        );
    }

    fn compile_expr_into_named_slot(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        name: &str,
        lines: &mut Vec<String>,
    ) {
        let slot = local_slot(depth, &function.name, name, &expr.ty);
        self.compile_expr_into_slot(function, depth, expr, &slot, lines);
    }

    fn compile_expr_into_slot(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if is_float_op(expr) {
            let provider = self.float_provider(function, depth, expr, lines);
            lines.push(format!(
                "data modify storage {}:runtime {} set compute default float {}",
                self.namespace,
                target.storage_path(),
                provider
            ));
            return;
        }
        match &expr.kind {
            IrExprKind::Int(value) => lines.push(format!(
                "scoreboard players set {} mcfc {}",
                target.numeric_name(),
                value
            )),
            IrExprKind::Float(value) => lines.push(format!(
                "data modify storage {}:runtime {} set value {}f",
                self.namespace,
                target.storage_path(),
                value
            )),
            IrExprKind::Bool(value) => lines.push(format!(
                "scoreboard players set {} mcfc {}",
                target.numeric_name(),
                if *value { 1 } else { 0 }
            )),
            IrExprKind::String(value) => lines.push(format!(
                "data modify storage {}:runtime {} set value {}",
                self.namespace,
                target.storage_path(),
                quoted(value)
            )),
            IrExprKind::InterpolatedString {
                template,
                placeholders,
            } => self.compile_interpolated_string(
                function,
                depth,
                template,
                placeholders,
                target,
                lines,
            ),
            IrExprKind::ArrayLiteral(values) => {
                self.compile_array_literal(function, depth, values, target, lines);
            }
            IrExprKind::DictLiteral(entries) => {
                self.compile_dict_literal(function, depth, entries, target, lines);
            }
            IrExprKind::StructLiteral { fields, .. } => {
                self.compile_struct_literal(function, depth, fields, target, lines);
            }
            IrExprKind::Selector(value) => self.write_query_slot(target, "", value, lines),
            IrExprKind::Block(value) => self.write_block_slot(target, "", value, lines),
            IrExprKind::Variable(name) => match expr.ty {
                Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => {
                    lines.push(format!(
                        "scoreboard players operation {} mcfc = {} mcfc",
                        target.numeric_name(),
                        numeric_slot(depth, &function.name, name)
                    ))
                }
                Type::String
                | Type::Float
                | Type::Array(_)
                | Type::Dict(_)
                | Type::Optional(_)
                | Type::Struct(_)
                | Type::EntityDef
                | Type::BlockDef
                | Type::ItemDef
                | Type::TextDef
                | Type::ItemSlot
                | Type::Bossbar => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    string_slot(depth, &function.name, name)
                )),
                Type::EntitySet
                | Type::EntityRef
                | Type::PlayerRef
                | Type::BlockRef
                | Type::Nbt => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    string_slot(depth, &function.name, name)
                )),
                Type::Void => {}
            },
            IrExprKind::Single(expr) => {
                self.compile_expr_into_slot(function, depth, expr, target, lines);
            }
            IrExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                let condition_temp = self.new_temp();
                let condition_slot =
                    local_slot(depth, &function.name, &condition_temp, &Type::Bool);
                self.compile_expr_into_slot(function, depth, condition, &condition_slot, lines);
                for (branch, matches, label) in
                    [(then_expr, 1, "cond_then"), (else_expr, 0, "cond_else")]
                {
                    let (path, name) = self.new_block(function, depth, label);
                    let mut branch_lines = Vec::new();
                    self.compile_expr_into_slot(function, depth, branch, target, &mut branch_lines);
                    self.files.insert(path, branch_lines.join("\n") + "\n");
                    lines.push(format!(
                        "execute if score {} mcfc matches {matches} run function {}:{}",
                        condition_slot.numeric_name(),
                        self.namespace,
                        name
                    ));
                }
            }
            IrExprKind::Bind { name, value, body } => {
                let slot = local_slot(depth, &function.name, name, &value.ty);
                self.compile_expr_into_slot(function, depth, value, &slot, lines);
                self.compile_expr_into_slot(function, depth, body, target, lines);
            }
            IrExprKind::At { anchor, value } => {
                let anchor_name = self.new_temp();
                let value_name = self.new_temp();
                let anchor_slot = local_slot(depth, &function.name, &anchor_name, &anchor.ty);
                let value_slot = local_slot(depth, &function.name, &value_name, &value.ty);
                self.compile_expr_into_slot(function, depth, anchor, &anchor_slot, lines);
                self.compile_expr_into_slot(function, depth, value, &value_slot, lines);
                self.compose_context_slots(
                    ContextKind::At,
                    &anchor_slot,
                    &value_slot,
                    target,
                    &value.ty,
                    lines,
                );
            }
            IrExprKind::As { anchor, value } => {
                let anchor_name = self.new_temp();
                let value_name = self.new_temp();
                let anchor_slot = local_slot(depth, &function.name, &anchor_name, &anchor.ty);
                let value_slot = local_slot(depth, &function.name, &value_name, &value.ty);
                self.compile_expr_into_slot(function, depth, anchor, &anchor_slot, lines);
                self.compile_expr_into_slot(function, depth, value, &value_slot, lines);
                self.compose_context_slots(
                    ContextKind::As,
                    &anchor_slot,
                    &value_slot,
                    target,
                    &value.ty,
                    lines,
                );
            }
            IrExprKind::Exists(expr) => {
                let temp = self.new_temp();
                let source_slot = local_slot(depth, &function.name, &temp, &expr.ty);
                self.compile_expr_into_slot(function, depth, expr, &source_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                lines.push(self.query_command(
                    &source_slot,
                    format!(
                        "execute if entity $(selector) run scoreboard players set {} mcfc 1",
                        target.numeric_name()
                    ),
                    true,
                ));
            }
            IrExprKind::HasData(expr) => {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                if let Some(rendered) =
                    self.render_storage_expr_lvalue_path(function, depth, expr, lines)
                {
                    lines.push(self.storage_path_command(
                        format!(
                            "execute if data storage {}:runtime {} run scoreboard players set {} mcfc 1",
                            self.namespace,
                            rendered.path,
                            target.numeric_name()
                        ),
                        rendered.macro_storage,
                    ));
                }
            }
            IrExprKind::Path(path) => {
                self.compile_path_read(function, depth, path, target, lines);
            }
            IrExprKind::Cast { kind, expr } => {
                self.compile_cast(function, depth, *kind, expr, target, lines);
            }
            IrExprKind::Unary { op, expr } => {
                self.compile_unary(function, depth, *op, expr, target, lines);
            }
            IrExprKind::Binary { op, left, right } => {
                self.compile_binary(function, depth, *op, left, right, target, lines);
            }
            IrExprKind::Call {
                function: callee,
                args,
            } => {
                if self.compile_builtin_call(function, depth, callee, args, target, lines) {
                    return;
                }
                // A call inside a recursion group reuses this depth's frames,
                // so the caller saves its own frame around it.
                let recursive = self.recursion_groups.get(&function.name).is_some()
                    && self.recursion_groups.get(&function.name)
                        == self.recursion_groups.get(callee);
                let callee_depth = if recursive { depth } else { depth + 1 };
                if let Some(info) = self.functions.get(callee).cloned() {
                    let mut call = Vec::new();
                    let mut temps = Vec::new();
                    // `f(x, f(y))`: the inner call writes the same param slots and
                    // control flag, so evaluate every argument before touching them.
                    let nested = args
                        .iter()
                        .any(|arg| expr_calls_any(arg, &BTreeSet::from([callee.clone()])));
                    if recursive || nested {
                        // The callee's params may be this frame's own slots.
                        for ((_, param_ty), arg) in info.params.iter().zip(args.iter()) {
                            let name = self.new_temp();
                            let temp = local_slot(depth, &function.name, &name, param_ty);
                            self.compile_expr_into_slot(function, depth, arg, &temp, lines);
                            temps.push(temp);
                        }
                    }
                    if recursive {
                        call.push(self.frame_call("save", depth, &function.name));
                    }
                    for (index, ((param_name, param_ty), arg)) in
                        info.params.iter().zip(args.iter()).enumerate()
                    {
                        let param = local_slot(callee_depth, callee, param_name, param_ty);
                        match temps.get(index) {
                            Some(temp) => call.push(self.copy_slot(param_ty, temp, &param)),
                            None => {
                                self.compile_expr_into_slot(function, depth, arg, &param, &mut call)
                            }
                        }
                    }
                    // After the arguments: a call inside them sets this flag when it returns.
                    call.push(format!(
                        "scoreboard players set {} mcfc 0",
                        control_slot(callee_depth, callee)
                    ));
                    call.push(format!(
                        "function {}:{}",
                        self.namespace,
                        self.function_entry_name(callee, callee_depth)
                    ));
                    let ret = SlotRef {
                        name: if is_score_type(&expr.ty) {
                            numeric_return_slot(callee_depth, callee)
                        } else {
                            string_return_slot(callee_depth, callee)
                        },
                    };
                    let has_value = expr.ty != Type::Void;
                    if !recursive {
                        if has_value {
                            call.push(self.copy_slot(&expr.ty, &ret, target));
                        }
                        lines.extend(call);
                        return;
                    }
                    // The callee shares this frame's control flag, so the call
                    // runs in its own function where no guard reads the flag
                    // before the restore puts it back. The value waits in a
                    // scratch slot because the restore rewrites this frame.
                    let scratch = SlotRef {
                        name: if is_score_type(&expr.ty) {
                            "$rec_ret"
                        } else {
                            "rec_ret"
                        }
                        .to_string(),
                    };
                    if has_value {
                        call.push(self.copy_slot(&expr.ty, &ret, &scratch));
                    }
                    call.push(self.frame_call("restore", depth, &function.name));
                    let (path, relative) = self.new_block(function, depth, "call");
                    self.files.insert(
                        path,
                        call.join(
                            "
",
                        ) + "
",
                    );
                    lines.push(format!("function {}:{relative}", self.namespace));
                    if has_value {
                        lines.push(self.copy_slot(&expr.ty, &scratch, target));
                    }
                }
            }
            IrExprKind::MethodCall {
                receiver,
                method,
                args,
            } => {
                self.compile_method_call(function, depth, receiver, method, args, target, lines);
            }
        }
    }

    fn compile_path_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        value: &IrExpr,
        lines: &mut Vec<String>,
    ) {
        let base_name = self.new_temp();
        let base_slot = local_slot(depth, &function.name, &base_name, &path.base.ty);
        self.compile_expr_into_slot(function, depth, &path.base, &base_slot, lines);

        let value_name = self.new_temp();
        let value_slot = local_slot(depth, &function.name, &value_name, &Type::Nbt);
        self.compile_value_as_nbt(function, depth, value, &value_slot, lines);

        if path.base.ty == Type::Bossbar {
            self.compile_bossbar_property_assign(function, depth, &base_slot, path, value, lines);
            return;
        }

        if path.base.ty == Type::ItemSlot {
            self.compile_item_slot_path_assign(
                function,
                depth,
                &base_slot,
                path,
                &value_slot,
                lines,
            );
            return;
        }

        if matches!(
            path.base.ty,
            Type::Array(_)
                | Type::Dict(_)
                | Type::Optional(_)
                | Type::Struct(_)
                | Type::EntityDef
                | Type::ItemDef
                | Type::TextDef
                | Type::BlockDef
                | Type::Nbt
        ) {
            if self.try_compile_storage_index_assign(function, depth, path, &value_slot, lines) {
                return;
            }
            if let Some(rendered) = self.render_storage_lvalue_path(function, depth, path, lines) {
                lines.push(self.storage_path_command(
                    format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}",
                        self.namespace,
                        rendered.path,
                        self.namespace,
                        value_slot.storage_path()
                    ),
                    rendered.macro_storage,
                ));
            }
            return;
        }

        if matches!(path.base.ty, Type::EntityRef | Type::PlayerRef) {
            if let Some(PathSegment::Field(first)) = path.segments.first()
                && first == "position"
                && path.segments.len() > 1
            {
                let pos_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::BlockRef);
                self.compose_entity_position_slot(&base_slot, &pos_slot, lines);
                let path_text = render_nbt_path_segments(normalize_runtime_nbt_segments(
                    &Type::BlockRef,
                    &path.segments[1..],
                ));
                let storage_target =
                    format!("{}:runtime {}", self.namespace, value_slot.storage_path());
                lines.push(self.block_command(
                    &pos_slot,
                    format!(
                        "data modify block $(pos) {} set from storage {}",
                        path_text, storage_target
                    ),
                    true,
                ));
                return;
            }
            if self.try_compile_player_path_assign(
                function,
                depth,
                &base_slot,
                path,
                value,
                &value_slot,
                lines,
            ) {
                return;
            }
        }

        let path_text = render_nbt_path_segments(normalize_runtime_nbt_segments(
            &path.base.ty,
            &path.segments,
        ));
        let storage_target = format!("{}:runtime {}", self.namespace, value_slot.storage_path());
        match path.base.ty {
            Type::EntityRef | Type::PlayerRef => lines.push(self.query_command(
                &base_slot,
                format!(
                    "data modify entity $(selector) {} set from storage {}",
                    path_text, storage_target
                ),
                true,
            )),
            Type::BlockRef => lines.push(self.block_command(
                &base_slot,
                format!(
                    "data modify block $(pos) {} set from storage {}",
                    path_text, storage_target
                ),
                true,
            )),
            _ => {}
        }
    }

    fn compile_bossbar_property_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        value: &IrExpr,
        lines: &mut Vec<String>,
    ) {
        let [PathSegment::Field(field)] = path.segments.as_slice() else {
            return;
        };
        match field.as_str() {
            "name" => {
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                if let IrExprKind::String(text) = &value.kind {
                    let component = selector_text_components(text).unwrap_or_else(|| quoted(text));
                    lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        format!("bossbar set $(id) name {}", component),
                    ));
                    return;
                }
                if value.ty == Type::TextDef {
                    let name_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::TextDef);
                    self.compile_expr_into_slot(function, depth, value, &name_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
                        self.namespace,
                        macro_slot.storage_path(),
                        self.namespace,
                        name_slot.storage_path()
                    ));
                    lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        "bossbar set $(id) name $(name)".to_string(),
                    ));
                    return;
                }
                let name_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, value, &name_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    name_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    "bossbar set $(id) name [\"$(name)\"]".to_string(),
                ));
            }
            "value" | "max" => {
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let value_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                self.compile_expr_into_slot(function, depth, value, &value_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                lines.push(format!(
                    "execute store result storage {}:runtime {}.value int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    macro_slot.storage_path(),
                    value_slot.numeric_name()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    format!("bossbar set $(id) {} $(value)", field),
                ));
            }
            "visible" => {
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let visible_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Bool);
                self.compile_expr_into_slot(function, depth, value, &visible_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                lines.push(format!(
                    "execute store result storage {}:runtime {}.visible int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    macro_slot.storage_path(),
                    visible_slot.numeric_name()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    "bossbar set $(id) visible $(visible)".to_string(),
                ));
            }
            "players" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &value.ty);
                self.compile_expr_into_slot(function, depth, value, &target_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "bossbar set $(id) players $(selector)".to_string(),
                    true,
                ));
            }
            _ => {}
        }
    }

    fn compile_path_read(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if let Some(first_string_index) = self.first_string_index_segment(path) {
            self.compile_string_indexed_path_read(
                function,
                depth,
                path,
                first_string_index,
                target,
                lines,
            );
            return;
        }

        let base_name = self.new_temp();
        let base_slot = local_slot(depth, &function.name, &base_name, &path.base.ty);
        self.compile_expr_into_slot(function, depth, &path.base, &base_slot, lines);

        if path.base.ty == Type::ItemSlot {
            self.compile_item_slot_path_read(function, depth, &base_slot, path, target, lines);
            return;
        }

        if matches!(
            path.base.ty,
            Type::Array(_)
                | Type::Dict(_)
                | Type::Optional(_)
                | Type::Struct(_)
                | Type::EntityDef
                | Type::BlockDef
                | Type::TextDef
                | Type::Nbt
        ) {
            // Component fields are optional; a missing one reads as missing,
            // not as whatever the slot held from the last call.
            if path.base.ty == Type::TextDef && path.ty == Type::Nbt {
                lines.push(format!(
                    "data remove storage {}:runtime {}",
                    self.namespace,
                    target.storage_path()
                ));
            }
            let path_text = self.render_storage_read_path(function, depth, path, &base_slot, lines);
            self.compile_storage_read_from_path(path_text, &path.ty, target, lines);
            return;
        }

        if matches!(path.base.ty, Type::EntityRef | Type::PlayerRef)
            && self.try_compile_player_path_read(function, depth, &base_slot, path, target, lines)
        {
            return;
        }

        let path_text = render_nbt_path_segments(normalize_runtime_nbt_segments(
            &path.base.ty,
            &path.segments,
        ));
        match path.base.ty {
            Type::EntityRef | Type::PlayerRef => lines.push(self.query_command(
                &base_slot,
                format!(
                    "data modify storage {}:runtime {} set from entity $(selector) {}",
                    self.namespace,
                    target.storage_path(),
                    path_text
                ),
                true,
            )),
            Type::BlockRef => lines.push(self.block_command(
                &base_slot,
                format!(
                    "data modify storage {}:runtime {} set from block $(pos) {}",
                    self.namespace,
                    target.storage_path(),
                    path_text
                ),
                true,
            )),
            _ => {}
        }
    }

    fn first_string_index_segment(&self, path: &IrPathExpr) -> Option<usize> {
        let mut current_ty = path.base.ty.clone();
        for (index, segment) in path.segments.iter().enumerate() {
            if current_ty == Type::String && matches!(segment, PathSegment::Index(_)) {
                return Some(index);
            }
            current_ty = path.segment_types.get(index).cloned().unwrap_or(current_ty);
        }
        None
    }

    fn compile_string_indexed_path_read(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        first_string_index: usize,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let source_name = self.new_temp();
        let mut source_slot = local_slot(depth, &function.name, &source_name, &Type::String);

        if first_string_index == 0 {
            self.compile_expr_into_slot(function, depth, &path.base, &source_slot, lines);
        } else {
            let prefix_path = IrPathExpr {
                base: path.base.clone(),
                segments: path.segments[..first_string_index].to_vec(),
                segment_types: path.segment_types[..first_string_index].to_vec(),
                ty: path.segment_types[first_string_index - 1].clone(),
            };
            self.compile_path_read(function, depth, &prefix_path, &source_slot, lines);
        }

        for segment_index in first_string_index..path.segments.len() {
            let PathSegment::Index(index) = &path.segments[segment_index] else {
                return;
            };
            let is_last = segment_index + 1 == path.segments.len();
            if is_last {
                self.compile_string_character_read(
                    function,
                    depth,
                    &source_slot,
                    index,
                    target,
                    lines,
                );
            } else {
                let next_name = self.new_temp();
                let next_slot = local_slot(depth, &function.name, &next_name, &Type::String);
                self.compile_string_character_read(
                    function,
                    depth,
                    &source_slot,
                    index,
                    &next_slot,
                    lines,
                );
                source_slot = next_slot;
            }
        }
    }

    fn compile_string_character_read(
        &mut self,
        function: &IrFunction,
        depth: usize,
        source: &SlotRef,
        index: &crate::ast::Expr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if let crate::ast::ExprKind::Int(value) = &index.kind
            && *value >= 0
            && let Some(end) = value.checked_add(1)
        {
            lines.push(format!(
                "data modify storage {}:runtime {} set string storage {}:runtime {} {} {}",
                self.namespace,
                target.storage_path(),
                self.namespace,
                source.storage_path(),
                value,
                end
            ));
            return;
        }

        let index_expr = self.lower_macro_path_expr(function, depth, index, &Type::Int);
        let index_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
        let start_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
        let end_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
        let len_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
        let macro_storage = format!(
            "frames.d{}.{}.__str_index{}",
            depth,
            sanitize(&function.name),
            self.new_temp()
        );

        self.compile_expr_into_slot(function, depth, &index_expr, &index_slot, lines);
        lines.push(format!(
            "scoreboard players operation {} mcfc = {} mcfc",
            start_slot.numeric_name(),
            index_slot.numeric_name()
        ));
        lines.push(format!(
            "execute store result score {} mcfc run data get storage {}:runtime {}",
            len_slot.numeric_name(),
            self.namespace,
            source.storage_path()
        ));
        lines.push(format!(
            "execute if score {} mcfc matches ..-1 run scoreboard players operation {} mcfc += {} mcfc",
            start_slot.numeric_name(),
            start_slot.numeric_name(),
            len_slot.numeric_name()
        ));
        lines.push(format!(
            "scoreboard players operation {} mcfc = {} mcfc",
            end_slot.numeric_name(),
            start_slot.numeric_name()
        ));
        lines.push(format!(
            "scoreboard players add {} mcfc 1",
            end_slot.numeric_name()
        ));
        lines.push(format!(
            "execute store result storage {}:runtime {}.start int 1 run scoreboard players get {} mcfc",
            self.namespace,
            macro_storage,
            start_slot.numeric_name()
        ));
        lines.push(format!(
            "execute store result storage {}:runtime {}.end int 1 run scoreboard players get {} mcfc",
            self.namespace,
            macro_storage,
            end_slot.numeric_name()
        ));
        lines.push(self.inline_macro_command(
            &macro_storage,
            format!(
                "data modify storage {}:runtime {} set string storage {}:runtime {} $(start) $(end)",
                self.namespace,
                target.storage_path(),
                self.namespace,
                source.storage_path()
            ),
        ));
    }

    fn compile_array_literal(
        &mut self,
        function: &IrFunction,
        depth: usize,
        values: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {} set value []",
            self.namespace,
            target.storage_path()
        ));
        for value in values {
            let temp = self.new_temp();
            let temp_slot = local_slot(depth, &function.name, &temp, &Type::Nbt);
            self.compile_value_as_nbt(function, depth, value, &temp_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {} append from storage {}:runtime {}",
                self.namespace,
                target.storage_path(),
                self.namespace,
                temp_slot.storage_path()
            ));
        }
    }

    fn compile_dict_literal(
        &mut self,
        function: &IrFunction,
        depth: usize,
        entries: &[(String, IrExpr)],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {} set value {{}}",
            self.namespace,
            target.storage_path()
        ));
        for (key, value) in entries {
            let temp = self.new_temp();
            let temp_slot = local_slot(depth, &function.name, &temp, &Type::Nbt);
            self.compile_value_as_nbt(function, depth, value, &temp_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.{} set from storage {}:runtime {}",
                self.namespace,
                target.storage_path(),
                key,
                self.namespace,
                temp_slot.storage_path()
            ));
        }
    }

    fn compile_struct_literal(
        &mut self,
        function: &IrFunction,
        depth: usize,
        fields: &[(String, IrExpr)],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {} set value {{}}",
            self.namespace,
            target.storage_path()
        ));
        for (field, value) in fields {
            let temp = self.new_temp();
            let temp_slot = local_slot(depth, &function.name, &temp, &Type::Nbt);
            self.compile_value_as_nbt(function, depth, value, &temp_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.{} set from storage {}:runtime {}",
                self.namespace,
                target.storage_path(),
                field,
                self.namespace,
                temp_slot.storage_path()
            ));
        }
    }

    fn compile_block_def_spec_string(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let spec_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::BlockDef);
        self.compile_expr_into_slot(function, depth, expr, &spec_slot, lines);
        let state_fields = self.known_block_builder_state_fields(function, expr);
        if state_fields.is_empty() {
            lines.push(format!(
                "data modify storage {}:runtime {} set from storage {}:runtime {}.id",
                self.namespace,
                target.storage_path(),
                self.namespace,
                spec_slot.storage_path()
            ));
            return;
        }

        let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        lines.push(format!(
            "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            spec_slot.storage_path()
        ));
        let mut rendered_states = Vec::new();
        for (index, field) in state_fields.iter().enumerate() {
            let placeholder = format!("s{}", index + 1);
            lines.push(format!(
                "data modify storage {}:runtime {}.{} set from storage {}:runtime {}.states.{}",
                self.namespace,
                macro_slot.storage_path(),
                placeholder,
                self.namespace,
                spec_slot.storage_path(),
                field
            ));
            rendered_states.push(format!("{}=$({})", field, placeholder));
        }
        let template = format!("$(id)[{}]", rendered_states.join(","));
        lines.push(self.inline_macro_command(
            macro_slot.storage_path(),
            format!(
                "data modify storage {}:runtime {} set value {}",
                self.namespace,
                target.storage_path(),
                quoted(&template)
            ),
        ));
    }

    fn known_block_builder_state_fields(
        &self,
        function: &IrFunction,
        expr: &IrExpr,
    ) -> Vec<String> {
        let name = match &expr.kind {
            IrExprKind::Variable(name) => Some(name.as_str()),
            IrExprKind::Path(path) => match &path.base.kind {
                IrExprKind::Variable(name) if path.base.ty == Type::BlockDef => Some(name.as_str()),
                _ => None,
            },
            _ => None,
        };
        name.and_then(|name| {
            self.block_builder_state_fields
                .get(&function.name)
                .and_then(|fields| fields.get(name))
                .cloned()
        })
        .unwrap_or_default()
    }

    fn render_storage_read_path(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        base_slot: &SlotRef,
        lines: &mut Vec<String>,
    ) -> RenderedStoragePath {
        self.render_storage_path(
            function,
            depth,
            base_slot.storage_path().to_string(),
            &path.base.ty,
            &path.segments,
            &path.segment_types,
            lines,
        )
    }

    fn render_storage_lvalue_path(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        lines: &mut Vec<String>,
    ) -> Option<RenderedStoragePath> {
        let IrExprKind::Variable(name) = &path.base.kind else {
            return None;
        };
        let root = string_slot(depth, &function.name, name);
        Some(self.render_storage_path(
            function,
            depth,
            root,
            &path.base.ty,
            &path.segments,
            &path.segment_types,
            lines,
        ))
    }

    fn render_storage_path(
        &mut self,
        function: &IrFunction,
        depth: usize,
        root: String,
        root_ty: &Type,
        segments: &[PathSegment],
        segment_types: &[Type],
        lines: &mut Vec<String>,
    ) -> RenderedStoragePath {
        let mut rendered = root;
        let mut current_ty = root_ty.clone();
        let mut macro_storage = None;
        let mut placeholder_index = 0usize;

        for (segment, next_ty) in segments.iter().zip(segment_types.iter()) {
            match (&current_ty, segment) {
                (Type::EntityDef, PathSegment::Field(field))
                | (Type::BlockDef, PathSegment::Field(field))
                | (Type::ItemDef, PathSegment::Field(field))
                | (Type::ItemSlot, PathSegment::Field(field)) => {
                    rendered.push('.');
                    rendered.push_str(field);
                    current_ty = next_ty.clone();
                }
                (Type::Array(element), PathSegment::Index(index)) => {
                    if let crate::ast::ExprKind::Int(value) = &index.kind {
                        rendered.push_str(&format!("[{}]", value));
                    } else {
                        placeholder_index += 1;
                        let name = format!("i{}", placeholder_index);
                        let storage = macro_storage.get_or_insert_with(|| {
                            format!(
                                "frames.d{}.{}.__path{}",
                                depth,
                                sanitize(&function.name),
                                self.new_temp()
                            )
                        });
                        self.compile_expr_to_macro_value(
                            function,
                            depth,
                            index,
                            &Type::Int,
                            storage,
                            &name,
                            lines,
                        );
                        rendered.push_str(&format!("[$({})]", name));
                    }
                    current_ty = *element.clone();
                }
                (Type::Dict(value), PathSegment::Index(index)) => {
                    if let crate::ast::ExprKind::String(key) = &index.kind {
                        rendered.push('.');
                        rendered.push_str(key);
                    } else {
                        placeholder_index += 1;
                        let name = format!("k{}", placeholder_index);
                        let storage = macro_storage.get_or_insert_with(|| {
                            format!(
                                "frames.d{}.{}.__path{}",
                                depth,
                                sanitize(&function.name),
                                self.new_temp()
                            )
                        });
                        self.compile_expr_to_macro_value(
                            function,
                            depth,
                            index,
                            &Type::String,
                            storage,
                            &name,
                            lines,
                        );
                        rendered.push_str(&format!(".$({})", name));
                    }
                    current_ty = *value.clone();
                }
                (_, PathSegment::Field(field)) => {
                    if !rendered.is_empty() {
                        rendered.push('.');
                    }
                    rendered.push_str(field);
                    current_ty = next_ty.clone();
                }
                (Type::Nbt, PathSegment::Index(index)) => {
                    match &index.kind {
                        crate::ast::ExprKind::Int(value) => {
                            rendered.push_str(&format!("[{}]", value));
                        }
                        crate::ast::ExprKind::String(value) => {
                            push_quoted_path_name(&mut rendered, value);
                        }
                        _ => {
                            placeholder_index += 1;
                            let name = format!("n{}", placeholder_index);
                            let storage = macro_storage.get_or_insert_with(|| {
                                format!(
                                    "frames.d{}.{}.__path{}",
                                    depth,
                                    sanitize(&function.name),
                                    self.new_temp()
                                )
                            });
                            let ty = match infer_dynamic_nbt_index_type(function, index) {
                                Some(Type::Int) => Type::Int,
                                _ => Type::String,
                            };
                            self.compile_expr_to_macro_value(
                                function, depth, index, &ty, storage, &name, lines,
                            );
                            match ty {
                                Type::Int => rendered.push_str(&format!("[$({})]", name)),
                                _ => push_quoted_macro_path_name(&mut rendered, &name),
                            }
                        }
                    }
                    current_ty = next_ty.clone();
                }
                (_, PathSegment::Index(index)) => {
                    if let crate::ast::ExprKind::Int(value) = &index.kind {
                        rendered.push_str(&format!("[{}]", value));
                    }
                    current_ty = next_ty.clone();
                }
            }
        }

        RenderedStoragePath {
            path: rendered,
            macro_storage,
        }
    }

    fn render_storage_lvalue_prefix_path(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        end_exclusive: usize,
        lines: &mut Vec<String>,
    ) -> Option<RenderedStoragePath> {
        let IrExprKind::Variable(name) = &path.base.kind else {
            return None;
        };
        let root = string_slot(depth, &function.name, name);
        Some(self.render_storage_path(
            function,
            depth,
            root,
            &path.base.ty,
            &path.segments[..end_exclusive],
            &path.segment_types[..end_exclusive],
            lines,
        ))
    }

    fn try_compile_storage_index_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        path: &IrPathExpr,
        value_slot: &SlotRef,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(PathSegment::Index(index)) = path.segments.last() else {
            return false;
        };
        let crate::ast::ExprKind::Int(index_value) = &index.kind else {
            return false;
        };
        if *index_value < 0 || path.segments.is_empty() {
            return false;
        }
        let parent_index = path.segments.len() - 1;
        let parent_ty = if parent_index == 0 {
            path.base.ty.clone()
        } else {
            path.segment_types[parent_index - 1].clone()
        };
        if !matches!(parent_ty, Type::Nbt | Type::Array(_)) {
            return false;
        }
        let Some(parent_rendered) =
            self.render_storage_lvalue_prefix_path(function, depth, path, parent_index, lines)
        else {
            return false;
        };
        let element_path = format!("{}[{}]", parent_rendered.path, index_value);
        lines.push(self.storage_path_command(
            format!(
                "execute unless data storage {}:runtime {}[] run data modify storage {}:runtime {} set value []",
                self.namespace,
                parent_rendered.path,
                self.namespace,
                parent_rendered.path
            ),
            parent_rendered.macro_storage.clone(),
        ));
        lines.push(self.storage_path_command(
            format!(
                "execute if data storage {}:runtime {} run data modify storage {}:runtime {} set from storage {}:runtime {}",
                self.namespace,
                element_path,
                self.namespace,
                element_path,
                self.namespace,
                value_slot.storage_path()
            ),
            parent_rendered.macro_storage.clone(),
        ));
        lines.push(self.storage_path_command(
            format!(
                "execute unless data storage {}:runtime {} run data modify storage {}:runtime {} insert {} from storage {}:runtime {}",
                self.namespace,
                element_path,
                self.namespace,
                parent_rendered.path,
                index_value,
                self.namespace,
                value_slot.storage_path()
            ),
            parent_rendered.macro_storage,
        ));
        true
    }

    fn compile_expr_to_macro_value(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &crate::ast::Expr,
        ty: &Type,
        macro_storage: &str,
        name: &str,
        lines: &mut Vec<String>,
    ) {
        let typed_expr = self.lower_macro_path_expr(function, depth, expr, ty);
        let temp = self.new_temp();
        let temp_slot = local_slot(depth, &function.name, &temp, ty);
        self.compile_expr_into_slot(function, depth, &typed_expr, &temp_slot, lines);
        match ty {
            Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => lines.push(format!(
                "execute store result storage {}:runtime {}.{} int 1 run scoreboard players get {} mcfc",
                self.namespace,
                macro_storage,
                name,
                temp_slot.numeric_name()
            )),
            _ => lines.push(format!(
                "data modify storage {}:runtime {}.{} set from storage {}:runtime {}",
                self.namespace,
                macro_storage,
                name,
                self.namespace,
                temp_slot.storage_path()
            )),
        }
    }

    fn lower_macro_path_expr(
        &self,
        function: &IrFunction,
        depth: usize,
        expr: &crate::ast::Expr,
        ty: &Type,
    ) -> IrExpr {
        let _ = (function, depth);
        match &expr.kind {
            crate::ast::ExprKind::Int(value) => IrExpr {
                ty: Type::Int,
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Int(*value),
            },
            crate::ast::ExprKind::String(value) => IrExpr {
                ty: Type::String,
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::String(value.clone()),
            },
            crate::ast::ExprKind::Bool(value) => IrExpr {
                ty: Type::Bool,
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Bool(*value),
            },
            crate::ast::ExprKind::Variable(name) => IrExpr {
                ty: ty.clone(),
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Variable(name.clone()),
            },
            crate::ast::ExprKind::Unary { op, expr } => IrExpr {
                ty: ty.clone(),
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Unary {
                    op: *op,
                    expr: Box::new(self.lower_macro_path_expr(function, depth, expr, ty)),
                },
            },
            crate::ast::ExprKind::Binary { op, left, right } => IrExpr {
                ty: ty.clone(),
                ref_kind: RefKind::Unknown,
                kind: IrExprKind::Binary {
                    op: *op,
                    left: Box::new(self.lower_macro_path_expr(function, depth, left, ty)),
                    right: Box::new(self.lower_macro_path_expr(function, depth, right, ty)),
                },
            },
            crate::ast::ExprKind::Call {
                function: callee,
                args,
            } => {
                let (return_type, params) = self
                    .functions
                    .get(callee)
                    .map(|info| (info.return_type.clone(), info.params.clone()))
                    .unwrap_or((ty.clone(), Vec::new()));
                IrExpr {
                    ty: return_type,
                    ref_kind: RefKind::Unknown,
                    kind: IrExprKind::Call {
                        function: callee.clone(),
                        args: args
                            .iter()
                            .enumerate()
                            .map(|(index, arg)| {
                                let arg_ty = params.get(index).map(|(_, ty)| ty).unwrap_or(ty);
                                self.lower_macro_path_expr(function, depth, arg, arg_ty)
                            })
                            .collect(),
                    },
                }
            }
            // `keys[order[i]]`: a list or map variable indexed further. The type
            // checker only lets these and the forms above reach here.
            crate::ast::ExprKind::Path(path) => {
                let crate::ast::ExprKind::Variable(name) = &path.base.kind else {
                    unreachable!("type checker allows only variable-based index paths")
                };
                let base_ty = function
                    .locals
                    .get(name)
                    .or_else(|| {
                        function
                            .params
                            .iter()
                            .find(|param| &param.name == name)
                            .map(|param| &param.ty)
                    })
                    .cloned()
                    .unwrap_or(Type::Void);
                let mut current = base_ty.clone();
                let mut segment_types = Vec::new();
                for _ in &path.segments {
                    current = match current {
                        Type::Array(inner) | Type::Dict(inner) => *inner,
                        other => other,
                    };
                    segment_types.push(current.clone());
                }
                IrExpr {
                    ty: current.clone(),
                    ref_kind: RefKind::Unknown,
                    kind: IrExprKind::Path(IrPathExpr {
                        base: Box::new(IrExpr {
                            ty: base_ty,
                            ref_kind: RefKind::Unknown,
                            kind: IrExprKind::Variable(name.clone()),
                        }),
                        segments: path.segments.clone(),
                        segment_types,
                        ty: current,
                    }),
                }
            }
            _ => unreachable!("type checker rejects index expressions it can't lower"),
        }
    }

    fn compile_storage_read_from_path(
        &mut self,
        rendered: RenderedStoragePath,
        ty: &Type,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        // A missing element or key leaves a `set from` target unchanged, so
        // start floats and strings at zero, as a failed `data get` does for ints.
        let zero = match ty {
            Type::Float => Some("0.0f"),
            Type::String => Some("\"\""),
            _ => None,
        };
        if let Some(zero) = zero {
            lines.push(format!(
                "data modify storage {}:runtime {} set value {zero}",
                self.namespace,
                target.storage_path()
            ));
        }
        let command = match ty {
            Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => format!(
                "execute store result score {} mcfc run data get storage {}:runtime {} 1",
                target.numeric_name(),
                self.namespace,
                rendered.path
            ),
            _ => format!(
                "data modify storage {}:runtime {} set from storage {}:runtime {}",
                self.namespace,
                target.storage_path(),
                self.namespace,
                rendered.path
            ),
        };
        lines.push(self.storage_path_command(command, rendered.macro_storage));
    }

    fn storage_path_command(&mut self, command: String, macro_storage: Option<String>) -> String {
        if let Some(storage) = macro_storage {
            let namespace = self.namespace.clone();
            let macro_name = self.ensure_inline_macro(command);
            format!(
                "function {}:{} with storage {}:runtime {}",
                namespace, macro_name, namespace, storage
            )
        } else {
            command
        }
    }

    fn compile_storage_receiver(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        lines: &mut Vec<String>,
    ) -> SlotRef {
        let receiver_name = self.new_temp();
        let receiver_slot = local_slot(depth, &function.name, &receiver_name, &receiver.ty);
        self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
        receiver_slot
    }

    fn render_storage_expr_lvalue_path(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        lines: &mut Vec<String>,
    ) -> Option<RenderedStoragePath> {
        match &expr.kind {
            IrExprKind::Variable(name) => Some(RenderedStoragePath {
                path: string_slot(depth, &function.name, name),
                macro_storage: None,
            }),
            IrExprKind::Path(path) => self.render_storage_lvalue_path(function, depth, path, lines),
            _ => None,
        }
    }

    fn render_dict_key_for_method(
        &mut self,
        function: &IrFunction,
        depth: usize,
        key: &IrExpr,
        root: String,
        existing_macro_storage: Option<String>,
        lines: &mut Vec<String>,
    ) -> RenderedStoragePath {
        if let IrExprKind::String(value) = &key.kind {
            return RenderedStoragePath {
                path: format!("{}.{}", root, value),
                macro_storage: existing_macro_storage,
            };
        }

        let macro_storage = existing_macro_storage.unwrap_or_else(|| {
            format!(
                "frames.d{}.{}.__path{}",
                depth,
                sanitize(&function.name),
                self.new_temp()
            )
        });
        let temp = self.new_temp();
        let temp_slot = local_slot(depth, &function.name, &temp, &Type::String);
        self.compile_expr_into_slot(function, depth, key, &temp_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.key set from storage {}:runtime {}",
            self.namespace,
            macro_storage,
            self.namespace,
            temp_slot.storage_path()
        ));
        RenderedStoragePath {
            path: format!("{}.\"$(key)\"", root),
            macro_storage: Some(macro_storage),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_entity_system_method(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        method: &str,
        args: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let ns = self.namespace.clone();
        let query = local_slot(depth, &function.name, &self.new_temp(), &receiver.ty);
        self.compile_expr_into_slot(function, depth, receiver, &query, lines);
        let path = query.storage_path().to_string();
        if let Some(key) = method.strip_prefix("input_") {
            let predicate = format!("data/{ns}/predicate/mcfc_input_{key}.json");
            self.files.insert(predicate, format!(
                "{{\"type\":\"minecraft:entity_properties\",\"entity\":\"this\",\"predicate\":{{\"type_specific/player\":{{\"input\":{{\"{key}\":true}}}}}}}}"
            ));
            lines.push(format!(
                "scoreboard players set {} mcfc 0",
                target.numeric_name()
            ));
            lines.push(self.query_command(&query, format!(
                "execute as $(selector) if predicate {ns}:mcfc_input_{key} run scoreboard players set {} mcfc 1",
                target.numeric_name()
            ), true));
            return;
        }
        for (index, arg) in args.iter().enumerate() {
            let slot = local_slot(depth, &function.name, &self.new_temp(), &arg.ty);
            self.compile_expr_into_slot(function, depth, arg, &slot, lines);
            let field = format!("arg{index}");
            if matches!(
                arg.ty,
                Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_)
            ) {
                if index == 0
                    && matches!(method, "getAttribute" | "setAttribute")
                    && matches!(&arg.ty, Type::Enum(name) if name == "std::attribute::Attribute")
                {
                    const IDS: &[&str] = &[
                        "movement_speed",
                        "jump_strength",
                        "gravity",
                        "step_height",
                        "scale",
                        "safe_fall_distance",
                        "knockback_resistance",
                    ];
                    for (value, id) in IDS.iter().enumerate() {
                        lines.push(format!(
                            "execute if score {} mcfc matches {value} run data modify storage {ns}:runtime {path}.{field} set value \"minecraft:{id}\"",
                            slot.numeric_name()
                        ));
                    }
                } else {
                    lines.push(format!(
                        "execute store result storage {ns}:runtime {path}.{field} int 1 run scoreboard players get {} mcfc",
                        slot.numeric_name()
                    ));
                }
            } else {
                let source = if arg.ty == Type::BlockRef {
                    format!("{}.pos", slot.storage_path())
                } else if matches!(arg.ty, Type::EntityRef | Type::PlayerRef | Type::EntitySet) {
                    format!("{}.selector", slot.storage_path())
                } else {
                    slot.storage_path().to_string()
                };
                lines.push(format!("data modify storage {ns}:runtime {path}.{field} set from storage {ns}:runtime {source}"));
            }
        }
        match method {
            "setVelocity" => {
                lines.push(format!(
                    "data modify storage {ns}:runtime {path}.motion set value [0.0d,0.0d,0.0d]"
                ));
                for index in 0..3 {
                    lines.push(format!(
                        "execute store result storage {ns}:runtime {path}.motion[{index}] double 0.0001 run data get storage {ns}:runtime {path}.arg{index} 10000"
                    ));
                }
                lines.push(self.query_command(&query, format!(
                    "data modify entity $(selector) Motion set from storage {ns}:runtime {path}.motion"
                ), true));
            }
            "addVelocity" => {
                // Players ignore Motion writes, so they get a generated impulse;
                // other entities get their Motion added to directly.
                for index in 0..3 {
                    let motion = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                    let delta = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                    lines.push(self.query_command(&query, format!(
                        "execute as $(selector) unless entity @s[type=minecraft:player] store result score {} mcfc run data get entity @s Motion[{index}] 10000",
                        motion.numeric_name()
                    ), true));
                    lines.push(format!(
                        "execute store result score {} mcfc run data get storage {ns}:runtime {path}.arg{index} 10000",
                        delta.numeric_name()
                    ));
                    lines.push(format!(
                        "scoreboard players operation {} mcfc += {} mcfc",
                        motion.numeric_name(),
                        delta.numeric_name()
                    ));
                    lines.push(self.query_command(&query, format!(
                        "execute as $(selector) unless entity @s[type=minecraft:player] store result entity @s Motion[{index}] double 0.0001 run scoreboard players get {} mcfc",
                        motion.numeric_name()
                    ), true));
                }
                self.uses_impulse = true;
                for (index, axis) in ["x", "y", "z"].iter().enumerate() {
                    lines.push(format!(
                        "data modify storage {ns}:runtime mcfc_impulse.{axis} set from storage {ns}:runtime {path}.arg{index}"
                    ));
                }
                lines.push(self.query_command(&query, format!(
                    "execute as $(selector) if entity @s[type=minecraft:player] run function {ns}:generated/impulse"
                ), true));
            }
            "setHealth" => {
                self.uses_health = true;
                lines.push(format!(
                    "data modify storage {ns}:runtime mcfc_health.target set from storage {ns}:runtime {path}.arg0"
                ));
                for command in [
                    format!(
                        "execute as $(selector) if entity @s[type=minecraft:player] run function {ns}:generated/set_health"
                    ),
                    format!(
                        "execute as $(selector) unless entity @s[type=minecraft:player] run data modify entity @s Health set from storage {ns}:runtime {path}.arg0"
                    ),
                ] {
                    lines.push(self.query_command(&query, command, true));
                }
            }
            "setFoodLevel" => {
                self.uses_food = true;
                for command in [
                    format!("execute as $(selector) store result score @s mcfc_food_goal run data get storage {ns}:runtime {path}.arg0 1"),
                    "execute as $(selector) if score @s mcfc_food_goal matches ..-1 run scoreboard players set @s mcfc_food_goal 0".to_string(),
                    "execute as $(selector) if score @s mcfc_food_goal matches 21.. run scoreboard players set @s mcfc_food_goal 20".to_string(),
                    "execute as $(selector) run scoreboard players set @s mcfc_food_active 1".to_string(),
                ] {
                    lines.push(self.query_command(&query, command, true));
                }
            }
            "getAttribute" => {
                let score = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    score.numeric_name()
                ));
                lines.push(self.query_command(&query, format!(
                    "execute store result score {} mcfc run attribute $(selector) $(arg0) get 1000",
                    score.numeric_name()
                ), true));
                lines.push(format!(
                    "execute store result storage {ns}:runtime {} float 0.001 run scoreboard players get {} mcfc",
                    target.storage_path(), score.numeric_name()
                ));
            }
            "setAttribute" => lines.push(self.query_command(
                &query,
                "attribute $(selector) $(arg0) base set $(arg1)".to_string(),
                true,
            )),
            "setRotation" => lines.push(self.query_command(
                &query,
                "rotate $(selector) $(arg0) $(arg1)".to_string(),
                true,
            )),
            "lookAt" => lines.push(
                self.query_command(
                    &query,
                    if args[0].ty == Type::BlockRef {
                        "rotate $(selector) facing $(arg0)"
                    } else {
                        "rotate $(selector) facing entity $(arg0) feet"
                    }
                    .to_string(),
                    true,
                ),
            ),
            "yawTo" | "pitchTo" => {
                let tag = format!("mcfc_angle_{}", self.new_temp());
                let marker = format!("@e[type=minecraft:marker,tag={tag},sort=nearest,limit=1]");
                lines.push(self.query_command(&query, format!(
                    "execute at $(selector) run summon minecraft:marker ~ ~ ~ {{Tags:[\"{tag}\"]}}"
                ), true));
                let facing = if args[0].ty == Type::BlockRef {
                    "facing $(arg0)".to_string()
                } else {
                    "facing entity $(arg0) feet".to_string()
                };
                lines.push(self.query_command(
                    &query,
                    format!("execute at $(selector) run tp {marker} ~ ~ ~ {facing}"),
                    true,
                ));
                let score = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                lines.push(self.query_command(&query, format!(
                    "execute at $(selector) store result score {} mcfc run data get entity {marker} Rotation[{}] 1000",
                    score.numeric_name(), if method == "yawTo" { 0 } else { 1 }
                ), true));
                lines.push(format!(
                    "execute store result storage {ns}:runtime {} float 0.001 run scoreboard players get {} mcfc",
                    target.storage_path(), score.numeric_name()
                ));
                lines.push(self.query_command(
                    &query,
                    format!("execute at $(selector) run kill {marker}"),
                    true,
                ));
            }
            "setInterpolationDuration" | "setInterpolationDelay" | "setTeleportDuration" => {
                let field = match method {
                    "setInterpolationDuration" => "interpolation_duration",
                    "setInterpolationDelay" => "start_interpolation",
                    _ => "teleport_duration",
                };
                lines.push(self.query_command(&query, format!(
                    "execute store result entity $(selector) {field} int 1 run data get storage {ns}:runtime {path}.arg0"
                ), true));
            }
            "setTranslation" | "setScale" | "setLeftRotation" | "animate" => {
                // Vec3 records are {x,y,z}; display transformations want [x,y,z].
                let as_list = |index: usize, lines: &mut Vec<String>| {
                    lines.push(format!(
                        "data modify storage {ns}:runtime {path}.list{index} set value [0f,0f,0f]"
                    ));
                    for (slot, axis) in ["x", "y", "z"].iter().enumerate() {
                        lines.push(format!(
                            "data modify storage {ns}:runtime {path}.list{index}[{slot}] set from storage {ns}:runtime {path}.arg{index}.{axis}"
                        ));
                    }
                };
                let mut writes = Vec::new();
                match method {
                    "setTranslation" | "setScale" => {
                        as_list(0, lines);
                        let field = if method == "setScale" {
                            "scale"
                        } else {
                            "translation"
                        };
                        writes.push(format!(
                            "transformation.{field} set from storage {ns}:runtime {path}.list0"
                        ));
                    }
                    "setLeftRotation" => {
                        as_list(1, lines);
                        writes.push(
                            "transformation.left_rotation set value {angle:0f,axis:[0f,1f,0f]}"
                                .to_string(),
                        );
                        writes.push(format!("transformation.left_rotation.angle set from storage {ns}:runtime {path}.arg0"));
                        writes.push(format!("transformation.left_rotation.axis set from storage {ns}:runtime {path}.list1"));
                    }
                    _ => {
                        as_list(1, lines);
                        as_list(2, lines);
                        writes.push(format!(
                            "transformation.translation set from storage {ns}:runtime {path}.list1"
                        ));
                        writes.push(format!(
                            "transformation.scale set from storage {ns}:runtime {path}.list2"
                        ));
                        writes.push(format!(
                            "interpolation_duration set from storage {ns}:runtime {path}.arg0"
                        ));
                        writes.push("start_interpolation set value 0".to_string());
                    }
                }
                for write in writes {
                    lines.push(self.query_command(
                        &query,
                        format!("data modify entity $(selector) {write}"),
                        true,
                    ));
                }
            }
            "setOwner" => {
                // Links are scoreboard ids: any entity can hold a score, but only
                // some have a vanilla `Owner` field.
                self.uses_ownership = true;
                for command in [
                    format!("execute as $(arg0) unless score @s mcfc_id matches 1.. run function {ns}:generated/assign_id"),
                    "execute as $(selector) run scoreboard players operation @s mcfc_owner = $(arg0) mcfc_id".to_string(),
                ] {
                    lines.push(self.query_command(&query, command, true));
                }
            }
            "getOwner" => {
                self.uses_ownership = true;
                // ponytail: scans every entity for the owner's id; keep an id index
                // in storage if packs with many links need it.
                let tag = format!("mcfc_owner_{}", self.new_temp());
                let out = target.storage_path();
                lines.push(format!("tag @e[tag={tag}] remove {tag}"));
                lines.push("scoreboard players set #owner mcfc_id 0".to_string());
                lines.push(self.query_command(&query,
                    "execute as $(selector) if score @s mcfc_owner matches 1.. run scoreboard players operation #owner mcfc_id = @s mcfc_owner".to_string(), true));
                lines.push(format!(
                    "execute if score #owner mcfc_id matches 1.. as @e if score @s mcfc_id = #owner mcfc_id run tag @s add {tag}"
                ));
                lines.push(self.query_command(&query, format!(
                    "execute if score #owner mcfc_id matches 0 as $(selector) on owner run tag @s add {tag}"
                ), true));
                lines.push(format!(
                    "data modify storage {ns}:runtime {out} set value {{present:0b}}"
                ));
                lines.push(format!(
                    "execute if entity @e[tag={tag}] run data modify storage {ns}:runtime {out} set value {{present:1b,value:{{prefix:\"\",selector:\"@e[tag={tag},limit=1]\"}}}}"
                ));
            }
            "getTargetBlock" | "getTargetEntity" => {
                self.uses_raycast = true;
                let entity = method == "getTargetEntity";
                let out = target.storage_path();
                lines.push(format!(
                    "execute store result score #ray_steps mcfc run data get storage {ns}:runtime {path}.arg0 10"
                ));
                lines.push(format!(
                    "scoreboard players set #ray_mode mcfc {}",
                    u8::from(entity)
                ));
                lines.push(self.query_command(&query, format!(
                    "execute as $(selector) at @s anchored eyes positioned ^ ^ ^ run function {ns}:generated/raycast/start"
                ), true));
                lines.push(format!(
                    "data modify storage {ns}:runtime {out} set value {{present:0b}}"
                ));
                if entity {
                    // ponytail: the hit keeps a per-call-site tag until that site casts
                    // again; a UUID selector would make the ref independent of the site.
                    let tag = format!("mcfc_ray_{}", self.new_temp());
                    lines.push(format!("tag @e[tag={tag}] remove {tag}"));
                    lines.push(format!(
                        "execute if score #ray_hit mcfc matches 2 run tag @e[tag=mcfc_ray_hit] add {tag}"
                    ));
                    lines.push("tag @e[tag=mcfc_ray_hit] remove mcfc_ray_hit".to_string());
                    lines.push(format!(
                        "execute if score #ray_hit mcfc matches 2 run data modify storage {ns}:runtime {out} set value {{present:1b,value:{{prefix:\"\",selector:\"@e[tag={tag},limit=1]\"}}}}"
                    ));
                } else {
                    lines.push(format!(
                        "execute if score #ray_hit mcfc matches 1 run data modify storage {ns}:runtime {out} set value {{present:1b,value:{{prefix:\"\"}}}}"
                    ));
                    lines.push(format!(
                        "execute if score #ray_hit mcfc matches 1 run data modify storage {ns}:runtime {out}.value.pos set from storage {ns}:runtime mcfc_ray.pos"
                    ));
                }
            }
            _ => {}
        }
    }

    /// `Log.*`: messages go to players tagged `mcfc.log` when their level is at
    /// least this pack's `#log.<ns>` score. `dump` shows any value through a
    /// `score` or `nbt` text component, so it needs no macro.
    fn compile_log(
        &mut self,
        function: &IrFunction,
        depth: usize,
        callee: &str,
        arg: &IrExpr,
        lines: &mut Vec<String>,
    ) {
        self.uses_log = true;
        let ns = self.namespace.clone();
        let level_of = |name: &str| {
            crate::types::LOG_LEVELS
                .iter()
                .position(|l| *l == name)
                .unwrap()
        };
        if callee == "log_level" {
            if let IrExprKind::String(level) = &arg.kind {
                lines.push(format!(
                    "scoreboard players set #log.{ns} mcfc {}",
                    level_of(level)
                ));
            }
            return;
        }
        let (level, label, color) = match callee {
            "log_debug" => ("debug", "DEBUG", "gray"),
            "log_warn" => ("warn", "WARN", "yellow"),
            "log_error" => ("error", "ERROR", "red"),
            "log_dump" => ("debug", "DUMP", "aqua"),
            "assert_fail" => ("error", "TEST", "red"),
            _ => ("info", "INFO", "white"),
        };
        let slot = local_slot(depth, &function.name, &self.new_temp(), &arg.ty);
        self.compile_expr_into_slot(function, depth, arg, &slot, lines);
        let body = if callee != "log_dump" {
            format!(
                "{{\"storage\":\"{ns}:runtime\",\"nbt\":{},\"color\":\"white\"}}",
                quoted(slot.storage_path())
            )
        } else if matches!(
            arg.ty,
            Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_)
        ) {
            format!(
                "{{\"score\":{{\"name\":{},\"objective\":\"mcfc\"}},\"color\":\"white\"}}",
                quoted(slot.numeric_name())
            )
        } else {
            format!(
                "{{\"storage\":\"{ns}:runtime\",\"nbt\":{},\"color\":\"white\"}}",
                quoted(slot.storage_path())
            )
        };
        if callee == "assert_fail" {
            // Failures always print; `/function ns:test` counts them.
            lines.push("scoreboard players set #test_failed mcfc 1".to_string());
            lines.push(format!(
                "tellraw @a [{{\"text\":\"[{ns} TEST] assertion failed at \",\"color\":\"{color}\"}},{body}]"
            ));
            return;
        }
        lines.push(format!(
            "execute if score #log.{ns} mcfc matches ..{} run tellraw @a[tag=mcfc.log] [{{\"text\":\"[{ns} {label}] \",\"color\":\"{color}\"}},{body}]",
            level_of(level)
        ));
    }

    /// The shared sidebar is the `mcfc_sidebar` objective; line `n` is the
    /// fake player `mcfc.line.n` with score `-n`, so line 0 is on top. A player
    /// receiver queues the change in `mcfc:agent sidebar` for mcfd-agent, which
    /// sends that player their own sidebar; without the agent it falls back to
    /// the shared one.
    fn compile_sidebar(
        &mut self,
        function: &IrFunction,
        depth: usize,
        player: Option<&IrExpr>,
        op: &str,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let ns = self.namespace.clone();
        let data = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        let path = data.storage_path().to_string();
        lines.push(format!(
            "data modify storage {ns}:runtime {path} set value {{text:{{text:\"\"}},value:{{text:\"\"}}}}"
        ));
        for (index, arg) in args.iter().enumerate() {
            let slot = local_slot(depth, &function.name, &self.new_temp(), &arg.ty);
            self.compile_expr_into_slot(function, depth, arg, &slot, lines);
            // `setLine`'s third argument is the right-aligned value.
            let field = if index == 2 { "value" } else { "text" };
            lines.push(if arg.ty == Type::Int {
                format!(
                    "execute store result storage {ns}:runtime {path}.line int 1 run scoreboard players get {} mcfc",
                    slot.numeric_name()
                )
            } else if arg.ty == Type::TextDef {
                format!(
                    "data modify storage {ns}:runtime {path}.{field} set from storage {ns}:runtime {}",
                    slot.storage_path()
                )
            } else {
                format!(
                    "data modify storage {ns}:runtime {path}.{field}.text set from storage {ns}:runtime {}",
                    slot.storage_path()
                )
            });
        }
        if let Some(player) = player.filter(|_| self.agent_enabled()) {
            let query = local_slot(depth, &function.name, &self.new_temp(), &player.ty);
            self.compile_expr_into_slot(function, depth, player, &query, lines);
            let kind = op.trim_start_matches("sidebar_");
            lines.push(format!(
                "data modify storage {ns}:runtime {path}.op set value \"{kind}\""
            ));
            lines.push(self.query_command(&query, format!(
                "execute as $(selector) run data modify storage {ns}:runtime {path}.uuid set from entity @s UUID"
            ), true));
            lines.push(format!(
                "data modify storage mcfc:agent sidebar append from storage {ns}:runtime {path}"
            ));
            return;
        }
        self.uses_sidebar = true;
        let command = match op {
            "sidebar_title" => {
                "scoreboard objectives modify mcfc_sidebar displayname $(text)".to_string()
            }
            "sidebar_line" => {
                lines.push(self.inline_macro_command(&path, format!(
                    "execute store result score mcfc.line.$(line) mcfc_sidebar run data get storage {ns}:runtime {path}.line -1"
                )));
                lines.push(self.inline_macro_command(&path, if args.len() == 3 {
                    "scoreboard players display numberformat mcfc.line.$(line) mcfc_sidebar fixed $(value)".to_string()
                } else {
                    "scoreboard players display numberformat mcfc.line.$(line) mcfc_sidebar".to_string()
                }));
                "scoreboard players display name mcfc.line.$(line) mcfc_sidebar $(text)".to_string()
            }
            "sidebar_remove_line" => {
                "scoreboard players reset mcfc.line.$(line) mcfc_sidebar".to_string()
            }
            _ => {
                lines.push("scoreboard players reset * mcfc_sidebar".to_string());
                return;
            }
        };
        lines.push(self.inline_macro_command(&path, command));
    }

    fn compile_method_call(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        method: &str,
        args: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        match method {
            "setVelocity"
            | "addVelocity"
            | "getAttribute"
            | "setAttribute"
            | "setRotation"
            | "lookAt"
            | "yawTo"
            | "pitchTo"
            | "setHealth"
            | "setFoodLevel"
            | "input_forward"
            | "input_backward"
            | "input_left"
            | "input_right"
            | "input_jump"
            | "input_sneak"
            | "input_sprint"
            | "getTargetBlock"
            | "getTargetEntity"
            | "setOwner"
            | "getOwner"
            | "setInterpolationDuration"
            | "setInterpolationDelay"
            | "setTeleportDuration"
            | "setTranslation"
            | "setScale"
            | "setLeftRotation"
            | "animate" => {
                self.compile_entity_system_method(
                    function, depth, receiver, method, args, target, lines,
                );
                return;
            }
            "setSidebarTitle" | "setSidebarLine" | "removeSidebarLine" | "clearSidebar" => {
                let op = match method {
                    "setSidebarTitle" => "sidebar_title",
                    "setSidebarLine" => "sidebar_line",
                    "removeSidebarLine" => "sidebar_remove_line",
                    _ => "sidebar_clear",
                };
                self.compile_sidebar(function, depth, Some(receiver), op, args, lines);
                return;
            }
            "get" if matches!(receiver.ty, Type::Array(_) | Type::Dict(_)) => {
                let source = self.compile_storage_receiver(function, depth, receiver, lines);
                let Some(key) = args.first() else { return };
                let rendered = if matches!(receiver.ty, Type::Dict(_)) {
                    if let IrExprKind::String(value) = &key.kind {
                        RenderedStoragePath {
                            path: format!("{}.{}", source.storage_path(), quoted(value)),
                            macro_storage: None,
                        }
                    } else {
                        self.render_dict_key_for_method(
                            function,
                            depth,
                            key,
                            source.storage_path().to_string(),
                            None,
                            lines,
                        )
                    }
                } else {
                    let index_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                    self.compile_expr_into_slot(function, depth, key, &index_slot, lines);
                    let macro_storage = format!(
                        "frames.d{}.{}.__path{}",
                        depth,
                        sanitize(&function.name),
                        self.new_temp()
                    );
                    lines.push(format!(
                        "execute store result storage {}:runtime {}.index int 1 run scoreboard players get {} mcfc",
                        self.namespace, macro_storage, index_slot.numeric_name()
                    ));
                    RenderedStoragePath {
                        path: format!("{}[$(index)]", source.storage_path()),
                        macro_storage: Some(macro_storage),
                    }
                };
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{present:0b}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(self.storage_path_command(format!(
                    "execute if data storage {}:runtime {} run data modify storage {}:runtime {}.present set value 1b",
                    self.namespace, rendered.path, self.namespace, target.storage_path()
                ), rendered.macro_storage.clone()));
                lines.push(self.storage_path_command(
                    format!(
                        "data modify storage {}:runtime {}.value set from storage {}:runtime {}",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        rendered.path
                    ),
                    rendered.macro_storage,
                ));
                return;
            }
            "isPresent" if matches!(receiver.ty, Type::Optional(_)) => {
                let source = self.compile_storage_receiver(function, depth, receiver, lines);
                lines.push(format!(
                    "execute store result score {} mcfc run data get storage {}:runtime {}.present 1",
                    target.numeric_name(), self.namespace, source.storage_path()
                ));
                return;
            }
            "get" if matches!(receiver.ty, Type::Optional(_)) => {
                let source = self.compile_storage_receiver(function, depth, receiver, lines);
                let Type::Optional(value_ty) = &receiver.ty else {
                    unreachable!()
                };
                // An empty Optional gives the type's empty value, like `getFirst()` on an empty list.
                if matches!(
                    value_ty.as_ref(),
                    Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_)
                ) {
                    lines.push(format!(
                        "execute store result score {} mcfc run data get storage {}:runtime {}.value 1",
                        target.numeric_name(),
                        self.namespace,
                        source.storage_path()
                    ));
                } else {
                    lines.push(format!(
                        "data remove storage {}:runtime {}",
                        self.namespace,
                        target.storage_path()
                    ));
                    lines.push(format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}.value",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        source.storage_path()
                    ));
                }
                return;
            }
            "orElse" if matches!(receiver.ty, Type::Optional(_)) => {
                let source = self.compile_storage_receiver(function, depth, receiver, lines);
                let Some(fallback) = args.first() else { return };
                self.compile_expr_into_slot(function, depth, fallback, target, lines);
                let present = local_slot(depth, &function.name, &self.new_temp(), &Type::Bool);
                lines.push(format!(
                    "execute store result score {} mcfc run data get storage {}:runtime {}.present 1",
                    present.numeric_name(), self.namespace, source.storage_path()
                ));
                let Type::Optional(value_ty) = &receiver.ty else {
                    unreachable!()
                };
                let command = if matches!(
                    value_ty.as_ref(),
                    Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_)
                ) {
                    format!(
                        "execute store result score {} mcfc run data get storage {}:runtime {}.value 1",
                        target.numeric_name(),
                        self.namespace,
                        source.storage_path()
                    )
                } else {
                    format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}.value",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        source.storage_path()
                    )
                };
                lines.push(format!(
                    "execute if score {} mcfc matches 1 run {}",
                    present.numeric_name(),
                    command
                ));
                return;
            }
            "cancel" if matches!(&receiver.ty, Type::Struct(name) if crate::language_catalog::event_kind_for_type(name).is_some()) =>
            {
                lines.push(format!(
                    "data modify storage {}:agent decision.cancel set value 1b",
                    self.namespace
                ));
                return;
            }
            "as_nbt" => {
                self.compile_value_as_nbt(function, depth, receiver, target, lines);
                return;
            }
            "light" | "biome" | "in_biome" | "environment" | "x" | "y" | "z" | "block_type"
            | "copy_to" | "block_state"
                if receiver.ty == Type::BlockRef =>
            {
                self.compile_block_query(function, depth, receiver, method, args, target, lines);
                return;
            }
            "clear" if receiver.ty == Type::ItemSlot => {
                let slot_handle = self.compile_storage_receiver(function, depth, receiver, lines);
                self.clear_item_slot_handle(function, depth, &slot_handle, lines);
                return;
            }
            "slice" if receiver.ty == Type::String => {
                let source = self.compile_storage_receiver(function, depth, receiver, lines);
                let command = |bounds: &str| {
                    format!(
                        "data modify storage {ns}:runtime {} set string storage {ns}:runtime {} {}",
                        target.storage_path(),
                        source.storage_path(),
                        bounds,
                        ns = self.namespace
                    )
                };
                let literal: Option<Vec<String>> = args
                    .iter()
                    .map(|arg| match arg.kind {
                        IrExprKind::Int(value) => Some(value.to_string()),
                        _ => None,
                    })
                    .collect();
                if let Some(bounds) = literal {
                    // The source is already copied to a temp, so clearing the
                    // target first is safe even for `s = s.slice(...)`.
                    lines.push(format!(
                        "data modify storage {}:runtime {} set value \"\"",
                        self.namespace,
                        target.storage_path()
                    ));
                    lines.push(command(&bounds.join(" ")));
                } else {
                    let placeholders: Vec<IrMacroPlaceholder> = args
                        .iter()
                        .enumerate()
                        .map(|(index, arg)| IrMacroPlaceholder {
                            key: format!("p{}", index + 1),
                            expr: arg.clone(),
                            ty: Type::Int,
                        })
                        .collect();
                    let bounds: Vec<String> = placeholders
                        .iter()
                        .map(|placeholder| format!("$({})", placeholder.key))
                        .collect();
                    let body = format!("${}", command(&bounds.join(" ")));
                    let fallback = Some(format!(
                        "data modify storage {}:runtime {} set value \"\"",
                        self.namespace,
                        target.storage_path()
                    ));
                    self.call_macro(
                        function,
                        depth,
                        "slice",
                        body,
                        &placeholders,
                        fallback,
                        false,
                        lines,
                    );
                }
                return;
            }
            "parse_int" if receiver.ty == Type::String => {
                // A value that is not a whole number makes the macro line fail
                // to parse, so the command never runs and the 0 stays.
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                let placeholders = [IrMacroPlaceholder {
                    key: "p1".to_string(),
                    expr: receiver.clone(),
                    ty: Type::String,
                }];
                let body = format!(
                    "$scoreboard players set {} mcfc $(p1)",
                    target.numeric_name()
                );
                self.call_macro(
                    function,
                    depth,
                    "parse_int",
                    body,
                    &placeholders,
                    None,
                    false,
                    lines,
                );
                return;
            }
            "len" => {
                let receiver_slot = self.compile_storage_receiver(function, depth, receiver, lines);
                lines.push(format!(
                    "execute store result score {} mcfc run data get storage {}:runtime {}",
                    target.numeric_name(),
                    self.namespace,
                    receiver_slot.storage_path()
                ));
                return;
            }
            "keys" if matches!(receiver.ty, Type::Dict(_)) => {
                let source = self.compile_storage_receiver(function, depth, receiver, lines);
                self.write_dict_keys_helpers();
                let ns = self.namespace.clone();
                lines.push(format!(
                    "data modify storage {ns}:runtime dict_keys set value {{text:\"\",keys:[]}}"
                ));
                lines.push(format!(
                    "data modify storage {ns}:runtime dict_keys.src set from storage {ns}:runtime {}",
                    source.storage_path()
                ));
                lines.push(format!("function {ns}:generated/dict_keys"));
                lines.push(format!(
                    "data modify storage {ns}:runtime {} set from storage {ns}:runtime dict_keys.keys",
                    target.storage_path()
                ));
                return;
            }
            "first" | "last" if matches!(receiver.ty, Type::Array(_)) => {
                let source = self.compile_storage_receiver(function, depth, receiver, lines);
                let index = if method == "first" { 0 } else { -1 };
                let Type::Array(element_ty) = &receiver.ty else {
                    unreachable!()
                };
                self.compile_storage_read_from_path(
                    RenderedStoragePath {
                        path: format!("{}[{}]", source.storage_path(), index),
                        macro_storage: None,
                    },
                    element_ty,
                    target,
                    lines,
                );
                return;
            }
            "contains" | "index_of" if matches!(receiver.ty, Type::Array(_)) => {
                let found = self.compile_array_index_of(function, depth, receiver, &args[0], lines);
                if method == "index_of" {
                    lines.push(format!(
                        "scoreboard players operation {} mcfc = {} mcfc",
                        target.numeric_name(),
                        found
                    ));
                } else {
                    lines.push(format!(
                        "scoreboard players set {} mcfc 0",
                        target.numeric_name()
                    ));
                    lines.push(format!(
                        "execute unless score {} mcfc matches -1 run scoreboard players set {} mcfc 1",
                        found,
                        target.numeric_name()
                    ));
                }
                return;
            }
            "clear" | "insert" | "reverse" if matches!(receiver.ty, Type::Array(_)) => {
                let Some(rendered) =
                    self.render_storage_expr_lvalue_path(function, depth, receiver, lines)
                else {
                    return;
                };
                let command = match method {
                    "clear" => format!(
                        "data modify storage {}:runtime {} set value []",
                        self.namespace, rendered.path
                    ),
                    "reverse" => {
                        let source =
                            self.compile_storage_receiver(function, depth, receiver, lines);
                        let reversed = self.compile_array_reverse(function, depth, &source, lines);
                        format!(
                            "data modify storage {}:runtime {} set from storage {}:runtime {}",
                            self.namespace, rendered.path, self.namespace, reversed
                        )
                    }
                    _ => {
                        let value = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                        self.compile_value_as_nbt(function, depth, &args[1], &value, lines);
                        let index = match args[0].kind {
                            IrExprKind::Int(index) => index.to_string(),
                            _ => "$(index)".to_string(),
                        };
                        let command = format!(
                            "data modify storage {}:runtime {} insert {} from storage {}:runtime {}",
                            self.namespace,
                            rendered.path,
                            index,
                            self.namespace,
                            value.storage_path()
                        );
                        if !matches!(args[0].kind, IrExprKind::Int(_)) {
                            let macro_storage =
                                rendered.macro_storage.clone().unwrap_or_else(|| {
                                    format!(
                                        "frames.d{}.{}.__path{}",
                                        depth,
                                        sanitize(&function.name),
                                        self.new_temp()
                                    )
                                });
                            let index_slot =
                                local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                            self.compile_expr_into_slot(
                                function,
                                depth,
                                &args[0],
                                &index_slot,
                                lines,
                            );
                            lines.push(format!(
                                "execute store result storage {}:runtime {}.index int 1 run scoreboard players get {} mcfc",
                                self.namespace,
                                macro_storage,
                                index_slot.numeric_name()
                            ));
                            lines.push(self.storage_path_command(command, Some(macro_storage)));
                            return;
                        }
                        command
                    }
                };
                lines.push(self.storage_path_command(command, rendered.macro_storage));
                return;
            }
            "push" => {
                if let Some(rendered) =
                    self.render_storage_expr_lvalue_path(function, depth, receiver, lines)
                    && let Some(value) = args.first()
                {
                    let temp = self.new_temp();
                    let temp_slot = local_slot(depth, &function.name, &temp, &Type::Nbt);
                    self.compile_value_as_nbt(function, depth, value, &temp_slot, lines);
                    lines.push(self.storage_path_command(
                        format!(
                            "data modify storage {}:runtime {} append from storage {}:runtime {}",
                            self.namespace,
                            rendered.path,
                            self.namespace,
                            temp_slot.storage_path()
                        ),
                        rendered.macro_storage,
                    ));
                }
                return;
            }
            "pop" => {
                if let Some(rendered) =
                    self.render_storage_expr_lvalue_path(function, depth, receiver, lines)
                {
                    let element_ty = match &receiver.ty {
                        Type::Array(element) => element.as_ref(),
                        _ => &Type::Nbt,
                    };
                    self.compile_storage_read_from_path(
                        RenderedStoragePath {
                            path: format!("{}[-1]", rendered.path),
                            macro_storage: rendered.macro_storage.clone(),
                        },
                        element_ty,
                        target,
                        lines,
                    );
                    lines.push(self.storage_path_command(
                        format!(
                            "data remove storage {}:runtime {}[-1]",
                            self.namespace, rendered.path
                        ),
                        rendered.macro_storage,
                    ));
                }
                return;
            }
            "has" => {
                let receiver_slot = self.compile_storage_receiver(function, depth, receiver, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                if let Some(key) = args.first() {
                    let key_rendered = self.render_dict_key_for_method(
                        function,
                        depth,
                        key,
                        receiver_slot.storage_path().to_string(),
                        None,
                        lines,
                    );
                    lines.push(self.storage_path_command(
                        format!(
                            "execute if data storage {}:runtime {} run scoreboard players set {} mcfc 1",
                            self.namespace,
                            key_rendered.path,
                            target.numeric_name()
                        ),
                        key_rendered.macro_storage,
                    ));
                }
                return;
            }
            "remove" => {
                if receiver.ty == Type::Bossbar {
                    let receiver_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &receiver.ty);
                    self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
                    let macro_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                        self.namespace,
                        macro_slot.storage_path(),
                        self.namespace,
                        receiver_slot.storage_path()
                    ));
                    lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        "bossbar remove $(id)".to_string(),
                    ));
                    return;
                }
                if matches!(receiver.ty, Type::Array(_)) {
                    if let Some(rendered) =
                        self.render_storage_expr_lvalue_path(function, depth, receiver, lines)
                    {
                        let element_ty = match &receiver.ty {
                            Type::Array(element) => element.as_ref(),
                            _ => &Type::Nbt,
                        };
                        if let Some(index) = args.first() {
                            let macro_storage =
                                rendered.macro_storage.clone().unwrap_or_else(|| {
                                    format!(
                                        "frames.d{}.{}.__path{}",
                                        depth,
                                        sanitize(&function.name),
                                        self.new_temp()
                                    )
                                });
                            let index_slot =
                                local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                            self.compile_expr_into_slot(function, depth, index, &index_slot, lines);
                            lines.push(format!(
                                "execute store result storage {}:runtime {}.index int 1 run scoreboard players get {} mcfc",
                                self.namespace,
                                macro_storage,
                                index_slot.numeric_name()
                            ));
                            self.compile_storage_read_from_path(
                                RenderedStoragePath {
                                    path: format!("{}[$(index)]", rendered.path),
                                    macro_storage: Some(macro_storage.clone()),
                                },
                                element_ty,
                                target,
                                lines,
                            );
                            lines.push(self.storage_path_command(
                                format!(
                                    "data remove storage {}:runtime {}[$(index)]",
                                    self.namespace, rendered.path
                                ),
                                Some(macro_storage),
                            ));
                        }
                    }
                    return;
                }
                if let Some(key) = args.first()
                    && let Some(receiver_path) =
                        self.render_storage_expr_lvalue_path(function, depth, receiver, lines)
                {
                    let key_rendered = self.render_dict_key_for_method(
                        function,
                        depth,
                        key,
                        receiver_path.path,
                        receiver_path.macro_storage,
                        lines,
                    );
                    lines.push(self.storage_path_command(
                        format!(
                            "data remove storage {}:runtime {}",
                            self.namespace, key_rendered.path
                        ),
                        key_rendered.macro_storage,
                    ));
                }
                return;
            }
            "teleport" | "damage" | "heal" | "give" | "clear" | "loot_give" | "tellraw"
            | "title" | "actionbar" | "debug_entity" => {
                if method == "give" && args.len() == 1 && args[0].ty == Type::ItemDef {
                    self.compile_entity_give_item_def(function, depth, receiver, &args[0], lines);
                    return;
                }
                let mut synthetic = Vec::with_capacity(args.len() + 1);
                synthetic.push(receiver.clone());
                synthetic.extend(args.iter().cloned());
                self.compile_builtin_call(function, depth, method, &synthetic, target, lines);
                return;
            }
            "playsound" => {
                if args.len() >= 2 {
                    let synthetic = vec![args[0].clone(), args[1].clone(), receiver.clone()];
                    self.compile_builtin_call(
                        function,
                        depth,
                        "playsound",
                        &synthetic,
                        target,
                        lines,
                    );
                }
                return;
            }
            "stopsound" => {
                if args.len() >= 2 {
                    let synthetic = vec![receiver.clone(), args[0].clone(), args[1].clone()];
                    self.compile_builtin_call(
                        function,
                        depth,
                        "stopsound",
                        &synthetic,
                        target,
                        lines,
                    );
                }
                return;
            }
            "summon" => {
                self.compile_block_summon_method(function, depth, receiver, args, target, lines);
                return;
            }
            "spawn_item" => {
                self.compile_block_spawn_item_method(
                    function, depth, receiver, args, target, lines,
                );
                return;
            }
            "loot_insert" | "loot_spawn" | "setblock" | "fill" | "debug_marker" => {
                let mut synthetic = Vec::with_capacity(args.len() + 1);
                synthetic.push(receiver.clone());
                synthetic.extend(args.iter().cloned());
                self.compile_builtin_call(function, depth, method, &synthetic, target, lines);
                return;
            }
            "isLoaded" => {
                let receiver_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &receiver.ty);
                self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                lines.push(self.block_command(
                    &receiver_slot,
                    format!(
                        "execute if loaded $(pos) run scoreboard players set {} mcfc 1",
                        target.numeric_name()
                    ),
                    true,
                ));
                return;
            }
            "is" => {
                let receiver_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &receiver.ty);
                self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                if let Some(arg) = args.first() {
                    let block_id_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                    self.compile_expr_into_slot(function, depth, arg, &block_id_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.block set from storage {}:runtime {}",
                        self.namespace,
                        receiver_slot.storage_path(),
                        self.namespace,
                        block_id_slot.storage_path()
                    ));
                    lines.push(self.block_command(
                        &receiver_slot,
                        format!(
                            "execute if block $(pos) $(block) run scoreboard players set {} mcfc 1",
                            target.numeric_name()
                        ),
                        true,
                    ));
                }
                return;
            }
            "particle" => {
                if let Some(name) = args.first() {
                    let mut synthetic = Vec::with_capacity(args.len() + 1);
                    synthetic.push(name.clone());
                    synthetic.push(receiver.clone());
                    synthetic.extend(args.iter().skip(1).cloned());
                    self.compile_builtin_call(
                        function, depth, "particle", &synthetic, target, lines,
                    );
                }
                return;
            }
            "add_tag" | "remove_tag" => {
                let receiver_name = self.new_temp();
                let receiver_slot = local_slot(depth, &function.name, &receiver_name, &receiver.ty);
                self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
                if let Some(arg) = args.first() {
                    let tag_name = self.new_temp();
                    let tag_slot = local_slot(depth, &function.name, &tag_name, &Type::String);
                    self.compile_expr_into_slot(function, depth, arg, &tag_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.tag set from storage {}:runtime {}",
                        self.namespace,
                        receiver_slot.storage_path(),
                        self.namespace,
                        tag_slot.storage_path()
                    ));
                    lines.push(self.query_command(
                        &receiver_slot,
                        format!(
                            "tag $(selector) {} $(tag)",
                            if method == "add_tag" { "add" } else { "remove" }
                        ),
                        true,
                    ));
                }
                return;
            }
            "has_tag" => {
                let receiver_name = self.new_temp();
                let receiver_slot = local_slot(depth, &function.name, &receiver_name, &receiver.ty);
                self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                if let Some(arg) = args.first() {
                    let tag_name = self.new_temp();
                    let tag_slot = local_slot(depth, &function.name, &tag_name, &Type::String);
                    self.compile_expr_into_slot(function, depth, arg, &tag_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.tag set from storage {}:runtime {}",
                        self.namespace,
                        receiver_slot.storage_path(),
                        self.namespace,
                        tag_slot.storage_path()
                    ));
                    lines.push(self.query_command(
                        &receiver_slot,
                        format!(
                            "execute as $(selector) if entity @s[tag=$(tag)] run scoreboard players set {} mcfc 1",
                            target.numeric_name()
                        ),
                        true,
                    ));
                }
                return;
            }
            // `clear` with a count of 0 removes nothing and returns how many match.
            "countItem" => {
                let receiver_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &receiver.ty);
                self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                if let Some(arg) = args.first() {
                    let item_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                    self.compile_expr_into_slot(function, depth, arg, &item_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.item set from storage {}:runtime {}",
                        self.namespace,
                        receiver_slot.storage_path(),
                        self.namespace,
                        item_slot.storage_path()
                    ));
                    lines.push(self.query_command(
                        &receiver_slot,
                        format!(
                            "execute store result score {} mcfc run clear $(selector) $(item) 0",
                            target.numeric_name()
                        ),
                        true,
                    ));
                }
                return;
            }
            _ => {}
        }
        if method != "effect" {
            return;
        }
        let receiver_name = self.new_temp();
        let receiver_slot = local_slot(depth, &function.name, &receiver_name, &receiver.ty);
        self.compile_expr_into_slot(function, depth, receiver, &receiver_slot, lines);

        let effect_name = self.new_temp();
        let duration_name = self.new_temp();
        let amplifier_name = self.new_temp();
        let effect_slot = local_slot(depth, &function.name, &effect_name, &Type::String);
        let duration_slot = local_slot(depth, &function.name, &duration_name, &Type::Int);
        let amplifier_slot = local_slot(depth, &function.name, &amplifier_name, &Type::Int);
        if let Some(arg) = args.first() {
            self.compile_expr_into_slot(function, depth, arg, &effect_slot, lines);
        }
        if let Some(arg) = args.get(1) {
            self.compile_expr_into_slot(function, depth, arg, &duration_slot, lines);
        }
        if let Some(arg) = args.get(2) {
            self.compile_expr_into_slot(function, depth, arg, &amplifier_slot, lines);
        }

        let composed_name = self.new_temp();
        let composed_slot = local_slot(depth, &function.name, &composed_name, &Type::EntityRef);
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            composed_slot.storage_path(),
            self.namespace,
            receiver_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
            self.namespace,
            composed_slot.storage_path(),
            self.namespace,
            receiver_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.effect set from storage {}:runtime {}",
            self.namespace,
            composed_slot.storage_path(),
            self.namespace,
            effect_slot.storage_path()
        ));
        lines.push(format!(
            "execute store result storage {}:runtime {}.duration int 1 run scoreboard players get {} mcfc",
            self.namespace,
            composed_slot.storage_path(),
            duration_slot.numeric_name()
        ));
        lines.push(format!(
            "execute store result storage {}:runtime {}.amplifier int 1 run scoreboard players get {} mcfc",
            self.namespace,
            composed_slot.storage_path(),
            amplifier_slot.numeric_name()
        ));
        lines.push(self.query_command(
            &composed_slot,
            "effect give $(selector) $(effect) $(duration) $(amplifier) true".to_string(),
            true,
        ));
    }

    fn compile_entity_give_item_def(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        item: &IrExpr,
        lines: &mut Vec<String>,
    ) {
        let target_slot = self.compile_storage_receiver(function, depth, receiver, lines);
        let item_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::ItemDef);
        self.compile_expr_into_slot(function, depth, item, &item_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.item set from storage {}:runtime {}.id",
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.data set from storage {}:runtime {}.nbt",
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.item_name",
            self.namespace,
            target_slot.storage_path()
        ));
        lines.push(format!(
            "execute if data storage {}:runtime {}.nbt.display.Name run data modify storage {}:runtime {}.item_name set from storage {}:runtime {}.nbt.display.Name",
            self.namespace,
            item_slot.storage_path(),
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.count set from storage {}:runtime {}.count",
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        let named_give = self.query_command(
            &target_slot,
            "give $(selector) $(item)[minecraft:custom_name='\"$(item_name)\"',minecraft:custom_data=$(data)] $(count)".to_string(),
            true,
        );
        let plain_give = self.query_command(
            &target_slot,
            "give $(selector) $(item)[minecraft:custom_data=$(data)] $(count)".to_string(),
            true,
        );
        lines.push(format!(
            "execute if data storage {}:runtime {}.item_name run {}",
            self.namespace,
            target_slot.storage_path(),
            named_give
        ));
        lines.push(format!(
            "execute unless data storage {}:runtime {}.item_name run {}",
            self.namespace,
            target_slot.storage_path(),
            plain_give
        ));
    }

    fn compile_block_summon_method(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        args: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let pos_slot = self.compile_storage_receiver(function, depth, receiver, lines);
        let payload_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        let entity_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        if args.first().is_some_and(|arg| arg.ty == Type::EntityDef) {
            let spec_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::EntityDef);
            self.compile_expr_into_slot(function, depth, &args[0], &spec_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {} set from storage {}:runtime {}.id",
                self.namespace,
                entity_slot.storage_path(),
                self.namespace,
                spec_slot.storage_path()
            ));
            lines.push(format!(
                "data modify storage {}:runtime {} set from storage {}:runtime {}.nbt",
                self.namespace,
                payload_slot.storage_path(),
                self.namespace,
                spec_slot.storage_path()
            ));
            lines.push(format!(
                "execute unless data storage {}:runtime {} run data modify storage {}:runtime {} set value {{}}",
                self.namespace,
                payload_slot.storage_path(),
                self.namespace,
                payload_slot.storage_path()
            ));
        } else {
            if let Some(arg) = args.first() {
                self.compile_expr_into_slot(function, depth, arg, &entity_slot, lines);
            }
            if let Some(arg) = args.get(1) {
                self.compile_value_as_nbt(function, depth, arg, &payload_slot, lines);
            } else {
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    payload_slot.storage_path()
                ));
            }
        }
        self.compile_summon_from_position_slot(
            function,
            depth,
            &pos_slot,
            &entity_slot,
            &payload_slot,
            target,
            lines,
        );
    }

    fn compile_block_spawn_item_method(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        args: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let pos_slot = self.compile_storage_receiver(function, depth, receiver, lines);
        let entity_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        let item_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        let payload_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        lines.push(format!(
            "data modify storage {}:runtime {} set value \"minecraft:item\"",
            self.namespace,
            entity_slot.storage_path()
        ));
        if let Some(item) = args.first() {
            self.compile_value_as_nbt(function, depth, item, &item_slot, lines);
        } else {
            lines.push(format!(
                "data modify storage {}:runtime {} set value {{id:\"minecraft:air\",Count:0b}}",
                self.namespace,
                item_slot.storage_path()
            ));
        }
        lines.push(format!(
            "data modify storage {}:runtime {} set value {{}}",
            self.namespace,
            payload_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.Item set from storage {}:runtime {}",
            self.namespace,
            payload_slot.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        self.compile_summon_from_position_slot(
            function,
            depth,
            &pos_slot,
            &entity_slot,
            &payload_slot,
            target,
            lines,
        );
    }

    fn compile_summon_from_position_slot(
        &mut self,
        function: &IrFunction,
        depth: usize,
        pos_slot: &SlotRef,
        entity_slot: &SlotRef,
        payload_slot: &SlotRef,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let summon_target = if target.storage_path() == "__void" {
            local_slot(depth, &function.name, &self.new_temp(), &Type::EntityRef)
        } else {
            target.clone()
        };
        let capture_tag = format!("mcfc_summon_capture_{}", self.new_temp());
        let ref_tag = format!("mcfc_summon_ref_{}", self.new_temp());
        lines.push(format!("tag @e[tag={}] remove {}", ref_tag, ref_tag));
        lines.push(format!(
            "tag @e[tag={}] remove {}",
            capture_tag, capture_tag
        ));
        lines.push(format!(
            "execute unless data storage {}:runtime {}.Tags[] run data modify storage {}:runtime {}.Tags set value []",
            self.namespace,
            payload_slot.storage_path(),
            self.namespace,
            payload_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.Tags append value {}",
            self.namespace,
            payload_slot.storage_path(),
            quoted(&capture_tag)
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.Tags append value {}",
            self.namespace,
            payload_slot.storage_path(),
            quoted(&ref_tag)
        ));
        let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            pos_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.pos set from storage {}:runtime {}.pos",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            pos_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.entity set from storage {}:runtime {}",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            entity_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.data set from storage {}:runtime {}",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            payload_slot.storage_path()
        ));
        lines.push(self.inline_macro_command(
            macro_slot.storage_path(),
            "$(prefix)summon $(entity) $(pos) $(data)".to_string(),
        ));
        self.write_query_slot(
            &summon_target,
            "",
            &format!("@e[tag={},sort=nearest,limit=1]", ref_tag),
            lines,
        );
        lines.push(self.query_command(
            &summon_target,
            format!("tag $(selector) remove {}", capture_tag),
            true,
        ));
        // Summoned entities are selected by id, so the result can be stored.
        self.stabilize_entity_ref(&summon_target, lines);
        lines.push(format!("tag @e[tag={ref_tag}] remove {ref_tag}"));
    }

    fn compile_builtin_call(
        &mut self,
        function: &IrFunction,
        depth: usize,
        callee: &str,
        args: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) -> bool {
        match callee {
            "block_at" => {
                // Absolute coordinates; macros render ints as plain numbers.
                let ns = self.namespace.clone();
                let coords = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                for (arg, axis) in args.iter().zip(["x", "y", "z"]) {
                    let slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                    self.compile_expr_into_slot(function, depth, arg, &slot, lines);
                    lines.push(format!(
                        "execute store result storage {ns}:runtime {}.{axis} int 1 run scoreboard players get {} mcfc",
                        coords.storage_path(),
                        slot.numeric_name()
                    ));
                }
                self.write_block_slot(target, "", "0 0 0", lines);
                let command = format!(
                    "data modify storage {ns}:runtime {}.pos set value \"$(x) $(y) $(z)\"",
                    target.storage_path()
                );
                lines.push(self.inline_macro_command(coords.storage_path(), command));
                true
            }
            "__mcfc_event_block" => {
                lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime mcfc_event.block",
                    self.namespace,
                    target.storage_path(),
                    self.namespace
                ));
                true
            }
            "find_first" if !self.functions.contains_key(callee) => {
                let Some(query) = args.first() else {
                    return true;
                };
                let source = self.compile_storage_receiver(function, depth, query, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{present:0b}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(self.query_command(
                    &source,
                    format!(
                        "execute if entity $(selector) run data modify storage {}:runtime {}.present set value 1b",
                        self.namespace, target.storage_path()
                    ),
                    true,
                ));
                lines.push(format!(
                    "execute if data storage {}:runtime {}{{present:1b}} run data modify storage {}:runtime {}.value set from storage {}:runtime {}",
                    self.namespace, target.storage_path(), self.namespace, target.storage_path(),
                    self.namespace, source.storage_path()
                ));
                true
            }
            "gamerule" if !self.functions.contains_key(callee) => {
                if let IrExprKind::String(name) = &args[0].kind {
                    lines.push(format!(
                        "execute store result score {} mcfc run gamerule {}",
                        target.numeric_name(),
                        name.trim_start_matches("minecraft:")
                    ));
                }
                true
            }
            "random_weighted" if !self.functions.contains_key(callee) => {
                let IrExprKind::ArrayLiteral(weights) = &args[0].kind else {
                    return true;
                };
                let entries: Vec<String> = weights
                    .iter()
                    .enumerate()
                    .filter_map(|(index, weight)| match weight.kind {
                        IrExprKind::Int(weight) => {
                            Some(format!("{{data:{index},weight:{weight}}}"))
                        }
                        _ => None,
                    })
                    .collect();
                lines.push(format!(
                    "execute store result score {} mcfc run compute default integer {{type:\"weighted_list\",distribution:[{}]}}",
                    target.numeric_name(),
                    entries.join(",")
                ));
                true
            }
            "random_binomial" if !self.functions.contains_key(callee) => {
                let n = match args[0].kind {
                    IrExprKind::Int(value) => value.to_string(),
                    _ => {
                        let slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                        self.compile_expr_into_slot(function, depth, &args[0], &slot, lines);
                        format!(
                            "{{type:\"score\",target:{{type:\"fixed\",name:{}}},score:\"mcfc\"}}",
                            slot.numeric_name()
                        )
                    }
                };
                let p = self.float_provider(function, depth, &args[1], lines);
                lines.push(format!(
                    "execute store result score {} mcfc run compute default integer {{type:\"binomial\",n:{n},p:{p}}}",
                    target.numeric_name()
                ));
                true
            }
            "game_time" | "world_time" | "border_size" if !self.functions.contains_key(callee) => {
                let query = match callee {
                    "game_time" => "time query gametime",
                    "world_time" => "time query time",
                    _ => "worldborder get",
                };
                lines.push(format!(
                    "execute store result score {} mcfc run {}",
                    target.numeric_name(),
                    query
                ));
                true
            }
            "random" => {
                if args.is_empty() {
                    lines.push(format!(
                        "execute store result score {} mcfc run random value 0..2147483647",
                        target.numeric_name()
                    ));
                    return true;
                }

                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let min_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                let max_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                match args {
                    [max] => {
                        lines.push(format!(
                            "scoreboard players set {} mcfc 0",
                            min_slot.numeric_name()
                        ));
                        self.compile_expr_into_slot(function, depth, max, &max_slot, lines);
                    }
                    [min, max] => {
                        self.compile_expr_into_slot(function, depth, min, &min_slot, lines);
                        self.compile_expr_into_slot(function, depth, max, &max_slot, lines);
                    }
                    _ => return true,
                }
                lines.push(format!(
                    "execute store result storage {}:runtime {}.min int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    macro_slot.storage_path(),
                    min_slot.numeric_name()
                ));
                lines.push(format!(
                    "execute store result storage {}:runtime {}.max int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    macro_slot.storage_path(),
                    max_slot.numeric_name()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    format!(
                        "execute store result score {} mcfc run random value $(min)..$(max)",
                        target.numeric_name()
                    ),
                ));
                true
            }
            "bossbar" => {
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    target.storage_path()
                ));
                if let IrExprKind::String(text) = &args[1].kind {
                    let component = selector_text_components(text).unwrap_or_else(|| quoted(text));
                    lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        format!("bossbar add $(id) {}", component),
                    ));
                    return true;
                }
                if args[1].ty == Type::TextDef {
                    let name_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::TextDef);
                    self.compile_expr_into_slot(function, depth, &args[1], &name_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
                        self.namespace,
                        macro_slot.storage_path(),
                        self.namespace,
                        name_slot.storage_path()
                    ));
                    lines.push(self.inline_macro_command(
                        macro_slot.storage_path(),
                        "bossbar add $(id) $(name)".to_string(),
                    ));
                    return true;
                }
                let name_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[1], &name_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    name_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    "bossbar add $(id) [\"$(name)\"]".to_string(),
                ));
                true
            }
            "entity" => {
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.nbt set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                true
            }
            "block_type" => {
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.states set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.nbt set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                true
            }
            "item" => {
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.count set value 1",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.nbt set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                true
            }
            "text_snbt" => {
                if let Some(IrExprKind::String(snbt)) = args.first().map(|arg| &arg.kind) {
                    lines.push(format!(
                        "data modify storage {}:runtime {} set value {snbt}",
                        self.namespace,
                        target.storage_path()
                    ));
                }
                true
            }
            "text" => {
                // An interpolated literal is built as a component whose dynamic
                // parts are sourced from storage by NBT path, not spliced into a
                // quoted string. This keeps runtime values (which may contain `"`
                // or `\`) from breaking the surrounding component.
                if let Some(IrExpr {
                    kind:
                        IrExprKind::InterpolatedString {
                            template,
                            placeholders,
                        },
                    ..
                }) = args.first()
                {
                    self.compile_text_interpolation(
                        function,
                        depth,
                        target,
                        template,
                        placeholders,
                        lines,
                    );
                    return true;
                }
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                if let Some(arg) = args.first() {
                    let text_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                    self.compile_expr_into_slot(function, depth, arg, &text_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.text set from storage {}:runtime {}",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        text_slot.storage_path()
                    ));
                }
                true
            }
            "summon" => {
                let summon_target = if target.storage_path() == "__void" {
                    local_slot(depth, &function.name, &self.new_temp(), &Type::EntityRef)
                } else {
                    target.clone()
                };
                let payload_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let entity_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                if args.first().is_some_and(|arg| arg.ty == Type::EntityDef) {
                    let spec_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::EntityDef);
                    self.compile_expr_into_slot(function, depth, &args[0], &spec_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}.id",
                        self.namespace,
                        entity_slot.storage_path(),
                        self.namespace,
                        spec_slot.storage_path()
                    ));
                    lines.push(format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}.nbt",
                        self.namespace,
                        payload_slot.storage_path(),
                        self.namespace,
                        spec_slot.storage_path()
                    ));
                } else {
                    if let Some(arg) = args.first() {
                        self.compile_expr_into_slot(function, depth, arg, &entity_slot, lines);
                    }
                    if let Some(arg) = args.get(1) {
                        self.compile_value_as_nbt(function, depth, arg, &payload_slot, lines);
                    } else {
                        lines.push(format!(
                            "data modify storage {}:runtime {} set value {{}}",
                            self.namespace,
                            payload_slot.storage_path()
                        ));
                    }
                }
                if args.first().is_some_and(|arg| arg.ty == Type::EntityDef) {
                    lines.push(format!(
                        "execute unless data storage {}:runtime {} run data modify storage {}:runtime {} set value {{}}",
                        self.namespace,
                        payload_slot.storage_path(),
                        self.namespace,
                        payload_slot.storage_path()
                    ));
                } else {
                    // handled above for the string overload
                }
                let capture_tag = format!("mcfc_summon_capture_{}", self.new_temp());
                let ref_tag = format!("mcfc_summon_ref_{}", self.new_temp());
                lines.push(format!("tag @e[tag={}] remove {}", ref_tag, ref_tag));
                lines.push(format!(
                    "tag @e[tag={}] remove {}",
                    capture_tag, capture_tag
                ));
                lines.push(format!(
                    "execute unless data storage {}:runtime {}.Tags[] run data modify storage {}:runtime {}.Tags set value []",
                    self.namespace,
                    payload_slot.storage_path(),
                    self.namespace,
                    payload_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.Tags append value {}",
                    self.namespace,
                    payload_slot.storage_path(),
                    quoted(&capture_tag)
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.Tags append value {}",
                    self.namespace,
                    payload_slot.storage_path(),
                    quoted(&ref_tag)
                ));
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                lines.push(format!(
                    "data modify storage {}:runtime {}.entity set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    entity_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.data set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    payload_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    "summon $(entity) ~ ~ ~ $(data)".to_string(),
                ));
                self.write_query_slot(
                    &summon_target,
                    "",
                    &format!("@e[tag={},sort=nearest,limit=1]", ref_tag),
                    lines,
                );
                lines.push(self.query_command(
                    &summon_target,
                    format!("tag $(selector) remove {}", capture_tag),
                    true,
                ));
                self.stabilize_entity_ref(&summon_target, lines);
                lines.push(format!("tag @e[tag={ref_tag}] remove {ref_tag}"));
                true
            }
            "teleport" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let destination_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &args[1].ty);
                self.compile_expr_into_slot(function, depth, &args[1], &destination_slot, lines);
                match args[1].ty {
                    Type::EntityRef | Type::PlayerRef | Type::EntitySet => {
                        lines.push(format!(
                            "data modify storage {}:runtime {}.dest set from storage {}:runtime {}.selector",
                            self.namespace,
                            target_slot.storage_path(),
                            self.namespace,
                            destination_slot.storage_path()
                        ));
                        lines.push(self.query_command(
                            &target_slot,
                            "teleport $(selector) $(dest)".to_string(),
                            true,
                        ));
                    }
                    Type::BlockRef => {
                        lines.push(format!(
                            "data modify storage {}:runtime {}.dest set from storage {}:runtime {}.pos",
                            self.namespace,
                            target_slot.storage_path(),
                            self.namespace,
                            destination_slot.storage_path()
                        ));
                        lines.push(self.query_command(
                            &target_slot,
                            "teleport $(selector) $(dest)".to_string(),
                            true,
                        ));
                    }
                    _ => {}
                }
                true
            }
            "damage" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let amount_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                self.compile_expr_into_slot(function, depth, &args[1], &amount_slot, lines);
                lines.push(format!(
                    "execute store result storage {}:runtime {}.amount int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target_slot.storage_path(),
                    amount_slot.numeric_name()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "damage $(selector) $(amount)".to_string(),
                    true,
                ));
                true
            }
            "heal" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let amount_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                self.compile_expr_into_slot(function, depth, &args[1], &amount_slot, lines);
                let health_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                lines.push(self.query_command(
                    &target_slot,
                    format!(
                        "execute store result score {} mcfc run data get entity $(selector) Health 1",
                        health_slot.numeric_name()
                    ),
                    true,
                ));
                lines.push(format!(
                    "scoreboard players operation {} mcfc += {} mcfc",
                    health_slot.numeric_name(),
                    amount_slot.numeric_name()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    format!(
                        "execute store result entity $(selector) Health float 1 run scoreboard players get {} mcfc",
                        health_slot.numeric_name()
                    ),
                    true,
                ));
                true
            }
            "give" | "clear" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let item_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[1], &item_slot, lines);
                let count_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                self.compile_expr_into_slot(function, depth, &args[2], &count_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.item set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    item_slot.storage_path()
                ));
                lines.push(format!(
                    "execute store result storage {}:runtime {}.count int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target_slot.storage_path(),
                    count_slot.numeric_name()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    format!("{} $(selector) $(item) $(count)", callee),
                    true,
                ));
                true
            }
            "loot_give" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let table_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[1], &table_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.table set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    table_slot.storage_path()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "loot give $(selector) loot $(table)".to_string(),
                    true,
                ));
                true
            }
            "loot_insert" | "loot_spawn" | "setblock" => {
                let block_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &block_slot, lines);
                if callee == "setblock" && args[1].ty == Type::BlockDef {
                    let block_string_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                    let block_data_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                    self.compile_block_def_spec_string(
                        function,
                        depth,
                        &args[1],
                        &block_string_slot,
                        lines,
                    );
                    lines.push(format!(
                        "data modify storage {}:runtime {}.block set from storage {}:runtime {}",
                        self.namespace,
                        block_slot.storage_path(),
                        self.namespace,
                        block_string_slot.storage_path()
                    ));
                    lines.push(self.block_command(
                        &block_slot,
                        "setblock $(pos) $(block)".to_string(),
                        true,
                    ));
                    self.compile_expr_into_slot(function, depth, &args[1], &block_data_slot, lines);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.data set from storage {}:runtime {}.nbt",
                        self.namespace,
                        block_slot.storage_path(),
                        self.namespace,
                        block_data_slot.storage_path()
                    ));
                    lines.push(self.block_command(
                        &block_slot,
                        "data merge block $(pos) $(data)".to_string(),
                        true,
                    ));
                } else {
                    let value_slot =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                    self.compile_expr_into_slot(function, depth, &args[1], &value_slot, lines);
                    let field = if callee == "setblock" {
                        "block"
                    } else {
                        "table"
                    };
                    lines.push(format!(
                        "data modify storage {}:runtime {}.{} set from storage {}:runtime {}",
                        self.namespace,
                        block_slot.storage_path(),
                        field,
                        self.namespace,
                        value_slot.storage_path()
                    ));
                    let command = match callee {
                        "loot_insert" => "loot insert $(pos) loot $(table)".to_string(),
                        "loot_spawn" => "loot spawn $(pos) loot $(table)".to_string(),
                        _ => "setblock $(pos) $(block)".to_string(),
                    };
                    lines.push(self.block_command(&block_slot, command, true));
                }
                true
            }
            "fill" => {
                let from_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                let to_slot = local_slot(depth, &function.name, &self.new_temp(), &args[1].ty);
                let block_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &from_slot, lines);
                self.compile_expr_into_slot(function, depth, &args[1], &to_slot, lines);
                if args[2].ty == Type::BlockDef {
                    self.compile_block_def_spec_string(
                        function,
                        depth,
                        &args[2],
                        &block_slot,
                        lines,
                    );
                } else {
                    self.compile_expr_into_slot(function, depth, &args[2], &block_slot, lines);
                }
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                lines.push(format!(
                    "data modify storage {}:runtime {}.from set from storage {}:runtime {}.pos",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    from_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.to set from storage {}:runtime {}.pos",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    to_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.block set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    block_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    "fill $(from) $(to) $(block)".to_string(),
                ));
                true
            }
            "tellraw" | "title" | "actionbar" => {
                self.compile_display_builtin(function, depth, callee, args, lines);
                true
            }
            "debug" => {
                self.compile_debug_builtin(function, depth, args, lines);
                true
            }
            "sidebar_title" | "sidebar_line" | "sidebar_remove_line" | "sidebar_clear" => {
                self.compile_sidebar(function, depth, None, callee, args, lines);
                true
            }
            "log_debug" | "log_info" | "log_warn" | "log_error" | "log_level" | "log_dump"
            | "assert_fail" => {
                self.compile_log(function, depth, callee, &args[0], lines);
                true
            }
            "debug_marker" => {
                self.compile_debug_marker_builtin(function, depth, args, lines);
                true
            }
            "debug_entity" => {
                self.compile_debug_entity_builtin(function, depth, args, lines);
                true
            }
            "bossbar_add" | "bossbar_name" => {
                self.compile_bossbar_text_builtin(function, depth, callee, args, lines);
                true
            }
            "bossbar_remove" | "bossbar_value" | "bossbar_max" | "bossbar_visible" => {
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                let command = match callee {
                    "bossbar_remove" => "bossbar remove $(id)".to_string(),
                    "bossbar_value" => {
                        let value_slot =
                            local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                        self.compile_expr_into_slot(function, depth, &args[1], &value_slot, lines);
                        lines.push(format!(
                            "execute store result storage {}:runtime {}.value int 1 run scoreboard players get {} mcfc",
                            self.namespace,
                            macro_slot.storage_path(),
                            value_slot.numeric_name()
                        ));
                        "bossbar set $(id) value $(value)".to_string()
                    }
                    "bossbar_max" => {
                        let value_slot =
                            local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                        self.compile_expr_into_slot(function, depth, &args[1], &value_slot, lines);
                        lines.push(format!(
                            "execute store result storage {}:runtime {}.value int 1 run scoreboard players get {} mcfc",
                            self.namespace,
                            macro_slot.storage_path(),
                            value_slot.numeric_name()
                        ));
                        "bossbar set $(id) max $(value)".to_string()
                    }
                    _ => {
                        let visible_slot =
                            local_slot(depth, &function.name, &self.new_temp(), &Type::Bool);
                        self.compile_expr_into_slot(
                            function,
                            depth,
                            &args[1],
                            &visible_slot,
                            lines,
                        );
                        lines.push(format!(
                            "execute store result storage {}:runtime {}.visible int 1 run scoreboard players get {} mcfc",
                            self.namespace,
                            macro_slot.storage_path(),
                            visible_slot.numeric_name()
                        ));
                        "bossbar set $(id) visible $(visible)".to_string()
                    }
                };
                lines.push(self.inline_macro_command(macro_slot.storage_path(), command));
                true
            }
            "bossbar_players" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[1].ty);
                self.compile_expr_into_slot(function, depth, &args[1], &target_slot, lines);
                let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    id_slot.storage_path()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "bossbar set $(id) players $(selector)".to_string(),
                    true,
                ));
                true
            }
            "playsound" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[2].ty);
                self.compile_expr_into_slot(function, depth, &args[2], &target_slot, lines);
                let sound_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                let category_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &sound_slot, lines);
                self.compile_expr_into_slot(function, depth, &args[1], &category_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.sound set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    sound_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.category set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    category_slot.storage_path()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "playsound $(sound) $(category) $(selector) ~ ~ ~ 1 1 1".to_string(),
                    true,
                ));
                true
            }
            "stopsound" => {
                let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
                self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
                let category_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                let sound_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[1], &category_slot, lines);
                self.compile_expr_into_slot(function, depth, &args[2], &sound_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.category set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    category_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.sound set from storage {}:runtime {}",
                    self.namespace,
                    target_slot.storage_path(),
                    self.namespace,
                    sound_slot.storage_path()
                ));
                lines.push(self.query_command(
                    &target_slot,
                    "stopsound $(selector) $(category) $(sound)".to_string(),
                    true,
                ));
                true
            }
            "particle" => {
                self.compile_particle_builtin(function, depth, args, lines);
                true
            }
            _ => false,
        }
    }

    fn compile_display_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        callee: &str,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
        self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
        if let IrExprKind::String(text) = &args[1].kind {
            let component = display_text_components(text).unwrap_or_else(|| quoted(text));
            let command = match callee {
                "tellraw" => format!("tellraw $(selector) {}", component),
                "title" => format!("title $(selector) title {}", component),
                _ => self.actionbar_command(&component, args),
            };
            lines.push(self.query_command(&target_slot, command, true));
            return;
        }
        if args[1].ty == Type::TextDef {
            let message_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::TextDef);
            self.compile_expr_into_slot(function, depth, &args[1], &message_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.message set from storage {}:runtime {}",
                self.namespace,
                target_slot.storage_path(),
                self.namespace,
                message_slot.storage_path()
            ));
            let command = match callee {
                "tellraw" => "tellraw $(selector) $(message)".to_string(),
                "title" => "title $(selector) title $(message)".to_string(),
                _ => self.actionbar_command("$(message)", args),
            };
            lines.push(self.query_command(&target_slot, command, true));
            return;
        }
        if let IrExprKind::InterpolatedString {
            template,
            placeholders,
        } = &args[1].kind
        {
            self.compile_interpolated_display_builtin(
                function,
                depth,
                callee,
                &target_slot,
                template,
                placeholders,
                args,
                lines,
            );
            return;
        }
        let message_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[1], &message_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.message set from storage {}:runtime {}",
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            message_slot.storage_path()
        ));
        let command = match callee {
            "tellraw" => "tellraw $(selector) [\"$(message)\"]".to_string(),
            "title" => "title $(selector) title [\"$(message)\"]".to_string(),
            _ => self.actionbar_command("[\"$(message)\"]", args),
        };
        lines.push(self.query_command(&target_slot, command, true));
    }

    /// Actionbars follow the Smithed Actionbar library's priorities, so packs
    /// don't overwrite each other's messages. See `emit_actionbar_runtime`.
    fn actionbar_command(&mut self, component: &str, args: &[IrExpr]) -> String {
        self.uses_actionbar = true;
        let priority = match args.get(2).map(|arg| &arg.kind) {
            Some(IrExprKind::String(priority)) => priority.as_str(),
            _ => "notification",
        };
        // A list, so `$(json)` in `show` substitutes it as quoted SNBT.
        format!(
            "execute as $(selector) run function {}:generated/actionbar/show {{json:[{component}],priority:\"{priority}\"}}",
            self.namespace
        )
    }

    /// With Smithed Actionbar loaded, messages go through it. Otherwise this is
    /// a port of its algorithm on the same objectives, so MCFC packs still
    /// coordinate: a message shows unless the player has a higher-priority one
    /// (or an `override`) frozen on screen, and freezes for 20 ticks.
    fn emit_actionbar_runtime(&mut self) {
        let ns = self.namespace.clone();
        let loaded = "score $default.freeze smithed.actionbar.const matches -2147483648..";
        let files = [
            (
                "show",
                format!(
                    "$data modify storage smithed.actionbar:input message set value {{json:$(json),priority:\"$(priority)\"}}
execute if {loaded} run return run function #smithed.actionbar:message
scoreboard players add @s smithed.actionbar.priority 0
execute if data storage smithed.actionbar:input message{{priority:\"override\"}} run scoreboard players set #actionbar mcfc 1
execute if data storage smithed.actionbar:input message{{priority:\"notification\"}} run scoreboard players set #actionbar mcfc 2
execute if data storage smithed.actionbar:input message{{priority:\"conditional\"}} run scoreboard players set #actionbar mcfc 3
execute if data storage smithed.actionbar:input message{{priority:\"persistent\"}} run scoreboard players set #actionbar mcfc 4
execute if score @s smithed.actionbar.priority matches 0 run return run function {ns}:generated/actionbar/display
execute unless score @s smithed.actionbar.priority matches 1 if score #actionbar mcfc <= @s smithed.actionbar.priority run function {ns}:generated/actionbar/display
"
                ),
            ),
            (
                "display",
                "title @s actionbar {\"storage\":\"smithed.actionbar:input\",\"nbt\":\"message.json[0]\",\"interpret\":true}
scoreboard players set @s smithed.actionbar.freeze 20
scoreboard players operation @s smithed.actionbar.priority = #actionbar mcfc
"
                .to_string(),
            ),
            (
                "tick",
                // Every MCFC pack runs this; the gametime stamp makes it count once a tick.
                format!(
                    "execute if {loaded} run return 0
execute store result score #now smithed.actionbar.freeze run time query gametime
execute if score #now smithed.actionbar.freeze = #last smithed.actionbar.freeze run return 0
scoreboard players operation #last smithed.actionbar.freeze = #now smithed.actionbar.freeze
scoreboard players set @a[scores={{smithed.actionbar.freeze=1}}] smithed.actionbar.priority 0
scoreboard players remove @a[scores={{smithed.actionbar.freeze=1..}}] smithed.actionbar.freeze 1
"
                ),
            ),
        ];
        for (name, body) in files {
            self.files.insert(
                format!("data/{ns}/function/generated/actionbar/{name}.mcfunction"),
                body,
            );
        }
        // Makes `#smithed.actionbar:message` valid without the library; tags merge with it.
        self.files.insert(
            "data/smithed.actionbar/tags/function/message.json".to_string(),
            "{
  \"values\": []
}
"
            .to_string(),
        );
        let setup = format!("data/{ns}/function/generated/setup.mcfunction");
        if let Some(body) = self.files.get_mut(&setup) {
            body.push_str(
                "scoreboard objectives add smithed.actionbar.priority dummy
scoreboard objectives add smithed.actionbar.freeze dummy
",
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_interpolated_display_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        callee: &str,
        target_slot: &SlotRef,
        template: &str,
        placeholders: &[IrMacroPlaceholder],
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        self.macro_counter += 1;
        let macro_id = self.macro_counter;
        let storage_base = macro_storage_base(depth, &function.name, macro_id);
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            storage_base,
            self.namespace,
            target_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
            self.namespace,
            storage_base,
            self.namespace,
            target_slot.storage_path()
        ));
        self.write_macro_placeholder_values(function, depth, &storage_base, placeholders, lines);

        let rewritten = rewrite_macro_template(template, placeholders);
        let component = display_text_components(&rewritten)
            .unwrap_or_else(|| format!("[{}]", quoted(&rewritten)));
        let command = match callee {
            "tellraw" => format!("$(prefix)tellraw $(selector) {}", component),
            "title" => format!("$(prefix)title $(selector) title {}", component),
            _ => format!("$(prefix){}", self.actionbar_command(&component, args)),
        };
        let namespace = self.namespace.clone();
        let macro_name = self.ensure_inline_macro(command);
        lines.push(format!(
            "function {}:{} with storage {}:runtime {}",
            namespace, macro_name, namespace, storage_base
        ));
    }

    fn compile_debug_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        if let IrExprKind::String(message) = &args[0].kind {
            lines.push(format!(
                "tellraw @a [{{\"text\":\"[MCFC debug] \",\"color\":\"gold\"}},{{\"text\":{},\"color\":\"white\"}}]",
                quoted(message)
            ));
            return;
        }

        let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        let message_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[0], &message_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.message set from storage {}:runtime {}",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            message_slot.storage_path()
        ));
        lines.push(self.inline_macro_command(
            macro_slot.storage_path(),
            "tellraw @a [{\"text\":\"[MCFC debug] \",\"color\":\"gold\"},{\"text\":\"$(message)\",\"color\":\"white\"}]".to_string(),
        ));
    }

    fn compile_debug_marker_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let pos_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
        self.compile_expr_into_slot(function, depth, &args[0], &pos_slot, lines);
        let label_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[1], &label_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.label set from storage {}:runtime {}",
            self.namespace,
            pos_slot.storage_path(),
            self.namespace,
            label_slot.storage_path()
        ));
        lines.push(self.block_command(
            &pos_slot,
            "tellraw @a [{\"text\":\"[MCFC marker] \",\"color\":\"aqua\"},{\"text\":\"$(label) at $(pos)\",\"color\":\"white\"}]".to_string(),
            true,
        ));
        lines.push(self.block_command(
            &pos_slot,
            "particle minecraft:happy_villager $(pos) 0.35 0.75 0.35 0 40 force @a".to_string(),
            true,
        ));
        lines.push(self.block_command(
            &pos_slot,
            "playsound minecraft:block.note_block.pling master @a $(pos) 1 1.6 1".to_string(),
            true,
        ));

        if let Some(block) = args.get(2) {
            let block_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
            self.compile_expr_into_slot(function, depth, block, &block_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.block set from storage {}:runtime {}",
                self.namespace,
                pos_slot.storage_path(),
                self.namespace,
                block_slot.storage_path()
            ));
            lines.push(self.block_command(
                &pos_slot,
                "setblock $(pos) $(block) replace".to_string(),
                true,
            ));
        }
    }

    fn compile_debug_entity_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let target_slot = local_slot(depth, &function.name, &self.new_temp(), &args[0].ty);
        self.compile_expr_into_slot(function, depth, &args[0], &target_slot, lines);
        let label_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[1], &label_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.label set from storage {}:runtime {}",
            self.namespace,
            target_slot.storage_path(),
            self.namespace,
            label_slot.storage_path()
        ));
        lines.push(self.query_command(
            &target_slot,
            "execute if entity $(selector) run tellraw @a [{\"text\":\"[MCFC entity] found \",\"color\":\"green\"},{\"text\":\"$(label) \"},{\"selector\":\"$(selector)\"}]".to_string(),
            true,
        ));
        lines.push(self.query_command(
            &target_slot,
            "execute unless entity $(selector) run tellraw @a [{\"text\":\"[MCFC entity] missing \",\"color\":\"red\"},{\"text\":\"$(label)\"}]".to_string(),
            true,
        ));
        lines.push(self.query_command(
            &target_slot,
            "execute if entity $(selector) run effect give $(selector) minecraft:glowing 3 0 true".to_string(),
            true,
        ));
    }

    fn compile_bossbar_text_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        callee: &str,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        let id_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[0], &id_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.id set from storage {}:runtime {}",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            id_slot.storage_path()
        ));
        if let IrExprKind::String(text) = &args[1].kind {
            let component = selector_text_components(text).unwrap_or_else(|| quoted(text));
            let command = if callee == "bossbar_add" {
                format!("bossbar add $(id) {}", component)
            } else {
                format!("bossbar set $(id) name {}", component)
            };
            lines.push(self.inline_macro_command(macro_slot.storage_path(), command));
            return;
        }
        if args[1].ty == Type::TextDef {
            let name_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::TextDef);
            self.compile_expr_into_slot(function, depth, &args[1], &name_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
                self.namespace,
                macro_slot.storage_path(),
                self.namespace,
                name_slot.storage_path()
            ));
            let command = if callee == "bossbar_add" {
                "bossbar add $(id) $(name)".to_string()
            } else {
                "bossbar set $(id) name $(name)".to_string()
            };
            lines.push(self.inline_macro_command(macro_slot.storage_path(), command));
            return;
        }
        let name_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[1], &name_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.name set from storage {}:runtime {}",
            self.namespace,
            macro_slot.storage_path(),
            self.namespace,
            name_slot.storage_path()
        ));
        let command = if callee == "bossbar_add" {
            "bossbar add $(id) [\"$(name)\"]".to_string()
        } else {
            "bossbar set $(id) name [\"$(name)\"]".to_string()
        };
        lines.push(self.inline_macro_command(macro_slot.storage_path(), command));
    }

    fn compile_particle_builtin(
        &mut self,
        function: &IrFunction,
        depth: usize,
        args: &[IrExpr],
        lines: &mut Vec<String>,
    ) {
        let pos_slot = local_slot(depth, &function.name, &self.new_temp(), &args[1].ty);
        self.compile_expr_into_slot(function, depth, &args[1], &pos_slot, lines);
        let particle_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
        self.compile_expr_into_slot(function, depth, &args[0], &particle_slot, lines);
        let count_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
        if let Some(arg) = args.get(2) {
            self.compile_expr_into_slot(function, depth, arg, &count_slot, lines);
        } else {
            lines.push(format!(
                "scoreboard players set {} mcfc 1",
                count_slot.numeric_name()
            ));
        }
        if let Some(viewers) = args.get(3) {
            let viewer_slot = local_slot(depth, &function.name, &self.new_temp(), &viewers.ty);
            self.compile_expr_into_slot(function, depth, viewers, &viewer_slot, lines);
            lines.push(format!(
                "data modify storage {}:runtime {}.particle set from storage {}:runtime {}",
                self.namespace,
                viewer_slot.storage_path(),
                self.namespace,
                particle_slot.storage_path()
            ));
            lines.push(format!(
                "data modify storage {}:runtime {}.pos set from storage {}:runtime {}.pos",
                self.namespace,
                viewer_slot.storage_path(),
                self.namespace,
                pos_slot.storage_path()
            ));
            lines.push(format!(
                "execute store result storage {}:runtime {}.count int 1 run scoreboard players get {} mcfc",
                self.namespace,
                viewer_slot.storage_path(),
                count_slot.numeric_name()
            ));
            lines.push(self.query_command(
                &viewer_slot,
                "particle $(particle) $(pos) 0 0 0 0 $(count) force $(selector)".to_string(),
                true,
            ));
            return;
        }
        lines.push(format!(
            "data modify storage {}:runtime {}.particle set from storage {}:runtime {}",
            self.namespace,
            pos_slot.storage_path(),
            self.namespace,
            particle_slot.storage_path()
        ));
        lines.push(format!(
            "execute store result storage {}:runtime {}.count int 1 run scoreboard players get {} mcfc",
            self.namespace,
            pos_slot.storage_path(),
            count_slot.numeric_name()
        ));
        lines.push(self.block_command(
            &pos_slot,
            "particle $(particle) $(pos) 0 0 0 0 $(count) force".to_string(),
            true,
        ));
    }

    fn is_storage_state_path(&self, path: &IrPathExpr) -> bool {
        if !matches!(path.segments.first(), Some(PathSegment::Field(name)) if name == "state") {
            return false;
        }
        let is_player = path.base.ref_kind == RefKind::Player;
        let mut fields = Vec::new();
        for segment in path.segments.iter().skip(1) {
            let PathSegment::Field(field) = segment else {
                break;
            };
            fields.push(field.as_str());
            if self
                .state_storage_paths
                .contains(&(is_player, fields.join(".")))
            {
                return true;
            }
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    fn compile_storage_state_access(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        source: Option<&SlotRef>,
        target: Option<&SlotRef>,
        lines: &mut Vec<String>,
    ) {
        let key_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        let key_path = key_slot.storage_path();
        lines.push(format!(
            "data modify storage {}:runtime {} set value {{u0:0,u1:0,u2:0,u3:0}}",
            self.namespace, key_path
        ));
        for member in ["prefix", "selector"] {
            lines.push(format!(
                "data modify storage {}:runtime {}.{} set from storage {}:runtime {}.{}",
                self.namespace,
                key_path,
                member,
                self.namespace,
                base_slot.storage_path(),
                member
            ));
        }
        for index in 0..4 {
            lines.push(self.query_command(
                base_slot,
                format!(
                    "data modify storage {}:runtime {}.u{} set from entity $(selector) UUID[{}]",
                    self.namespace, key_path, index, index
                ),
                true,
            ));
        }
        let owner = if path.base.ref_kind == RefKind::Player {
            "players"
        } else {
            "entities"
        };
        let state_path = render_nbt_path_segments(&path.segments[1..]);
        let owner_path = format!("{}.\"$(u0)_$(u1)_$(u2)_$(u3)\"", owner);
        let storage_path = format!("{}.{}", owner_path, state_path);
        let command = if let Some(source) = source {
            lines.push(format!(
                "execute unless data storage {}:state {} run data modify storage {}:state {} set value {{}}",
                self.namespace, owner, self.namespace, owner
            ));
            let mut parent_paths = vec![owner_path.clone()];
            let mut nested = owner_path.clone();
            for segment in path.segments.iter().skip(1).take(path.segments.len() - 2) {
                if let PathSegment::Field(field) = segment {
                    nested.push('.');
                    nested.push_str(field);
                    parent_paths.push(nested.clone());
                }
            }
            for parent_path in parent_paths {
                lines.push(self.inline_macro_command(
                    key_path,
                    format!(
                        "$(prefix)execute if entity $(selector) unless data storage {}:state {} run data modify storage {}:state {} set value {{}}",
                        self.namespace, parent_path, self.namespace, parent_path
                    ),
                ));
            }
            format!(
                "$(prefix)execute if entity $(selector) run data modify storage {}:state {} set from storage {}:runtime {}",
                self.namespace,
                storage_path,
                self.namespace,
                source.storage_path()
            )
        } else if let Some(target) = target {
            match path.ty {
                Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => {
                    lines.push(format!(
                        "scoreboard players set {} mcfc 0",
                        target.numeric_name()
                    ));
                    format!(
                        "$(prefix)execute if entity $(selector) store result score {} mcfc run data get storage {}:state {} 1",
                        target.numeric_name(),
                        self.namespace,
                        storage_path
                    )
                }
                _ => {
                    let default = match path.ty {
                        Type::String => "\"\"",
                        Type::Float => "0.0f",
                        _ => "{}",
                    };
                    lines.push(format!(
                        "data modify storage {}:runtime {} set value {}",
                        self.namespace,
                        target.storage_path(),
                        default
                    ));
                    format!(
                        "$(prefix)execute if entity $(selector) run data modify storage {}:runtime {} set from storage {}:state {}",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        storage_path
                    )
                }
            }
        } else {
            return;
        };
        lines.push(self.inline_macro_command(key_path, command));
    }

    fn try_compile_player_path_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        value: &IrExpr,
        value_slot: &SlotRef,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(PathSegment::Field(first)) = path.segments.first() else {
            return false;
        };
        match first.as_str() {
            "nbt" if path.base.ref_kind == RefKind::Player => true,
            "state" => {
                if path.segments.len() == 1 {
                    return false;
                }
                if self.is_storage_state_path(path) {
                    if matches!(value.ty, Type::EntityRef | Type::PlayerRef) {
                        self.stabilize_entity_ref(value_slot, lines);
                    }
                    self.compile_storage_state_access(
                        function,
                        depth,
                        base_slot,
                        path,
                        Some(value_slot),
                        None,
                        lines,
                    );
                    return true;
                }
                let objective = state_objective(path.base.ref_kind, &path.segments[1..]);
                let temp_name = self.new_temp();
                let temp_slot = local_slot(depth, &function.name, &temp_name, &Type::Int);
                self.compile_expr_into_slot(function, depth, value, &temp_slot, lines);
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "scoreboard players operation $(selector) {} = {} mcfc",
                        objective,
                        temp_slot.numeric_name()
                    ),
                    true,
                ));
                true
            }
            "tags" => {
                if path.base.ref_kind != RefKind::Player {
                    return false;
                }
                let tag = render_path_segments(&path.segments[1..]);
                let temp_name = self.new_temp();
                let temp_slot = local_slot(depth, &function.name, &temp_name, &Type::Bool);
                self.compile_expr_into_slot(function, depth, value, &temp_slot, lines);
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "execute if score {} mcfc matches 1 run tag $(selector) add {}",
                        temp_slot.numeric_name(),
                        tag
                    ),
                    true,
                ));
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "execute if score {} mcfc matches 0 run tag $(selector) remove {}",
                        temp_slot.numeric_name(),
                        tag
                    ),
                    true,
                ));
                true
            }
            "team" => {
                lines.push(format!(
                    "data modify storage {}:runtime {}.team set from storage {}:runtime {}",
                    self.namespace,
                    base_slot.storage_path(),
                    self.namespace,
                    value_slot.storage_path()
                ));
                lines.push(self.query_command(
                    base_slot,
                    "team join $(team) $(selector)".to_string(),
                    true,
                ));
                true
            }
            "inventory" | "hotbar" => {
                let Some((namespace, index)) = player_item_slot_index_from_path(path) else {
                    return false;
                };
                let slot_handle =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::ItemSlot);
                self.load_player_item_slot(
                    function,
                    depth,
                    base_slot,
                    namespace,
                    index,
                    &slot_handle,
                    lines,
                );
                if path.segments.len() == 2 {
                    self.populate_item_slot_from_item_def(
                        function,
                        depth,
                        value,
                        &slot_handle,
                        lines,
                    );
                } else if let Some(PathSegment::Field(field)) = path.segments.get(2) {
                    if field == "name" {
                        lines.push(format!(
                            "data modify storage {}:runtime {}.nbt.display.Name set from storage {}:runtime {}",
                            self.namespace,
                            slot_handle.storage_path(),
                            self.namespace,
                            value_slot.storage_path()
                        ));
                    } else {
                        let rendered = self.render_storage_path(
                            function,
                            depth,
                            slot_handle.storage_path().to_string(),
                            &Type::ItemSlot,
                            &path.segments[2..],
                            &path.segment_types[2..],
                            lines,
                        );
                        lines.push(self.storage_path_command(
                            format!(
                                "data modify storage {}:runtime {} set from storage {}:runtime {}",
                                self.namespace,
                                rendered.path,
                                self.namespace,
                                value_slot.storage_path()
                            ),
                            rendered.macro_storage,
                        ));
                    }
                }
                self.sync_item_slot_handle(function, depth, &slot_handle, lines);
                true
            }
            "mainhand" | "offhand" | "head" | "chest" | "legs" | "feet" => self
                .compile_player_mainhand_assign(
                    function,
                    depth,
                    base_slot,
                    first,
                    &path.segments[1..],
                    value,
                    lines,
                ),
            _ => false,
        }
    }

    fn try_compile_player_path_read(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(PathSegment::Field(first)) = path.segments.first() else {
            return false;
        };
        match first.as_str() {
            "position" => {
                let pos_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::BlockRef);
                self.compose_entity_position_slot(base_slot, &pos_slot, lines);
                if path.segments.len() == 1 {
                    lines.push(format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        pos_slot.storage_path()
                    ));
                } else {
                    let path_text = render_nbt_path_segments(normalize_runtime_nbt_segments(
                        &Type::BlockRef,
                        &path.segments[1..],
                    ));
                    lines.push(self.block_command(
                        &pos_slot,
                        format!(
                            "data modify storage {}:runtime {} set from block $(pos) {}",
                            self.namespace,
                            target.storage_path(),
                            path_text
                        ),
                        true,
                    ));
                }
                true
            }
            "nbt" => {
                if path.base.ref_kind != RefKind::Player {
                    return false;
                }
                let path_text = render_nbt_path_segments(&path.segments[1..]);
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "data modify storage {}:runtime {} set from entity $(selector) {}",
                        self.namespace,
                        target.storage_path(),
                        path_text
                    ),
                    true,
                ));
                true
            }
            "state" => {
                if path.segments.len() == 1 {
                    return false;
                }
                if self.is_storage_state_path(path) {
                    self.compile_storage_state_access(
                        function,
                        depth,
                        base_slot,
                        path,
                        None,
                        Some(target),
                        lines,
                    );
                    return true;
                }
                let objective = state_objective(path.base.ref_kind, &path.segments[1..]);
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "scoreboard players operation {} mcfc = $(selector) {}",
                        target.numeric_name(),
                        objective
                    ),
                    true,
                ));
                true
            }
            "tags" => {
                if path.base.ref_kind != RefKind::Player {
                    return false;
                }
                let tag = render_path_segments(&path.segments[1..]);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value 0",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "execute as $(selector) if entity @s[tag={}] run data modify storage {}:runtime {} set value 1",
                        tag,
                        self.namespace,
                        target.storage_path()
                    ),
                    true,
                ));
                true
            }
            "team" => {
                lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.team",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
                true
            }
            "inventory" | "hotbar" => {
                let Some((namespace, index)) = player_item_slot_index_from_path(path) else {
                    return false;
                };
                let slot_handle =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::ItemSlot);
                self.load_player_item_slot(
                    function,
                    depth,
                    base_slot,
                    namespace,
                    index,
                    &slot_handle,
                    lines,
                );
                if path.segments.len() == 2 {
                    lines.push(format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        slot_handle.storage_path()
                    ));
                } else if let Some(PathSegment::Field(field)) = path.segments.get(2) {
                    if field == "name" {
                        lines.push(format!(
                            "data modify storage {}:runtime {} set value \"\"",
                            self.namespace,
                            target.storage_path()
                        ));
                        lines.push(format!(
                            "execute if data storage {}:runtime {}.nbt.display.Name run data modify storage {}:runtime {} set from storage {}:runtime {}.nbt.display.Name",
                            self.namespace,
                            slot_handle.storage_path(),
                            self.namespace,
                            target.storage_path(),
                            self.namespace,
                            slot_handle.storage_path()
                        ));
                    } else {
                        let rendered = self.render_storage_path(
                            function,
                            depth,
                            slot_handle.storage_path().to_string(),
                            &Type::ItemSlot,
                            &path.segments[2..],
                            &path.segment_types[2..],
                            lines,
                        );
                        self.compile_storage_read_from_path(rendered, &path.ty, target, lines);
                    }
                }
                true
            }
            "mainhand" | "offhand" | "head" | "chest" | "legs" | "feet" => {
                let slot_handle =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::ItemSlot);
                self.load_entity_equipment_slot(base_slot, first, &slot_handle, lines);
                if path.segments.len() == 1 {
                    lines.push(format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        slot_handle.storage_path()
                    ));
                } else if let Some(PathSegment::Field(field)) = path.segments.get(1) {
                    if field == "name" {
                        lines.push(format!(
                            "data modify storage {}:runtime {} set value \"\"",
                            self.namespace,
                            target.storage_path()
                        ));
                        lines.push(format!(
                            "execute if data storage {}:runtime {}.nbt.display.Name run data modify storage {}:runtime {} set from storage {}:runtime {}.nbt.display.Name",
                            self.namespace,
                            slot_handle.storage_path(),
                            self.namespace,
                            target.storage_path(),
                            self.namespace,
                            slot_handle.storage_path()
                        ));
                    } else {
                        let rendered = self.render_storage_path(
                            function,
                            depth,
                            slot_handle.storage_path().to_string(),
                            &Type::ItemSlot,
                            &path.segments[1..],
                            &path.segment_types[1..],
                            lines,
                        );
                        self.compile_storage_read_from_path(rendered, &path.ty, target, lines);
                    }
                }
                true
            }
            _ => false,
        }
    }

    fn compile_item_slot_path_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        value_slot: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if let Some(PathSegment::Field(field)) = path.segments.first() {
            if field == "name" {
                lines.push(format!(
                    "data modify storage {}:runtime {}.nbt.display.Name set from storage {}:runtime {}",
                    self.namespace,
                    base_slot.storage_path(),
                    self.namespace,
                    value_slot.storage_path()
                ));
            } else {
                let rendered = self.render_storage_path(
                    function,
                    depth,
                    base_slot.storage_path().to_string(),
                    &Type::ItemSlot,
                    &path.segments,
                    &path.segment_types,
                    lines,
                );
                lines.push(self.storage_path_command(
                    format!(
                        "data modify storage {}:runtime {} set from storage {}:runtime {}",
                        self.namespace,
                        rendered.path,
                        self.namespace,
                        value_slot.storage_path()
                    ),
                    rendered.macro_storage,
                ));
            }
            self.sync_item_slot_handle(function, depth, base_slot, lines);
        }
    }

    fn compile_item_slot_path_read(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        path: &IrPathExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if let Some(PathSegment::Field(field)) = path.segments.first()
            && field == "name"
        {
            lines.push(format!(
                "data modify storage {}:runtime {} set value \"\"",
                self.namespace,
                target.storage_path()
            ));
            lines.push(format!(
                    "execute if data storage {}:runtime {}.nbt.display.Name run data modify storage {}:runtime {} set from storage {}:runtime {}.nbt.display.Name",
                    self.namespace,
                    base_slot.storage_path(),
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    base_slot.storage_path()
                ));
            return;
        }
        let rendered = self.render_storage_path(
            function,
            depth,
            base_slot.storage_path().to_string(),
            &Type::ItemSlot,
            &path.segments,
            &path.segment_types,
            lines,
        );
        self.compile_storage_read_from_path(rendered, &path.ty, target, lines);
    }

    fn load_player_item_slot(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        namespace: &str,
        index: &crate::ast::Expr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            target.storage_path(),
            self.namespace,
            base_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
            self.namespace,
            target.storage_path(),
            self.namespace,
            base_slot.storage_path()
        ));
        match &index.kind {
            crate::ast::ExprKind::Int(logical_index) => {
                let slot_index = player_slot_nbt_index(namespace, *logical_index);
                lines.push(format!(
                    "data modify storage {}:runtime {}.logical_slot set value {}",
                    self.namespace,
                    target.storage_path(),
                    logical_index
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.slot set value {}",
                    self.namespace,
                    target.storage_path(),
                    slot_index
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.command_slot set value {}",
                    self.namespace,
                    target.storage_path(),
                    quoted(&player_item_command_slot(namespace, *logical_index))
                ));
            }
            _ => {
                self.compile_expr_to_macro_value(
                    function,
                    depth,
                    index,
                    &Type::Int,
                    target.storage_path(),
                    "logical_slot",
                    lines,
                );
                if namespace == "inventory" {
                    let slot_index =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                    lines.push(format!(
                        "execute store result score {} mcfc run data get storage {}:runtime {}.logical_slot 1",
                        slot_index.numeric_name(),
                        self.namespace,
                        target.storage_path()
                    ));
                    lines.push(format!(
                        "scoreboard players add {} mcfc 9",
                        slot_index.numeric_name()
                    ));
                    lines.push(format!(
                        "execute store result storage {}:runtime {}.slot int 1 run scoreboard players get {} mcfc",
                        self.namespace,
                        target.storage_path(),
                        slot_index.numeric_name()
                    ));
                } else {
                    lines.push(format!(
                        "data modify storage {}:runtime {}.slot set from storage {}:runtime {}.logical_slot",
                        self.namespace,
                        target.storage_path(),
                        self.namespace,
                        target.storage_path()
                    ));
                }
                lines.push(self.inline_macro_command(
                    target.storage_path(),
                    format!(
                        "data modify storage {}:runtime {}.command_slot set value \"{}.$(logical_slot)\"",
                        self.namespace,
                        target.storage_path(),
                        namespace
                    ),
                ));
            }
        }
        let slot_path = "Inventory[{Slot:$(slot)b}]";
        lines.push(format!(
            "data modify storage {}:runtime {}.exists set value 0",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.id set value \"\"",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.count set value 0",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.nbt set value {{}}",
            self.namespace,
            target.storage_path()
        ));
        lines.push(self.query_command(
            target,
            format!(
                "execute if data entity $(selector) {} run data modify storage {}:runtime {}.exists set value 1",
                slot_path,
                self.namespace,
                target.storage_path()
            ),
            true,
        ));
        lines.push(self.query_command(
            target,
            format!(
                "execute if data entity $(selector) {} run data modify storage {}:runtime {}.id set from entity $(selector) {}.id",
                slot_path,
                self.namespace,
                target.storage_path(),
                slot_path
            ),
            true,
        ));
        lines.push(self.query_command(
            target,
            format!(
                "execute if data entity $(selector) {} run execute store result storage {}:runtime {}.count int 1 run data get entity $(selector) {}.Count 1",
                slot_path,
                self.namespace,
                target.storage_path(),
                slot_path
            ),
            true,
        ));
        lines.push(self.query_command(
            target,
            format!(
                "execute if data entity $(selector) {} run data modify storage {}:runtime {}.nbt set from entity $(selector) {}",
                slot_path,
                self.namespace,
                target.storage_path(),
                slot_path
            ),
            true,
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.nbt.id",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.nbt.Count",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.nbt.Slot",
            self.namespace,
            target.storage_path()
        ));
    }

    fn load_entity_equipment_slot(
        &mut self,
        base_slot: &SlotRef,
        slot_name: &str,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            target.storage_path(),
            self.namespace,
            base_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
            self.namespace,
            target.storage_path(),
            self.namespace,
            base_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.command_slot set value {}",
            self.namespace,
            target.storage_path(),
            quoted(equipment_slot_name(slot_name))
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.exists set value 0",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.id set value \"\"",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.count set value 0",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.nbt set value {{}}",
            self.namespace,
            target.storage_path()
        ));
        for source_path in equipment_read_nbt_paths(slot_name) {
            lines.push(self.query_command(
                target,
                format!(
                    "execute if data entity $(selector) {} run data modify storage {}:runtime {}.exists set value 1",
                    source_path,
                    self.namespace,
                    target.storage_path()
                ),
                true,
            ));
            lines.push(self.query_command(
                target,
                format!(
                    "execute if data entity $(selector) {} run data modify storage {}:runtime {}.id set from entity $(selector) {}.id",
                    source_path,
                    self.namespace,
                    target.storage_path(),
                    source_path
                ),
                true,
            ));
            lines.push(self.query_command(
                target,
                format!(
                    "execute if data entity $(selector) {} run execute store result storage {}:runtime {}.count int 1 run data get entity $(selector) {}.Count 1",
                    source_path,
                    self.namespace,
                    target.storage_path(),
                    source_path
                ),
                true,
            ));
            lines.push(self.query_command(
                target,
                format!(
                    "execute if data entity $(selector) {} run data modify storage {}:runtime {}.nbt set from entity $(selector) {}",
                    source_path,
                    self.namespace,
                    target.storage_path(),
                    source_path
                ),
                true,
            ));
        }
        lines.push(format!(
            "data remove storage {}:runtime {}.nbt.id",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.nbt.Count",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.nbt.Slot",
            self.namespace,
            target.storage_path()
        ));
    }

    fn populate_item_slot_from_item_def(
        &mut self,
        function: &IrFunction,
        depth: usize,
        value: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let item_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::ItemDef);
        self.compile_expr_into_slot(function, depth, value, &item_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {}.exists set value 1",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
            self.namespace,
            target.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.count set from storage {}:runtime {}.count",
            self.namespace,
            target.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.nbt set from storage {}:runtime {}.nbt",
            self.namespace,
            target.storage_path(),
            self.namespace,
            item_slot.storage_path()
        ));
    }

    fn clear_item_slot_handle(
        &mut self,
        function: &IrFunction,
        depth: usize,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.exists set value 0",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.id set value \"\"",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.count set value 0",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.nbt set value {{}}",
            self.namespace,
            target.storage_path()
        ));
        self.sync_item_slot_handle(function, depth, target, lines);
    }

    fn sync_item_slot_handle(
        &mut self,
        function: &IrFunction,
        depth: usize,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let exists_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Bool);
        lines.push(format!(
            "execute store result score {} mcfc run data get storage {}:runtime {}.exists 1",
            exists_slot.numeric_name(),
            self.namespace,
            target.storage_path()
        ));
        lines.push(self.query_command(
            target,
            "item replace entity $(selector) $(command_slot) with air".to_string(),
            true,
        ));
        lines.push(format!(
            "data remove storage {}:runtime {}.item_name",
            self.namespace,
            target.storage_path()
        ));
        lines.push(format!(
            "execute if score {} mcfc matches 1 if data storage {}:runtime {}.nbt.display.Name run data modify storage {}:runtime {}.item_name set from storage {}:runtime {}.nbt.display.Name",
            exists_slot.numeric_name(),
            self.namespace,
            target.storage_path(),
            self.namespace,
            target.storage_path(),
            self.namespace,
            target.storage_path()
        ));
        let named_replace = self.query_command(
            target,
            "item replace entity $(selector) $(command_slot) with $(id)[minecraft:custom_name='\"$(item_name)\"',minecraft:custom_data=$(nbt)] $(count)".to_string(),
            true,
        );
        let plain_replace = self.query_command(
            target,
            "item replace entity $(selector) $(command_slot) with $(id)[minecraft:custom_data=$(nbt)] $(count)".to_string(),
            true,
        );
        lines.push(format!(
            "execute if score {} mcfc matches 1 if data storage {}:runtime {}.item_name run {}",
            exists_slot.numeric_name(),
            self.namespace,
            target.storage_path(),
            named_replace
        ));
        lines.push(format!(
            "execute if score {} mcfc matches 1 unless data storage {}:runtime {}.item_name run {}",
            exists_slot.numeric_name(),
            self.namespace,
            target.storage_path(),
            plain_replace
        ));
    }

    fn compile_player_mainhand_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        base_slot: &SlotRef,
        slot_name: &str,
        segments: &[PathSegment],
        value: &IrExpr,
        lines: &mut Vec<String>,
    ) -> bool {
        let Some(PathSegment::Field(field)) = segments.first() else {
            return false;
        };
        match field.as_str() {
            "name" => {
                let name_temp = self.new_temp();
                let name_slot = local_slot(depth, &function.name, &name_temp, &Type::String);
                self.compile_expr_into_slot(function, depth, value, &name_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.item_name set from storage {}:runtime {}",
                    self.namespace,
                    base_slot.storage_path(),
                    self.namespace,
                    name_slot.storage_path()
                ));
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "item modify entity $(selector) {} {{\"function\":\"minecraft:set_name\",\"name\":\"$(item_name)\",\"target\":\"custom_name\"}}",
                        equipment_slot_name(slot_name)
                    ),
                    true,
                ));
                true
            }
            "count" => {
                let count_temp = self.new_temp();
                let count_slot = local_slot(depth, &function.name, &count_temp, &Type::Int);
                self.compile_expr_into_slot(function, depth, value, &count_slot, lines);
                lines.push(format!(
                    "execute store result storage {}:runtime {}.count int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    base_slot.storage_path(),
                    count_slot.numeric_name()
                ));
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "item modify entity $(selector) {} {{\"function\":\"minecraft:set_count\",\"count\":$(count)}}",
                        equipment_slot_name(slot_name)
                    ),
                    true,
                ));
                true
            }
            "item" => {
                if value.ty == Type::ItemDef {
                    let slot_handle =
                        local_slot(depth, &function.name, &self.new_temp(), &Type::ItemSlot);
                    lines.push(format!(
                        "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
                        self.namespace,
                        slot_handle.storage_path(),
                        self.namespace,
                        base_slot.storage_path()
                    ));
                    lines.push(format!(
                        "data modify storage {}:runtime {}.command_slot set value {}",
                        self.namespace,
                        slot_handle.storage_path(),
                        quoted(equipment_slot_name(slot_name))
                    ));
                    self.populate_item_slot_from_item_def(
                        function,
                        depth,
                        value,
                        &slot_handle,
                        lines,
                    );
                    self.sync_item_slot_handle(function, depth, &slot_handle, lines);
                    return true;
                }
                let item_temp = self.new_temp();
                let item_slot = local_slot(depth, &function.name, &item_temp, &Type::String);
                self.compile_expr_into_slot(function, depth, value, &item_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {}.item_id set from storage {}:runtime {}",
                    self.namespace,
                    base_slot.storage_path(),
                    self.namespace,
                    item_slot.storage_path()
                ));
                lines.push(self.query_command(
                    base_slot,
                    format!(
                        "item replace entity $(selector) {} with $(item_id)",
                        equipment_slot_name(slot_name)
                    ),
                    true,
                ));
                true
            }
            _ => false,
        }
    }

    fn compile_cast(
        &mut self,
        function: &IrFunction,
        depth: usize,
        kind: CastKind,
        expr: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let temp_name = self.new_temp();
        let temp_slot = local_slot(depth, &function.name, &temp_name, &Type::Nbt);
        self.compile_expr_into_slot(function, depth, expr, &temp_slot, lines);
        match kind {
            CastKind::Int | CastKind::Bool => {
                lines.push(format!(
                    "execute store result score {} mcfc run data get storage {}:runtime {} 1",
                    target.numeric_name(),
                    self.namespace,
                    temp_slot.storage_path()
                ));
                if matches!(kind, CastKind::Bool) {
                    let raw = self.new_temp();
                    let raw_slot = local_slot(depth, &function.name, &raw, &Type::Int);
                    lines.push(format!(
                        "scoreboard players operation {} mcfc = {} mcfc",
                        raw_slot.numeric_name(),
                        target.numeric_name()
                    ));
                    lines.push(format!(
                        "scoreboard players set {} mcfc 0",
                        target.numeric_name()
                    ));
                    lines.push(format!(
                        "execute unless score {} mcfc matches 0 run scoreboard players set {} mcfc 1",
                        raw_slot.numeric_name(),
                        target.numeric_name()
                    ));
                }
            }
            CastKind::String => lines.push(format!(
                "data modify storage {}:runtime {} set from storage {}:runtime {}",
                self.namespace,
                target.storage_path(),
                self.namespace,
                temp_slot.storage_path()
            )),
            CastKind::Float => unreachable!("float casts lower through float_provider"),
        }
    }

    /// Render a float expression as one `/compute` float provider, so a whole
    /// arithmetic tree costs a single command. Leaves that are not float
    /// operations are evaluated into temp slots first and read back by path.
    fn float_provider(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        lines: &mut Vec<String>,
    ) -> String {
        match &expr.kind {
            IrExprKind::Float(value) => value.clone(),
            IrExprKind::Variable(name) => {
                self.storage_provider(&string_slot(depth, &function.name, name))
            }
            IrExprKind::Unary {
                op: UnaryOp::Neg,
                expr,
            } => format!(
                "{{type:\"negate\",input:{}}}",
                self.float_provider(function, depth, expr, lines)
            ),
            IrExprKind::Binary { op, left, right } if is_float_op(expr) => {
                let left = self.float_provider(function, depth, left, lines);
                let right = self.float_provider(function, depth, right, lines);
                match op {
                    BinaryOp::Add => format!("{{type:\"add\",inputs:[{},{}]}}", left, right),
                    BinaryOp::Mul => format!("{{type:\"mul\",inputs:[{},{}]}}", left, right),
                    BinaryOp::Sub => format!("{{type:\"sub\",left:{},right:{}}}", left, right),
                    // a - b * floor(a / b), so the sign follows `b` like the int `%`.
                    BinaryOp::Rem => format!(
                        "{{type:\"sub\",left:{left},right:{{type:\"mul\",inputs:[{right},{{type:\"floor\",input:{{type:\"div\",left:{left},right:{right}}}}}]}}}}"
                    ),
                    _ => format!("{{type:\"div\",left:{},right:{}}}", left, right),
                }
            }
            IrExprKind::Cast {
                kind: CastKind::Float,
                expr: inner,
            } if inner.ty == Type::Int => {
                let input = match &inner.kind {
                    IrExprKind::Int(value) => value.to_string(),
                    _ => {
                        let temp = self.new_temp();
                        let slot = local_slot(depth, &function.name, &temp, &Type::Int);
                        self.compile_expr_into_slot(function, depth, inner, &slot, lines);
                        format!(
                            "{{type:\"score\",target:{{type:\"fixed\",name:{}}},score:\"mcfc\"}}",
                            quoted(slot.numeric_name())
                        )
                    }
                };
                format!("{{type:\"from_int\",input:{}}}", input)
            }
            IrExprKind::Cast {
                kind: CastKind::Float,
                expr: inner,
            } => {
                let temp = self.new_temp();
                let slot = local_slot(depth, &function.name, &temp, &Type::Nbt);
                self.compile_expr_into_slot(function, depth, inner, &slot, lines);
                self.storage_provider(slot.storage_path())
            }
            IrExprKind::MethodCall {
                receiver,
                method,
                args,
            } if is_float_op(expr) => {
                let value = self.float_provider(function, depth, receiver, lines);
                let args: Vec<String> = args
                    .iter()
                    .map(|arg| self.float_provider(function, depth, arg, lines))
                    .collect();
                let single =
                    |kind: &str, input: &str| format!("{{type:\"{}\",input:{}}}", kind, input);
                let many = |kind: &str, inputs: &[&str]| {
                    format!("{{type:\"{}\",inputs:[{}]}}", kind, inputs.join(","))
                };
                match method.as_str() {
                    "trunc" => single("truncate", &value),
                    "tan" => format!(
                        "{{type:\"div\",left:{},right:{}}}",
                        single("sin", &value),
                        single("cos", &value)
                    ),
                    "pow" => format!("{{type:\"pow\",base:{},exponent:{}}}", value, args[0]),
                    "hypot" => many("length", &[&value, &args[0]]),
                    _ => single(method, &value),
                }
            }
            _ => {
                let temp = self.new_temp();
                let slot = local_slot(depth, &function.name, &temp, &Type::Float);
                self.compile_expr_into_slot(function, depth, expr, &slot, lines);
                self.storage_provider(slot.storage_path())
            }
        }
    }

    fn storage_provider(&self, path: &str) -> String {
        format!(
            "{{type:\"storage\",storage:\"{}:runtime\",path:{}}}",
            self.namespace,
            quoted(path)
        )
    }

    /// Compare two floats through the sign of `floor(a - b)` and `floor(b - a)`.
    #[allow(clippy::too_many_arguments)]
    fn compile_float_comparison(
        &mut self,
        function: &IrFunction,
        depth: usize,
        op: BinaryOp,
        left: &IrExpr,
        right: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let left = self.float_provider(function, depth, left, lines);
        let right = self.float_provider(function, depth, right, lines);
        let mut floor_diff = |a: &str, b: &str, lines: &mut Vec<String>| {
            let slot = numeric_slot(depth, &function.name, &self.new_temp());
            lines.push(format!(
                "execute store result score {} mcfc run compute default float {{type:\"sub\",left:{},right:{}}}",
                slot, a, b
            ));
            slot
        };
        let a_minus_b = floor_diff(&left, &right, lines);
        let b_minus_a = floor_diff(&right, &left, lines);
        let target = target.numeric_name();
        let (initial, condition, value) = match op {
            BinaryOp::Lt => (0, format!("if score {} mcfc matches ..-1", a_minus_b), 1),
            BinaryOp::Gt => (0, format!("if score {} mcfc matches ..-1", b_minus_a), 1),
            BinaryOp::Lte => (0, format!("if score {} mcfc matches 0..", b_minus_a), 1),
            BinaryOp::Gte => (0, format!("if score {} mcfc matches 0..", a_minus_b), 1),
            BinaryOp::Eq | BinaryOp::NotEq => (
                if op == BinaryOp::Eq { 0 } else { 1 },
                format!(
                    "if score {} mcfc matches 0.. if score {} mcfc matches 0..",
                    a_minus_b, b_minus_a
                ),
                if op == BinaryOp::Eq { 1 } else { 0 },
            ),
            _ => unreachable!(),
        };
        lines.push(format!(
            "scoreboard players set {} mcfc {}",
            target, initial
        ));
        lines.push(format!(
            "execute {} run scoreboard players set {} mcfc {}",
            condition, target, value
        ));
    }

    fn compile_value_as_nbt(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        match expr.ty {
            Type::Nbt | Type::TextDef => {
                self.compile_expr_into_slot(function, depth, expr, target, lines)
            }
            Type::EntityDef => {
                let spec_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::EntityDef);
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                self.compile_expr_into_slot(function, depth, expr, &spec_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.data set from storage {}:runtime {}.nbt",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    format!(
                        "data merge storage {}:runtime {} $(data)",
                        self.namespace,
                        target.storage_path()
                    ),
                ));
            }
            Type::BlockDef => {
                let spec_slot =
                    local_slot(depth, &function.name, &self.new_temp(), &Type::BlockDef);
                self.compile_expr_into_slot(function, depth, expr, &spec_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.nbt",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
            }
            Type::ItemDef => {
                let spec_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::ItemDef);
                let macro_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
                let count_slot = local_slot(depth, &function.name, &self.new_temp(), &Type::Int);
                self.compile_expr_into_slot(function, depth, expr, &spec_slot, lines);
                lines.push(format!(
                    "data modify storage {}:runtime {} set value {{}}",
                    self.namespace,
                    target.storage_path()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.id set from storage {}:runtime {}.id",
                    self.namespace,
                    target.storage_path(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
                lines.push(format!(
                    "execute store result score {} mcfc run data get storage {}:runtime {}.count 1",
                    count_slot.numeric_name(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
                lines.push(format!(
                    "execute store result storage {}:runtime {}.Count byte 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target.storage_path(),
                    count_slot.numeric_name()
                ));
                lines.push(format!(
                    "data modify storage {}:runtime {}.data set from storage {}:runtime {}.nbt",
                    self.namespace,
                    macro_slot.storage_path(),
                    self.namespace,
                    spec_slot.storage_path()
                ));
                lines.push(self.inline_macro_command(
                    macro_slot.storage_path(),
                    format!(
                        "data merge storage {}:runtime {} $(data)",
                        self.namespace,
                        target.storage_path()
                    ),
                ));
            }
            Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => {
                let temp = self.new_temp();
                let temp_slot = local_slot(depth, &function.name, &temp, &expr.ty);
                self.compile_expr_into_slot(function, depth, expr, &temp_slot, lines);
                lines.push(format!(
                    "execute store result storage {}:runtime {} int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target.storage_path(),
                    temp_slot.numeric_name()
                ));
            }
            Type::String
            | Type::Float
            | Type::Array(_)
            | Type::Dict(_)
            | Type::Optional(_)
            | Type::Struct(_)
            | Type::ItemSlot
            | Type::Bossbar
            | Type::EntitySet
            | Type::EntityRef
            | Type::PlayerRef
            | Type::BlockRef => {
                self.compile_expr_into_slot(function, depth, expr, target, lines);
            }
            _ => {}
        }
    }

    fn write_query_slot(
        &self,
        target: &SlotRef,
        prefix: &str,
        selector: &str,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set value {}",
            self.namespace,
            target.storage_path(),
            quoted(prefix)
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.selector set value {}",
            self.namespace,
            target.storage_path(),
            quoted(selector)
        ));
    }

    fn write_block_slot(&self, target: &SlotRef, prefix: &str, pos: &str, lines: &mut Vec<String>) {
        lines.push(format!(
            "data modify storage {}:runtime {}.prefix set value {}",
            self.namespace,
            target.storage_path(),
            quoted(prefix)
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.pos set value {}",
            self.namespace,
            target.storage_path(),
            quoted(pos)
        ));
    }

    fn compose_context_slots(
        &mut self,
        kind: ContextKind,
        anchor: &SlotRef,
        value: &SlotRef,
        target: &SlotRef,
        ty: &Type,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.__anchor_prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            target.storage_path(),
            self.namespace,
            anchor.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.__anchor_selector set from storage {}:runtime {}.selector",
            self.namespace,
            target.storage_path(),
            self.namespace,
            anchor.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.__value_prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            target.storage_path(),
            self.namespace,
            value.storage_path()
        ));
        lines.push(self.inline_macro_command(
            target.storage_path(),
            format!(
                "data modify storage {}:runtime {}.prefix set value \"$(__anchor_prefix)execute {} $(__anchor_selector) run $(__value_prefix)\"",
                self.namespace,
                target.storage_path(),
                context_execute_keyword(kind)
            ),
        ));
        match ty {
            Type::EntitySet | Type::EntityRef | Type::PlayerRef => lines.push(format!(
                "data modify storage {}:runtime {}.selector set from storage {}:runtime {}.selector",
                self.namespace,
                target.storage_path(),
                self.namespace,
                value.storage_path()
            )),
            Type::BlockRef => lines.push(format!(
                "data modify storage {}:runtime {}.pos set from storage {}:runtime {}.pos",
                self.namespace,
                target.storage_path(),
                self.namespace,
                value.storage_path()
            )),
            _ => {}
        }
    }

    fn compose_entity_position_slot(
        &mut self,
        entity: &SlotRef,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        lines.push(format!(
            "data modify storage {}:runtime {}.__anchor_prefix set from storage {}:runtime {}.prefix",
            self.namespace,
            target.storage_path(),
            self.namespace,
            entity.storage_path()
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.__anchor_selector set from storage {}:runtime {}.selector",
            self.namespace,
            target.storage_path(),
            self.namespace,
            entity.storage_path()
        ));
        lines.push(self.inline_macro_command(
            target.storage_path(),
            format!(
                "data modify storage {}:runtime {}.prefix set value \"$(__anchor_prefix)execute at $(__anchor_selector) run \"",
                self.namespace,
                target.storage_path()
            ),
        ));
        lines.push(format!(
            "data modify storage {}:runtime {}.pos set value \"~ ~ ~\"",
            self.namespace,
            target.storage_path()
        ));
    }

    /// A handle that selects nobody: `mcfc_id` scores start at 1.
    const NO_ENTITY_HANDLE: &'static str =
        "{prefix:\"\",selector:\"@e[scores={mcfc_id=0},limit=1]\"}";

    /// Rewrites an entity reference so it stays valid after this function:
    /// the entity gets a unique `mcfc_id` score and `slot` selects by it. Stored
    /// `Entity` state holds such a handle; a missing entity gets id 0, nobody.
    fn stabilize_entity_ref(&mut self, slot: &SlotRef, lines: &mut Vec<String>) {
        self.uses_ownership = true;
        let ns = self.namespace.clone();
        let path = slot.storage_path();
        lines.push(self.query_command(
            slot,
            format!(
                "execute as $(selector) unless score @s mcfc_id matches 1.. run function {ns}:generated/assign_id"
            ),
            true,
        ));
        lines.push(format!(
            "data modify storage {ns}:runtime {path}.id set value 0"
        ));
        lines.push(self.query_command(
            slot,
            format!(
                "execute store result storage {ns}:runtime {path}.id int 1 run scoreboard players get $(selector) mcfc_id"
            ),
            true,
        ));
        lines.push(format!(
            "data modify storage {ns}:runtime {path}.prefix set value \"\""
        ));
        lines.push(self.inline_macro_command(
            path,
            format!(
                "data modify storage {ns}:runtime {path}.selector set value \"@e[scores={{mcfc_id=$(id)}},limit=1]\""
            ),
        ));
    }

    /// `token = entity;` for an `Entity` world state stores a handle to it.
    fn compile_world_entity_assign(
        &mut self,
        function: &IrFunction,
        depth: usize,
        name: &str,
        value: &IrExpr,
        lines: &mut Vec<String>,
    ) {
        let value_slot = local_slot(depth, &function.name, &self.new_temp(), &value.ty);
        self.compile_expr_into_slot(function, depth, value, &value_slot, lines);
        self.stabilize_entity_ref(&value_slot, lines);
        lines.push(format!(
            "data modify storage {}:runtime {} set from storage {}:runtime {}",
            self.namespace,
            string_slot(0, "", name),
            self.namespace,
            value_slot.storage_path()
        ));
    }

    fn query_command(&mut self, slot: &SlotRef, command: String, wrap_macro: bool) -> String {
        let storage = slot.storage_path();
        let macro_line = format!("$(prefix){}", command);
        let relative = if wrap_macro { macro_line } else { command };
        let namespace = self.namespace.clone();
        let macro_name = self.ensure_inline_macro(relative);
        format!(
            "function {}:{} with storage {}:runtime {}",
            namespace, macro_name, namespace, storage
        )
    }

    fn inline_macro_command(&mut self, storage_path: &str, command: String) -> String {
        let namespace = self.namespace.clone();
        let macro_name = self.ensure_inline_macro(command);
        format!(
            "function {}:{} with storage {}:runtime {}",
            namespace, macro_name, namespace, storage_path
        )
    }

    /// `light()`, `biome()`, `in_biome(id)` and `environment(attribute)` on
    /// a `block_ref`. Each runs positioned at the block through its macro.
    #[allow(clippy::too_many_arguments)]
    fn compile_block_query(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        method: &str,
        args: &[IrExpr],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let ns = self.namespace.clone();
        let block = local_slot(depth, &function.name, &self.new_temp(), &Type::BlockRef);
        self.compile_expr_into_slot(function, depth, receiver, &block, lines);
        match method {
            "x" | "y" | "z" => {
                // Relative positions resolve where this runs; a marker aligned
                // to the block reports the whole-number corner.
                let axis = match method {
                    "x" => 0,
                    "y" => 1,
                    _ => 2,
                };
                let tag = format!("mcfc_block_{}", self.new_temp());
                for command in [
                    format!(
                        "execute positioned $(pos) align xyz run summon minecraft:marker ~ ~ ~ {{Tags:[\"{tag}\"]}}"
                    ),
                    format!(
                        "execute store result score {} mcfc run data get entity @e[type=minecraft:marker,tag={tag},limit=1] Pos[{axis}]",
                        target.numeric_name()
                    ),
                    format!("kill @e[type=minecraft:marker,tag={tag}]"),
                ] {
                    lines.push(self.block_command(&block, command, true));
                }
            }
            "light" => {
                self.write_light_probe(0, 15);
                lines.push("scoreboard players set #light mcfc 0".to_string());
                lines.push(self.block_command(
                    &block,
                    format!("execute positioned $(pos) run function {ns}:generated/light_0_15"),
                    true,
                ));
                lines.push(format!(
                    "scoreboard players operation {} mcfc = #light mcfc",
                    target.numeric_name()
                ));
            }
            "biome" => {
                let probe: Vec<String> = crate::minecraft_ids::BIOME_IDS
                    .iter()
                    .map(|id| {
                        format!(
                            "execute if biome ~ ~ ~ {id} run return run data modify storage {ns}:runtime biome_probe set value \"{id}\""
                        )
                    })
                    .collect();
                self.files.insert(
                    format!("data/{ns}/function/generated/biome_probe.mcfunction"),
                    probe.join("\n") + "\n",
                );
                lines.push(format!(
                    "data modify storage {ns}:runtime biome_probe set value \"\""
                ));
                lines.push(self.block_command(
                    &block,
                    format!("execute positioned $(pos) run function {ns}:generated/biome_probe"),
                    true,
                ));
                lines.push(format!(
                    "data modify storage {ns}:runtime {} set from storage {ns}:runtime biome_probe",
                    target.storage_path()
                ));
            }
            "copy_to" => {
                let to = local_slot(depth, &function.name, &self.new_temp(), &Type::BlockRef);
                self.compile_expr_into_slot(function, depth, &args[0], &to, lines);
                lines.push(format!(
                    "data modify storage {ns}:runtime {}.to set from storage {ns}:runtime {}.pos",
                    block.storage_path(),
                    to.storage_path()
                ));
                lines.push(self.block_command(
                    &block,
                    "clone $(pos) $(pos) $(to)".to_string(),
                    true,
                ));
            }
            "block_state" => {
                let IrExprKind::String(name) = &args[0].kind else {
                    return;
                };
                let values = crate::minecraft_ids::BLOCK_PROPERTIES
                    .iter()
                    .find(|(property, _)| property == name)
                    .map_or(&[][..], |(_, values)| *values);
                // Tag predicates only match blocks that have the property.
                self.files.insert(
                    format!("data/{ns}/tags/block/mcfc_all_blocks.json"),
                    render_tag_file(
                        &crate::minecraft_ids::BLOCK_IDS
                            .iter()
                            .map(|id| id.to_string())
                            .collect::<Vec<_>>(),
                    ),
                );
                let probe = values
                    .iter()
                    .map(|value| format!("execute if block ~ ~ ~ #{ns}:mcfc_all_blocks[{name}={value}] run return run data modify storage {ns}:runtime block_state set value \"{value}\"
"))
                    .collect::<String>();
                self.files.insert(
                    format!("data/{ns}/function/generated/block_state/{name}.mcfunction"),
                    probe,
                );
                lines.push(format!(
                    "data modify storage {ns}:runtime block_state set value \"\""
                ));
                lines.push(self.block_command(
                    &block,
                    format!(
                        "execute positioned $(pos) run function {ns}:generated/block_state/{name}"
                    ),
                    true,
                ));
                lines.push(format!(
                    "data modify storage {ns}:runtime {} set from storage {ns}:runtime block_state",
                    target.storage_path()
                ));
            }
            "block_type" => {
                self.emit_block_type_probe();
                lines.push(format!(
                    "data modify storage {ns}:runtime block_type set value \"\""
                ));
                lines.push(self.block_command(
                    &block,
                    format!("execute positioned $(pos) run function {ns}:generated/block_type/0"),
                    true,
                ));
                lines.push(format!(
                    "data modify storage {ns}:runtime {} set from storage {ns}:runtime block_type",
                    target.storage_path()
                ));
            }
            "in_biome" => {
                let id = local_slot(depth, &function.name, &self.new_temp(), &Type::String);
                self.compile_expr_into_slot(function, depth, &args[0], &id, lines);
                lines.push(format!(
                    "data modify storage {ns}:runtime {}.biome set from storage {ns}:runtime {}",
                    block.storage_path(),
                    id.storage_path()
                ));
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                lines.push(self.block_command(
                    &block,
                    format!(
                        "execute if biome $(pos) $(biome) run scoreboard players set {} mcfc 1",
                        target.numeric_name()
                    ),
                    true,
                ));
            }
            _ => {
                let IrExprKind::String(attribute) = &args[0].kind else {
                    return;
                };
                let attribute = attribute.trim_start_matches("minecraft:");
                lines.push(self.block_command(
                    &block,
                    format!(
                        "data modify storage {ns}:runtime {} set compute block $(pos) float {{type:\"environment_attribute\",attribute:\"minecraft:{attribute}\"}}",
                        target.storage_path()
                    ),
                    true,
                ));
            }
        }
    }

    /// Binary search for the light level between `low` and `high`, one
    /// `location_check` predicate per level of the tree. Leaves `#light`.
    fn write_light_probe(&mut self, low: u8, high: u8) {
        let ns = self.namespace.clone();
        let body = if low == high {
            format!("scoreboard players set #light mcfc {low}")
        } else {
            let mid = (low + high).div_ceil(2);
            self.write_light_probe(mid, high);
            self.write_light_probe(low, mid - 1);
            format!(
                "execute if predicate {{type:\"location_check\",predicate:{{light:{{light:{{min:{mid}}}}}}}}} run return run function {ns}:generated/light_{mid}_{high}\nfunction {ns}:generated/light_{low}_{}",
                mid - 1
            )
        };
        self.files.insert(
            format!("data/{ns}/function/generated/light_{low}_{high}.mcfunction"),
            body + "\n",
        );
    }

    fn block_command(&mut self, slot: &SlotRef, command: String, wrap_macro: bool) -> String {
        let storage = slot.storage_path();
        let macro_line = format!("$(prefix){}", command);
        let relative = if wrap_macro { macro_line } else { command };
        let namespace = self.namespace.clone();
        let macro_name = self.ensure_inline_macro(relative);
        format!(
            "function {}:{} with storage {}:runtime {}",
            namespace, macro_name, namespace, storage
        )
    }

    fn ensure_inline_macro(&mut self, command: String) -> String {
        self.macro_counter += 1;
        let relative = format!("generated/internal_macro_{}", self.macro_counter);
        let path = format!("data/{}/function/{}.mcfunction", self.namespace, relative);
        self.files.insert(path, format!("${}\n", command));
        relative
    }

    fn compile_unary(
        &mut self,
        function: &IrFunction,
        depth: usize,
        op: UnaryOp,
        expr: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if op == UnaryOp::Not {
            let target = target.numeric_name();
            match self
                .compile_condition(function, depth, expr, lines)
                .as_slice()
            {
                [clause] => lines.push(format!(
                    "execute store success score {target} mcfc {}",
                    negate_clause(clause)
                )),
                clauses => {
                    lines.push(format!(
                        "execute store success score {target} mcfc {}",
                        clauses.join(" ")
                    ));
                    lines.push(format!(
                        "execute store success score {target} mcfc unless score {target} mcfc matches 1"
                    ));
                }
            }
            return;
        }
        let temp = self.new_temp();
        let temp_slot = local_slot(depth, &function.name, &temp, &expr.ty);
        self.compile_expr_into_slot(function, depth, expr, &temp_slot, lines);

        match op {
            UnaryOp::Not => unreachable!("handled above"),
            UnaryOp::Neg => {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                lines.push(format!(
                    "scoreboard players operation {} mcfc -= {} mcfc",
                    target.numeric_name(),
                    temp_slot.numeric_name()
                ));
            }
        }
    }

    /// The score holding an int, bool or enum operand: a variable's own slot,
    /// or a temp the value is computed into.
    fn numeric_operand(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        lines: &mut Vec<String>,
    ) -> String {
        if let IrExprKind::Variable(name) = &expr.kind {
            return numeric_slot(depth, &function.name, name);
        }
        let temp = self.new_temp();
        let slot = local_slot(depth, &function.name, &temp, &expr.ty);
        self.compile_expr_into_slot(function, depth, expr, &slot, lines);
        slot.numeric_name().to_string()
    }

    /// `execute` clauses that hold exactly when `expr` is true, so a branch can
    /// test the condition directly instead of through a 0/1 temp.
    fn compile_condition(
        &mut self,
        function: &IrFunction,
        depth: usize,
        expr: &IrExpr,
        lines: &mut Vec<String>,
    ) -> Vec<String> {
        match &expr.kind {
            IrExprKind::Binary { op, left, right } if is_score_comparison(*op, left, right) => {
                vec![self.comparison_clause(function, depth, *op, left, right, lines)]
            }
            IrExprKind::Binary {
                op: BinaryOp::And,
                left,
                right,
            } if is_simple_condition(right) => {
                let mut clauses = self.compile_condition(function, depth, left, lines);
                clauses.extend(self.compile_condition(function, depth, right, lines));
                clauses
            }
            IrExprKind::Unary {
                op: UnaryOp::Not,
                expr: inner,
            } => {
                let mut clauses = self.compile_condition(function, depth, inner, lines);
                if let [clause] = clauses.as_mut_slice() {
                    *clause = negate_clause(clause);
                    return clauses;
                }
                let temp = self.new_temp();
                let slot = numeric_slot(depth, &function.name, &temp);
                lines.push(format!(
                    "execute store success score {slot} mcfc {}",
                    clauses.join(" ")
                ));
                vec![format!("unless score {slot} mcfc matches 1")]
            }
            _ => {
                let slot = self.numeric_operand(function, depth, expr, lines);
                vec![format!("if score {slot} mcfc matches 1")]
            }
        }
    }

    /// One `if|unless score ...` clause comparing two int, bool or enum values.
    fn comparison_clause(
        &mut self,
        function: &IrFunction,
        depth: usize,
        op: BinaryOp,
        left: &IrExpr,
        right: &IrExpr,
        lines: &mut Vec<String>,
    ) -> String {
        let keyword = if op == BinaryOp::NotEq {
            "unless"
        } else {
            "if"
        };
        // A literal on either side becomes a `matches` range.
        let literal = |expr: &IrExpr| match expr.kind {
            IrExprKind::Int(value) => Some(value),
            IrExprKind::Bool(value) => Some(i64::from(value)),
            _ => None,
        };
        let flipped = match op {
            BinaryOp::Lt => BinaryOp::Gt,
            BinaryOp::Lte => BinaryOp::Gte,
            BinaryOp::Gt => BinaryOp::Lt,
            BinaryOp::Gte => BinaryOp::Lte,
            other => other,
        };
        let literal_side = match (literal(left), literal(right)) {
            (_, Some(value)) => Some((left, op, value)),
            (Some(value), None) => Some((right, flipped, value)),
            (None, None) => None,
        };
        if let Some((operand, op, value)) = literal_side
            && let Some(range) = literal_range(op, value)
        {
            let slot = self.numeric_operand(function, depth, operand, lines);
            return format!("{keyword} score {slot} mcfc matches {range}");
        }
        let left_slot = self.numeric_operand(function, depth, left, lines);
        let right_slot = self.numeric_operand(function, depth, right, lines);
        let operator = match op {
            BinaryOp::Eq | BinaryOp::NotEq => "=",
            BinaryOp::Lt => "<",
            BinaryOp::Lte => "<=",
            BinaryOp::Gt => ">",
            BinaryOp::Gte => ">=",
            _ => unreachable!("not a comparison"),
        };
        format!("{keyword} score {left_slot} mcfc {operator} {right_slot} mcfc")
    }

    fn compile_binary(
        &mut self,
        function: &IrFunction,
        depth: usize,
        op: BinaryOp,
        left: &IrExpr,
        right: &IrExpr,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if is_score_comparison(op, left, right) {
            let clause = self.comparison_clause(function, depth, op, left, right, lines);
            lines.push(format!(
                "execute store success score {} mcfc {clause}",
                target.numeric_name()
            ));
            return;
        }
        match op {
            BinaryOp::And => {
                let left_temp = self.new_temp();
                let left_slot = local_slot(depth, &function.name, &left_temp, &left.ty);
                self.compile_expr_into_slot(function, depth, left, &left_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                let (rhs_path, rhs_name) = self.new_block(function, depth, "logic_and_rhs");
                let mut rhs_lines = Vec::new();
                self.compile_expr_into_slot(function, depth, right, target, &mut rhs_lines);
                self.files.insert(rhs_path, rhs_lines.join("\n") + "\n");
                lines.push(format!(
                    "execute if score {} mcfc matches 1 run function {}:{}",
                    left_slot.numeric_name(),
                    self.namespace,
                    rhs_name
                ));
                return;
            }
            BinaryOp::Or => {
                let left_temp = self.new_temp();
                let left_slot = local_slot(depth, &function.name, &left_temp, &left.ty);
                self.compile_expr_into_slot(function, depth, left, &left_slot, lines);
                lines.push(format!(
                    "scoreboard players set {} mcfc 1",
                    target.numeric_name()
                ));
                let (rhs_path, rhs_name) = self.new_block(function, depth, "logic_or_rhs");
                let mut rhs_lines = Vec::new();
                self.compile_expr_into_slot(function, depth, right, target, &mut rhs_lines);
                self.files.insert(rhs_path, rhs_lines.join("\n") + "\n");
                lines.push(format!(
                    "execute if score {} mcfc matches 0 run function {}:{}",
                    left_slot.numeric_name(),
                    self.namespace,
                    rhs_name
                ));
                return;
            }
            _ if left.ty == Type::Float => {
                self.compile_float_comparison(function, depth, op, left, right, target, lines);
                return;
            }
            _ => {}
        }

        let left_temp = self.new_temp();
        let right_temp = self.new_temp();
        let left_slot = local_slot(depth, &function.name, &left_temp, &left.ty);
        let right_slot = local_slot(depth, &function.name, &right_temp, &right.ty);
        self.compile_expr_into_slot(function, depth, left, &left_slot, lines);
        self.compile_expr_into_slot(function, depth, right, &right_slot, lines);

        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                lines.push(format!(
                    "scoreboard players operation {} mcfc = {} mcfc",
                    target.numeric_name(),
                    left_slot.numeric_name()
                ));
                let operator = match op {
                    BinaryOp::Add => "+=",
                    BinaryOp::Sub => "-=",
                    BinaryOp::Mul => "*=",
                    BinaryOp::Div => "/=",
                    BinaryOp::Rem => "%=",
                    _ => unreachable!(),
                };
                lines.push(format!(
                    "scoreboard players operation {} mcfc {} {} mcfc",
                    target.numeric_name(),
                    operator,
                    right_slot.numeric_name()
                ));
            }
            BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor
            | BinaryOp::Shl
            | BinaryOp::Shr => {
                self.uses_bitwise = true;
                let ns = &self.namespace;
                let (mode, runtime) = match op {
                    BinaryOp::BitAnd => (0, "bitwise"),
                    BinaryOp::BitOr => (1, "bitwise"),
                    BinaryOp::BitXor => (2, "bitwise"),
                    BinaryOp::Shl => (0, "shift"),
                    _ => (1, "shift"),
                };
                lines.extend([
                    format!(
                        "scoreboard players operation #bit_a mcfc = {} mcfc",
                        left_slot.numeric_name()
                    ),
                    format!(
                        "scoreboard players operation #bit_b mcfc = {} mcfc",
                        right_slot.numeric_name()
                    ),
                    format!("scoreboard players set #bit_op mcfc {mode}"),
                    format!("function {ns}:generated/bitwise/{runtime}"),
                    format!(
                        "scoreboard players operation {} mcfc = #bit_r mcfc",
                        target.numeric_name()
                    ),
                ]);
            }
            // Records compare as whole compounds, the same way.
            BinaryOp::Eq | BinaryOp::NotEq
                if matches!(left.ty, Type::String | Type::Struct(_))
                    && matches!(right.ty, Type::String | Type::Struct(_)) =>
            {
                self.compile_string_equality(op, &left_slot, &right_slot, target, lines);
            }
            BinaryOp::Eq
            | BinaryOp::NotEq
            | BinaryOp::Lt
            | BinaryOp::Lte
            | BinaryOp::Gt
            | BinaryOp::Gte => {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                let (keyword, operator) = match op {
                    BinaryOp::Eq => ("if", "="),
                    BinaryOp::NotEq => ("unless", "="),
                    BinaryOp::Lt => ("if", "<"),
                    BinaryOp::Lte => ("if", "<="),
                    BinaryOp::Gt => ("if", ">"),
                    BinaryOp::Gte => ("if", ">="),
                    _ => unreachable!(),
                };
                lines.push(format!(
                    "execute {} score {} mcfc {} {} mcfc run scoreboard players set {} mcfc 1",
                    keyword,
                    left_slot.numeric_name(),
                    operator,
                    right_slot.numeric_name(),
                    target.numeric_name()
                ));
            }
            BinaryOp::And | BinaryOp::Or => unreachable!(),
        }
    }

    fn compile_string_equality(
        &mut self,
        op: BinaryOp,
        left_slot: &SlotRef,
        right_slot: &SlotRef,
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        let compare_storage = format!("frames.__cmp{}", self.new_temp());
        let compare_result = numeric_slot(0, "__cmp", &self.new_temp());
        lines.push(format!(
            "data modify storage {}:runtime {} set from storage {}:runtime {}",
            self.namespace,
            compare_storage,
            self.namespace,
            left_slot.storage_path()
        ));
        lines.push(format!(
            "execute store success score {} mcfc run data modify storage {}:runtime {} set from storage {}:runtime {}",
            compare_result,
            self.namespace,
            compare_storage,
            self.namespace,
            right_slot.storage_path()
        ));

        match op {
            BinaryOp::Eq => {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                lines.push(format!(
                    "execute if score {} mcfc matches 0 run scoreboard players set {} mcfc 1",
                    compare_result,
                    target.numeric_name()
                ));
            }
            BinaryOp::NotEq => {
                lines.push(format!(
                    "scoreboard players set {} mcfc 0",
                    target.numeric_name()
                ));
                lines.push(format!(
                    "execute if score {} mcfc matches 1 run scoreboard players set {} mcfc 1",
                    compare_result,
                    target.numeric_name()
                ));
            }
            _ => unreachable!(),
        }
    }

    fn function_entry_name(&self, function: &str, depth: usize) -> String {
        format!("generated/{}__d{}__entry", path_name(function), depth)
    }

    /// `function ...` to save or restore `function`'s frame at `depth`; the
    /// bodies are written by `emit_frame_stack` once every file exists.
    fn frame_call(&mut self, kind: &str, depth: usize, function: &str) -> String {
        self.frame_saves.insert((depth, function.to_string()));
        format!(
            "function {}:generated/{}__d{}__{kind}",
            self.namespace,
            path_name(function),
            depth
        )
    }

    fn copy_slot(&self, ty: &Type, from: &SlotRef, to: &SlotRef) -> String {
        if is_score_type(ty) {
            format!(
                "scoreboard players operation {} mcfc = {} mcfc",
                to.numeric_name(),
                from.numeric_name()
            )
        } else {
            let ns = &self.namespace;
            format!(
                "data modify storage {ns}:runtime {} set from storage {ns}:runtime {}",
                to.storage_path(),
                from.storage_path()
            )
        }
    }

    /// Frame save/restore for recursive calls: every `$d<depth>_<fn>_*` score
    /// the pack mentions, plus the `frames.d<depth>.<fn>` storage, pushed onto
    /// the `stack` list and popped back.
    fn emit_frame_stack(&mut self) {
        let ns = self.namespace.clone();
        for (depth, function) in std::mem::take(&mut self.frame_saves) {
            let prefix = format!("$d{depth}_{}_", sanitize(&function));
            let mut scores = BTreeSet::new();
            for (path, body) in &self.files {
                if !path.ends_with(".mcfunction") {
                    continue;
                }
                for (at, _) in body.match_indices(&prefix) {
                    let len = body[at + 1..]
                        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
                        .map_or(body.len() - at, |end| end + 1);
                    scores.insert(body[at..at + len].to_string());
                }
            }
            let frame = format!("frames.d{depth}.{}", sanitize(&function));
            let mut save = vec![
                format!("data modify storage {ns}:runtime stack append value {{}}"),
                format!(
                    "data modify storage {ns}:runtime stack[-1].frame set from storage {ns}:runtime {frame}"
                ),
            ];
            let mut restore = Vec::new();
            for (index, score) in scores.iter().enumerate() {
                save.push(format!("execute store result storage {ns}:runtime stack[-1].s{index} int 1 run scoreboard players get {score} mcfc"));
                restore.push(format!("execute store result score {score} mcfc run data get storage {ns}:runtime stack[-1].s{index}"));
            }
            restore.push(format!("data remove storage {ns}:runtime {frame}"));
            restore.push(format!("data modify storage {ns}:runtime {frame} set from storage {ns}:runtime stack[-1].frame"));
            restore.push(format!("data remove storage {ns}:runtime stack[-1]"));
            let base = format!(
                "data/{ns}/function/generated/{}__d{depth}",
                path_name(&function)
            );
            self.files
                .insert(format!("{base}__save.mcfunction"), save.join("\n") + "\n");
            self.files.insert(
                format!("{base}__restore.mcfunction"),
                restore.join("\n") + "\n",
            );
        }
    }

    fn emit_macro_command(
        &mut self,
        function: &IrFunction,
        depth: usize,
        template: &str,
        placeholders: &[IrMacroPlaceholder],
        lines: &mut Vec<String>,
    ) {
        let template = expand_display_text_sugar(template);
        if placeholders.is_empty() {
            lines.push(template);
            return;
        }
        let rendered_template = rewrite_macro_template(&template, placeholders);

        self.macro_counter += 1;
        let macro_id = self.macro_counter;
        let relative = format!(
            "generated/{}__d{}__macro_{}",
            path_name(&function.name),
            depth,
            macro_id
        );
        let path = format!("data/{}/function/{}.mcfunction", self.namespace, relative);
        self.files.insert(path, format!("${}\n", rendered_template));

        let storage_base = macro_storage_base(depth, &function.name, macro_id);
        for placeholder in placeholders {
            let source_temp = self.new_temp();
            let source_slot = local_slot(depth, &function.name, &source_temp, &placeholder.ty);
            self.compile_expr_into_slot(function, depth, &placeholder.expr, &source_slot, lines);
            let target_path = format!("{}.{}", storage_base, placeholder.key);
            match placeholder.ty {
                Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => lines.push(format!(
                    "execute store result storage {}:runtime {} int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target_path,
                    source_slot.numeric_name()
                )),
                Type::Float => self.float_to_text(source_slot.storage_path(), &target_path, lines),
                Type::String | Type::Nbt | Type::TextDef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::EntitySet | Type::EntityRef | Type::PlayerRef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.selector",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::BlockRef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.pos",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::Array(_)
                | Type::Dict(_)
                | Type::Optional(_)
                | Type::Struct(_)
                | Type::EntityDef
                | Type::BlockDef
                | Type::ItemDef
                | Type::ItemSlot
                | Type::Bossbar => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::Void => {}
            }
        }
        lines.push(format!(
            "function {}:{} with storage {}:runtime {}",
            self.namespace, relative, self.namespace, storage_base
        ));
    }

    fn compile_interpolated_string(
        &mut self,
        function: &IrFunction,
        depth: usize,
        template: &str,
        placeholders: &[IrMacroPlaceholder],
        target: &SlotRef,
        lines: &mut Vec<String>,
    ) {
        if placeholders.is_empty() {
            lines.push(format!(
                "data modify storage {}:runtime {} set value {}",
                self.namespace,
                target.storage_path(),
                quoted(template)
            ));
            return;
        }

        let body = format!(
            "$data modify storage {}:runtime {} set value {}",
            self.namespace,
            target.storage_path(),
            quoted(&rewrite_macro_template(template, placeholders))
        );
        let fallback = Some(format!(
            "data modify storage {}:runtime {} set value \"\"",
            self.namespace,
            target.storage_path()
        ));
        self.call_macro(
            function,
            depth,
            "string",
            body,
            placeholders,
            fallback,
            true,
            lines,
        );
    }

    /// Build a `text(...)` component for an interpolated literal without splicing
    /// runtime values into a quoted string. Literal runs become compile-time
    /// string parts; each placeholder becomes an NBT-sourced sub-component that
    /// Minecraft resolves at render time, so a value containing `"`/`\` cannot
    /// corrupt the component. The result is `{text:"",extra:[...]}` so callers
    /// can still decorate it (`.color`, `.italic`, ...).
    fn compile_text_interpolation(
        &mut self,
        function: &IrFunction,
        depth: usize,
        target: &SlotRef,
        template: &str,
        placeholders: &[IrMacroPlaceholder],
        lines: &mut Vec<String>,
    ) {
        self.macro_counter += 1;
        let storage_base = macro_storage_base(depth, &function.name, self.macro_counter);
        self.write_macro_placeholder_values(function, depth, &storage_base, placeholders, lines);

        let parts: Vec<String> = split_macro_template(template, placeholders.len())
            .into_iter()
            .filter_map(|segment| match segment {
                TemplateSegment::Literal(text) if text.is_empty() => None,
                TemplateSegment::Literal(text) => Some(quoted(&text)),
                TemplateSegment::Placeholder(index) => placeholders.get(index).map(|placeholder| {
                    let path = format!("{}.{}", storage_base, placeholder.key);
                    format!(
                        "{{storage:\"{}:runtime\",nbt:{}}}",
                        self.namespace,
                        quoted(&path)
                    )
                }),
            })
            .collect();
        lines.push(format!(
            "data modify storage {}:runtime {} set value {{text:\"\",extra:[{}]}}",
            self.namespace,
            target.storage_path(),
            parts.join(",")
        ));
    }

    /// Emit a loop that finds the first element equal to `value`. Returns the
    /// score holder with its index, or -1. Two tags are equal when copying one
    /// onto the other changes nothing, so `data modify` reports no success.
    fn compile_array_index_of(
        &mut self,
        function: &IrFunction,
        depth: usize,
        array: &IrExpr,
        value: &IrExpr,
        lines: &mut Vec<String>,
    ) -> String {
        let ns = self.namespace.clone();
        let rest = self.compile_storage_receiver(function, depth, array, lines);
        let needle = local_slot(depth, &function.name, &self.new_temp(), &Type::Nbt);
        self.compile_value_as_nbt(function, depth, value, &needle, lines);
        let probe = string_slot(depth, &function.name, &self.new_temp());
        let found = numeric_slot(depth, &function.name, &self.new_temp());
        let index = numeric_slot(depth, &function.name, &self.new_temp());
        let changed = numeric_slot(depth, &function.name, &self.new_temp());
        let rest = rest.storage_path();
        let needle = needle.storage_path();
        let (path, name) = self.new_block(function, depth, "index_of");
        let body = [
            format!(
                "data modify storage {ns}:runtime {probe} set from storage {ns}:runtime {rest}[0]"
            ),
            format!(
                "execute store success score {changed} mcfc run data modify storage {ns}:runtime {probe} set from storage {ns}:runtime {needle}"
            ),
            format!(
                "execute if score {changed} mcfc matches 0 run scoreboard players operation {found} mcfc = {index} mcfc"
            ),
            format!("data remove storage {ns}:runtime {rest}[0]"),
            format!("scoreboard players add {index} mcfc 1"),
            format!(
                "execute if score {changed} mcfc matches 1 if data storage {ns}:runtime {rest}[0] run function {ns}:{name}"
            ),
        ];
        self.files.insert(path, body.join("\n") + "\n");
        lines.push(format!("scoreboard players set {found} mcfc -1"));
        lines.push(format!("scoreboard players set {index} mcfc 0"));
        lines.push(format!(
            "execute if data storage {ns}:runtime {rest}[0] run function {ns}:{name}"
        ));
        found
    }

    /// Emit a loop that moves every element of `source` to the front of a new
    /// list, emptying `source`. Returns the storage path of the reversed list.
    fn compile_array_reverse(
        &mut self,
        function: &IrFunction,
        depth: usize,
        source: &SlotRef,
        lines: &mut Vec<String>,
    ) -> String {
        let ns = self.namespace.clone();
        let reversed = string_slot(depth, &function.name, &self.new_temp());
        let source = source.storage_path();
        let (path, name) = self.new_block(function, depth, "reverse");
        let body = [
            format!(
                "data modify storage {ns}:runtime {reversed} prepend from storage {ns}:runtime {source}[0]"
            ),
            format!("data remove storage {ns}:runtime {source}[0]"),
            format!("execute if data storage {ns}:runtime {source}[0] run function {ns}:{name}"),
        ];
        self.files.insert(path, body.join("\n") + "\n");
        lines.push(format!(
            "data modify storage {ns}:runtime {reversed} set value []"
        ));
        lines.push(format!(
            "execute if data storage {ns}:runtime {source}[0] run function {ns}:{name}"
        ));
        reversed
    }

    /// Write `body` as a generated macro function and call it with the
    /// placeholder values. `fallback` runs after the values are copied and
    /// before the call, so it cannot clobber an argument that shares the
    /// target slot. It is what remains when Minecraft skips an unparseable
    /// macro line, such as a string value containing `"`.
    #[allow(clippy::too_many_arguments)]
    fn call_macro(
        &mut self,
        function: &IrFunction,
        depth: usize,
        label: &str,
        body: String,
        placeholders: &[IrMacroPlaceholder],
        fallback: Option<String>,
        escape_strings: bool,
        lines: &mut Vec<String>,
    ) {
        self.macro_counter += 1;
        let macro_id = self.macro_counter;
        let relative = format!(
            "generated/{}__d{}__{}_{}",
            path_name(&function.name),
            depth,
            label,
            macro_id
        );
        let path = format!("data/{}/function/{}.mcfunction", self.namespace, relative);
        self.files.insert(path, body + "\n");
        let storage_base = macro_storage_base(depth, &function.name, macro_id);
        self.write_macro_placeholder_values(function, depth, &storage_base, placeholders, lines);
        if escape_strings {
            // String values go inside "...", so their quotes and backslashes
            // need escaping.
            let ns = &self.namespace;
            for placeholder in placeholders.iter().filter(|p| p.ty == Type::String) {
                let path = format!("{storage_base}.{}", placeholder.key);
                lines.push(format!(
                    "data modify storage {ns}:runtime escape.c.v set from storage {ns}:runtime {path}"
                ));
                lines.push(format!("function {ns}:generated/escape_string"));
                lines.push(format!(
                    "data modify storage {ns}:runtime {path} set from storage {ns}:runtime escape.s"
                ));
            }
            self.uses_escape |= placeholders.iter().any(|p| p.ty == Type::String);
        }
        lines.extend(fallback);
        lines.push(format!(
            "function {}:{} with storage {}:runtime {}",
            self.namespace, relative, self.namespace, storage_base
        ));
    }

    /// Write the helpers behind `dict.keys()`. No command lists a compound's
    /// keys, so a macro prints the dict as SNBT text and a state machine
    /// walks it one character at a time, slicing out each top-level key.
    /// States: 0 expects a key, 1 is in a bare key, 2 in a quoted key, 3 in
    /// a value, 4 in a quoted string inside a value, 5 skips to the `:` after
    /// a quoted key. `#dk_q` is the open quote (1 `"`, 2 `'`), `#dk_esc` marks
    /// a backslash, `#dk_depth` counts brackets inside a value.
    fn write_dict_keys_helpers(&mut self) {
        let ns = self.namespace.clone();
        let s = format!("storage {ns}:runtime dict_keys");
        let is = |c: &str| format!("execute if data storage {ns}:runtime dict_keys{{c:{c}}}");
        let quote_open = |state: u8| {
            vec![
                format!("{} run scoreboard players set #dk_q mcfc 1", is("'\"'")),
                format!("{} run scoreboard players set #dk_q mcfc 2", is("\"'\"")),
                format!(
                    "execute unless score #dk_q mcfc matches 0 run scoreboard players set #dk_state mcfc {state}"
                ),
            ]
        };
        let emit_key = format!(
            "execute store result {s}.end int 1 run scoreboard players get #dk_pos mcfc\nfunction {ns}:generated/dict_keys_emit with {s}"
        );
        let mut s0 = vec![
            format!("{} run return 0", is("\" \"")),
            "scoreboard players set #dk_q mcfc 0".to_string(),
            format!(
                "{} run return run scoreboard players set #dk_state mcfc 9",
                is("\"}\"")
            ),
        ];
        s0.extend(quote_open(2));
        s0.push("scoreboard players operation #dk_start mcfc = #dk_pos mcfc".to_string());
        s0.push(
            "execute if score #dk_state mcfc matches 2 run scoreboard players add #dk_start mcfc 1"
                .to_string(),
        );
        s0.push(
            "execute if score #dk_state mcfc matches 0 run scoreboard players set #dk_state mcfc 1"
                .to_string(),
        );
        s0.push(format!(
            "execute store result {s}.start int 1 run scoreboard players get #dk_start mcfc"
        ));
        let string_step = |close_state: u8, emit: bool| {
            let mut lines = vec![
                "execute if score #dk_esc mcfc matches 1 run return run scoreboard players set #dk_esc mcfc 0".to_string(),
                format!("{} run return run scoreboard players set #dk_esc mcfc 1", is("\"\\\\\"")),
                "scoreboard players set #dk_close mcfc 0".to_string(),
                format!("execute if score #dk_q mcfc matches 1 {} run scoreboard players set #dk_close mcfc 1", is("'\"'").trim_start_matches("execute ")),
                format!("execute if score #dk_q mcfc matches 2 {} run scoreboard players set #dk_close mcfc 1", is("\"'\"").trim_start_matches("execute ")),
                "execute if score #dk_close mcfc matches 0 run return 0".to_string(),
            ];
            if emit {
                lines.push(emit_key.clone());
            }
            lines.push(format!(
                "scoreboard players set #dk_state mcfc {close_state}"
            ));
            lines
        };
        let mut s3 = vec!["scoreboard players set #dk_q mcfc 0".to_string()];
        s3.extend(quote_open(4));
        s3.push("execute unless score #dk_q mcfc matches 0 run return 0".to_string());
        s3.push(format!(
            "{} run return run scoreboard players add #dk_depth mcfc 1",
            is("\"{\"")
        ));
        s3.push(format!(
            "{} run return run scoreboard players add #dk_depth mcfc 1",
            is("\"[\"")
        ));
        s3.push(format!("execute if score #dk_depth mcfc matches 0 {} run return run scoreboard players set #dk_state mcfc 9", is("\"}\"").trim_start_matches("execute ")));
        s3.push(format!(
            "{} run return run scoreboard players remove #dk_depth mcfc 1",
            is("\"}\"")
        ));
        s3.push(format!(
            "{} run return run scoreboard players remove #dk_depth mcfc 1",
            is("\"]\"")
        ));
        s3.push(format!("execute if score #dk_depth mcfc matches 0 {} run scoreboard players set #dk_state mcfc 0", is("\",\"").trim_start_matches("execute ")));
        let helpers: Vec<(&str, Vec<String>)> = vec![
            (
                "dict_keys",
                vec![
                    format!("function {ns}:generated/dict_keys_text with {s}"),
                    // Drop the opening brace; an empty or unprintable dict has no keys.
                    format!("execute unless data {s}{{text:\"\"}} run data modify {s}.text set string {s}.text 1"),
                    format!("data modify {s}.orig set from {s}.text"),
                    "scoreboard players set #dk_state mcfc 0".to_string(),
                    "scoreboard players set #dk_pos mcfc 0".to_string(),
                    "scoreboard players set #dk_esc mcfc 0".to_string(),
                    "scoreboard players set #dk_depth mcfc 0".to_string(),
                    format!("execute unless data {s}{{text:\"\"}} run function {ns}:generated/dict_keys_char"),
                ],
            ),
            (
                "dict_keys_text",
                vec![format!("$data modify {s}.text set value '$(src)'")],
            ),
            (
                "dict_keys_emit",
                vec![format!("$data modify {s}.keys append string {s}.orig $(start) $(end)")],
            ),
            (
                "dict_keys_char",
                vec![
                    format!("data modify {s}.c set string {s}.text 0 1"),
                    format!("data modify {s}.text set string {s}.text 1"),
                    format!("function {ns}:generated/dict_keys_state"),
                    "scoreboard players add #dk_pos mcfc 1".to_string(),
                    format!("execute unless score #dk_state mcfc matches 9 unless data {s}{{text:\"\"}} run function {ns}:generated/dict_keys_char"),
                ],
            ),
            (
                // One handler per character, so a state change waits for the next one.
                "dict_keys_state",
                (0..=5)
                    .map(|state| {
                        format!(
                            "execute if score #dk_state mcfc matches {state} run return run function {ns}:generated/dict_keys_s{state}"
                        )
                    })
                    .collect(),
            ),
            ("dict_keys_s0", s0),
            (
                "dict_keys_s5",
                vec![
                    format!("execute unless data {s}{{c:\":\"}} run return 0"),
                    "scoreboard players set #dk_state mcfc 3".to_string(),
                ],
            ),
            (
                "dict_keys_s1",
                vec![
                    format!("execute unless data {s}{{c:\":\"}} run return 0"),
                    emit_key.clone(),
                    "scoreboard players set #dk_state mcfc 3".to_string(),
                ],
            ),
            ("dict_keys_s2", string_step(5, true)),
            ("dict_keys_s3", s3),
            ("dict_keys_s4", string_step(3, false)),
        ];
        for (name, body) in helpers {
            self.files.insert(
                format!("data/{ns}/function/generated/{name}.mcfunction"),
                body.join("\n") + "\n",
            );
        }
    }

    /// Write the shared merge sort for `array<int>` (`sort_*`) or
    /// `array<float>` (`fsort_*`, comparing through `/compute`), which works on the
    /// `sort` storage compound: `src` is the input, `q` a queue of sorted runs,
    /// `a` and `b` the two runs being merged into `c`. `generated/sort_slice`
    /// does at most `SORT_STEPS_PER_TICK` steps, one element each, and sets
    /// `#sort_done` once `q[0]` holds the result. Each step reads or removes
    /// index 0 only, so no macros run. Callers swap their own state in and
    /// out, so sorts at different sites can be in progress at the same time.
    fn write_sort_helpers(&mut self, float: bool) {
        let ns = self.namespace.clone();
        let s = format!("storage {ns}:runtime sort");
        let p = if float { "fsort" } else { "sort" };
        let at = |path: &str| {
            format!("{{type:\"storage\",storage:\"{ns}:runtime\",path:\"sort.{path}\"}}")
        };
        // `#sort_cmp` is below 0 when `left < right`.
        let compare = |left: &str, right: &str| {
            format!(
                "execute store result score #sort_cmp mcfc run compute default float {{type:\"sub\",left:{},right:{}}}",
                at(left),
                at(right)
            )
        };
        let (slice, run, step) = if float {
            (
                vec![format!("function {ns}:generated/{p}_loop")],
                vec![
                    compare("src[0]", "last"),
                    format!(
                        "execute if score #sort_cmp mcfc matches ..-1 run data modify {s}.q append value []"
                    ),
                    format!("data modify {s}.q[-1] append from {s}.src[0]"),
                    format!("data modify {s}.last set from {s}.src[0]"),
                    format!("data remove {s}.src[0]"),
                ],
                vec![
                    compare("b[0]", "a[0]"),
                    format!(
                        "execute if score #sort_cmp mcfc matches 0.. run data modify {s}.c append from {s}.a[0]"
                    ),
                    format!("execute if score #sort_cmp mcfc matches 0.. run data remove {s}.a[0]"),
                    format!(
                        "execute if score #sort_cmp mcfc matches ..-1 run data modify {s}.c append from {s}.b[0]"
                    ),
                    format!(
                        "execute if score #sort_cmp mcfc matches ..-1 run data remove {s}.b[0]"
                    ),
                ],
            )
        } else {
            (
                vec![
                    format!("execute store result score #sort_last mcfc run data get {s}.last"),
                    format!("function {ns}:generated/{p}_loop"),
                    format!(
                        "execute store result {s}.last int 1 run scoreboard players get #sort_last mcfc"
                    ),
                ],
                vec![
                    format!("execute store result score #sort_b mcfc run data get {s}.src[0]"),
                    format!(
                        "execute if score #sort_b mcfc < #sort_last mcfc run data modify {s}.q append value []"
                    ),
                    format!("data modify {s}.q[-1] append from {s}.src[0]"),
                    "scoreboard players operation #sort_last mcfc = #sort_b mcfc".to_string(),
                    format!("data remove {s}.src[0]"),
                ],
                vec![
                    format!("execute store result score #sort_a mcfc run data get {s}.a[0]"),
                    format!("execute store result score #sort_b mcfc run data get {s}.b[0]"),
                    format!(
                        "execute if score #sort_a mcfc <= #sort_b mcfc run data modify {s}.c append from {s}.a[0]"
                    ),
                    format!(
                        "execute if score #sort_a mcfc <= #sort_b mcfc run data remove {s}.a[0]"
                    ),
                    format!(
                        "execute if score #sort_a mcfc > #sort_b mcfc run data modify {s}.c append from {s}.b[0]"
                    ),
                    format!(
                        "execute if score #sort_a mcfc > #sort_b mcfc run data remove {s}.b[0]"
                    ),
                ],
            )
        };
        let mut slice_lines = vec![
            "scoreboard players set #sort_done mcfc 0".to_string(),
            format!("scoreboard players set #sort_budget mcfc {SORT_STEPS_PER_TICK}"),
        ];
        slice_lines.extend(slice);
        let helpers = [
            ("slice", slice_lines),
            (
                // Eight steps per call keeps the loop overhead low; steps
                // after the sort is done return at once.
                "loop",
                vec![
                    format!("function {ns}:generated/{p}_unit"),
                    format!("function {ns}:generated/{p}_unit"),
                    format!("function {ns}:generated/{p}_unit"),
                    format!("function {ns}:generated/{p}_unit"),
                    format!("function {ns}:generated/{p}_unit"),
                    format!("function {ns}:generated/{p}_unit"),
                    format!("function {ns}:generated/{p}_unit"),
                    format!("function {ns}:generated/{p}_unit"),
                    "scoreboard players remove #sort_budget mcfc 8".to_string(),
                    format!(
                        "execute if score #sort_done mcfc matches 0 if score #sort_budget mcfc matches 1.. run function {ns}:generated/{p}_loop"
                    ),
                ],
            ),
            (
                "unit",
                vec![
                    "execute if score #sort_done mcfc matches 1 run return 0".to_string(),
                    format!(
                        "execute if data {s}.src[0] run return run function {ns}:generated/{p}_run"
                    ),
                    format!(
                        "execute if data {s}.a[0] if data {s}.b[0] run return run function {ns}:generated/{p}_step"
                    ),
                    format!("function {ns}:generated/{p}_next"),
                ],
            ),
            // Move one input element onto the last run, or start a new run
            // when it is smaller than the previous element.
            ("run", run),
            ("step", step),
            (
                // Finish the current merge, then take the next two runs off
                // the front of the queue. One run left means done.
                "next",
                vec![
                    format!("data modify {s}.c append from {s}.a[]"),
                    format!("data modify {s}.c append from {s}.b[]"),
                    format!("execute if data {s}.c[0] run data modify {s}.q append from {s}.c"),
                    format!("data modify {s}.a set value []"),
                    format!("data modify {s}.b set value []"),
                    format!("data modify {s}.c set value []"),
                    format!(
                        "execute unless data {s}.q[1] run return run scoreboard players set #sort_done mcfc 1"
                    ),
                    format!("data modify {s}.a set from {s}.q[0]"),
                    format!("data modify {s}.b set from {s}.q[1]"),
                    format!("data remove {s}.q[0]"),
                    format!("data remove {s}.q[0]"),
                ],
            ),
        ];
        for (name, body) in helpers {
            self.files.insert(
                format!("data/{ns}/function/generated/{p}_{name}.mcfunction"),
                body.join("\n") + "\n",
            );
        }
    }

    /// Lower `xs.sort()`. The first slice runs right away; if the array is
    /// not sorted by then, the function suspends and a per-site tick
    /// function runs one slice per tick until it is, then resumes.
    #[allow(clippy::too_many_arguments)]
    fn emit_sort(
        &mut self,
        function: &IrFunction,
        depth: usize,
        receiver: &IrExpr,
        tail: &[ContinuationItem],
        guard: &Guard,
        loop_ctx: Option<&LoopContext>,
        resume_context: Option<&ContextResume>,
        lines: &mut Vec<String>,
    ) {
        let ns = self.namespace.clone();
        let float = receiver.ty == Type::Array(Box::new(Type::Float));
        self.write_sort_helpers(float);
        let (helper, lowest) = if float {
            ("fsort", "-3.4e38f")
        } else {
            ("sort", "-2147483648")
        };
        let state = string_slot(depth, &function.name, &format!("_sort{}", self.new_temp()));
        let mut prefix = Vec::new();
        if let Some(rendered) =
            self.render_storage_expr_lvalue_path(function, depth, receiver, &mut prefix)
        {
            prefix.push(self.storage_path_command(
                format!(
                    "data modify storage {ns}:runtime {} set from storage {ns}:runtime {state}.q[0]",
                    rendered.path
                ),
                rendered.macro_storage,
            ));
        }
        prefix.push(format!("data remove storage {ns}:runtime {state}"));
        let continuation = self.emit_sleep_continuation(
            function,
            depth,
            tail,
            guard,
            loop_ctx,
            resume_context,
            prefix,
        );
        let slice = [
            format!("data modify storage {ns}:runtime sort set from storage {ns}:runtime {state}"),
            format!("function {ns}:generated/{helper}_slice"),
            format!("data modify storage {ns}:runtime {state} set from storage {ns}:runtime sort"),
        ];
        let (tick_path, tick_name) = self.new_block(function, depth, "sort_tick");
        let mut tick_lines = slice.to_vec();
        tick_lines.push(format!(
            "execute if score #sort_done mcfc matches 0 run schedule function {ns}:{tick_name} 1t"
        ));
        tick_lines.push(format!(
            "execute if score #sort_done mcfc matches 1 run function {ns}:{continuation}"
        ));
        self.files.insert(tick_path, tick_lines.join("\n") + "\n");

        let mut stmt_lines = vec![format!(
            "data modify storage {ns}:runtime {state} set value {{q:[[]],a:[],b:[],c:[],last:{lowest}}}"
        )];
        if let Some(rendered) =
            self.render_storage_expr_lvalue_path(function, depth, receiver, &mut stmt_lines)
        {
            stmt_lines.push(self.storage_path_command(
                format!(
                    "data modify storage {ns}:runtime {state}.src set from storage {ns}:runtime {}",
                    rendered.path
                ),
                rendered.macro_storage,
            ));
        }
        stmt_lines.extend(slice);
        stmt_lines.push(format!(
            "execute if score #sort_done mcfc matches 0 run schedule function {ns}:{tick_name} 1t"
        ));
        self.extend_guarded(lines, guard, stmt_lines);
        self.suspend_or_continue(
            function,
            depth,
            "if score #sort_done mcfc matches 0",
            &continuation,
            guard,
            lines,
        );
    }

    /// Lower a statement whose call can pause: `f()`, `let x = f()`,
    /// `x = f()` or `return f()`. The rest of the function goes in a
    /// continuation. If `f` pauses, it stores the continuation's name in its
    /// frame and calls it when it finishes; otherwise the continuation runs
    /// right away.
    #[allow(clippy::too_many_arguments)]
    fn emit_suspending_call(
        &mut self,
        function: &IrFunction,
        depth: usize,
        stmt: &IrStmt,
        call: &IrExpr,
        callee: &str,
        tail: &[ContinuationItem],
        guard: &Guard,
        loop_ctx: Option<&LoopContext>,
        resume_context: Option<&ContextResume>,
        lines: &mut Vec<String>,
    ) {
        let ns = self.namespace.clone();
        let callee_depth = depth + 1;
        let callee_susp = susp_slot(callee_depth, callee);
        let resume = string_slot(callee_depth, callee, "__resume");
        let result = return_slot(callee_depth, callee, &call.ty);
        let bind = |target: SlotRef| match call.ty {
            Type::Void => None,
            Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => {
                Some(format!(
                    "scoreboard players operation {} mcfc = {} mcfc",
                    target.numeric_name(),
                    result.numeric_name()
                ))
            }
            _ => Some(format!(
                "data modify storage {ns}:runtime {} set from storage {ns}:runtime {}",
                target.storage_path(),
                result.storage_path()
            )),
        };
        let mut prefix = Vec::new();
        match stmt {
            IrStmt::Let { name, .. }
            | IrStmt::Assign {
                target: IrAssignTarget::Variable(name),
                ..
            } => prefix.extend(bind(local_slot(depth, &function.name, name, &call.ty))),
            IrStmt::Return(_) => {
                prefix.extend(bind(return_slot(depth, &function.name, &call.ty)));
                prefix.push(format!(
                    "scoreboard players set {} mcfc 1",
                    control_slot(depth, &function.name)
                ));
            }
            _ => {}
        }
        let continuation = self.emit_sleep_continuation(
            function,
            depth,
            tail,
            guard,
            loop_ctx,
            resume_context,
            prefix,
        );
        let mut stmt_lines = vec![
            format!("data remove storage {ns}:runtime {resume}"),
            format!("scoreboard players set {callee_susp} mcfc 0"),
        ];
        let scratch = self.new_temp();
        self.compile_expr_into_named_slot(function, depth, call, &scratch, &mut stmt_lines);
        stmt_lines.push(format!(
            "execute if score {callee_susp} mcfc matches 1 run data modify storage {ns}:runtime {resume}.fn set value \"{ns}:{continuation}\""
        ));
        self.extend_guarded(lines, guard, stmt_lines);
        self.suspend_or_continue(
            function,
            depth,
            &format!("if score {callee_susp} mcfc matches 1"),
            &continuation,
            guard,
            lines,
        );
    }

    /// When `condition` holds, suspend this function; otherwise run the
    /// continuation now. The continuation finishes the whole function, so
    /// afterwards the function counts as returned and enclosing blocks and
    /// loops stop.
    fn suspend_or_continue(
        &mut self,
        function: &IrFunction,
        depth: usize,
        condition: &str,
        continuation: &str,
        guard: &Guard,
        lines: &mut Vec<String>,
    ) {
        let ns = self.namespace.clone();
        let ctrl = control_slot(depth, &function.name);
        let susp = susp_slot(depth, &function.name);
        lines.push(guard.wrap(format!(
            "execute {condition} run scoreboard players set {susp} mcfc 1"
        )));
        lines.push(guard.wrap(format!(
            "execute {condition} run scoreboard players set {ctrl} mcfc 1"
        )));
        let (path, name) = self.new_block(function, depth, "continue_now");
        self.files.insert(
            path,
            format!("function {ns}:{continuation}\nscoreboard players set {ctrl} mcfc 1\n"),
        );
        lines.push(guard.wrap(format!("function {ns}:{name}")));
    }

    /// The last line of every continuation: once the function has really
    /// finished (not paused again), resume the caller that is waiting on it.
    /// The stored name is removed before the call so it runs only once.
    fn finish_line(&mut self, function: &IrFunction, depth: usize) -> String {
        let ns = self.namespace.clone();
        let resume = string_slot(depth, &function.name, "__resume");
        let relative = format!(
            "generated/{}__d{}__finish",
            path_name(&function.name),
            depth
        );
        self.files.insert(
            format!("data/{ns}/function/{relative}.mcfunction"),
            [
                format!("data modify storage {ns}:runtime resume_call set from storage {ns}:runtime {resume}"),
                format!("data remove storage {ns}:runtime {resume}"),
                format!("function {ns}:generated/resume_caller with storage {ns}:runtime resume_call"),
            ]
            .join("\n")
                + "\n",
        );
        self.files.insert(
            format!("data/{ns}/function/generated/resume_caller.mcfunction"),
            "$function $(fn)\n".to_string(),
        );
        format!(
            "execute if score {} mcfc matches 0 if data storage {ns}:runtime {resume} run function {ns}:{relative}",
            susp_slot(depth, &function.name)
        )
    }

    /// Write the text of the float at `source` to `target`. A macro prints
    /// floats without a leading zero (`.5`, `-.5`), so a shared helper adds
    /// it back. The helper uses fixed scratch storage, which is safe because
    /// it never calls back into user code.
    fn float_to_text(&mut self, source: &str, target: &str, lines: &mut Vec<String>) {
        let ns = self.namespace.clone();
        let scratch = format!("storage {ns}:runtime float_text");
        let helpers = [
            (
                "float_text_raw",
                "$data modify storage NS:runtime float_text.out set value \"$(v)\"",
            ),
            (
                "float_text_zero",
                "$data modify storage NS:runtime float_text.out set value \"0$(out)\"",
            ),
            (
                "float_text_negative_zero",
                "$data modify storage NS:runtime float_text.out set value \"-0$(rest)\"",
            ),
        ];
        for (name, body) in helpers {
            self.files.insert(
                format!("data/{ns}/function/generated/{name}.mcfunction"),
                body.replace("NS", &ns) + "\n",
            );
        }
        let flag = "#float_text mcfc";
        let main = [
            format!("function {ns}:generated/float_text_raw with {scratch}"),
            format!("data modify {scratch}.head set string {scratch}.out 0 1"),
            format!(
                "execute store success score {flag} run data modify {scratch}.head set value \".\""
            ),
            format!(
                "execute if score {flag} matches 0 run function {ns}:generated/float_text_zero with {scratch}"
            ),
            format!("data modify {scratch}.head set string {scratch}.out 0 2"),
            format!(
                "execute store success score {flag} run data modify {scratch}.head set value \"-.\""
            ),
            format!(
                "execute if score {flag} matches 0 run data modify {scratch}.rest set string {scratch}.out 1"
            ),
            format!(
                "execute if score {flag} matches 0 run function {ns}:generated/float_text_negative_zero with {scratch}"
            ),
        ];
        self.files.insert(
            format!("data/{ns}/function/generated/float_text.mcfunction"),
            main.join("\n") + "\n",
        );
        lines.push(format!(
            "data modify {scratch}.v set from storage {ns}:runtime {source}"
        ));
        lines.push(format!("function {ns}:generated/float_text"));
        lines.push(format!(
            "data modify storage {ns}:runtime {target} set from {scratch}.out"
        ));
    }

    fn write_macro_placeholder_values(
        &mut self,
        function: &IrFunction,
        depth: usize,
        storage_base: &str,
        placeholders: &[IrMacroPlaceholder],
        lines: &mut Vec<String>,
    ) {
        for placeholder in placeholders {
            let source_temp = self.new_temp();
            let source_slot = local_slot(depth, &function.name, &source_temp, &placeholder.ty);
            self.compile_expr_into_slot(function, depth, &placeholder.expr, &source_slot, lines);
            let target_path = format!("{}.{}", storage_base, placeholder.key);
            match placeholder.ty {
                Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => lines.push(format!(
                    "execute store result storage {}:runtime {} int 1 run scoreboard players get {} mcfc",
                    self.namespace,
                    target_path,
                    source_slot.numeric_name()
                )),
                Type::Float => self.float_to_text(source_slot.storage_path(), &target_path, lines),
                Type::String | Type::Nbt | Type::TextDef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::EntitySet | Type::EntityRef | Type::PlayerRef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.selector",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::BlockRef => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}.pos",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::Array(_)
                | Type::Dict(_)
                | Type::Optional(_)
                | Type::Struct(_)
                | Type::EntityDef
                | Type::BlockDef
                | Type::ItemDef
                | Type::ItemSlot
                | Type::Bossbar => lines.push(format!(
                    "data modify storage {}:runtime {} set from storage {}:runtime {}",
                    self.namespace,
                    target_path,
                    self.namespace,
                    source_slot.storage_path()
                )),
                Type::Void => {}
            }
        }
    }

    fn function_entry_path(&self, function: &str, depth: usize) -> String {
        format!(
            "data/{}/function/{}.mcfunction",
            self.namespace,
            self.function_entry_name(function, depth)
        )
    }

    fn new_block(&mut self, function: &IrFunction, depth: usize, label: &str) -> (String, String) {
        self.block_counter += 1;
        let relative = format!(
            "generated/{}__d{}__{}_{}",
            path_name(&function.name),
            depth,
            label,
            self.block_counter
        );
        (
            format!("data/{}/function/{}.mcfunction", self.namespace, relative),
            relative,
        )
    }

    fn new_temp(&mut self) -> String {
        self.temp_counter += 1;
        format!("__tmp{}", self.temp_counter)
    }
}

fn is_score_type(ty: &Type) -> bool {
    matches!(ty, Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_))
}

/// A comparison that `execute if score` can test directly.
fn is_score_comparison(op: BinaryOp, left: &IrExpr, right: &IrExpr) -> bool {
    matches!(
        op,
        BinaryOp::Eq
            | BinaryOp::NotEq
            | BinaryOp::Lt
            | BinaryOp::Lte
            | BinaryOp::Gt
            | BinaryOp::Gte
    ) && is_score_type(&left.ty)
        && is_score_type(&right.ty)
}

/// A condition `compile_condition` turns into clauses without emitting any
/// command, so it is safe to test even when an `&&` would short-circuit it.
fn is_simple_condition(expr: &IrExpr) -> bool {
    let plain = |expr: &IrExpr| {
        matches!(
            expr.kind,
            IrExprKind::Variable(_) | IrExprKind::Int(_) | IrExprKind::Bool(_)
        )
    };
    match &expr.kind {
        IrExprKind::Variable(_) => true,
        IrExprKind::Binary { op, left, right } if is_score_comparison(*op, left, right) => {
            plain(left) && plain(right)
        }
        IrExprKind::Binary {
            op: BinaryOp::And,
            left,
            right,
        } => is_simple_condition(left) && is_simple_condition(right),
        _ => false,
    }
}

fn negate_clause(clause: &str) -> String {
    match clause.strip_prefix("if ") {
        Some(rest) => format!("unless {rest}"),
        None => format!("if {}", clause.strip_prefix("unless ").unwrap_or(clause)),
    }
}

/// The `matches` range for `score <op> value`, if it fits in a score.
fn literal_range(op: BinaryOp, value: i64) -> Option<String> {
    let fits = |bound: i64| (i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&bound);
    let range = match op {
        BinaryOp::Eq | BinaryOp::NotEq => format!("{value}"),
        BinaryOp::Lt => format!("..{}", value.checked_sub(1).filter(|v| fits(*v))?),
        BinaryOp::Lte => format!("..{value}"),
        BinaryOp::Gt => format!("{}..", value.checked_add(1).filter(|v| fits(*v))?),
        BinaryOp::Gte => format!("{value}.."),
        _ => return None,
    };
    fits(value).then_some(range)
}

fn continuation_after_stmts(stmts: &[IrStmt], tail: &[ContinuationItem]) -> Vec<ContinuationItem> {
    stmts
        .iter()
        .cloned()
        .map(|stmt| ContinuationItem::Stmt(Box::new(stmt)))
        .chain(tail.iter().cloned())
        .collect()
}

#[derive(Debug, Clone)]
struct SlotRef {
    name: String,
}

impl SlotRef {
    fn numeric_name(&self) -> &str {
        &self.name
    }

    fn storage_path(&self) -> &str {
        &self.name
    }
}

fn local_slot(depth: usize, function: &str, name: &str, ty: &Type) -> SlotRef {
    match ty {
        Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => SlotRef {
            name: numeric_slot(depth, function, name),
        },
        Type::String
        | Type::Float
        | Type::Array(_)
        | Type::Dict(_)
        | Type::Optional(_)
        | Type::Struct(_)
        | Type::EntityDef
        | Type::BlockDef
        | Type::ItemDef
        | Type::TextDef
        | Type::ItemSlot
        | Type::Bossbar
        | Type::EntitySet
        | Type::EntityRef
        | Type::PlayerRef
        | Type::BlockRef
        | Type::Nbt => SlotRef {
            name: string_slot(depth, function, name),
        },
        Type::Void => SlotRef {
            name: "__void".to_string(),
        },
    }
}

fn return_slot(depth: usize, function: &str, ty: &Type) -> SlotRef {
    match ty {
        Type::Int | Type::Bool | Type::Enum(_) | Type::Class(_) | Type::Generic(..) => SlotRef {
            name: numeric_return_slot(depth, function),
        },
        Type::String
        | Type::Float
        | Type::Array(_)
        | Type::Dict(_)
        | Type::Optional(_)
        | Type::Struct(_)
        | Type::EntityDef
        | Type::BlockDef
        | Type::ItemDef
        | Type::TextDef
        | Type::ItemSlot
        | Type::Bossbar
        | Type::EntitySet
        | Type::EntityRef
        | Type::PlayerRef
        | Type::BlockRef
        | Type::Nbt => SlotRef {
            name: string_return_slot(depth, function),
        },
        Type::Void => SlotRef {
            name: "__void".to_string(),
        },
    }
}

/// Set to 1 while the function is paused (sleeping, waiting on a host call,
/// sorting or waiting on a paused callee), so its caller pauses too.
fn susp_slot(depth: usize, function: &str) -> String {
    format!("$d{}_{}__susp", depth, sanitize(function))
}

/// Functions that can pause: they sleep, wait on a host call, sort, or call a
/// function that can. `async:` bodies pause on their own and don't count.
pub(crate) fn suspending_functions(program: &IrProgram) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    loop {
        let before = set.len();
        for function in &program.functions {
            if stmts_can_pause(&function.body, &set) {
                set.insert(function.name.clone());
            }
        }
        if set.len() == before {
            return set;
        }
    }
}

fn stmts_can_pause(stmts: &[IrStmt], set: &BTreeSet<String>) -> bool {
    let mut found = false;
    visit_stmt_exprs(stmts, &mut |expr| found |= expr_calls_any(expr, set));
    found
        || stmts.iter().any(|stmt| match stmt {
            IrStmt::Sleep { .. } | IrStmt::HostCall { .. } => true,
            IrStmt::Expr(expr) => sort_receiver(expr).is_some(),
            IrStmt::If {
                then_body,
                else_body,
                ..
            } => stmts_can_pause(then_body, set) || stmts_can_pause(else_body, set),
            IrStmt::While { body, step, .. } => {
                stmts_can_pause(body, set) || stmts_can_pause(step, set)
            }
            IrStmt::For { body, .. } | IrStmt::Context { body, .. } => stmts_can_pause(body, set),
            _ => false,
        })
}

/// Call `visit` on every expression directly in `stmts` (not in nested
/// bodies, which the callers walk themselves).
fn visit_stmt_exprs<'a>(stmts: &'a [IrStmt], visit: &mut dyn FnMut(&'a IrExpr)) {
    for stmt in stmts {
        match stmt {
            IrStmt::Let { value, .. } | IrStmt::Expr(value) | IrStmt::Return(Some(value)) => {
                visit(value)
            }
            IrStmt::Assign { target, value } => {
                if let IrAssignTarget::Path(path) = target {
                    visit(&path.base);
                }
                visit(value);
            }
            IrStmt::If { condition, .. } | IrStmt::While { condition, .. } => visit(condition),
            IrStmt::For { iterable, .. } => visit(iterable),
            IrStmt::Context { anchor, .. } => visit(anchor),
            IrStmt::MacroCommand { placeholders, .. } => {
                placeholders.iter().for_each(|p| visit(&p.expr))
            }
            IrStmt::Sleep { duration, .. } => visit(duration),
            IrStmt::HostCall { args, .. } => args.iter().for_each(&mut *visit),
            _ => {}
        }
    }
}

/// The names of `set` functions called anywhere inside `expr`.
fn calls_in_expr<'a>(expr: &'a IrExpr, set: &BTreeSet<String>, out: &mut Vec<&'a str>) {
    let each = |exprs: &'a [IrExpr], out: &mut Vec<&'a str>| {
        exprs.iter().for_each(|e| calls_in_expr(e, set, out))
    };
    match &expr.kind {
        IrExprKind::Call { function, args } => {
            if set.contains(function) {
                out.push(function);
            }
            each(args, out);
        }
        IrExprKind::MethodCall { receiver, args, .. } => {
            calls_in_expr(receiver, set, out);
            each(args, out);
        }
        IrExprKind::ArrayLiteral(values) => each(values, out),
        IrExprKind::DictLiteral(values) | IrExprKind::StructLiteral { fields: values, .. } => {
            values.iter().for_each(|(_, v)| calls_in_expr(v, set, out))
        }
        IrExprKind::InterpolatedString { placeholders, .. } => placeholders
            .iter()
            .for_each(|p| calls_in_expr(&p.expr, set, out)),
        IrExprKind::Binary { left, right, .. }
        | IrExprKind::Bind {
            value: left,
            body: right,
            ..
        } => {
            calls_in_expr(left, set, out);
            calls_in_expr(right, set, out);
        }
        IrExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            calls_in_expr(condition, set, out);
            calls_in_expr(then_expr, set, out);
            calls_in_expr(else_expr, set, out);
        }
        IrExprKind::At { anchor, value } | IrExprKind::As { anchor, value } => {
            calls_in_expr(anchor, set, out);
            calls_in_expr(value, set, out);
        }
        IrExprKind::Unary { expr, .. }
        | IrExprKind::Single(expr)
        | IrExprKind::Exists(expr)
        | IrExprKind::HasData(expr)
        | IrExprKind::Cast { expr, .. } => calls_in_expr(expr, set, out),
        IrExprKind::Path(path) => calls_in_expr(&path.base, set, out),
        _ => {}
    }
}

fn expr_calls_any(expr: &IrExpr, set: &BTreeSet<String>) -> bool {
    let mut out = Vec::new();
    calls_in_expr(expr, set, &mut out);
    !out.is_empty()
}

/// A statement that is exactly a call to a function that can pause, in one
/// of the positions that can wait for it.
fn suspending_call<'a>(stmt: &'a IrStmt, set: &BTreeSet<String>) -> Option<(&'a IrExpr, &'a str)> {
    let value = match stmt {
        IrStmt::Let { value, .. }
        | IrStmt::Assign {
            target: IrAssignTarget::Variable(_),
            value,
        }
        | IrStmt::Expr(value)
        | IrStmt::Return(Some(value)) => value,
        _ => return None,
    };
    match &value.kind {
        IrExprKind::Call { function, .. } if set.contains(function) => Some((value, function)),
        _ => None,
    }
}

fn sort_receiver(expr: &IrExpr) -> Option<&IrExpr> {
    match &expr.kind {
        IrExprKind::MethodCall {
            receiver, method, ..
        } if method == "sort" && matches!(receiver.ty, Type::Array(_)) => Some(receiver),
        _ => None,
    }
}

/// Calls to functions that can pause in places that cannot wait, such as
/// inside a condition or another call's arguments. Returns (caller, callee).
pub(crate) fn misplaced_suspending_calls(program: &IrProgram) -> Vec<(String, String)> {
    fn walk(stmts: &[IrStmt], set: &BTreeSet<String>, out: &mut Vec<String>) {
        for stmt in stmts {
            let mut found = Vec::new();
            if let Some((call, _)) = suspending_call(stmt, set) {
                if let IrExprKind::Call { args, .. } = &call.kind {
                    args.iter()
                        .for_each(|arg| calls_in_expr(arg, set, &mut found));
                }
            } else {
                visit_stmt_exprs(std::slice::from_ref(stmt), &mut |expr| {
                    calls_in_expr(expr, set, &mut found)
                });
            }
            out.extend(found.into_iter().map(str::to_string));
            match stmt {
                IrStmt::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    walk(then_body, set, out);
                    walk(else_body, set, out);
                }
                IrStmt::While { body, step, .. } => {
                    walk(body, set, out);
                    walk(step, set, out);
                }
                IrStmt::For { body, .. } | IrStmt::Context { body, .. } => walk(body, set, out),
                _ => {}
            }
        }
    }
    let set = suspending_functions(program);
    let mut misplaced = Vec::new();
    for function in &program.functions {
        let mut callees = Vec::new();
        walk(&function.body, &set, &mut callees);
        callees.dedup();
        misplaced.extend(
            callees
                .into_iter()
                .map(|callee| (function.name.clone(), callee)),
        );
    }
    misplaced
}

fn control_slot(depth: usize, function: &str) -> String {
    format!("$d{}_{}__ctrl", depth, sanitize(function))
}

fn numeric_slot(depth: usize, function: &str, name: &str) -> String {
    if let Some(world) = name.strip_prefix(crate::types::WORLD_STATE_PREFIX) {
        return format!("$world_{}", sanitize(world));
    }
    format!("$d{}_{}_{}", depth, sanitize(function), sanitize(name))
}

fn numeric_return_slot(depth: usize, function: &str) -> String {
    format!("$d{}_{}__ret", depth, sanitize(function))
}

fn string_slot(depth: usize, function: &str, name: &str) -> String {
    if let Some(world) = name.strip_prefix(crate::types::WORLD_STATE_PREFIX) {
        return format!("world.{}", sanitize(world));
    }
    format!(
        "frames.d{}.{}.{}",
        depth,
        sanitize(function),
        sanitize(name)
    )
}

fn string_return_slot(depth: usize, function: &str) -> String {
    format!("frames.d{}.{}.__ret", depth, sanitize(function))
}

/// A function name as a resource path segment: `onJoin` -> `on_join`.
// ponytail: `fooBar` and `foo_bar` share a path; reject the pair if that bites.
fn path_name(function: &str) -> String {
    crate::parser::resource_name(&sanitize(function))
}

fn sanitize(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn equipment_slot_name(slot_name: &str) -> &'static str {
    match slot_name {
        "mainhand" => "weapon.mainhand",
        "offhand" => "weapon.offhand",
        "head" => "armor.head",
        "chest" => "armor.chest",
        "legs" => "armor.legs",
        "feet" => "armor.feet",
        _ => "weapon.mainhand",
    }
}

fn equipment_read_nbt_paths(slot_name: &str) -> &'static [&'static str] {
    match slot_name {
        "mainhand" => &["SelectedItem", "HandItems[0]"],
        "offhand" => &["Inventory[{Slot:-106b}]", "HandItems[1]"],
        "head" => &["Inventory[{Slot:103b}]", "ArmorItems[3]"],
        "chest" => &["Inventory[{Slot:102b}]", "ArmorItems[2]"],
        "legs" => &["Inventory[{Slot:101b}]", "ArmorItems[1]"],
        "feet" => &["Inventory[{Slot:100b}]", "ArmorItems[0]"],
        _ => &["SelectedItem"],
    }
}

/// Float expressions that `float_provider` fuses into one `/compute` provider.
fn is_float_op(expr: &IrExpr) -> bool {
    expr.ty == Type::Float
        && match &expr.kind {
            IrExprKind::Unary { .. } | IrExprKind::Binary { .. } | IrExprKind::Cast { .. } => true,
            IrExprKind::MethodCall { receiver, .. } => receiver.ty == Type::Float,
            _ => false,
        }
}

fn quoted(value: &str) -> String {
    format!("{:?}", value)
}

/// Render a TOML basic string. The debug escaping (`\\`, `\"`, control chars)
/// matches TOML's basic-string rules, which keeps Windows paths valid.
fn toml_string(value: &str) -> String {
    format!("{:?}", value)
}

fn expand_display_text_sugar(command: &str) -> String {
    expand_json_text_command_sugar(command)
        .or_else(|| expand_say_sugar(command))
        .unwrap_or_else(|| command.to_string())
}

fn expand_json_text_command_sugar(command: &str) -> Option<String> {
    if let Some(rest) = command.strip_prefix("tellraw ") {
        let (target, message) = split_first_word(rest)?;
        let json = quoted_message_to_selector_json(message.trim_start())?;
        return Some(format!("tellraw {} {}", target, json));
    }

    if let Some(rest) = command.strip_prefix("title ") {
        let (target, rest) = split_first_word(rest)?;
        let (mode, message) = split_first_word(rest.trim_start())?;
        if !matches!(mode, "title" | "subtitle" | "actionbar") {
            return None;
        }
        let json = quoted_message_to_selector_json(message.trim_start())?;
        return Some(format!("title {} {} {}", target, mode, json));
    }

    None
}

fn expand_say_sugar(command: &str) -> Option<String> {
    let message = command.strip_prefix("say ")?.trim_start();
    let text = parse_display_message(message)?;
    let json = selector_text_components(&text)?;
    Some(format!("tellraw @a {}", json))
}

fn quoted_message_to_selector_json(message: &str) -> Option<String> {
    let (text, trailing) = parse_quoted_message(message)?;
    trailing.trim().is_empty().then_some(())?;
    selector_text_components(&text)
}

fn display_text_components(text: &str) -> Option<String> {
    selector_text_components_with_self_replacement(text, Some("$(selector)"))
}

fn split_first_word(value: &str) -> Option<(&str, &str)> {
    let split_at = value
        .char_indices()
        .find_map(|(index, ch)| ch.is_whitespace().then_some(index))?;
    Some((&value[..split_at], &value[split_at..]))
}

fn parse_quoted_message(value: &str) -> Option<(String, &str)> {
    let mut chars = value.char_indices();
    if chars.next()?.1 != '"' {
        return None;
    }

    let mut text = String::new();
    let mut escaped = false;
    for (index, ch) in chars {
        if escaped {
            text.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => return Some((text, &value[index + ch.len_utf8()..])),
            _ => text.push(ch),
        }
    }
    None
}

fn parse_display_message(value: &str) -> Option<String> {
    if let Some((text, trailing)) = parse_quoted_message(value)
        && trailing.trim().is_empty()
    {
        return Some(text);
    }
    Some(value.to_string())
}

fn selector_text_components(text: &str) -> Option<String> {
    selector_text_components_with_self_replacement(text, None)
}

fn selector_text_components_with_self_replacement(
    text: &str,
    self_replacement: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    let mut plain = String::new();
    let mut changed = false;
    let mut index = 0usize;

    while index < text.len() {
        let rest = &text[index..];
        if let Some(selector_len) = selector_token_len(rest) {
            if !plain.is_empty() {
                parts.push(quoted(&plain));
                plain.clear();
            }
            let selector = &rest[..selector_len];
            let rendered_selector = if selector == "@s" {
                self_replacement.unwrap_or(selector)
            } else {
                selector
            };
            parts.push(format!("{{\"selector\":{}}}", quoted(rendered_selector)));
            index += selector_len;
            changed = true;
        } else {
            let ch = rest.chars().next().expect("non-empty string");
            plain.push(ch);
            index += ch.len_utf8();
        }
    }

    if !plain.is_empty() {
        parts.push(quoted(&plain));
    }

    changed.then(|| format!("[{}]", parts.join(",")))
}

fn selector_token_len(value: &str) -> Option<usize> {
    let bytes = value.as_bytes();
    if bytes.len() < 2 || bytes[0] != b'@' || !matches!(bytes[1], b'p' | b'a' | b'r' | b's' | b'e')
    {
        return None;
    }

    if bytes.get(2) != Some(&b'[') {
        return Some(2);
    }

    let mut depth = 0usize;
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in value.char_indices().skip(2) {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(quote_ch) = quote {
            if ch == quote_ch {
                quote = None;
            }
            continue;
        }
        if ch == '"' || ch == '\'' {
            quote = Some(ch);
            continue;
        }
        if ch == '[' {
            depth += 1;
        } else if ch == ']' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return Some(index + ch.len_utf8());
            }
        }
    }

    Some(2)
}

fn context_execute_keyword(kind: ContextKind) -> &'static str {
    match kind {
        ContextKind::As => "as",
        ContextKind::At => "at",
    }
}

fn render_path_segments(segments: &[PathSegment]) -> String {
    let mut rendered = String::new();
    for segment in segments {
        match segment {
            PathSegment::Field(name) => push_path_name(&mut rendered, name),
            PathSegment::Index(index) => {
                let value = match &index.kind {
                    crate::ast::ExprKind::Int(value) => *value,
                    _ => 0,
                };
                rendered.push_str(&format!("[{}]", value));
            }
        }
    }
    rendered
}

fn render_nbt_path_segments(segments: &[PathSegment]) -> String {
    let mut rendered = String::new();
    for segment in segments {
        match segment {
            PathSegment::Field(name) => push_path_name(&mut rendered, name),
            PathSegment::Index(index) => match &index.kind {
                crate::ast::ExprKind::Int(value) => rendered.push_str(&format!("[{}]", value)),
                crate::ast::ExprKind::String(value) => push_quoted_path_name(&mut rendered, value),
                _ => rendered.push_str("[0]"),
            },
        }
    }
    rendered
}

fn push_path_name(rendered: &mut String, name: &str) {
    if !rendered.is_empty() {
        rendered.push('.');
    }
    rendered.push_str(name);
}

fn push_quoted_path_name(rendered: &mut String, name: &str) {
    if !rendered.is_empty() {
        rendered.push('.');
    }
    rendered.push_str(&quoted(name));
}

fn push_quoted_macro_path_name(rendered: &mut String, placeholder: &str) {
    if !rendered.is_empty() {
        rendered.push('.');
    }
    rendered.push('"');
    rendered.push_str(&format!("$({})", placeholder));
    rendered.push('"');
}

fn normalize_runtime_nbt_segments<'a>(
    base_ty: &Type,
    segments: &'a [PathSegment],
) -> &'a [PathSegment] {
    if matches!(base_ty, Type::EntityRef | Type::PlayerRef | Type::BlockRef)
        && segments.len() > 1
        && matches!(segments.first(), Some(PathSegment::Field(field)) if field == "nbt")
    {
        &segments[1..]
    } else {
        segments
    }
}

fn infer_dynamic_nbt_index_type(function: &IrFunction, expr: &crate::ast::Expr) -> Option<Type> {
    match &expr.kind {
        crate::ast::ExprKind::Int(_) => Some(Type::Int),
        crate::ast::ExprKind::Float(_) => Some(Type::Float),
        crate::ast::ExprKind::String(_) => Some(Type::String),
        crate::ast::ExprKind::Variable(name) => function.locals.get(name).cloned(),
        crate::ast::ExprKind::Unary { expr, .. } => infer_dynamic_nbt_index_type(function, expr),
        crate::ast::ExprKind::Binary { .. }
        | crate::ast::ExprKind::Call { .. }
        | crate::ast::ExprKind::MethodCall { .. }
        | crate::ast::ExprKind::Bool(_)
        | crate::ast::ExprKind::Path(_)
        | crate::ast::ExprKind::ArrayLiteral(_)
        | crate::ast::ExprKind::DictLiteral(_)
        | crate::ast::ExprKind::StructLiteral { .. }
        | crate::ast::ExprKind::New { .. }
        | crate::ast::ExprKind::Conditional { .. }
        | crate::ast::ExprKind::InstanceOf { .. }
        | crate::ast::ExprKind::Cast { .. }
        | crate::ast::ExprKind::Lambda { .. }
        | crate::ast::ExprKind::MethodRef { .. }
        | crate::ast::ExprKind::Switch { .. } => None,
    }
}

/// An ordered piece of an interpolated string: either literal text or the
/// (zero-based) index of a `$(...)` placeholder.
enum TemplateSegment {
    Literal(String),
    Placeholder(usize),
}

/// Split an interpolated template into ordered literal/placeholder segments.
/// Shares the `$(...)` scanning rules with [`rewrite_macro_template`] (paren
/// matching, nested quotes) and is char-based so multi-byte literals survive.
fn split_macro_template(template: &str, placeholder_count: usize) -> Vec<TemplateSegment> {
    let chars: Vec<char> = template.chars().collect();
    let mut index = 0usize;
    let mut segments = Vec::new();
    let mut literal = String::new();
    let mut placeholder_index = 0usize;
    while index < chars.len() {
        if index + 1 < chars.len() && chars[index] == '$' && chars[index + 1] == '(' {
            if !literal.is_empty() {
                segments.push(TemplateSegment::Literal(std::mem::take(&mut literal)));
            }
            index += 2;
            let mut paren_depth = 1usize;
            let mut in_string = false;
            let mut string_delim = '"';
            while index < chars.len() {
                let ch = chars[index];
                if in_string {
                    if ch == '\\' {
                        index += 2;
                        continue;
                    }
                    if ch == string_delim {
                        in_string = false;
                    }
                    index += 1;
                    continue;
                }
                match ch {
                    '"' | '\'' => {
                        in_string = true;
                        string_delim = ch;
                    }
                    '(' => paren_depth += 1,
                    ')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            if placeholder_index < placeholder_count {
                segments.push(TemplateSegment::Placeholder(placeholder_index));
                placeholder_index += 1;
            }
        } else {
            literal.push(chars[index]);
        }
        index += 1;
    }
    if !literal.is_empty() {
        segments.push(TemplateSegment::Literal(literal));
    }
    segments
}

fn rewrite_macro_template(template: &str, placeholders: &[IrMacroPlaceholder]) -> String {
    // Iterate over chars, not bytes: the structural delimiters (`$ ( ) " ' \`)
    // are all ASCII, but the literal text between placeholders may contain
    // multi-byte UTF-8 (e.g. “ ” — ·). Indexing `as_bytes()` and pushing
    // `byte as char` would split those into mojibake.
    let chars: Vec<char> = template.chars().collect();
    let mut index = 0usize;
    let mut out = String::new();
    let mut placeholder_index = 0usize;
    while index < chars.len() {
        if index + 1 < chars.len() && chars[index] == '$' && chars[index + 1] == '(' {
            let start = index + 2;
            index = start;
            let mut paren_depth = 1usize;
            let mut in_string = false;
            let mut string_delim = '"';
            while index < chars.len() {
                let ch = chars[index];
                if in_string {
                    if ch == '\\' {
                        index += 2;
                        continue;
                    }
                    if ch == string_delim {
                        in_string = false;
                    }
                    index += 1;
                    continue;
                }
                match ch {
                    '"' | '\'' => {
                        in_string = true;
                        string_delim = ch;
                    }
                    '(' => paren_depth += 1,
                    ')' => {
                        paren_depth -= 1;
                        if paren_depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            if let Some(placeholder) = placeholders.get(placeholder_index) {
                out.push_str(&format!("$({})", placeholder.key));
                placeholder_index += 1;
            }
        } else {
            out.push(chars[index]);
        }
        index += 1;
    }
    out
}

fn player_state_objective(segments: &[PathSegment]) -> String {
    format!("mcfs_{}", sanitize(&render_path_segments(segments)))
}

fn entity_state_objective(segments: &[PathSegment]) -> String {
    format!("mcfe_{}", sanitize(&render_path_segments(segments)))
}

fn state_objective(ref_kind: RefKind, segments: &[PathSegment]) -> String {
    if ref_kind == RefKind::Player {
        player_state_objective(segments)
    } else {
        entity_state_objective(segments)
    }
}

fn player_item_slot_index_from_path(path: &IrPathExpr) -> Option<(&str, &crate::ast::Expr)> {
    let PathSegment::Field(namespace) = path.segments.first()? else {
        return None;
    };
    if !matches!(namespace.as_str(), "inventory" | "hotbar") {
        return None;
    }
    let PathSegment::Index(index) = path.segments.get(1)? else {
        return None;
    };
    Some((namespace.as_str(), index))
}

fn player_slot_nbt_index(namespace: &str, logical_index: i64) -> i64 {
    match namespace {
        "inventory" => logical_index + 9,
        _ => logical_index,
    }
}

fn player_item_command_slot(namespace: &str, logical_index: i64) -> String {
    match namespace {
        "inventory" => format!("inventory.{}", logical_index),
        _ => format!("hotbar.{}", logical_index),
    }
}

fn collect_block_builder_state_fields(
    program: &IrProgram,
) -> BTreeMap<String, BTreeMap<String, Vec<String>>> {
    let mut fields = BTreeMap::<String, BTreeMap<String, BTreeMap<String, ()>>>::new();
    for function in &program.functions {
        collect_block_builder_state_fields_from_stmts(&function.name, &function.body, &mut fields);
    }
    fields
        .into_iter()
        .map(|(function, vars)| {
            (
                function,
                vars.into_iter()
                    .map(|(name, fields)| (name, fields.into_keys().collect()))
                    .collect(),
            )
        })
        .collect()
}

fn collect_block_builder_state_fields_from_stmts(
    function_name: &str,
    stmts: &[IrStmt],
    fields: &mut BTreeMap<String, BTreeMap<String, BTreeMap<String, ()>>>,
) {
    for stmt in stmts {
        match stmt {
            IrStmt::Assign {
                target: IrAssignTarget::Path(path),
                ..
            } => collect_block_builder_state_fields_from_path(function_name, path, fields),
            IrStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_block_builder_state_fields_from_stmts(function_name, then_body, fields);
                collect_block_builder_state_fields_from_stmts(function_name, else_body, fields);
            }
            IrStmt::While { body, step, .. } => {
                collect_block_builder_state_fields_from_stmts(function_name, body, fields);
                collect_block_builder_state_fields_from_stmts(function_name, step, fields);
            }
            IrStmt::Context { body, .. } | IrStmt::For { body, .. } => {
                collect_block_builder_state_fields_from_stmts(function_name, body, fields);
            }
            IrStmt::Async { function, .. } => {
                collect_block_builder_state_fields_from_stmts(
                    &function.name,
                    &function.body,
                    fields,
                );
            }
            _ => {}
        }
    }
}

fn collect_block_builder_state_fields_from_path(
    function_name: &str,
    path: &IrPathExpr,
    fields: &mut BTreeMap<String, BTreeMap<String, BTreeMap<String, ()>>>,
) {
    if path.base.ty != Type::BlockDef {
        return;
    }
    let IrExprKind::Variable(name) = &path.base.kind else {
        return;
    };
    let [PathSegment::Field(root), PathSegment::Field(field), ..] = path.segments.as_slice() else {
        return;
    };
    if root != "states" {
        return;
    }
    fields
        .entry(function_name.to_string())
        .or_default()
        .entry(name.clone())
        .or_default()
        .insert(field.clone(), ());
}

fn collect_state_objectives(program: &IrProgram) -> Vec<ManagedObjective> {
    let mut names = BTreeMap::<String, Option<String>>::new();
    for state in &program.player_states {
        if !matches!(state.ty, Type::Int | Type::Bool) {
            names.insert(
                format!(
                    "__storage_{}_{}",
                    state.owner == crate::ast::StateOwner::Player,
                    state.path.join(".")
                ),
                None,
            );
            continue;
        }
        let segments = state
            .path
            .iter()
            .map(|segment| PathSegment::Field(segment.clone()))
            .collect::<Vec<_>>();
        names.insert(
            if state.owner == crate::ast::StateOwner::Player {
                player_state_objective(&segments)
            } else {
                entity_state_objective(&segments)
            },
            if state.owner == crate::ast::StateOwner::Player {
                Some(state.display_name.clone())
            } else {
                None
            },
        );
    }
    for function in &program.functions {
        collect_objectives_from_stmts(&function.body, &mut names);
    }
    names
        .into_iter()
        .filter(|(objective, _)| !objective.starts_with("__storage_"))
        .map(|(objective, display_name)| ManagedObjective {
            objective,
            display_name,
        })
        .collect()
}

fn collect_objectives_from_stmts(stmts: &[IrStmt], names: &mut BTreeMap<String, Option<String>>) {
    for stmt in stmts {
        match stmt {
            IrStmt::Assign {
                target: IrAssignTarget::Path(path),
                ..
            } => collect_objectives_from_path(path, names),
            IrStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_objectives_from_expr(condition, names);
                collect_objectives_from_stmts(then_body, names);
                collect_objectives_from_stmts(else_body, names);
            }
            IrStmt::While {
                condition,
                body,
                step,
            } => {
                collect_objectives_from_expr(condition, names);
                collect_objectives_from_stmts(body, names);
                collect_objectives_from_stmts(step, names);
            }
            IrStmt::For { iterable, body, .. } => {
                collect_objectives_from_expr(iterable, names);
                collect_objectives_from_stmts(body, names);
            }
            IrStmt::Context { anchor, body, .. } => {
                collect_objectives_from_expr(anchor, names);
                collect_objectives_from_stmts(body, names);
            }
            IrStmt::Async { function, .. } => {
                collect_objectives_from_stmts(&function.body, names);
            }
            IrStmt::HostCall { args, .. } => {
                for arg in args {
                    collect_objectives_from_expr(arg, names);
                }
            }
            IrStmt::Let { value, .. }
            | IrStmt::Return(Some(value))
            | IrStmt::Sleep {
                duration: value, ..
            }
            | IrStmt::Expr(value) => collect_objectives_from_expr(value, names),
            IrStmt::MacroCommand { .. }
            | IrStmt::RawCommand(_)
            | IrStmt::Break
            | IrStmt::Continue
            | IrStmt::Return(None)
            | IrStmt::Assign { .. } => {}
        }
    }
}

fn collect_objectives_from_expr(expr: &IrExpr, names: &mut BTreeMap<String, Option<String>>) {
    match &expr.kind {
        IrExprKind::Path(path) => collect_objectives_from_path(path, names),
        IrExprKind::Unary { expr, .. }
        | IrExprKind::Single(expr)
        | IrExprKind::Exists(expr)
        | IrExprKind::HasData(expr)
        | IrExprKind::Cast { expr, .. } => collect_objectives_from_expr(expr, names),
        IrExprKind::Binary { left, right, .. } => {
            collect_objectives_from_expr(left, names);
            collect_objectives_from_expr(right, names);
        }
        IrExprKind::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            collect_objectives_from_expr(condition, names);
            collect_objectives_from_expr(then_expr, names);
            collect_objectives_from_expr(else_expr, names);
        }
        IrExprKind::Bind { value, body, .. } => {
            collect_objectives_from_expr(value, names);
            collect_objectives_from_expr(body, names);
        }
        IrExprKind::Call { args, .. } => {
            for arg in args {
                collect_objectives_from_expr(arg, names);
            }
        }
        IrExprKind::ArrayLiteral(values) => {
            for value in values {
                collect_objectives_from_expr(value, names);
            }
        }
        IrExprKind::DictLiteral(entries) => {
            for (_, value) in entries {
                collect_objectives_from_expr(value, names);
            }
        }
        IrExprKind::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                collect_objectives_from_expr(value, names);
            }
        }
        IrExprKind::MethodCall { receiver, args, .. } => {
            collect_objectives_from_expr(receiver, names);
            for arg in args {
                collect_objectives_from_expr(arg, names);
            }
        }
        IrExprKind::InterpolatedString { placeholders, .. } => {
            for placeholder in placeholders {
                collect_objectives_from_expr(&placeholder.expr, names);
            }
        }
        IrExprKind::At { anchor, value } | IrExprKind::As { anchor, value } => {
            collect_objectives_from_expr(anchor, names);
            collect_objectives_from_expr(value, names);
        }
        IrExprKind::Int(_)
        | IrExprKind::Float(_)
        | IrExprKind::Bool(_)
        | IrExprKind::String(_)
        | IrExprKind::Variable(_)
        | IrExprKind::Selector(_)
        | IrExprKind::Block(_) => {}
    }
}

fn collect_objectives_from_path(path: &IrPathExpr, names: &mut BTreeMap<String, Option<String>>) {
    if path.segments.len() > 1
        && matches!(path.segments.first(), Some(PathSegment::Field(name)) if name == "state")
        && matches!(path.base.ty, Type::EntityRef | Type::PlayerRef)
    {
        let is_player = path.base.ref_kind == RefKind::Player;
        let mut fields = Vec::new();
        let mut storage = false;
        for segment in path.segments.iter().skip(1) {
            let PathSegment::Field(field) = segment else {
                break;
            };
            fields.push(field.as_str());
            storage |= names.contains_key(&format!("__storage_{}_{}", is_player, fields.join(".")));
        }
        if !storage {
            names
                .entry(state_objective(path.base.ref_kind, &path.segments[1..]))
                .or_insert(None);
        }
    }
    collect_objectives_from_expr(&path.base, names);
}

fn macro_storage_base(depth: usize, function: &str, macro_id: usize) -> String {
    format!(
        "frames.d{}.{}.__macro{}",
        depth,
        sanitize(function),
        macro_id
    )
}

fn render_tag_file(values: &[String]) -> String {
    let body = values
        .iter()
        .map(|value| format!("    \"{}\"", value))
        .collect::<Vec<_>>()
        .join(",\n");
    format!("{{\n  \"values\": [\n{}\n  ]\n}}\n", body)
}

fn has_special_tick(program: &IrProgram) -> bool {
    program.functions.iter().any(|function| {
        !function.generated
            && function.name == "tick"
            && function.params.is_empty()
            && function.return_type == Type::Void
    })
}

fn discover_bukkit_runtime(program: &IrProgram) -> BukkitRuntime {
    let mut runtime = BukkitRuntime::default();
    let mut command_objectives = BTreeSet::new();
    for function in &program.functions {
        // Async bodies inside a handler are named after it, such as
        // `__mcfc_command_buy__async_1`; only the declared handler is a hook.
        if function.generated {
            continue;
        }
        let name = &function.name;
        if let Some(event) = name.strip_prefix("__mcfc_agent_event_") {
            if function.params.len() == 1
                && function.params[0].ty
                    == Type::Struct(crate::language_catalog::event_type_name(event))
            {
                runtime.agent_handlers.push(AgentEventHandler {
                    event: event.to_string(),
                    handler: name.clone(),
                    parameter: function.params[0].name.clone(),
                    decision: ir_function_contains_cancel(function),
                });
            }
            continue;
        }
        if let Some(kind) = name.strip_prefix("__mcfc_event_") {
            match kind {
                "player_join" => runtime.join_handlers.push(name.clone()),
                "player_death" => runtime.death_handlers.push(name.clone()),
                _ if crate::language_catalog::ADVANCEMENT_EVENTS
                    .iter()
                    .any(|(event, _)| *event == kind) =>
                {
                    runtime
                        .advancement_handlers
                        .push((kind.to_string(), name.clone()));
                }
                _ => {}
            }
            continue;
        }
        if let Some(command) = name.strip_prefix("__mcfc_command_") {
            let (command, hex) = command.rsplit_once("__menu_").unwrap_or((command, ""));
            let objective = bukkit_command_objective(command, &mut command_objectives);
            if name.contains("__menu_") {
                let bytes = (0..hex.len())
                    .step_by(2)
                    .filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                    .collect::<Vec<_>>();
                runtime.menu_buttons.push((
                    String::from_utf8_lossy(&bytes).into_owned(),
                    objective.clone(),
                ));
            }
            runtime.commands.push(BukkitCommand {
                command: command.to_string(),
                handler: name.clone(),
                objective,
            });
            continue;
        }
        if let Some(rest) = name.strip_prefix("__mcfc_task_") {
            if let Some((task, ticks)) = rest.rsplit_once("_every_ticks_") {
                if let Ok(ticks) = ticks.parse::<u32>() {
                    runtime.every_tasks.push((task.to_string(), ticks));
                }
            } else if let Some((task, ticks)) = rest.rsplit_once("_after_ticks_")
                && let Ok(ticks) = ticks.parse::<u32>()
            {
                runtime.after_tasks.push((task.to_string(), ticks));
            }
        }
    }
    runtime
}

/// Ray steps (0.1 blocks) covering the player's `block` or `entity` reach, plus
/// one block because reach is measured to the target's edge, not its inside.
fn ray_reach_steps(kind: &str) -> String {
    format!(
        "execute store result score #ray_steps mcfc run attribute @s minecraft:{kind}_interaction_range get 10
scoreboard players add #ray_steps mcfc 10"
    )
}

pub(crate) fn ir_function_contains_cancel(function: &IrFunction) -> bool {
    fn contains_expr(value: &IrExpr) -> bool {
        match &value.kind {
            IrExprKind::MethodCall {
                method,
                receiver,
                args,
            } => method == "cancel" || contains_expr(receiver) || args.iter().any(contains_expr),
            IrExprKind::Call { args, .. } | IrExprKind::ArrayLiteral(args) => {
                args.iter().any(contains_expr)
            }
            IrExprKind::Binary { left, right, .. }
            | IrExprKind::Bind {
                value: left,
                body: right,
                ..
            } => contains_expr(left) || contains_expr(right),
            IrExprKind::Conditional {
                condition,
                then_expr,
                else_expr,
            } => contains_expr(condition) || contains_expr(then_expr) || contains_expr(else_expr),
            IrExprKind::Unary { expr, .. }
            | IrExprKind::Single(expr)
            | IrExprKind::Exists(expr)
            | IrExprKind::HasData(expr)
            | IrExprKind::Cast { expr, .. } => contains_expr(expr),
            IrExprKind::At { anchor, value } | IrExprKind::As { anchor, value } => {
                contains_expr(anchor) || contains_expr(value)
            }
            IrExprKind::DictLiteral(values) | IrExprKind::StructLiteral { fields: values, .. } => {
                values.iter().any(|(_, value)| contains_expr(value))
            }
            _ => false,
        }
    }
    fn contains_statements(values: &[IrStmt]) -> bool {
        values.iter().any(|statement| match statement {
            IrStmt::Expr(value) => contains_expr(value),
            IrStmt::Let { value, .. } => contains_expr(value),
            IrStmt::Assign { value, .. } => contains_expr(value),
            IrStmt::If {
                condition,
                then_body,
                else_body,
            } => {
                contains_expr(condition)
                    || contains_statements(then_body)
                    || contains_statements(else_body)
            }
            IrStmt::While {
                condition,
                body,
                step,
            } => contains_expr(condition) || contains_statements(body) || contains_statements(step),
            IrStmt::For { body, .. } | IrStmt::Context { body, .. } => contains_statements(body),
            IrStmt::Async { function, .. } => contains_statements(&function.body),
            IrStmt::Return(Some(value)) => contains_expr(value),
            _ => false,
        })
    }
    contains_statements(&function.body)
}

fn is_bukkit_generated_function(name: &str) -> bool {
    name.starts_with("__mcfc_event_")
        || name.starts_with("__mcfc_agent_event_")
        || name.starts_with("__mcfc_command_")
        || name.starts_with("__mcfc_task_")
        || name.starts_with("__mcfc_test_")
}

fn bukkit_command_objective(command: &str, used: &mut BTreeSet<String>) -> String {
    let sanitized = sanitize(command);
    let base = if sanitized.is_empty() {
        "command"
    } else {
        sanitized.as_str()
    };
    // `@Command("buy")` is `/trigger buy`. Two names that sanitize alike get `_1`, `_2`...
    if used.insert(base.to_string()) {
        return base.to_string();
    }
    for counter in 1u32.. {
        let objective = format!("{}_{}", base, base36(counter));
        if used.insert(objective.clone()) {
            return objective;
        }
    }
    unreachable!("unbounded command objective counter should always produce a value")
}

fn base36(mut value: u32) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    loop {
        out.push(DIGITS[(value % 36) as usize] as char);
        value /= 36;
        if value == 0 {
            break;
        }
    }
    out.iter().rev().collect()
}

fn bukkit_join_tag(namespace: &str) -> String {
    let suffix: String = sanitize(namespace).chars().take(24).collect();
    format!("mcfc_join_{}", suffix)
}

fn program_uses_rpc(program: &IrProgram) -> bool {
    program
        .functions
        .iter()
        .any(|function| stmts_use_rpc(&function.body))
}

fn stmts_use_rpc(stmts: &[IrStmt]) -> bool {
    stmts.iter().any(stmt_uses_rpc)
}

fn stmt_uses_rpc(stmt: &IrStmt) -> bool {
    match stmt {
        IrStmt::HostCall { .. } => true,
        IrStmt::If {
            then_body,
            else_body,
            ..
        } => stmts_use_rpc(then_body) || stmts_use_rpc(else_body),
        IrStmt::While { body, step, .. } => stmts_use_rpc(body) || stmts_use_rpc(step),
        IrStmt::For { body, .. } | IrStmt::Context { body, .. } => stmts_use_rpc(body),
        IrStmt::Async { function, .. } => stmts_use_rpc(&function.body),
        _ => false,
    }
}
