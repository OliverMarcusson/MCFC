use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::{Function, Program, Type};
use crate::backend::{self, BackendOptions, BuildArtifacts, ExportedFunction};
use crate::diagnostics::Diagnostics;
use crate::diagnostics::{Diagnostic, Span};
use crate::ir::{self, IrProgram};
use crate::modules::{self, ModuleSource};
use crate::optimizer;
use crate::pack_opt;
use crate::parser;
use crate::project::{HelperConfig, collect_asset_files, collect_source_files, load_manifest};
use crate::types::{self, HostModules, TypedProgram};

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub namespace: String,
    pub emit_ast: bool,
    pub emit_ir: bool,
    pub clean: bool,
    pub load_tag_values: Option<Vec<String>>,
    pub tick_tag_values: Option<Vec<String>>,
    pub exports: Vec<ExportedFunction>,
    pub optimize: bool,
    /// Run the whole-pack optimizer on the emitted commands (`pack_opt`).
    pub optimize_pack: bool,
    pub helper: Option<HelperConfig>,
    /// Module layout of a merged multi-file source; empty for a single source.
    pub modules: Vec<ModuleSource>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            namespace: "mcfc".to_string(),
            emit_ast: false,
            emit_ir: false,
            clean: false,
            load_tag_values: None,
            tick_tag_values: None,
            exports: Vec::new(),
            optimize: true,
            optimize_pack: true,
            helper: None,
            modules: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompileResult {
    pub typed_program: TypedProgram,
    pub ir_program: IrProgram,
    pub artifacts: BuildArtifacts,
}

pub fn compile_source(
    source: &str,
    options: &CompileOptions,
) -> Result<CompileResult, Diagnostics> {
    let ast = parser::parse(source)?;
    if ast
        .functions
        .iter()
        .any(|function| function.name.starts_with("__mcfc_agent_event_"))
        && !options
            .helper
            .as_ref()
            .and_then(|helper| helper.agent.as_ref())
            .is_some_and(|agent| agent.enabled)
    {
        let mut diagnostics = Diagnostics::new();
        diagnostics.push(Diagnostic::new(
            "agent event handlers require '[helper.agent] enabled = true'",
            Span::new(1, 1),
        ));
        return Err(diagnostics);
    }
    validate_agent_manifest(options)?;
    let ast = modules::resolve(ast, &options.modules)?;
    let ast = normalize_special_functions(ast)?;
    let host_modules = HostModules::from_helper(options.helper.as_ref());
    let typed_program =
        prune_unreachable(types::type_check(&ast, &host_modules)?, &options.exports);
    let ir_program = ir::lower(&typed_program);
    validate_decision_handlers(&ir_program)?;
    validate_suspending_calls(&ir_program)?;
    let ir_program = if options.optimize {
        optimizer::optimize(ir_program)
    } else {
        ir_program
    };
    let mut artifacts = backend::generate(
        &ir_program,
        &BackendOptions {
            namespace: options.namespace.clone(),
            load_tag_values: options.load_tag_values.clone(),
            tick_tag_values: options.tick_tag_values.clone(),
            exports: options.exports.clone(),
            helper: options.helper.clone(),
        },
    );
    if options.optimize && options.optimize_pack {
        pack_opt::optimize(&mut artifacts.files);
    }
    Ok(CompileResult {
        typed_program,
        ir_program,
        artifacts,
    })
}

/// Drops functions nothing can reach, so unused helpers (and unused `std`
/// functions) never reach the datapack. Everything is still type-checked first.
/// Roots: `main`, `tick`, event/command/task handlers, `[[export]]` functions,
/// and the zero-argument `void` functions the backend auto-exports.
fn prune_unreachable(mut program: TypedProgram, exports: &[ExportedFunction]) -> TypedProgram {
    let mut reachable: BTreeSet<String> = program
        .functions
        .iter()
        .filter(|function| {
            let auto_exported = function.params.is_empty()
                && function.return_type == Type::Void
                && !function.name.starts_with("std::");
            function.name == "main"
                || function.name == "tick"
                || function.name.starts_with("__mcfc_")
                || auto_exported
                || exports
                    .iter()
                    .any(|export| export.function == function.name)
        })
        .map(|function| function.name.clone())
        .collect();
    let mut pending: Vec<String> = reachable.iter().cloned().collect();
    while let Some(name) = pending.pop() {
        let Some(function) = program.functions.iter().find(|f| f.name == name) else {
            continue;
        };
        for callee in &function.called_functions {
            if reachable.insert(callee.clone()) {
                pending.push(callee.clone());
            }
        }
    }
    program
        .functions
        .retain(|function| reachable.contains(&function.name));
    program
        .recursion_groups
        .retain(|name, _| reachable.contains(name));
    program
}

fn validate_suspending_calls(program: &IrProgram) -> Result<(), Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    // A paused frame lives in fixed slots that a recursive call would reuse.
    let suspending = backend::suspending_functions(program);
    for name in program.recursion_groups.keys() {
        if suspending.contains(name) {
            diagnostics.push(Diagnostic::new(
                format!("'{name}' is recursive, so it cannot sleep, sort or wait on a host call"),
                Span::new(1, 1),
            ));
        }
    }
    for (caller, callee) in backend::misplaced_suspending_calls(program) {
        diagnostics.push(Diagnostic::new(
            format!(
                "'{callee}' can pause (it sleeps, sorts or waits on a host call), so '{caller}' must call it on its own line, as 'var x = {callee}(...);', 'x = {callee}(...);' or 'return {callee}(...);'"
            ),
            Span::new(1, 1),
        ));
    }
    diagnostics.into_result(())
}

