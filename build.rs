mod build_mcdoc;

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use build_mcdoc::{
    NbtSchemaSnapshot, SchemaNode, TARGET_MINECRAFT_VERSION, TARGET_VANILLA_MCDOC_REF,
    build_nbt_schema_snapshot_from_tarball,
};
use serde_json::Value;

const REGISTRY_SOURCE_URL: &str =
    "https://raw.githubusercontent.com/misode/mcmeta/summary/registries/data.json";
const REGISTRY_SNAPSHOT_PATH: &str = "data/minecraft_registries_snapshot.json";
const VANILLA_MCDOC_TARBALL_URL: &str = "https://github.com/SpyglassMC/vanilla-mcdoc/archive/6ef5413a6b0dcd4cbf448aedeebead491221c5cb.tar.gz";
const VANILLA_MCDOC_TARBALL_PATH: &str = "data/vanilla_mcdoc_26.1.2.tar.gz";

#[derive(Debug)]
struct RegistrySnapshot {
    block: Vec<String>,
    item: Vec<String>,
    entity_type: Vec<String>,
    loot_table: Vec<String>,
    particle_type: Vec<String>,
    sound_event: Vec<String>,
    mob_effect: Vec<String>,
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=build_mcdoc.rs");
    println!("cargo:rerun-if-changed={}", REGISTRY_SNAPSHOT_PATH);
    println!("cargo:rerun-if-changed={}", VANILLA_MCDOC_TARBALL_PATH);

    let registry_snapshot = match fetch_remote_registry_snapshot() {
        Ok(snapshot) => snapshot,
        Err(error) => {
            println!(
                "cargo:warning=failed to fetch minecraft registry data from {} ({error}); using bundled snapshot",
                REGISTRY_SOURCE_URL
            );
            load_registry_snapshot_file().unwrap_or_else(|snapshot_error| {
                panic!(
                    "failed to load bundled minecraft registry snapshot '{}': {}",
                    REGISTRY_SNAPSHOT_PATH, snapshot_error
                )
            })
        }
    };

    let vanilla_mcdoc_tarball = match fetch_remote_vanilla_mcdoc_tarball() {
        Ok(tarball) => tarball,
        Err(error) => {
            println!(
                "cargo:warning=failed to fetch vanilla-mcdoc tarball from {} ({error}); using bundled snapshot",
                VANILLA_MCDOC_TARBALL_URL
            );
            load_local_vanilla_mcdoc_tarball().unwrap_or_else(|snapshot_error| {
                panic!(
                    "failed to load bundled vanilla-mcdoc snapshot '{}': {}",
                    VANILLA_MCDOC_TARBALL_PATH, snapshot_error
                )
            })
        }
    };

    let nbt_snapshot = build_nbt_schema_snapshot_from_tarball(
        &vanilla_mcdoc_tarball,
        &registry_snapshot.block,
        &registry_snapshot.item,
        &registry_snapshot.entity_type,
    )
    .unwrap_or_else(|error| panic!("failed to build NBT schema from vanilla-mcdoc: {error}"));

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR should be set"));
    fs::write(
        out_dir.join("minecraft_ids.rs"),
        render_registry_module(&registry_snapshot),
    )
    .expect("failed to write generated minecraft id module");
    fs::write(
        out_dir.join("minecraft_nbt_schema.rs"),
        render_nbt_schema_module(&nbt_snapshot),
    )
    .expect("failed to write generated minecraft nbt schema module");
}

fn fetch_remote_registry_snapshot() -> Result<RegistrySnapshot, String> {
    let response = ureq::get(REGISTRY_SOURCE_URL)
        .call()
        .map_err(|error| error.to_string())?;
    let body = response.into_string().map_err(|error| error.to_string())?;
    extract_snapshot_from_registry_data(&body)
}

fn fetch_remote_vanilla_mcdoc_tarball() -> Result<Vec<u8>, String> {
    let response = ureq::get(VANILLA_MCDOC_TARBALL_URL)
        .call()
        .map_err(|error| error.to_string())?;
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    use std::io::Read as _;
    reader
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    Ok(bytes)
}

