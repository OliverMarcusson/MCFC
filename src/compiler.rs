use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::{Function, Program, Type};
use crate::backend::{self, BackendOptions, BuildArtifacts, ExportedFunction};
use crate::diagnostics::Diagnostics;
use crate::diagnostics::{Diagnostic, Span};
use crate::ir::{self, IrProgram};
use crate::optimizer;
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
    pub helper: Option<HelperConfig>,
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
            helper: None,
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
    // Bukkit-inspired declarations are intentionally a small, vanilla-safe
    // surface over ordinary MCFC functions.  Desugaring before lexing keeps
    // the core language and all existing source files backwards compatible.
    let normalized_source = normalize_bukkit_declarations_source(source);
    if normalized_source.contains("fn __mcfc_agent_event_")
        && !options
            .helper
            .as_ref()
            .and_then(|helper| helper.agent.as_ref())
            .is_some_and(|agent| agent.enabled)
    {
        let mut diagnostics = Diagnostics::new();
        diagnostics.push(Diagnostic::new(
            "agent event declarations require '[helper.agent] enabled = true'",
            Span::new(1, 1),
        ));
        return Err(diagnostics);
    }
    validate_agent_manifest(options)?;
    let ast = parser::parse(&normalized_source)?;
    let ast = normalize_special_functions(ast)?;
    let host_modules = HostModules::from_helper(options.helper.as_ref());
    let typed_program = types::type_check(&ast, &host_modules)?;
    let ir_program = ir::lower(&typed_program);
    validate_decision_handlers(&ir_program)?;
    let ir_program = if options.optimize {
        optimizer::optimize(ir_program)
    } else {
        ir_program
    };
    let artifacts = backend::generate(
        &ir_program,
        &BackendOptions {
            namespace: options.namespace.clone(),
            load_tag_values: options.load_tag_values.clone(),
            tick_tag_values: options.tick_tag_values.clone(),
            exports: options.exports.clone(),
            helper: options.helper.clone(),
        },
    );
    Ok(CompileResult {
        typed_program,
        ir_program,
        artifacts,
    })
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
                format!("unknown 26.2 agent event '{}'", event),
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

/// Expand the first vanilla Bukkit-style declarations into ordinary functions.
///
/// The generated names are consumed by the backend to install their runtime
/// drivers.  Keeping this as a source-normalisation pass lets old parser and
/// library users continue to operate on the stable MCFC AST.
/// Lower the source-only Bukkit/Paper-inspired declarations into the compact
/// parser core. Kept crate-visible so the language server validates precisely
/// the same surface accepted by the compiler.
pub(crate) fn normalize_bukkit_declarations_source(source: &str) -> String {
    source
        .lines()
        .map(|line| normalize_bukkit_declaration_line(line))
        .collect::<Vec<_>>()
        .join("\n")
        + if source.ends_with('\n') { "\n" } else { "" }
}

