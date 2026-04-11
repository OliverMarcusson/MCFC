use std::fs;
use std::path::{Path, PathBuf};

use crate::backend::{self, BackendOptions, BuildArtifacts, ExportedFunction};
use crate::diagnostics::Diagnostics;
use crate::ir::{self, IrProgram};
use crate::parser;
use crate::project::{collect_asset_files, collect_source_files, load_manifest};
use crate::types::{self, TypedProgram};

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub namespace: String,
    pub emit_ast: bool,
    pub emit_ir: bool,
    pub clean: bool,
    pub load_tag_values: Option<Vec<String>>,
    pub tick_tag_values: Option<Vec<String>>,
    pub exports: Vec<ExportedFunction>,
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
    let typed_program = types::type_check(&ast)?;
    let ir_program = ir::lower(&typed_program);
    let artifacts = backend::generate(
        &ir_program,
        &BackendOptions {
            namespace: options.namespace.clone(),
            load_tag_values: options.load_tag_values.clone(),
            tick_tag_values: options.tick_tag_values.clone(),
            exports: options.exports.clone(),
        },
    );
    Ok(CompileResult {
        typed_program,
        ir_program,
        artifacts,
    })
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
fn main() -> void
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
    teleport(pig, block("~ ~1 ~"))
    damage(pig, 2)
    heal(pig, 1)
    give(pig, "minecraft:apple", 2)
    clear(pig, "minecraft:apple", 1)
    loot_give(pig, "minecraft:chests/simple_dungeon")
    return
end
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
fn main() -> void
    let pig = single(selector("@e[type=pig,limit=1]"))
    let pos = block("~ ~ ~")
    tellraw(pig, "hello @s")
    title(pig, "Danger")
    actionbar(pig, "Run")
    bossbar_add("mcfc:test", "Boss @s")
    bossbar_value("mcfc:test", 10)
    bossbar_max("mcfc:test", 20)
    bossbar_visible("mcfc:test", true)
    bossbar_players("mcfc:test", pig)
    bossbar_name("mcfc:test", "Still here")
    playsound("minecraft:entity.experience_orb.pickup", "master", pig)
    stopsound(pig, "master", "minecraft:entity.experience_orb.pickup")
    particle("minecraft:flame", pos)
    particle("minecraft:smoke", pos, 4, pig)
    loot_insert(pos, "minecraft:chests/simple_dungeon")
    loot_spawn(pos, "minecraft:chests/simple_dungeon")
    setblock(pos, "minecraft:stone")
    fill(pos, block("~1 ~1 ~1"), "minecraft:glass")
    return
end
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
    fn compiles_debug_builtins() {
        let source = r#"
fn main() -> void
    let pig = single(selector("@e[type=pig,limit=1]"))
    let pos = block("~ ~1 ~")
    debug("checkpoint")
    debug_marker(pos, "marker")
    debug_marker(pos, "block marker", "minecraft:gold_block")
    debug_entity(pig, "nearest pig")
    return
end
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
fn main() -> void
    let player = single(selector("@p"))
    heal(player, 1)
end
"#,
            &CompileOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(player_error.contains("known non-player"));

        let ambiguous_error = compile_source(
            r#"
fn main() -> void
    let target = single(selector("@e"))
    heal(target, 1)
end
"#,
            &CompileOptions::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(ambiguous_error.contains("ambiguous 'entity_ref'"));
    }
}