fn load_registry_snapshot_file() -> Result<RegistrySnapshot, String> {
    let input = fs::read_to_string(REGISTRY_SNAPSHOT_PATH).map_err(|error| error.to_string())?;
    extract_snapshot_from_snapshot_file(&input)
}

fn load_local_vanilla_mcdoc_tarball() -> Result<Vec<u8>, String> {
    fs::read(VANILLA_MCDOC_TARBALL_PATH).map_err(|error| error.to_string())
}

fn extract_snapshot_from_registry_data(input: &str) -> Result<RegistrySnapshot, String> {
    let value: Value = serde_json::from_str(input).map_err(|error| error.to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "registry source payload must be a JSON object".to_string())?;

    let mut loot_table = read_string_array(object, "loot_table")?;
    if let Some(extra) = object.get("experiment/trade_rebalance/loot_table") {
        loot_table.extend(read_string_array_from_value(
            extra,
            "experiment/trade_rebalance/loot_table",
        )?);
    }

    let mut snapshot = RegistrySnapshot {
        block: read_string_array(object, "block")?,
        item: read_string_array(object, "item")?,
        entity_type: read_string_array(object, "entity_type")?,
        loot_table,
        particle_type: read_string_array(object, "particle_type")?,
        sound_event: read_string_array(object, "sound_event")?,
        mob_effect: read_string_array(object, "mob_effect")?,
    };
    normalize_snapshot(&mut snapshot);
    Ok(snapshot)
}

fn extract_snapshot_from_snapshot_file(input: &str) -> Result<RegistrySnapshot, String> {
    let value: Value = serde_json::from_str(input).map_err(|error| error.to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "snapshot payload must be a JSON object".to_string())?;
    let mut snapshot = RegistrySnapshot {
        block: read_string_array(object, "block")?,
        item: read_string_array(object, "item")?,
        entity_type: read_string_array(object, "entity_type")?,
        loot_table: read_string_array(object, "loot_table")?,
        particle_type: read_string_array(object, "particle_type")?,
        sound_event: read_string_array(object, "sound_event")?,
        mob_effect: read_string_array(object, "mob_effect")?,
    };
    normalize_snapshot(&mut snapshot);
    Ok(snapshot)
}

fn normalize_snapshot(snapshot: &mut RegistrySnapshot) {
    dedup_sorted(&mut snapshot.block);
    dedup_sorted(&mut snapshot.item);
    dedup_sorted(&mut snapshot.entity_type);
    dedup_sorted(&mut snapshot.loot_table);
    dedup_sorted(&mut snapshot.particle_type);
    dedup_sorted(&mut snapshot.sound_event);
    dedup_sorted(&mut snapshot.mob_effect);
}