fn normalize_bukkit_declaration_line(line: &str) -> String {
    // These declarations are top-level by design.  Nested text is left alone,
    // including comments and strings in function bodies.
    if line.starts_with(char::is_whitespace) {
        return line.replace(".data.", ".state.");
    }

    let trimmed = line.trim();
    if let Some((kind, parameter)) = parse_agent_event_declaration(trimmed) {
        return format!("fn __mcfc_agent_event_{}({}) -> void:", kind, parameter);
    }
    if let Some(kind) = trimmed
        .strip_prefix("event ")
        .and_then(|value| value.strip_suffix(':'))
        .filter(|value| is_mcfc_identifier(value))
    {
        return format!("fn __mcfc_event_{}() -> void:", kind);
    }
    if let Some(signature) = trimmed
        .strip_prefix("command ")
        .and_then(|value| value.strip_suffix(':'))
    {
        if let Some((name, rest)) = signature.split_once('(') {
            if is_mcfc_identifier(name.trim()) {
                return format!("fn __mcfc_command_{}({}", name.trim(), rest);
            }
        }
        if is_mcfc_identifier(signature.trim()) {
            return format!("fn __mcfc_command_{}() -> void:", signature.trim());
        }
    }
    if let Some(rest) = trimmed.strip_prefix("task ") {
        if let Some((name, schedule)) = rest.split_once(' ') {
            if let Some(ticks) = schedule
                .strip_prefix("every_ticks(")
                .and_then(|value| value.strip_suffix("):"))
                .filter(|value| {
                    value
                        .parse::<u32>()
                        .ok()
                        .filter(|value| *value > 0)
                        .is_some()
                })
            {
                return format!("fn __mcfc_task_{}_every_ticks_{}() -> void:", name, ticks);
            }
            if let Some(ticks) = schedule
                .strip_prefix("after_ticks(")
                .and_then(|value| value.strip_suffix("):"))
                .filter(|value| {
                    value
                        .parse::<u32>()
                        .ok()
                        .filter(|value| *value > 0)
                        .is_some()
                })
            {
                return format!("fn __mcfc_task_{}_after_ticks_{}() -> void:", name, ticks);
            }
        }
    }
    if let Some(rest) = trimmed.strip_prefix("data player.") {
        if let Some((name, declaration)) = rest.split_once(':') {
            if is_mcfc_identifier(name) {
                if let Some((ty, default)) = declaration.split_once('=') {
                    let ty = ty.trim();
                    let default = default.trim();
                    // Vanilla scoreboards initialise to zero/false.  Rejecting
                    // non-zero defaults here would turn a useful parse error
                    // into a silent semantic surprise, so leave them for the
                    // normal parser until default initialisers are added.
                    if matches!((ty, default), ("int", "0") | ("bool", "false")) {
                        return format!("player_state {}: {} = \"{}\"", name, ty, name);
                    }
                }
            }
        }
    }
    line.replace(".data.", ".state.")
}

/// Parse the typed form of an agent event declaration. Vanilla lifecycle events
/// intentionally retain the shorter `event player_join:` form; JVM-backed
/// events carry their explicit, compiler-provided payload type.
fn parse_agent_event_declaration(line: &str) -> Option<(&str, &str)> {
    let rest = line.strip_prefix("event ")?;
    let (kind, parameter) = rest.split_once('(')?;
    let kind = kind.trim();
    let parameter = parameter.strip_suffix("):")?.trim();
    let (name, ty) = parameter.split_once(':')?;
    let name = name.trim();
    let ty = ty.trim();
    let expected = crate::language_catalog::agent_event_payload_type(kind)?;
    (is_mcfc_identifier(kind) && is_mcfc_identifier(name) && ty == expected)
        .then_some((kind, parameter))
}

fn is_mcfc_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some(ch) if ch == '_' || ch.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn normalize_special_functions(mut program: Program) -> Result<Program, Diagnostics> {
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
            params: Vec::new(),
            return_type: Type::Void,
            body: Vec::new(),
            span: program.functions[first_index].span.clone(),
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
    let source = fs::read_to_string(input)
        .map_err(|error| format!("failed to read '{}': {}", input.display(), error))?;
    let compiled = compile_source(&source, options)
        .map_err(|diagnostics| render_diagnostics(&diagnostics, &source))?;
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
    let sources = collect_source_files(&source_root)?;
    if sources.is_empty() {
        return Err(format!(
            "no '.mcf' files found under '{}'",
            source_root.display()
        ));
    }

    let merged_source = merge_project_sources(&sources)?;
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
            function: item.function.clone(),
        })
        .collect();
    if manifest.helper.is_some() {
        effective.helper = manifest.helper.clone();
    }

    let mut compiled = compile_source(&merged_source, &effective)
        .map_err(|diagnostics| render_diagnostics(&diagnostics, &merged_source))?;
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