fn validate_decision_handlers(program: &IrProgram) -> Result<(), Diagnostics> {
    let cancellable = [
        "chat",
        "inventory_click",
        "player_action",
        "block_break",
        "player_interact_block",
        "player_interact_item",
        "entity_interact",
        "entity_attack",
        "item_held_change",
        "inventory_close",
        "player_swing",
        "player_action_toggle",
        "player_respawn_request",
        "item_rename",
        "trade_select",
        "sign_change",
        "book_edit",
        "beacon_effect",
        "recipe_place",
        "item_pick",
        "entity_teleport",
        "game_mode_request",
        "player_abilities",
    ];
    let mut diagnostics = Diagnostics::new();
    for function in &program.functions {
        let Some(event) = function.name.strip_prefix("__mcfc_agent_event_") else {
            continue;
        };
        if backend::ir_function_contains_cancel(function) && !cancellable.contains(&event) {
            diagnostics.push(Diagnostic::new(
                format!(
                    "event.cancel() is not supported for observation-only event '{}'",
                    event
                ),
                Span::new(1, 1),
            ));
        }
    }
    diagnostics.into_result(())
}

fn validate_agent_manifest(options: &CompileOptions) -> Result<(), Diagnostics> {
    let Some(agent) = options
        .helper
        .as_ref()
        .and_then(|helper| helper.agent.as_ref())
    else {
        return Ok(());
    };
    let observable = [
        "chat",
        "inventory_click",
        "player_action",
        "block_break",
        "player_interact_block",
        "player_interact_item",
        "entity_interact",
        "entity_attack",
        "item_held_change",
        "inventory_close",
        "player_swing",
        "player_action_toggle",
        "player_respawn_request",
        "item_rename",
        "trade_select",
        "sign_change",
        "book_edit",
        "beacon_effect",
        "recipe_place",
        "item_pick",
        "entity_teleport",
        "game_mode_request",
        "player_abilities",
        "player_connect",
        "player_quit",
        "player_respawn",
        "player_damage",
        "player_teleport",
        "player_item_drop",
        "player_item_pickup",
        "inventory_open",
        "game_mode_change",
    ];
    let mut diagnostics = Diagnostics::new();
    for event in &agent.events {
        if !observable.contains(&event.as_str()) {
            diagnostics.push(Diagnostic::new(
                format!("unknown 26.3 agent event '{}'", event),
                Span::new(1, 1),
            ));
        }
    }
    if !agent.cancel_events.is_empty() {
        diagnostics.push(Diagnostic::new(
            "[helper.agent].cancel_events was removed; it cannot cancel observation-only events. Call event.cancel() inside a cancellable typed event handler instead",
            Span::new(1, 1),
        ));
    }
    for command in &agent.commands {
        if !is_mcfc_identifier(command) {
            diagnostics.push(Diagnostic::new(
                format!(
                    "agent command '{}' is not a valid command identifier",
                    command
                ),
                Span::new(1, 1),
            ));
        }
    }
    diagnostics.into_result(())
}

fn is_mcfc_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