fn dedup_sorted(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

fn read_string_array(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Vec<String>, String> {
    let value = object
        .get(key)
        .ok_or_else(|| format!("missing registry key '{key}'"))?;
    read_string_array_from_value(value, key)
}

fn read_string_array_from_value(value: &Value, key: &str) -> Result<Vec<String>, String> {
    let array = value
        .as_array()
        .ok_or_else(|| format!("registry '{key}' must be an array"))?;
    let mut values = Vec::with_capacity(array.len());
    for entry in array {
        let text = entry
            .as_str()
            .ok_or_else(|| format!("registry '{key}' contains a non-string entry"))?;
        values.push(text.to_string());
    }
    Ok(values)
}

fn render_registry_module(snapshot: &RegistrySnapshot) -> String {
    let mut output = String::new();
    writeln!(
        output,
        "// Generated by build.rs from {} or the bundled snapshot.",
        REGISTRY_SOURCE_URL
    )
    .unwrap();
    render_prefixed_slice(&mut output, "BLOCK_IDS", &snapshot.block);
    render_prefixed_slice(&mut output, "ITEM_IDS", &snapshot.item);
    render_prefixed_slice(&mut output, "ENTITY_IDS", &snapshot.entity_type);
    render_prefixed_slice(&mut output, "LOOT_TABLE_IDS", &snapshot.loot_table);
    render_prefixed_slice(&mut output, "PARTICLE_IDS", &snapshot.particle_type);
    render_prefixed_slice(&mut output, "SOUND_EVENT_IDS", &snapshot.sound_event);
    render_prefixed_slice(&mut output, "EFFECT_IDS", &snapshot.mob_effect);
    output
}

fn render_prefixed_slice(output: &mut String, name: &str, values: &[String]) {
    writeln!(output, "pub static {name}: &[&str] = &[").unwrap();
    for value in values {
        writeln!(output, "    \"minecraft:{}\",", value.escape_default()).unwrap();
    }
    writeln!(output, "];\n").unwrap();
}

fn render_nbt_schema_module(snapshot: &NbtSchemaSnapshot) -> String {
    let mut renderer = SchemaRenderer::default();
    let entity_entries = renderer.render_root_table("ENTITY_ROOTS", &snapshot.entity);
    let block_entries = renderer.render_root_table("BLOCK_ROOTS", &snapshot.block);
    let item_entries = renderer.render_root_table("ITEM_ROOTS", &snapshot.item);

    let mut output = String::new();
    writeln!(
        output,
        "// Generated by build.rs from SpyglassMC/vanilla-mcdoc ref {} for Minecraft {}.",
        TARGET_VANILLA_MCDOC_REF, TARGET_MINECRAFT_VERSION,
    )
    .unwrap();
    output.push_str(&renderer.definitions);
    output.push_str(&entity_entries);
    output.push_str(&block_entries);
    output.push_str(&item_entries);
    output.push_str(
        "pub fn generated_root(category: NbtSchemaCategory, id: Option<&str>) -> Option<&'static NbtSchemaNode> {\n",
    );
    output.push_str("    let entries = match category {\n");
    output.push_str("        NbtSchemaCategory::Entity => ENTITY_ROOTS,\n");
    output.push_str("        NbtSchemaCategory::Block => BLOCK_ROOTS,\n");
    output.push_str("        NbtSchemaCategory::Item => ITEM_ROOTS,\n");
    output.push_str("    };\n");
    output.push_str("    if let Some(id) = id {\n");
    output.push_str(
        "        if let Some((_, node)) = entries.iter().find(|(entry_id, _)| *entry_id == id) {\n",
    );
    output.push_str("            return Some(*node);\n");
    output.push_str("        }\n");
    output.push_str("    }\n");
    output.push_str("    entries.iter().find(|(entry_id, _)| *entry_id == \"__default__\").map(|(_, node)| *node)\n");
    output.push_str("}\n");
    output
}

#[derive(Default)]
struct SchemaRenderer {
    definitions: String,
    next_id: usize,
}

impl SchemaRenderer {
    fn render_root_table(
        &mut self,
        name: &str,
        roots: &std::collections::BTreeMap<String, SchemaNode>,
    ) -> String {
        let mut output = String::new();
        writeln!(output, "static {name}: &[(&str, &NbtSchemaNode)] = &[").unwrap();
        for (id, node) in roots {
            let node_name = self.render_node(node);
            writeln!(output, "    ({:?}, &{}),", id, node_name).unwrap();
        }
        writeln!(output, "];\n").unwrap();
        output
    }

    fn render_node(&mut self, node: &SchemaNode) -> String {
        let id = self.next_id;
        self.next_id += 1;
        let fields_name = format!("NBT_SCHEMA_FIELDS_{}", id);
        let node_name = format!("NBT_SCHEMA_NODE_{}", id);
        let mut rendered_fields = Vec::with_capacity(node.fields.len());
        for field in &node.fields {
            let child_expr = match &field.node {
                Some(child) => format!("Some(&{})", self.render_node(child)),
                None => "None".to_string(),
            };
            rendered_fields.push(format!(
                "    NbtSchemaField {{ name: {:?}, detail: {:?}, documentation: {:?}, node: {} }},",
                field.name, field.detail, field.documentation, child_expr
            ));
        }
        writeln!(
            self.definitions,
            "static {fields_name}: &[NbtSchemaField] = &["
        )
        .unwrap();
        for field in rendered_fields {
            writeln!(self.definitions, "{field}").unwrap();
        }
        writeln!(self.definitions, "];\n").unwrap();
        writeln!(
            self.definitions,
            "static {node_name}: NbtSchemaNode = NbtSchemaNode {{ detail: {:?}, documentation: {:?}, fields: {fields_name} }};\n",
            node.detail,
            node.documentation,
        )
        .unwrap();
        node_name
    }
}