fn render_diagnostics(diagnostics: &Diagnostics, source: &str) -> String {
    diagnostics
        .0
        .iter()
        .map(|diagnostic| diagnostic.render(source))
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub fn canonicalize_output_path(out_dir: &Path) -> PathBuf {
    out_dir.to_path_buf()
}

fn merge_project_sources(files: &[PathBuf]) -> Result<String, String> {
    let mut merged = String::new();
    for file in files {
        let source = fs::read_to_string(file)
            .map_err(|error| format!("failed to read '{}': {}", file.display(), error))?;
        merged.push_str(&format!("# source: {}\n", file.display()));
        merged.push_str(&source);
        if !source.ends_with('\n') {
            merged.push('\n');
        }
        merged.push('\n');
    }
    Ok(merged)
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

    #[test]
    fn compiles_gameplay_entity_and_inventory_builtins() {
        let source = r#"
fn main() -> void:
    let pig = summon("minecraft:pig")
    pig.add_tag("elite")
    let tagged = pig.has_tag("elite")
    pig.remove_tag("elite")
    pig.team = "red"
    pig.mainhand.item = "minecraft:carrot_on_a_stick"
    pig.offhand.item = "minecraft:shield"
    pig.head.name = "Captain"
    pig.chest.count = 1
    pig.effect("speed", 10, 1)
    pig.teleport(block("~ ~1 ~"))
    pig.damage(2)
    pig.heal(1)
    pig.give("minecraft:apple", 2)
    pig.clear("minecraft:apple", 1)
    pig.loot_give("minecraft:chests/simple_dungeon")
    return
"#;

        let result =
            compile_source(source, &CompileOptions::default()).expect("source should compile");
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
fn main() -> void:
    let pig = single(selector("@e[type=pig,limit=1]"))
    let pos = block("~ ~ ~")
    pig.tellraw("hello @s")
    pig.title("Danger")
    pig.actionbar("Run")
    let bb = bossbar("mcfc:test", "Boss @s")
    bb.value = 10
    bb.max = 20
    bb.visible = true
    bb.players = pig
    bb.name = "Still here"
    pig.playsound("minecraft:entity.experience_orb.pickup", "master")
    pig.stopsound("master", "minecraft:entity.experience_orb.pickup")
    pos.particle("minecraft:flame")
    pos.particle("minecraft:smoke", 4, pig)
    pos.loot_insert("minecraft:chests/simple_dungeon")
    pos.loot_spawn("minecraft:chests/simple_dungeon")
    pos.setblock("minecraft:stone")
    pos.fill(block("~1 ~1 ~1"), "minecraft:glass")
    return
"#;

        let result =
            compile_source(source, &CompileOptions::default()).expect("source should compile");
        let files = result
            .artifacts
            .files
            .values()
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
        assert!(files.contains("tellraw $(selector) [\"hello \",{"));
        assert!(files.contains("title $(selector) title \"Danger\""));
        assert!(files.contains("title $(selector) actionbar \"Run\""));
        assert!(files.contains("bossbar add $(id) [\"Boss \",{"));
        assert!(files.contains("bossbar set $(id) value $(value)"));
        assert!(files.contains("bossbar set $(id) max $(value)"));
        assert!(files.contains("bossbar set $(id) visible $(visible)"));
        assert!(files.contains("bossbar set $(id) players $(selector)"));
        assert!(files.contains("bossbar set $(id) name \"Still here\""));
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
fn main() -> void:
    let pig = entity("minecraft:pig")
    pig.name = "Builder Pig"
    pig.no_ai = true
    let spawned = summon(pig)
    let chest = block_type("minecraft:chest")
    chest.states.facing = "north"
    chest.name = "Loot"
    let pos = block("~ ~ ~")
    pos.setblock(chest)
    pos.fill(block("~1 ~1 ~1"), chest)
    return
"#;

        let result =
            compile_source(source, &CompileOptions::default()).expect("source should compile");
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
fn roll() -> int:
    return random()
fn main() -> void:
    let any = random()
    let bounded = random(6)
    let between = random(1, 20)
    bounded = random(between)
    let combined = random() + roll()
    mcf "say $(random(1, 3))"
    return
"#;

        let result =
            compile_source(source, &CompileOptions::default()).expect("source should compile");
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
fn main() -> void:
    let demo_title = "MCFC Demo $(random(100))"
    let player = single(selector("@p"))
    player.tellraw(demo_title)
    return
"#;

        let result =
            compile_source(source, &CompileOptions::default()).expect("source should compile");
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
fn main() -> void:
    let q = \"hi\"
    let line = \"\u{201c}$(q)\u{201d} \u{2014} done\"
    let player = single(selector(\"@p\"))
    player.tellraw(line)
    return
";
        let result =
            compile_source(source, &CompileOptions::default()).expect("source should compile");
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
fn main() -> void:
    let who = \"world\"
    let line = text(\"hi $(who)!\")
    let player = single(selector(\"@p\"))
    player.tellraw(line)
    return
";
        let result =
            compile_source(source, &CompileOptions::default()).expect("source should compile");
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
fn main() -> void:
    let player = single(selector("@p"))
    let flag = true

    sleep(1)
    mc "say after straight sleep"

    if flag:
        sleep(1)
        mc "say after if sleep"
    mc "say after if"

    at(player):
        sleep(1)
        mc "say after context sleep"
    let i = 0
    while i < 2:
        sleep(1)
        i = i + 1
    for n in 0..2:
        sleep(1)
        mc "say after for sleep"
    mc "say done"
    return
"#;

        let result =
            compile_source(source, &CompileOptions::default()).expect("source should compile");
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
fn main() -> void:
    let player = single(selector("@p"))
    let bb = bossbar("mcfc:demo", "MCFC Bossbar")
    let count = 5
    bb.value = count
    bb.max = 10
    bb.visible = true
    bb.players = player
    player.position.particle("minecraft:happy_villager", 20, player)
    async:
        sleep(5)
        bb.remove()
        player.position.setblock("minecraft:gold_block")
    count = 7
    player.tellraw("caller continues")
    return
"#;

        let result =
            compile_source(source, &CompileOptions::default()).expect("source should compile");
        let files = result.artifacts.files;
        let joined = files.values().cloned().collect::<Vec<_>>().join("\n");
        let entry = files
            .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
            .unwrap();

        assert!(joined.contains("bossbar add $(id) \"MCFC Bossbar\""));
        assert!(joined.contains("bossbar set $(id) value $(value)"));
        assert!(joined.contains("bossbar set $(id) max $(value)"));
        assert!(joined.contains("bossbar set $(id) visible $(visible)"));
        assert!(joined.contains("bossbar set $(id) players $(selector)"));
        assert!(joined.contains("bossbar remove $(id)"));
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
fn main() -> void:
    async:
        return
"#,
            &CompileOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(async_error.contains("return may not appear inside an async block"));

        let legacy_error = compile_source(
            r#"
fn main() -> void:
    let player = single(selector("@p"))
    tellraw(player, "old")
"#,
            &CompileOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(legacy_error.contains("target.tellraw(message)"));

        let book_error = compile_source(
            r#"
@book
fn main() -> void:
    return
"#,
            &CompileOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(book_error.contains("unknown annotation '@book'"));
    }

    #[test]
    fn rejects_invalid_random_and_sleep_usage() {
        let source = r#"
fn main() -> void:
    let bad_sleep = sleep(1)
    random(sleep(1))
    mcf "say $(sleep(1))"
    sleep(0)
    sleep("bad")
    let bad_random = random("bad")
    let too_many = random(1, 2, 3)
    return
"#;

        let error = compile_source(source, &CompileOptions::default()).unwrap_err();
        let rendered = error.to_string();
        assert!(rendered.contains("sleep(...) may only appear as a standalone statement"));
        assert!(rendered.contains("sleep(...) seconds must be at least 1"));
        assert!(rendered.contains("sleep(...) seconds must have type 'int'"));
        assert!(rendered.contains("argument 1 for 'random' must be 'int'"));
        assert!(rendered.contains("wrong arity for 'random': expected 0, 1, or 2, found 3"));

        let string_error = compile_source(
            r#"
fn main() -> void:
    let bad = "value $(sleep(1))"
    return
"#,
            &CompileOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(string_error.contains("sleep(...) may only appear as a standalone statement"));
    }

    #[test]
    fn compiles_debug_builtins() {
        let source = r#"
fn main() -> void:
    let pig = single(selector("@e[type=pig,limit=1]"))
    let pos = block("~ ~1 ~")
    debug("checkpoint")
    pos.debug_marker("marker")
    pos.debug_marker("block marker", "minecraft:gold_block")
    pig.debug_entity("nearest pig")
    return
"#;

        let result =
            compile_source(source, &CompileOptions::default()).expect("source should compile");
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
fn main() -> void:
    let player = single(selector("@p"))
    player.heal(1)
"#,
            &CompileOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(player_error.contains("known non-player"));

        let ambiguous_error = compile_source(
            r#"
fn main() -> void:
    let target = single(selector("@e"))
    target.heal(1)
"#,
            &CompileOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(ambiguous_error.contains("ambiguous 'entity_ref'"));
    }

    #[test]
    fn compiles_vanilla_bukkit_declarations_and_data_aliases() {
        let result = compile_source(
            r#"
data player.coins: int = 0

event player_join:
    let player = single(selector("@s"))
    player.data.coins = player.data.coins + 1

event player_death:
    debug("dead")

command status:
    debug("status")

task pulse every_ticks(20):
    debug("pulse")

task later after_ticks(5):
    debug("later")
"#,
            &CompileOptions::default(),
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
        assert!(runtime.contains("mcfcc_status"));
        assert!(runtime.contains("#mcfct_pulse"));
        assert!(files.contains_key("data/mcfc/function/generated/bukkit/load.mcfunction"));
        assert!(files.contains_key("data/mcfc/function/generated/bukkit/player_join.mcfunction"));
        let generated = files.values().cloned().collect::<Vec<_>>().join("\n");
        assert!(generated.contains("@s"));
        assert!(!generated.contains("@s[limit=1]"));
    }

    #[test]
    fn bukkit_command_objectives_are_unique_after_truncation() {
        let result = compile_source(
            r#"
command abcdefghij_one:
    debug("one")

command abcdefghij_two:
    debug("two")
"#,
            &CompileOptions::default(),
        )
        .expect("commands with shared objective prefixes should compile");
        let files = &result.artifacts.files;
        let setup = files
            .get("data/mcfc/function/generated/setup.mcfunction")
            .expect("setup function");
        assert!(setup.contains("scoreboard objectives add mcfcc_abcdefghij trigger"));
        assert!(setup.contains("scoreboard objectives add mcfcc_abcdefgh_1 trigger"));
        let runtime = files
            .get("data/mcfc/function/generated/bukkit/tick.mcfunction")
            .expect("Bukkit runtime");
        assert!(runtime.contains("execute as @a[scores={mcfcc_abcdefghij=1..}] run function mcfc:generated/bukkit/command/abcdefghij_one"));
        assert!(runtime.contains("execute as @a[scores={mcfcc_abcdefgh_1=1..}] run function mcfc:generated/bukkit/command/abcdefghij_two"));
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
            ..CompileOptions::default()
        };
        let result = compile_source(
            r#"
event chat(event: chat_event):
    event.player.send_message(event.message)
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
            ..CompileOptions::default()
        };
        let result = compile_source(
            "event chat(event: chat_event):\n    event.cancel()\n",
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
            ..CompileOptions::default()
        };
        let error = compile_source(
            "event player_connect(event: agent_event):\n    event.cancel()\n",
            &options,
        )
        .expect_err("lifecycle event cancellation must fail");
        assert!(error.to_string().contains("observation-only"));
    }

    #[test]
    fn agent_event_requires_agent_manifest_capability() {
        let error = compile_source(
            "event chat(event: chat_event):\n    debug(event.message)\n",
            &CompileOptions::default(),
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
            ..CompileOptions::default()
        };
        let result = compile_source(
            r#"
event player_interact_block(event: player_interact_block_event):
    event.player.send_message(event.face)
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
            ..CompileOptions::default()
        };
        let error = compile_source("fn main() -> void:\n    return\n", &options)
            .unwrap_err()
            .to_string();
        assert!(error.contains("observation-only"));
    }

    #[test]
    fn bukkit_data_rejects_non_zero_defaults_until_managed_storage_exists() {
        let error = compile_source("data player.coins: int = 10\n", &CompileOptions::default())
            .unwrap_err()
            .to_string();
        assert!(error.contains("expected player_state"));
    }
}