pub(crate) fn normalize_special_functions(mut program: Program) -> Result<Program, Diagnostics> {
    let mut diagnostics = Diagnostics::new();
    let tick_indices = program
        .functions
        .iter()
        .enumerate()
        .filter_map(|(index, function)| {
            if function.name == "tick" {
                Some(index)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let tick_void_indices = tick_indices
        .iter()
        .copied()
        .filter(|index| {
            let function = &program.functions[*index];
            function.params.is_empty() && function.return_type == Type::Void
        })
        .collect::<Vec<_>>();

    if !tick_void_indices.is_empty() {
        for index in tick_indices.iter().copied() {
            if !tick_void_indices.contains(&index) {
                diagnostics.push(Diagnostic::new(
                    "tick() is reserved for the datapack tick function when a zero-argument tick function is present",
                    program.functions[index].span.clone(),
                ));
            }
        }
        let first_index = tick_void_indices[0];
        let mut merged = Function {
            name: "tick".to_string(),
            is_pub: false,
            type_params: Vec::new(),
            bounds: Vec::new(),
            params: Vec::new(),
            return_type: Type::Void,
            body: Vec::new(),
            span: program.functions[first_index].span.clone(),
            end: program.functions[first_index].end,
            owner: None,
            module: String::new(),
            is_abstract: false,
            is_override: false,
            varargs: false,
        };
        for index in &tick_void_indices {
            merged.body.extend(program.functions[*index].body.clone());
        }
        let mut next_functions = Vec::with_capacity(program.functions.len());
        for (index, function) in program.functions.into_iter().enumerate() {
            if index == first_index {
                next_functions.push(merged.clone());
            } else if !tick_void_indices.contains(&index) {
                next_functions.push(function);
            }
        }
        program.functions = next_functions;
    } else {
        for index in tick_indices {
            let function = &program.functions[index];
            if function.params.is_empty() && function.return_type != Type::Void {
                diagnostics.push(Diagnostic::new(
                    "tick() must return 'void'",
                    function.span.clone(),
                ));
            }
        }
    }

    diagnostics.into_result(program)
}

pub fn compile_file(
    input: &Path,
    out_dir: &Path,
    options: &CompileOptions,
) -> Result<CompileResult, String> {
    let root = input.parent().unwrap_or(Path::new(""));
    let compiled = compile_module_tree(input, &[], root, options)?;
    write_output(out_dir, &compiled, options)?;
    Ok(compiled)
}

pub fn compile_project(
    manifest_path: &Path,
    out_dir: &Path,
    options: &CompileOptions,
) -> Result<CompileResult, String> {
    let manifest = load_manifest(manifest_path)?;
    let project_root = manifest_path.parent().ok_or_else(|| {
        format!(
            "manifest '{}' has no parent directory",
            manifest_path.display()
        )
    })?;
    let source_root = project_root.join(&manifest.source_dir);
    let asset_root = project_root.join(&manifest.asset_dir);
    let root_file = source_root.join("main.mcf");
    if !root_file.is_file() {
        return Err(format!(
            "project root module '{}' not found",
            root_file.display()
        ));
    }

    let mut effective = options.clone();
    effective.namespace = manifest.namespace.clone();
    if !manifest.load.is_empty() {
        effective.load_tag_values = Some(manifest.load.clone());
    }
    if !manifest.tick.is_empty() {
        effective.tick_tag_values = Some(manifest.tick.clone());
    }
    effective.exports = manifest
        .export
        .iter()
        .map(|item| ExportedFunction {
            path: item.path.clone(),
            // `util.announce` in the manifest names the function `util::announce`.
            function: item.function.replace('.', "::"),
        })
        .collect();
    if manifest.helper.is_some() {
        effective.helper = manifest.helper.clone();
    }

    let files = collect_source_files(&source_root)?;
    let mut compiled = compile_module_tree(&root_file, &files, project_root, &effective)?;
    copy_project_assets(&asset_root, &mut compiled.artifacts)?;
    write_output(out_dir, &compiled, &effective)?;
    Ok(compiled)
}

pub fn project_default_out_dir(manifest_path: &Path) -> Result<Option<PathBuf>, String> {
    let manifest = load_manifest(manifest_path)?;
    Ok(manifest
        .out_dir
        .map(|relative| manifest_path.parent().unwrap().join(relative)))
}

fn write_output(
    out_dir: &Path,
    compiled: &CompileResult,
    options: &CompileOptions,
) -> Result<(), String> {
    if options.clean && out_dir.exists() {
        fs::remove_dir_all(out_dir).map_err(|error| error.to_string())?;
    }
    fs::create_dir_all(out_dir).map_err(|error| error.to_string())?;
    for (relative, contents) in &compiled.artifacts.files {
        let destination = out_dir.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(destination, contents).map_err(|error| error.to_string())?;
    }
    if options.emit_ast {
        write_debug_file(
            &out_dir.join("debug").join("typed_program.txt"),
            format!("{:#?}\n", compiled.typed_program),
        )?;
    }
    if options.emit_ir {
        write_debug_file(
            &out_dir.join("debug").join("ir.txt"),
            format!("{:#?}\n", compiled.ir_program),
        )?;
    }
    Ok(())
}

fn write_debug_file(path: &Path, contents: String) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    fs::write(path, contents).map_err(|error| error.to_string())
}

pub fn canonicalize_output_path(out_dir: &Path) -> PathBuf {
    out_dir.to_path_buf()
}

/// Compiles `root_file` as the root module plus `files` as the modules their paths name.
fn compile_module_tree(
    root_file: &Path,
    files: &[PathBuf],
    display_root: &Path,
    options: &CompileOptions,
) -> Result<CompileResult, String> {
    let loaded = modules::load(root_file, files, &|file: &Path| {
        fs::read_to_string(file)
            .map_err(|error| format!("failed to read '{}': {}", file.display(), error))
    })?;
    let options = CompileOptions {
        modules: loaded.modules.clone(),
        ..options.clone()
    };
    let compiled = compile_source(&loaded.merged, &options)
        .map_err(|diagnostics| render_module_diagnostics(&diagnostics, &loaded, display_root))?;
    Ok(compiled)
}

fn render_module_diagnostics(
    diagnostics: &Diagnostics,
    loaded: &modules::LoadedModules,
    display_root: &Path,
) -> String {
    let locate = |line: usize| {
        let module = loaded
            .modules
            .iter()
            .rev()
            .find(|module| module.first_line <= line)
            .unwrap_or(&loaded.modules[0]);
        let shown = module
            .file
            .strip_prefix(display_root)
            .unwrap_or(&module.file);
        (
            format!("{}:", shown.display()),
            line.saturating_sub(module.first_line) + 1,
        )
    };
    diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.render_mapped(&loaded.merged, locate))
        .collect::<Vec<_>>()
        .join(
            "

",
        )
}

fn copy_project_assets(asset_root: &Path, artifacts: &mut BuildArtifacts) -> Result<(), String> {
    for file in collect_asset_files(asset_root)? {
        let relative = file
            .strip_prefix(asset_root)
            .map_err(|error| error.to_string())?;
        let normalized = relative.to_string_lossy().replace('\\', "/");
        let contents = fs::read_to_string(&file)
            .map_err(|error| format!("failed to read '{}': {}", file.display(), error))?;
        artifacts.files.insert(normalized, contents);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{CompileOptions, compile_source};

    /// These tests check how the backend lowers code, so they read its output
    /// before the whole-pack optimizer rewrites it.
    fn lowering() -> CompileOptions {
        CompileOptions {
            optimize_pack: false,
            ..CompileOptions::default()
        }
    }

    #[test]
    fn compiles_gameplay_entity_and_inventory_builtins() {
        let source = r#"
void main() {
    var pig = summon("minecraft:pig");
    pig.addTag("elite");
    var tagged = pig.hasTag("elite");
    pig.removeTag("elite");
    pig.team = "red";
    pig.mainhand.item = "minecraft:carrot_on_a_stick";
    pig.offhand.item = "minecraft:shield";
    pig.head.name = "Captain";
    pig.chest.count = 1;
    pig.effect("speed", 10, 1);
    pig.teleport(Block.of("~ ~1 ~"));
    pig.damage(2);
    pig.heal(1);
    pig.give("minecraft:apple", 2);
    pig.clear("minecraft:apple", 1);
    pig.lootGive("minecraft:chests/simple_dungeon");
    return;
}
"#;

        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(files.contains("summon $(entity) ~ ~ ~ $(data)"));
        assert!(files.contains("mcfc_summon_capture_"));
        assert!(files.contains("mcfc_summon_ref_"));
        assert!(files.contains("tag $(selector) add $(tag)"));
        assert!(files.contains("tag $(selector) remove $(tag)"));
        assert!(files.contains("if entity @s[tag=$(tag)]"));
        assert!(files.contains("team join $(team) $(selector)"));
        assert!(files.contains("item replace entity $(selector) weapon.mainhand with $(item_id)"));
        assert!(files.contains("item replace entity $(selector) weapon.offhand with $(item_id)"));
        assert!(files.contains(
            "item modify entity $(selector) armor.head {\"function\":\"minecraft:set_name\""
        ));
        assert!(files.contains(
            "item modify entity $(selector) armor.chest {\"function\":\"minecraft:set_count\""
        ));
        assert!(files.contains("effect give $(selector) $(effect) $(duration) $(amplifier) true"));
        assert!(files.contains("teleport $(selector) $(dest)"));
        assert!(files.contains("damage $(selector) $(amount)"));
        assert!(files.contains("store result entity $(selector) Health float 1"));
        assert!(files.contains("give $(selector) $(item) $(count)"));
        assert!(files.contains("clear $(selector) $(item) $(count)"));
        assert!(files.contains("loot give $(selector) loot $(table)"));
    }

    #[test]
    fn compiles_ui_audio_particle_and_world_builtins() {
        let source = r#"
void main() {
    var pig = Selector.of("@e[type=pig,limit=1]").getFirst();
    var pos = Block.of("~ ~ ~");
    pig.sendMessage("hello @s");
    pig.sendTitle("Danger");
    pig.sendActionBar("Run");
    var bb = new BossBar("mcfc:test", "Boss @s");
    pig.playSound("minecraft:entity.experience_orb.pickup", "master");
    pig.stopSound("master", "minecraft:entity.experience_orb.pickup");
    pos.spawnParticle("minecraft:flame");
    pos.spawnParticle("minecraft:smoke", 4, pig);
    pos.lootInsert("minecraft:chests/simple_dungeon");
    pos.lootSpawn("minecraft:chests/simple_dungeon");
    pos.setBlock("minecraft:stone");
    pos.fill(Block.of("~1 ~1 ~1"), "minecraft:glass");
    return;
}
"#;

        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(files.contains("tellraw $(selector) [\"hello \",{"));
        assert!(files.contains("title $(selector) title \"Danger\""));
        assert!(
            files.contains("generated/actionbar/show {json:[\"Run\"],priority:\"notification\"}")
        );
        assert!(files.contains("bossbar add $(id) [\"Boss \",{"));
        assert!(files.contains("playsound $(sound) $(category) $(selector)"));
        assert!(files.contains("stopsound $(selector) $(category) $(sound)"));
        assert!(files.contains("particle $(particle) $(pos) 0 0 0 0 $(count) force"));
        assert!(files.contains("loot insert $(pos) loot $(table)"));
        assert!(files.contains("loot spawn $(pos) loot $(table)"));
        assert!(files.contains("setblock $(pos) $(block)"));
        assert!(files.contains("fill $(from) $(to) $(block)"));
    }

    #[test]
    fn compiles_entity_and_block_builders() {
        let source = r#"
void main() {
    var pig = new EntityData("minecraft:pig");
    pig.setName("Builder Pig");
    pig.setNoAi(true);
    var spawned = summon(pig);
    var chest = new BlockData("minecraft:chest");
    chest.states.facing = "north";
    chest.setName("Loot");
    var pos = Block.of("~ ~ ~");
    pos.setBlock(chest);
    pos.fill(Block.of("~1 ~1 ~1"), chest);
    return;
}
"#;

        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(files.contains(".id set from storage"));
        assert!(files.contains(".nbt.CustomName set from storage"));
        assert!(files.contains(".nbt.NoAI set from storage"));
        assert!(files.contains("summon $(entity) ~ ~ ~ $(data)"));
        assert!(files.contains("setblock $(pos) $(block)"));
        assert!(files.contains("data merge block $(pos) $(data)"));
        assert!(files.contains("$(id)[facing=$(s1)]"));
        assert!(files.contains("fill $(from) $(to) $(block)"));
    }

    #[test]
    fn compiles_random_builtin_forms() {
        let source = r#"
int roll() {
    return random();
}
void main() {
    var any = random();
    var bounded = random(6);
    var between = random(1, 20);
    bounded = random(between);
    var combined = random() + roll();
    mcf("say $(random(1, 3))");
    return;
}
"#;

        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        assert!(files.contains("random value 0..2147483647"));
        assert!(files.contains("random value $(min)..$(max)"));
        assert!(files.contains("execute store result storage mcfc:runtime"));
        assert!(files.contains("with storage mcfc:runtime"));
    }

    #[test]
    fn compiles_interpolated_string_literals() {
        let source = r#"
void main() {
    var demo_title = "MCFC Demo $(random(100))";
    var player = Selector.of("@p").getFirst();
    player.sendMessage(demo_title);
    return;
}
"#;

        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        assert!(files.contains("random value $(min)..$(max)"));
        assert!(files.contains("set value \"MCFC Demo $(p1)\""));
        assert!(files.contains("data modify storage mcfc:runtime frames.d0.main.demo_title"));
    }

    #[test]
    fn interpolated_string_preserves_multibyte_literals() {
        // Multi-byte literal text around a placeholder must survive intact;
        // a byte-wise rewrite would corrupt “ ” into mojibake.
        let source = "
void main() {
    var q = \"hi\";
    var line = \"\u{201c}$(q)\u{201d} \u{2014} done\";
    var player = Selector.of(\"@p\").getFirst();
    player.sendMessage(line);
    return;
}
";
        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            files.contains("set value \"\u{201c}$(p1)\u{201d} \u{2014} done\""),
            "interpolated template should keep multi-byte characters intact"
        );
        assert!(
            !files.contains('\u{00e2}'),
            "no Latin-1 mojibake should leak into generated functions"
        );
    }

    #[test]
    fn text_interpolation_sources_dynamic_parts_from_storage() {
        // text("...$(x)...") must build a component that reads the runtime value
        // by NBT path, never splicing it into a quoted string (which a value
        // containing a quote could break).
        let source = "
void main() {
    var who = \"world\";
    var line = new Component(\"hi $(who)!\");
    var player = Selector.of(\"@p\").getFirst();
    player.sendMessage(line);
    return;
}
";
        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            files.contains("set value {text:\"\",extra:[\"hi \",{storage:\"mcfc:runtime\",nbt:")
                && files.contains("},\"!\"]}"),
            "text() interpolation should emit an nbt-sourced extra component"
        );
        // The dynamic part must not be spliced into a quoted literal.
        assert!(
            !files.contains("set value \"hi $(p1)!\""),
            "text() interpolation must not splice the value into a quoted string"
        );
    }

    #[test]
    fn compiles_sleep_continuations() {
        let source = r#"
void main() {
    var player = Selector.of("@p").getFirst();
    var flag = true;

    sleep(1);
    mc("say after straight sleep");

    if (flag) {
        sleep(1);
        mc("say after if sleep");
    }
    mc("say after if");

    Execute.at(player, () -> {
        sleep(1);
        mc("say after context sleep");
    });
    var i = 0;
    while (i < 2) {
        sleep(1);
        i = i + 1;
    }
    for (int n = 0; n < 2; n++) {
        sleep(1);
        mc("say after for sleep");
    }
    mc("say done");
    return;
}
"#;

        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result.artifacts.files;
        let joined = files.values().cloned().collect::<Vec<_>>().join("\n");
        let entry = files
            .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
            .unwrap();

        assert!(joined.contains("schedule function mcfc:generated/main__d0__sleep_resume_"));
        assert!(joined.contains("$(seconds)s"));
        assert!(joined.contains(
            "execute at $(selector) run function mcfc:generated/main__d0__sleep_context_"
        ));
        assert!(joined.contains("scoreboard players set $d0_main__ctrl mcfc 1"));
        assert!(joined.contains("say after straight sleep"));
        assert!(joined.contains("say after if sleep"));
        assert!(joined.contains("say after context sleep"));
        assert!(joined.contains("say after for sleep"));
        assert!(joined.contains("say done"));
        assert!(!entry.contains("say after straight sleep"));
    }

    #[test]
    fn compiles_async_blocks_and_entity_position() {
        let source = r#"
void main() {
    var player = Selector.of("@p").getFirst();
    var bb = new BossBar("mcfc:demo", "MCFC Bossbar");
    var count = 5;
    player.position.spawnParticle("minecraft:happy_villager", 20, player);
    Thread.start(() -> {
        sleep(5);
        player.sendMessage("later");
        player.position.setBlock("minecraft:gold_block");
    });
    count = 7;
    player.sendMessage("caller continues");
    return;
}
"#;

        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result.artifacts.files;
        let joined = files.values().cloned().collect::<Vec<_>>().join("\n");
        let entry = files
            .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
            .unwrap();

        assert!(joined.contains("bossbar add $(id) \"MCFC Bossbar\""));
        assert!(
            joined.contains("schedule function mcfc:generated/main__async_1__d0__sleep_resume_")
        );
        assert!(joined.contains(
            "prefix set value \"$(__anchor_prefix)execute at $(__anchor_selector) run \""
        ));
        assert!(joined.contains("setblock $(pos) $(block)"));
        assert!(entry.contains("function mcfc:generated/main__async_1__d0__entry"));
        assert!(joined.contains("caller continues"));
    }

    #[test]
    fn rejects_async_return_old_builtins_and_book_annotation() {
        let async_error = compile_source(
            r#"
void main() {
    Thread.start(() -> {
        return;
    });
}
"#,
            &lowering(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            async_error
                .contains("return may not appear inside the lambda passed to Thread.start(...)")
        );

        let legacy_error = compile_source(
            r#"
void main() {
    var player = Selector.of("@p").getFirst();
    tellraw(player, "old");
}
"#,
            &lowering(),
        )
        .unwrap_err()
        .to_string();
        assert!(legacy_error.contains("target.sendMessage(message)"));

        let book_error = compile_source(
            r#"
@book
void main() {
    return;
}
"#,
            &lowering(),
        )
        .unwrap_err()
        .to_string();
        assert!(book_error.contains("unknown annotation '@book'"));
    }

    #[test]
    fn rejects_invalid_random_and_sleep_usage() {
        let source = r#"
void main() {
    var bad_sleep = sleep(1);
    random(sleep(1));
    mcf("say $(sleep(1))");
    sleep(0);
    sleep("bad");
    var bad_random = random("bad");
    var too_many = random(1, 2, 3);
    return;
}
"#;

        let error = compile_source(source, &lowering()).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("sleep(...) may only appear as a standalone statement"));
        assert!(rendered.contains("sleep(...) seconds must be at least 1"));
        assert!(rendered.contains("sleep(...) seconds must have type 'int'"));
        assert!(rendered.contains("argument 1 for 'random' must be 'int'"));
        assert!(rendered.contains("wrong arity for 'random': expected 0, 1, or 2, found 3"));

        let string_error = compile_source(
            r#"
void main() {
    var bad = "value $(sleep(1))";
    return;
}
"#,
            &lowering(),
        )
        .unwrap_err()
        .to_string();
        assert!(string_error.contains("sleep(...) may only appear as a standalone statement"));
    }

    #[test]
    fn compiles_debug_builtins() {
        let source = r#"
void main() {
    var pig = Selector.of("@e[type=pig,limit=1]").getFirst();
    var pos = Block.of("~ ~1 ~");
    debug("checkpoint");
    pos.debugMarker("marker");
    pos.debugMarker("block marker", "minecraft:gold_block");
    pig.debugEntity("nearest pig");
    return;
}
"#;

        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(files.contains("[MCFC debug]"));
        assert!(files.contains("[MCFC marker]"));
        assert!(files.contains("particle minecraft:happy_villager $(pos)"));
        assert!(files.contains("playsound minecraft:block.note_block.pling master @a $(pos)"));
        assert!(files.contains("setblock $(pos) $(block) replace"));
        assert!(files.contains("[MCFC entity] found"));
        assert!(files.contains("effect give $(selector) minecraft:glowing 3 0 true"));
    }

    #[test]
    fn heal_rejects_player_and_ambiguous_targets() {
        let player_error = compile_source(
            r#"
void main() {
    var player = Selector.of("@p").getFirst();
    player.heal(1);
}
"#,
            &lowering(),
        )
        .unwrap_err()
        .to_string();
        assert!(player_error.contains("known non-player"));

        let ambiguous_error = compile_source(
            r#"
void main() {
    var target = Selector.of("@e").getFirst();
    target.heal(1);
}
"#,
            &lowering(),
        )
        .unwrap_err()
        .to_string();
        assert!(ambiguous_error.contains("ambiguous 'Entity'"));
    }

    #[test]
    fn async_and_sleep_in_handlers_do_not_register_extra_commands() {
        let result = compile_source(
            r#"
@Command("buy")
void buy() {
    var player = Selector.of("@s").getFirst();
    Thread.start(() -> {
        sleep(3);
        player.sendMessage("later");
    });
    sleepTicks(5);
    player.sendMessage("done");
}

@EventHandler
void onPlayerJoin(PlayerJoinEvent event) {
    Thread.start(() -> {
        sleep(1);
        debug("joined");
    });
}

@Every(ticks = 20)
void pulse() {
    Thread.start(() -> {
        sleep(1);
        debug("pulse");
    });
}
"#,
            &lowering(),
        )
        .expect("handlers with async and sleep should compile");
        let generated = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(generated.contains("scoreboard objectives add buy trigger"));
        assert!(!generated.contains("buy_1"), "extra trigger objective");
        let agent_commands = result
            .artifacts
            .files
            .keys()
            .filter(|path| path.contains("/agent/command/"))
            .collect::<Vec<_>>();
        assert_eq!(agent_commands.len(), 1, "{agent_commands:?}");
    }

    #[test]
    fn compiles_vanilla_bukkit_declarations() {
        let result = compile_source(
            r#"
@PlayerState("Coins") int coins;

@EventHandler
void onPlayerJoin(PlayerJoinEvent event) {
    Player player = event.player();
    player.state.coins = player.state.coins + 1;
}

@EventHandler
void onPlayerDeath(PlayerDeathEvent event) {
    debug("dead");
}

@Command("status")
void status() {
    debug("status");
}

@Every(ticks = 20)
void pulse() {
    debug("pulse");
}

@After(ticks = 5)
void later() {
    debug("later");
}
"#,
            &lowering(),
        )
        .expect("Bukkit declarations should compile");
        let files = &result.artifacts.files;
        let tick = files
            .get("data/minecraft/tags/function/tick.json")
            .expect("Bukkit runtime tick tag");
        assert!(tick.contains("generated/bukkit/tick"));
        let runtime = files
            .get("data/mcfc/function/generated/bukkit/tick.mcfunction")
            .expect("Bukkit runtime");
        assert!(runtime.contains("mcfc_join_mcfc"));
        assert!(runtime.contains("mcfc_deaths matches 1.."));
        assert!(runtime.contains("scores={status=1..}"));
        assert!(runtime.contains("#mcfct_pulse"));
        assert!(files.contains_key("data/mcfc/function/generated/bukkit/load.mcfunction"));
        assert!(files.contains_key("data/mcfc/function/generated/bukkit/player_join.mcfunction"));
        let generated = files.values().cloned().collect::<Vec<_>>().join("\n");
        assert!(generated.contains("@s"));
        assert!(!generated.contains("@s[limit=1]"));
    }

    #[test]
    fn bukkit_command_objectives_use_the_full_command_name() {
        let result = compile_source(
            r#"
@Command("abcdefghij_one")
void abcdefghijOne() {
    debug("one");
}

@Command("abcdefghij_two")
void abcdefghijTwo() {
    debug("two");
}
"#,
            &lowering(),
        )
        .expect("commands with shared prefixes should compile");
        let files = &result.artifacts.files;
        let setup = files
            .get("data/mcfc/function/generated/setup.mcfunction")
            .expect("setup function");
        assert!(setup.contains("scoreboard objectives add abcdefghij_one trigger"));
        assert!(setup.contains("scoreboard objectives add abcdefghij_two trigger"));
        let runtime = files
            .get("data/mcfc/function/generated/bukkit/tick.mcfunction")
            .expect("Bukkit runtime");
        assert!(runtime.contains("execute as @a[scores={abcdefghij_one=1..}] run function mcfc:generated/bukkit/command/abcdefghij_one"));
        assert!(runtime.contains("execute as @a[scores={abcdefghij_two=1..}] run function mcfc:generated/bukkit/command/abcdefghij_two"));
    }

    #[test]
    fn compiles_typed_agent_event_declaration_and_wrapper() {
        let options = CompileOptions {
            helper: Some(crate::project::HelperConfig {
                agent: Some(crate::project::AgentConfig {
                    enabled: true,
                    events: Vec::new(),
                    commands: Vec::new(),
                    cancel_events: Vec::new(),
                }),
                ..Default::default()
            }),
            ..lowering()
        };
        let result = compile_source(
            r#"
@EventHandler
void onChat(ChatEvent event) {
    event.player().sendMessage(event.message());
}
"#,
            &options,
        )
        .expect("typed agent event should compile");
        let files = &result.artifacts.files;
        let wrapper = files
            .get("data/mcfc/function/agent/event/chat.mcfunction")
            .expect("agent event wrapper");
        assert!(wrapper.contains("storage mcfc:agent current"));
        assert!(wrapper.contains("__mcfc_agent_event_chat"));
        let descriptor = files.get("mcfd.pack.toml").expect("agent descriptor");
        assert!(descriptor.contains("events = [\"chat\"]"));
    }

    #[test]
    fn compiles_explicit_agent_event_cancellation_into_a_decider_route() {
        let options = CompileOptions {
            helper: Some(crate::project::HelperConfig {
                agent: Some(crate::project::AgentConfig {
                    enabled: true,
                    events: Vec::new(),
                    commands: Vec::new(),
                    cancel_events: Vec::new(),
                }),
                ..crate::project::HelperConfig::default()
            }),
            ..lowering()
        };
        let result = compile_source(
            "@EventHandler\nvoid onChat(ChatEvent event) {\n    event.cancel();\n}\n",
            &options,
        )
        .expect("cancellable packet event should compile");
        let descriptor = result
            .artifacts
            .files
            .get("mcfd.pack.toml")
            .expect("descriptor");
        assert!(descriptor.contains("deciders = [\"chat\"]"));
        assert!(
            result
                .artifacts
                .files
                .values()
                .any(|contents| contents.contains("decision.cancel set value 1b"))
        );
    }

    #[test]
    fn rejects_cancel_call_for_observation_only_agent_event() {
        let options = CompileOptions {
            helper: Some(crate::project::HelperConfig {
                agent: Some(crate::project::AgentConfig {
                    enabled: true,
                    events: Vec::new(),
                    commands: Vec::new(),
                    cancel_events: Vec::new(),
                }),
                ..crate::project::HelperConfig::default()
            }),
            ..lowering()
        };
        let error = compile_source(
            "@EventHandler\nvoid onPlayerConnect(PlayerConnectEvent event) {\n    event.cancel();\n}\n",
            &options,
        )
        .expect_err("lifecycle event cancellation must fail");
        assert!(error.to_string().contains("observation-only"));
    }

    #[test]
    fn agent_event_requires_agent_manifest_capability() {
        let error = compile_source(
            "@EventHandler\nvoid onChat(ChatEvent event) {\n    debug(event.message());\n}\n",
            &lowering(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("helper.agent"));
    }

    #[test]
    fn compiles_expanded_agent_event_with_generic_payload() {
        let options = CompileOptions {
            helper: Some(crate::project::HelperConfig {
                agent: Some(crate::project::AgentConfig {
                    enabled: true,
                    events: Vec::new(),
                    commands: Vec::new(),
                    cancel_events: Vec::new(),
                }),
                ..Default::default()
            }),
            ..lowering()
        };
        let result = compile_source(
            r#"
@EventHandler
void onPlayerInteractBlock(PlayerInteractBlockEvent event) {
    event.player().sendMessage(event.face());
}
"#,
            &options,
        )
        .expect("expanded agent event should compile");
        let descriptor = result
            .artifacts
            .files
            .get("mcfd.pack.toml")
            .expect("agent descriptor");
        assert!(descriptor.contains("events = [\"player_interact_block\"]"));
        assert!(
            result
                .artifacts
                .files
                .contains_key("data/mcfc/function/agent/event/player_interact_block.mcfunction")
        );
    }

    #[test]
    fn rejects_cancelling_observation_only_agent_events() {
        let options = CompileOptions {
            helper: Some(crate::project::HelperConfig {
                agent: Some(crate::project::AgentConfig {
                    enabled: true,
                    events: Vec::new(),
                    commands: Vec::new(),
                    cancel_events: vec!["player_damage".to_string()],
                }),
                ..Default::default()
            }),
            ..lowering()
        };
        let error = compile_source("void main() {\n    return;\n}\n", &options)
            .unwrap_err()
            .to_string();
        assert!(error.contains("observation-only"));
    }

    #[test]
    fn compiles_world_reads_random_distributions_and_dict_keys() {
        let source = r#"
void main() {
    var spot = Block.of("~ ~ ~");
    var light = spot.getLightLevel();
    var biome = spot.getBiome();
    var plains = spot.inBiome("plains");
    var sky = spot.getEnvironment("gameplay/sky_light_level");
    var rule = gamerule("max_entity_cramming");
    var pick = randomWeighted(List.of(3, 1));
    var hits = randomBinomial(10, 0.5);
    var player = Selector.of("@p").getFirst();
    var dx = player.getLookX();
    var d = Map.of("wood", 2);
    var ks = d.keySet();
    var n = d.size();
    return;
}
"#;
        let result = compile_source(source, &lowering()).expect("source should compile");
        let files = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(files.contains("function mcfc:generated/light_0_15"));
        assert!(files.contains("predicate:{light:{light:{min:8}}}"));
        assert!(files.contains("execute if biome ~ ~ ~ minecraft:plains run return run"));
        assert!(files.contains("execute if biome $(pos) $(biome)"));
        assert!(files.contains("attribute:\"minecraft:gameplay/sky_light_level\""));
        assert!(files.contains("run gamerule max_entity_cramming"));
        assert!(files.contains("distribution:[{data:0,weight:3},{data:1,weight:1}]"));
        assert!(files.contains("{type:\"binomial\",n:10,p:"));
        assert!(files.contains("Rotation[0]"));
        assert!(files.contains("function mcfc:generated/dict_keys"));
    }

    #[test]
    fn rejects_unknown_world_ids_and_bad_weights() {
        let error = compile_source(
            r#"
void main() {
    var spot = Block.of("~ ~ ~");
    var a = spot.inBiome("moon");
    var b = spot.getEnvironment("visual/fog_color");
    var c = gamerule("no_such_rule");
    var d = randomWeighted(List.of(1, -2));
    var e = randomBinomial(3, 4);
    return;
}
"#,
            &lowering(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("unknown biome 'moon'"));
        assert!(error.contains("unknown numeric environment attribute 'visual/fog_color'"));
        assert!(error.contains("unknown game rule 'no_such_rule'"));
        assert!(error.contains("randomWeighted(...) needs a literal list of weights"));
        assert!(error.contains("randomBinomial(n, p) needs an 'int' and a 'float'"));
    }

    fn compiled_files(source: &str) -> String {
        compile_source(source, &lowering())
            .expect("source should compile")
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn builds_selectors_from_methods() {
        let files = compiled_files(
            r#"
void greet(Selector<Player> players) {
    for (var p : players) {
        p.sendActionBar("hi");
    }
}

void main() {
    String role = "boss";
    int most = 3;
    var bosses = Selector.entities()
        .type("minecraft:zombie").tag("elite").notTag(role)
        .distance(0.5, 16).score("hp", 1, 20).score("kills", "5..")
        .sort("nearest").limit(most);
    bosses.addTag("seen");
    int n = bosses.count();
    boolean none = Selector.allPlayers().gameMode("creative").isEmpty();
    greet(Selector.allPlayers().team("red"));
    greet(Selector.entities().tag("x").players());
    var pig = Selector.of("@e[type=pig,limit=1]").getFirst();
    boolean elite = pig.matches(Selector.entities().tag("elite"));
    var named = Selector.of("@e[tag=$(role)]");
    named.addTag("marked");
    var nearest = Selector.nearestPlayer().getFirst();
    return;
}
"#,
        );
        assert!(files.contains(
            "@e[type=minecraft:zombie,tag=elite,tag=!$(p1),distance=0.5..16,scores={hp=1..20,kills=5..},sort=nearest,limit=$(p2)]"
        ));
        assert!(files.contains("execute store result score"));
        assert!(files.contains("if entity $(selector)"));
        assert!(files.contains("@a[gamemode=creative]"));
        assert!(files.contains("@a[team=red]"));
        assert!(files.contains("@e[tag=x,type=minecraft:player]"));
        assert!(files.contains("@s[tag=elite]"));
        assert!(files.contains("@e[tag=$(p1)]"));
        assert!(!files.contains("@p[limit=1]"));
    }

    #[test]
    fn rejects_bad_selectors() {
        let error = compile_source(
            r#"
void takesPlayers(Selector<Player> players) {}

void main() {
    var a = Selector.of("@e[colour=red]");
    var b = Selector.of("@a[type=minecraft:pig]");
    var c = Selector.of("@e[type=minecraft:chicke]");
    var d = Selector.entities().limit(1).limit(2);
    var e = Selector.self().sort("nearest");
    takesPlayers(Selector.entities());
    var f = Selector.entities();
    var g = f.tag("x");
    var h = Selector.of("@e[type=pig]").players();
    return;
}
"#,
            &lowering(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("unknown selector argument 'colour'"));
        assert!(error.contains("'@a' only selects players"));
        assert!(error.contains("unknown entity type"));
        assert!(error.contains("'limit' can only be given once"));
        assert!(error.contains("'@s' is one entity, so it can't take 'sort'"));
        assert!(error.contains("Selector<Player>"));
        assert!(error.contains("needs a selector the compiler can see"));
        assert!(error.contains("never matches players"));
    }
}
