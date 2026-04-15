use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use mcfc::compiler::{CompileOptions, compile_project, compile_source};

#[test]
fn compiles_straight_line_program() {
    let source = r#"
fn main() -> void:
    let a = 5
    let b = 7
    let text = "done"
    b = a + b
    mc "say done"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/mcfc/function/main.mcfunction")
    );
    let load_tag = result
        .artifacts
        .files
        .get("data/minecraft/tags/function/load.json")
        .unwrap();
    assert!(load_tag.contains("\"mcfc:main\""));
    let main = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("scoreboard players set $d0_main_a mcfc 5"));
    assert!(main.contains("say done"));
}

#[test]
fn compiles_program_with_comments() {
    let source = r#"
# top-level comment
fn main() -> void: # signature comment:
    let a = 1 # inline comment
    # inside block
    mc "say done"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let main = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("say done"));
}

#[test]
fn compiles_single_quoted_strings() {
    let source = r#"
fn main() -> void:
    let a = 'done'
    mc 'say "done"'
    mcf 'say $(a)'
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let main = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    let macro_file = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__macro_1.mcfunction")
        .unwrap();

    assert!(main.contains("say \"done\""));
    assert!(macro_file.contains("$say $(p1)"));
}

#[test]
fn compiles_macro_command_with_storage_call() {
    let source = r#"
fn main() -> void:
    let amount = 5
    let label = "hello"
    mcf "xp add @a $(amount) levels"
    mcf "say $(label)"
    mc "say $(amount)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let main = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("with storage mcfc:runtime frames.d0.main.__macro1"));
    assert!(main.contains("with storage mcfc:runtime frames.d0.main.__macro2"));
    assert!(main.contains("say $(amount)"));

    let macro_file = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__macro_1.mcfunction")
        .unwrap();
    assert!(macro_file.contains("$xp add @a $(p1) levels"));
}

#[test]
fn compiles_entity_queries_and_iteration() {
    let source = r#"
fn main() -> void:
    let pigs = selector("@e[type=pig,limit=3]")
    for pig in pigs:
        pig.CustomName = "Hello"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let main = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("selector set value \"@e[type=pig,limit=3]\""));
    assert!(result.artifacts.files.values().any(|file| {
        file.contains("execute as $(selector) run function mcfc:generated/main__d0__for_each_1")
    }));
    assert!(result
        .artifacts
        .files
        .values()
        .any(|file| file.contains("data modify entity $(selector) CustomName set from storage")));
}

#[test]
fn compiles_single_exists_and_context_composition() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@a[tag=hunter]"))
    if exists(player):
        let nearest = single(at(player, selector("@e[type=pig,sort=nearest]")))
        if exists(nearest):
            nearest.CustomName = "Target"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let main = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("@a[tag=hunter,limit=1]"));
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("execute at "))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("@e[type=pig,sort=nearest,limit=1]"))
    );
}

#[test]
fn single_plain_player_name_stays_a_player_target() {
    let source = r#"
fn main() -> void:
    let player = single(selector("FaithlessMC"))
    player.tellraw("hi")
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("selector set value \"FaithlessMC\""));
    assert!(!joined.contains("FaithlessMC[limit=1]"));
    assert!(joined.contains("tellraw $(selector)"));
    assert!(joined.contains("hi"));
}

#[test]
fn object_display_methods_expand_message_at_s_to_the_target_selector() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@a"))
    player.tellraw("*@s* Expression test: $(32)")
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("{\"selector\":\"$(selector)\"}"));
    assert!(!joined.contains("{\"selector\":\"@s\"}"));
    assert!(joined.contains("Expression test: $(p1)"));
}

#[test]
fn compiles_as_value_context_composition() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    if exists(player):
        let self_ref = single(as(player, selector("@s")))
        self_ref.tags.welcomed = true
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("execute as "))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("tag $(selector) add welcomed"))
    );
}

#[test]
fn compiles_as_and_at_context_blocks() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    as(player):
        mcf 'tellraw @s "welcome @s"'
        mc 'title @s actionbar "title @s"'
        mc "say hello @s"
    at(player):
        mc "say here"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    assert!(result.artifacts.files.values().any(|file| {
        file.contains("execute as $(selector) run function mcfc:generated/main__d0__context_as_")
    }));
    assert!(result.artifacts.files.values().any(|file| {
        file.contains("execute at $(selector) run function mcfc:generated/main__d0__context_at_")
    }));
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("tellraw @s [\"welcome \",{\"selector\":\"@s\"}]"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| { file.contains("title @s actionbar [\"title \",{\"selector\":\"@s\"}]") })
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("tellraw @a [\"hello \",{\"selector\":\"@s\"}]"))
    );
    assert!(
        !result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("tellraw @s \"welcome @s\""))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("$$(prefix)execute as $(selector) run function"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("say here"))
    );
}

#[test]
fn compiles_nested_context_blocks() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    at(player):
        as(selector("@e[type=pig,limit=1]")):
            mc "say @s"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    assert!(result.artifacts.files.values().any(|file| {
        file.contains("execute at $(selector) run function mcfc:generated/main__d0__context_at_")
    }));
    assert!(result.artifacts.files.values().any(|file| {
        file.contains("execute as $(selector) run function mcfc:generated/main__d0__context_as_")
    }));
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("tellraw @a [{\"selector\":\"@s\"}]"))
    );
}

#[test]
fn compiles_text_def_display_components() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@a"))
    let msg = text("Hello")
    msg.color = "gold"
    msg.bold = true
    msg.hover_event.action = "show_text"
    msg.hover_event.value = text("Hover!")
    msg.extra = [text(" world")]
    player.tellraw(msg)
    let bb = bossbar("mcfc:test", msg)
    bb.name = msg
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");

    assert!(joined.contains(".text set from storage"));
    assert!(joined.contains(".color set from storage"));
    assert!(joined.contains(".bold set from storage"));
    assert!(joined.contains(".hover_event.action set from storage"));
    assert!(joined.contains(".hover_event.value set from storage"));
    assert!(joined.contains(".extra set from storage"));
    assert!(joined.contains("tellraw $(selector) $(message)"));
    assert!(joined.contains("bossbar add $(id) $(name)"));
    assert!(joined.contains("bossbar set $(id) name $(name)"));
}

#[test]
fn compiles_block_paths_and_nbt_casts() {
    let source = r#"
fn main() -> void:
    let chest = block("~ ~ ~")
    chest.CustomName = "Loot"
    let name = string(chest.CustomName)
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let main = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("pos set value \"~ ~ ~\""));
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("data modify block $(pos) CustomName set from storage"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("set from block $(pos) CustomName"))
    );
}

#[test]
fn compiles_explicit_runtime_entity_and_block_nbt_paths() {
    let source = r#"
fn main() -> void:
    let ent1 = single(selector("@e[type=pig,limit=1]"))
    let ent2 = single(selector("@e[type=cow,limit=1]"))
    let chest = block("~ ~ ~")
    ent1.nbt.Rotation = ent2.nbt.Rotation
    let rot = ent1.nbt.Rotation
    chest.nbt.CustomName = "Loot"
    let name = string(chest.nbt.CustomName)
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("data modify entity $(selector) Rotation set from storage"));
    assert!(joined.contains("set from entity $(selector) Rotation"));
    assert!(joined.contains("data modify block $(pos) CustomName set from storage"));
    assert!(joined.contains("set from block $(pos) CustomName"));
    assert!(!joined.contains("data modify entity $(selector) nbt.Rotation set from storage"));
    assert!(!joined.contains("set from entity $(selector) nbt.Rotation"));
    assert!(!joined.contains("data modify block $(pos) nbt.CustomName set from storage"));
    assert!(!joined.contains("set from block $(pos) nbt.CustomName"));
}

#[test]
fn quotes_string_index_nbt_segments_in_runtime_and_storage_paths() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    let chest = block("~ ~ ~")
    let page = string(player.nbt.SelectedItem.components["minecraft:writable_book_content"].pages[0].raw)
    let weird = string(player.inventory[0].nbt.foo["A [crazy name]!"].baz)
    chest.nbt.Items[1].components["minecraft:written_book_content"].author = page
    chest.nbt.foo["A [crazy name]!"].value = weird
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains(
        "set from entity $(selector) SelectedItem.components.\"minecraft:writable_book_content\".pages[0].raw"
    ));
    assert!(joined.contains(
        "data modify block $(pos) Items[1].components.\"minecraft:written_book_content\".author set from storage"
    ));
    assert!(joined.contains(".nbt.foo.\"A [crazy name]!\".baz"));
    assert!(
        joined.contains("data modify block $(pos) foo.\"A [crazy name]!\".value set from storage")
    );
    assert!(!joined.contains("SelectedItem.components[0].pages[0].raw"));
    assert!(!joined.contains("Items[1].components[0].author"));
}

#[test]
fn quotes_dynamic_string_index_nbt_segments_on_storage_backed_paths() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    let payload = player.inventory[0].nbt
    let key = "A [crazy name]!"
    let value = string(payload.foo[key].bar)
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("foo.\"$(n1)\".bar"));
    assert!(joined.contains("key set value \"A [crazy name]!\""));
    assert!(!joined.contains("foo.$(n1).bar"));
}

#[test]
fn compiles_entity_and_block_builder_paths() {
    let source = r#"
fn main() -> void:
    let pig = entity("minecraft:pig")
    pig.name = "Boss"
    pig.glowing = true
    let spawned = summon(pig)
    let chest = block_type("minecraft:chest")
    chest.states.facing = "north"
    chest.name = "Loot"
    let pos = block("~ ~ ~")
    pos.setblock(chest)
    pos.fill(block("~1 ~1 ~1"), chest)
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains(".nbt.CustomName set from storage"));
    assert!(joined.contains(".nbt.Glowing set from storage"));
    assert!(joined.contains("summon $(entity) ~ ~ ~ $(data)"));
    assert!(joined.contains("$(id)[facing=$(s1)]"));
    assert!(joined.contains("data merge block $(pos) $(data)"));
    assert!(joined.contains("fill $(from) $(to) $(block)"));
}

#[test]
fn compiles_player_safe_api_surfaces() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    if exists(player):
        let air = int(player.nbt.Air)
        player.state.quest_stage = 3
        let stage = int(player.state.quest_stage)
        player.tags.infected = true
        let infected = bool(player.tags.infected)
        player.team = "red"
        player.mainhand.name = "MCFC Blade"
        player.mainhand.item = "minecraft:carrot_on_a_stick"
        player.mainhand.count = 1
        player.effect("speed", 10, 1)
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let setup = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/setup.mcfunction")
        .unwrap();
    assert!(setup.contains("scoreboard objectives add mcfs_quest_stage dummy"));
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("data modify storage mcfc:runtime")
                && file.contains("set from entity $(selector) Air"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("scoreboard players operation $(selector) mcfs_quest_stage"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("tag $(selector) add infected"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("execute as $(selector) if entity @s[tag=infected]"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("team join $(team) $(selector)"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("item modify entity $(selector) weapon.mainhand"))
    );
    assert!(result.artifacts.files.values().any(|file| {
        file.contains("item replace entity $(selector) weapon.mainhand with $(item_id)")
    }));
    assert!(result.artifacts.files.values().any(|file| {
        file.contains("effect give $(selector) $(effect) $(duration) $(amplifier) true")
    }));
    assert!(result.artifacts.files.values().any(|file| {
        file.contains(
            "item modify entity $(selector) weapon.mainhand {\"function\":\"minecraft:set_count\",\"count\":$(count)}",
        )
    }));
    assert!(!result.artifacts.files.values().any(|file| {
        file.contains(
            "\"function\":\"minecraft:set_count\",\"count\":{\"type\":\"minecraft:storage\"",
        )
    }));
}

#[test]
fn compiles_equipment_slot_reads_via_item_slot_surface() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    let hand = player.mainhand
    let present = hand.exists
    let id = hand.id
    let count = hand.count
    let custom = string(hand.nbt.CustomModelData)
    mcf "say $(present) $(id) $(count) $(custom)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("SelectedItem.id"));
    assert!(joined.contains("HandItems[0].id"));
    assert!(joined.contains(".exists set value 0"));
    assert!(!joined.contains("set from entity $(selector) mainhand.id"));
}

#[test]
fn compiles_generic_entity_state_reads_and_writes() {
    let source = r#"
fn main() -> void:
    let marker = single(selector("@e[type=minecraft:marker,limit=1]"))
    marker.state.decay = 0
    marker.state.decay = marker.state.decay + 1
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let setup = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/setup.mcfunction")
        .unwrap();
    assert!(setup.contains("scoreboard objectives add mcfe_decay dummy"));
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("scoreboard players operation $(selector) mcfe_decay"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("scoreboard players get $(selector) mcfe_decay"))
    );
}

#[test]
fn compiles_generic_entity_bool_state_conditions() {
    let source = r#"
fn main() -> void:
    let mob = single(selector("@e[type=minecraft:pig,limit=1]"))
    mob.state.alert = true
    if mob.state.alert:
        mob.tellraw("x")
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let setup = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/setup.mcfunction")
        .unwrap();
    assert!(setup.contains("scoreboard objectives add mcfe_alert dummy"));
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("scoreboard players operation $(selector) mcfe_alert"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("scoreboard players get $(selector) mcfe_alert"))
    );
    assert!(
        result
            .artifacts
            .files
            .values()
            .any(|file| file.contains("tellraw $(selector)"))
    );
}

#[test]
fn compiles_item_builders_and_player_inventory_slots() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    let sword = item("minecraft:diamond_sword")
    let idx = 7
    sword.count = 2
    sword.name = "Blade"
    sword.nbt.CustomModelData = 7

    let payload = sword.as_nbt()
    player.give(sword)
    player.hotbar[0] = item("minecraft:stick")
    player.hotbar[idx] = sword
    player.inventory[5] = sword
    player.inventory[5].count = 16
    player.inventory[idx].count = 4
    player.inventory[5].name = "Stored"

    let exists = player.inventory[3].exists
    let item_id = player.inventory[3].id
    let count = player.inventory[3].count
    let item_data = player.inventory[3].nbt

    player.hotbar[2].clear()
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("set value \"minecraft:diamond_sword\""));
    assert!(joined.contains(".count set value 1"));
    assert!(joined.contains(".nbt.display.Name set from storage"));
    assert!(joined.contains(".nbt.CustomModelData set from storage"));
    assert!(joined.contains(
        "give $(selector) $(item)[minecraft:custom_name='\"$(item_name)\"',minecraft:custom_data=$(data)] $(count)"
    ));
    assert!(joined.contains("give $(selector) $(item)[minecraft:custom_data=$(data)] $(count)"));
    assert!(joined.contains("Inventory[{Slot:$(slot)b}]"));
    assert!(joined.contains(".slot set value 14"));
    assert!(joined.contains(".slot set value 2"));
    assert!(joined.contains(".command_slot set value \"hotbar.0\""));
    assert!(joined.contains(".command_slot set value \"inventory.5\""));
    assert!(joined.contains(".logical_slot"));
    assert!(joined.contains("scoreboard players add"));
    assert!(joined.contains("set value \"hotbar.$(logical_slot)\""));
    assert!(joined.contains(
        "item replace entity $(selector) $(command_slot) with $(id)[minecraft:custom_name='\"$(item_name)\"',minecraft:custom_data=$(nbt)] $(count)"
    ));
    assert!(joined.contains(
        "item replace entity $(selector) $(command_slot) with $(id)[minecraft:custom_data=$(nbt)] $(count)"
    ));
    assert!(joined.contains(".exists set value 0"));
}

#[test]
fn compiles_runtime_item_slot_nbt_reads_and_writes() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    player.inventory[1].nbt = player.inventory[0].nbt
    player.inventory[1].nbt.CustomModelData = player.inventory[0].nbt.CustomModelData
    let payload = player.inventory[1].nbt
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains(".command_slot set value \"inventory.1\""));
    assert!(joined.contains(".command_slot set value \"inventory.0\""));
    assert!(joined.contains("Inventory[{Slot:$(slot)b}]"));
    assert!(joined.contains(".nbt set from storage"));
    assert!(joined.contains(".nbt.CustomModelData set from storage"));
    assert!(joined.contains(
        "item replace entity $(selector) $(command_slot) with $(id)[minecraft:custom_data=$(nbt)] $(count)"
    ));
}

#[test]
fn compiles_player_ref_inventory_assertions_and_params() {
    let source = r#"
fn equip(player: player_ref, idx: int, stack: item_def) -> void:
    player.hotbar[idx] = stack
    player.inventory[idx].count = 3
    return
fn main() -> void:
    let target = single(selector("@e[limit=1]"))
    let stack = item("minecraft:book")
    let player = player_ref(target)
    equip(target, 7, stack)
    player.hotbar[1] = stack
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("Inventory[{Slot:$(slot)b}]"));
    assert!(joined.contains("set value \"inventory.$(logical_slot)\""));
    assert!(joined.contains(".command_slot set value \"hotbar.1\""));
}

#[test]
fn compiles_position_owned_summons_and_spawned_items() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    let pig = block("1 64 1").summon(entity("minecraft:pig"))
    let inline = entity("minecraft:pig")
    inline.name = "Inline"
    let pig_with_data = block("~ ~ ~").summon("minecraft:pig", inline.as_nbt())
    let rel = at(player, block("~1 ~ ~"))
    let pig_relative = rel.summon("minecraft:pig")
    let pig_above = at(player, block("~ ~10 ~")).summon("minecraft:pig")
    let drop = block("~ ~ ~").spawn_item(item("minecraft:apple"))
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("pos set value \"1 64 1\""));
    assert!(joined.contains("pos set value \"~1 ~ ~\""));
    assert!(joined.contains("pos set value \"~ ~10 ~\""));
    assert!(joined.contains("prefix set value \"$(__anchor_prefix)execute at $(__anchor_selector) run $(__value_prefix)\""));
    assert!(!joined.contains(".prefix append value \"execute at \""));
    assert!(joined.contains("summon $(entity) $(pos) $(data)"));
    assert!(joined.contains("minecraft:item"));
    assert!(joined.contains(".Item set from storage"));
    assert!(joined.contains("execute at "));
}

#[test]
fn compiles_entity_builder_as_nbt_in_nbt_contexts() {
    let source = r#"
fn echo(value: nbt) -> nbt:
    return value
fn make_passenger() -> nbt:
    let chicken = entity("minecraft:chicken")
    chicken.name = "Marcusson"
    return chicken
fn main() -> void:
    let pig = entity("minecraft:pig")
    pig.name = "Ljungan"
    pig.glowing = true

    let chicken = entity("minecraft:chicken")
    chicken.name = "Marcusson"
    chicken.tags = ["cooler-tag"]

    pig.nbt.Passengers[0] = chicken
    pig.nbt.Passengers = [chicken]
    pig.nbt.Debug = {"passenger": chicken}

    let payload = pig.nbt
    payload = chicken

    let echoed = echo(chicken)
    let returned = make_passenger()
    let explicit = chicken.as_nbt()
    let spawned = summon("minecraft:pig", chicken)
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("Passengers set value []"));
    assert!(joined.contains("Passengers[0] set from storage"));
    assert!(joined.contains("Passengers insert 0 from storage"));
    assert!(joined.contains(".passenger set from storage"));
    assert!(joined.contains("data merge storage mcfc:runtime"));
    assert!(joined.contains("summon $(entity) ~ ~ ~ $(data)"));
}

#[test]
fn compiles_nested_entity_builder_passengers() {
    let source = r#"
fn main() -> void:
    let pig = entity("minecraft:pig")
    pig.name = "Dinnerbone"
    pig.glowing = true
    pig.tags = ["cool-tag"]

    let chicken = entity("minecraft:chicken")
    chicken.name = "Marcusson"
    chicken.tags = ["cooler-tag"]

    let villager = entity("minecraft:villager")

    chicken.nbt.Passengers[0] = villager
    pig.nbt.Passengers[0] = chicken

    summon(pig)
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("chicken.nbt.Passengers set value []"));
    assert!(joined.contains("chicken.nbt.Passengers insert 0 from storage"));
    assert!(joined.contains("pig.nbt.Passengers set value []"));
    assert!(joined.contains("pig.nbt.Passengers insert 0 from storage"));
    assert!(joined.contains("data merge storage mcfc:runtime"));
    assert!(joined.contains("summon $(entity) ~ ~ ~ $(data)"));
}

#[test]
fn compiles_block_builder_as_nbt_payload_only() {
    let source = r#"
fn echo(value: nbt) -> nbt:
    return value
fn main() -> void:
    let chest = block_type("minecraft:chest")
    chest.states.facing = "north"
    chest.name = "Loot"
    chest.lock = "secret"

    let payload = chest.nbt
    payload = chest

    let echoed = echo(chest)
    let explicit = chest.as_nbt()

    let holder = entity("minecraft:armor_stand")
    holder.nbt.DisplayState = chest
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains(".nbt.Lock set from storage"));
    assert!(joined.contains("DisplayState set from storage"));
    assert!(!joined.contains("DisplayState.id"));
    assert!(!joined.contains("DisplayState.states"));
}

#[test]
fn compiles_block_ref_is_checks() {
    let source = r#"
fn main() -> void:
    let below = block("~ ~-1 ~")
    let absolute = block("10 64 10")
    if below.is("minecraft:air"):
        below.setblock("minecraft:purple_concrete")
    if absolute.is("minecraft:stone"):
        absolute.setblock("minecraft:gold_block")
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("execute if block $(pos) $(block) run scoreboard players set"));
    assert!(joined.contains("set value \"minecraft:air\""));
    assert!(joined.contains("set value \"minecraft:stone\""));
    assert!(joined.contains("setblock $(pos) $(block)"));
}

#[test]
fn compiles_async_bossbars_without_default_tick_tag() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    let bb = bossbar("mcfc:test", "Boss")
    bb.value = 5
    bb.players = player

    async:
        sleep(5)
        bb.remove()
        player.position.setblock("minecraft:gold_block")
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    assert!(
        !result
            .artifacts
            .files
            .contains_key("data/minecraft/tags/function/tick.json")
    );
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("bossbar add $(id) \"Boss\""));
    assert!(joined.contains("bossbar set $(id) value $(value)"));
    assert!(joined.contains("bossbar set $(id) players $(selector)"));
    assert!(joined.contains("bossbar remove $(id)"));
    assert!(
        joined.contains(
            "prefix set value \"$(__anchor_prefix)execute at $(__anchor_selector) run \""
        )
    );
    assert!(joined.contains("setblock $(pos) $(block)"));
}

#[test]
fn exposes_no_arg_void_functions_and_special_tick() {
    let source = r#"
fn reset() -> void:
    mc "say reset"

fn helper(value: int) -> void:
    mc "say helper"

fn answer() -> int:
    return 42

fn tick() -> void:
    mc "say first"

fn tick() -> void:
    mc "say second"
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/mcfc/function/reset.mcfunction")
    );
    assert!(
        !result
            .artifacts
            .files
            .contains_key("data/mcfc/function/helper.mcfunction")
    );
    assert!(
        !result
            .artifacts
            .files
            .contains_key("data/mcfc/function/answer.mcfunction")
    );
    let tick = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/tick__d0__entry.mcfunction")
        .unwrap();
    assert!(tick.contains("say first"));
    assert!(tick.contains("say second"));
    let tick_tag = result
        .artifacts
        .files
        .get("data/minecraft/tags/function/tick.json")
        .unwrap();
    assert!(tick_tag.contains("\"mcfc:tick\""));
}

#[test]
fn compiles_tick_sleep_player_state_display_and_equipment_item_defs() {
    let source = r#"
player_state money: int = "Money"

fn main() -> void:
    let player = single(selector("@p"))
    let helmet = item("minecraft:golden_helmet")
    helmet.count = 1
    helmet.name = "Crown"
    player.head.item = helmet
    player.state.money = 5
    sleep_ticks(5)
    mc "say done"
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let setup = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/setup.mcfunction")
        .unwrap();
    assert!(setup.contains("scoreboard objectives add mcfs_money dummy \"Money\""));
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("schedule function mcfc:"));
    assert!(joined.contains("$(ticks)t"));
    assert!(joined.contains("armor.head"));
    assert!(joined.contains("minecraft:custom_name"));
}

#[test]
fn optimizer_folds_literal_branches_and_can_be_disabled() {
    let source = r#"
fn main() -> void:
    let value = 1 + 2 * 3
    if false:
        mc "say hidden"
    else:
        mc "say shown"
"#;

    let optimized =
        compile_source(source, &CompileOptions::default()).expect("source should compile");
    let unoptimized = compile_source(
        source,
        &CompileOptions {
            optimize: false,
            ..CompileOptions::default()
        },
    )
    .expect("source should compile");
    let optimized_main = optimized
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    let unoptimized_joined = unoptimized
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(optimized_main.contains("scoreboard players set $d0_main_value mcfc 7"));
    assert!(optimized_main.contains("say shown"));
    assert!(!optimized_main.contains("say hidden"));
    assert!(unoptimized_joined.contains("say hidden"));
}

#[test]
fn compiles_if_and_while_blocks() {
    let source = r#"
fn inc(x: int) -> int:
    return x + 1
fn main() -> void:
    let a = 0
    while a < 3:
        if a == 1:
            a = inc(a)
        a = a + 1
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let generated_files: Vec<_> = result
        .artifacts
        .files
        .keys()
        .filter(|path| path.contains("while_") || path.contains("if_then"))
        .collect();
    assert!(
        !generated_files.is_empty(),
        "expected generated block files"
    );
}

#[test]
fn compiles_else_for_logic_and_loop_control() {
    let source = r#"
fn main() -> void:
    for i in 0..=5:
        if i == 0 or not false:
            continue
        else if i == 3:
            break
        else:
            mc "say loop"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let generated_files: Vec<_> = result.artifacts.files.keys().cloned().collect();
    assert!(generated_files.iter().any(|path| path.contains("if_else")));
    assert!(generated_files.iter().any(|path| path.contains("for_cond")));
    assert!(generated_files.iter().any(|path| path.contains("for_step")));
    assert!(
        generated_files
            .iter()
            .any(|path| path.contains("logic_or_rhs"))
    );
}

#[test]
fn compiles_string_equality() {
    let source = r#"
fn main() -> void:
    let a = "done"
    let b = "done"
    if a == b:
        mc "say equal"
    if a != "other":
        mc "say diff"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let main = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("execute store success score $d0___cmp_"));
    assert!(main.contains("data modify storage mcfc:runtime frames.__cmp__tmp"));
}

#[test]
fn compiles_string_character_index_reads() {
    let source = r#"
fn main() -> void:
    let book_content = "Book"
    let first = book_content[0]
    let last = book_content[-1]
    let idx = 1
    let second = book_content[idx]
    mcf "say $(first) $(last) $(second)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("set string storage mcfc:runtime"));
    assert!(joined.contains("with storage mcfc:runtime frames.d0.main.__str_index"));
    assert!(joined.contains("matches ..-1 run scoreboard players operation"));
}

#[test]
fn compiles_string_character_index_reads_through_prefix_paths() {
    let source = r#"
fn main() -> void:
    let words = ["hello"]
    let second = words[0][1]
    mcf "say $(second)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let joined = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("set string storage mcfc:runtime"));
}

#[test]
fn compiles_storage_backed_arrays() {
    let source = r#"
fn pick(xs: array<int>, index: int) -> int:
    return xs[index]
fn main() -> void:
    let values = [1, 2, 3]
    let i = 1
    values.push(4)
    let popped = values.pop()
    let size = values.len()
    values[i] = popped + size
    let selected = pick(values, i)
    mcf "say $(selected)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let files = result.artifacts.files;
    let main = files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("data modify storage mcfc:runtime frames.d0.main.values set value []"));
    assert!(main.contains("append from storage mcfc:runtime"));
    assert!(main.contains("data remove storage mcfc:runtime frames.d0.main.values[-1]"));
    assert!(main.contains(
        "execute store result score $d0_main_size mcfc run data get storage mcfc:runtime"
    ));
    assert!(
        files
            .values()
            .any(|file| file.contains("with storage mcfc:runtime frames.d0.main.__path"))
    );
    assert!(files.values().any(|file| file.contains("$(i1)")));
}

#[test]
fn compiles_array_remove() {
    let source = r#"
fn main() -> void:
    let values = [3, 5, 8]
    let first = values.remove(0)
    let second = values.remove(1)
    mcf "say $(first)"
    mcf "say $(second)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let files = result.artifacts.files;
    assert!(files.values().any(|file| {
        file.contains("data remove storage mcfc:runtime frames.d0.main.values[$(index)]")
    }));
    assert!(
        files
            .values()
            .any(|file| file.contains("execute store result storage mcfc:runtime"))
    );
}

#[test]
fn compiles_array_remove_at() {
    let source = r#"
fn main() -> void:
    let values = [3, 5, 8]
    let first = values.remove_at(0)
    let second = values.remove_at(1)
    mcf "say $(first)"
    mcf "say $(second)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let files = result.artifacts.files;
    assert!(files.values().any(|file| {
        file.contains("data remove storage mcfc:runtime frames.d0.main.values[$(index)]")
    }));
    assert!(
        files
            .values()
            .any(|file| file.contains("execute store result storage mcfc:runtime"))
    );
}

#[test]
fn compiles_array_for_each() {
    let source = r#"
fn main() -> void:
    let values = [1, 2, 3]
    for value in values:
        mcf "say $(value)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let files: Vec<_> = result.artifacts.files.values().cloned().collect();
    assert!(files.iter().any(|file| file.contains("for_each_cond")));
    assert!(files.iter().any(|file| file.contains("for_each_step")));
    assert!(files.iter().any(|file| file.contains("__for_each")
        && file.contains("internal_macro")
        || file.contains("[$(index)]")));
}

#[test]
fn compiles_storage_backed_dictionaries() {
    let source = r#"
fn main() -> void:
    let counts = {"wood": 12, "stone": 4}
    let key = "wood"
    counts[key] = 13
    let has_wood = counts.has(key)
    counts.remove("stone")
    let amount = counts[key]
    if has_wood:
        mcf "say $(amount)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let files = result.artifacts.files;
    let main = files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("data modify storage mcfc:runtime frames.d0.main.counts set value {}"));
    assert!(main.contains("frames.d0.main.counts.wood"));
    assert!(main.contains("data remove storage mcfc:runtime frames.d0.main.counts.stone"));
    assert!(
        files
            .values()
            .any(|file| file.contains(".$(k1)") || file.contains(".$(key)"))
    );
    assert!(
        files
            .values()
            .any(|file| file.contains("execute if data storage mcfc:runtime"))
    );
}

#[test]
fn compiles_has_data_with_dynamic_storage_nbt_paths() {
    let source = r#"
fn probe(store: dict<nbt>, key: string, index: int) -> bool:
    return has_data(store[key].items[index].name)
fn main() -> void:
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let files = result.artifacts.files;
    assert!(
        files
            .values()
            .any(|file| file.contains("execute if data storage mcfc:runtime"))
    );
    assert!(
        files
            .values()
            .any(|file| file.contains(".$(n1)") || file.contains(".$(k1)"))
    );
    assert!(
        files
            .values()
            .any(|file| file.contains("[$(n") || file.contains("[$(i"))
    );
}

#[test]
fn compiles_string_match_dispatch() {
    let source = r#"
fn main() -> void:
    let action = "jump"
    match action:
        "pathfind" => mc "say move"
        "jump" => mc "say leap"
        "idle" => mc "say wait"
        else => mc "say default"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let files = result.artifacts.files;
    let main = files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("execute store success score $d0___cmp_"));
    assert!(main.contains("scoreboard players set $d0_main___tmp1 mcfc 0"));
    assert!(main.contains("matches 0 run scoreboard players set $d0_main___tmp1 mcfc 1"));
    assert!(files.values().any(|file| file.contains("say default")));
}

#[test]
fn compiles_struct_literals_and_field_access() {
    let source = r#"
struct Action:
    action: string
    duration: int
fn tick(action: Action) -> int:
    return action.duration
fn main() -> void:
    let action = Action{action: "idle", duration: 40}
    let actions = [action]
    let first = actions[0]
    let duration = tick(first)
    mcf "say $(duration)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let files = result.artifacts.files;
    let main = files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("frames.d0.main.action set value {}"));
    assert!(files.values().any(|file| file.contains(".duration")));
}

#[test]
fn rejects_invalid_struct_usage() {
    let source = r#"
struct Action:
    action: string
    duration: int
fn main() -> void:
    let bad = Action{action: "idle"}
    let wrong = Action{action: 1, duration: 5}
    let also_bad = bad.missing
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("missing field 'Action.duration'"));
    assert!(rendered.contains("field 'Action.action' expects 'string', found 'int'"));
    assert!(rendered.contains("unknown field 'Action.missing'"));
}

#[test]
fn rejects_invalid_string_index_usage() {
    let source = r#"
fn main() -> void:
    let book_content = "Book"
    let bad = book_content["x"]
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("string index must have type 'int'")
    );
}

#[test]
fn rejects_invalid_collection_usage() {
    let source = r#"
fn bad_param(xs: array<entity_ref>) -> void:
    return
fn bad_has_data(player: entity_ref, key: int) -> bool:
    return has_data(player.nbt[key])
fn main() -> void:
    let arr = [1, 2]
    let dict = {"wood": 1}
    let empty = []
    let bad_mix = [1, "two"]
    let bad_index = arr["x"]
    let bad_remove = arr.remove_at("x")
    let bad_remove_alias = arr.remove("x")
    let bad_key = dict[1]
    let bad_refs = [selector("@a")]
    arr.push("bad")
    dict["bad-key"] = 2
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("empty array literals require type context"));
    assert!(rendered.contains("array literals must contain values of one type"));
    assert!(rendered.contains("array index must have type 'int'"));
    assert!(rendered.contains("dictionary key must have type 'string'"));
    assert!(rendered.contains("has_data(...) requires a storage-backed variable or path"));
    assert!(rendered.contains("push(...) value must be 'int', found 'string'"));
    assert!(rendered.contains("remove_at(...) index must be 'int'"));
    assert!(rendered.contains("remove(...) index must be 'int'"));
    assert!(rendered.contains("dictionary key 'bad-key' is not storage-path-safe"));
    assert!(rendered.contains("dynamic nbt path indices require a storage-backed base"));
    assert!(rendered.contains("collection values may not have unsupported type 'entity_ref'"));
    assert!(rendered.contains("collection values may not have unsupported type 'entity_set'"));
}

#[test]
fn for_bounds_are_evaluated_once() {
    let source = r#"
fn start() -> int:
    return 1
fn finish() -> int:
    return 3
fn main() -> void:
    for i in start()..=finish():
        mc "say loop"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let main = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert_eq!(main.matches("generated/start__d1__entry").count(), 1);
    assert_eq!(main.matches("generated/finish__d1__entry").count(), 1);
}

#[test]
fn rejects_invalid_loop_control_logic_and_for_usage() {
    let source = r#"
fn main() -> void:
    break
    continue
    let i = 0
    for i in 0.."bad":
        return
    for item in 1:
        return
    if 1 and true:
        return
    if "a" < "b":
        return
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("'break' may only appear inside a loop"));
    assert!(rendered.contains("'continue' may only appear inside a loop"));
    assert!(rendered.contains("variable 'i' is already defined"));
    assert!(rendered.contains("for range end must have type 'int'"));
    assert!(rendered.contains("for-each iteration requires an 'entity_set' or 'array'"));
    assert!(rendered.contains("logical operators require 'bool' operands"));
    assert!(rendered.contains("strings only support '==' and '!=' comparisons"));
}

#[test]
fn rejects_invalid_match_usage() {
    let source = r#"
fn main() -> void:
    let bad = 1
    match bad:
        "a" => mc "say a"
        "a" => mc "say b"
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("match value must have type 'string'"));
    assert!(rendered.contains("duplicate match arm 'a'"));
}

#[test]
fn rejects_invalid_query_usage() {
    let source = r#"
fn main() -> void:
    let bad = single(selector("@e[type=pig,limit=2]"))
    let also_bad = selector("@e[type=pig]")
    also_bad.CustomName = "Nope"
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("single(selector(...)) requires no limit or 'limit=1'"));
    assert!(rendered.contains(
        "path assignment requires an 'entity_ref', 'block_ref', bossbar, or storage-backed base",
    ));
}

#[test]
fn rejects_unsafe_player_writes() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    player.CustomName = "Nope"
    player.nbt.SelectedItem = "bad"
    player.state.story = "hello"
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("player path access must use 'player.nbt', 'player.state', 'player.tags', 'player.team', 'player.position', 'player.inventory[index]', 'player.hotbar[index]', or an equipment namespace such as 'mainhand'"));
    assert!(rendered.contains("player.nbt.* is read-only"));
    assert!(rendered.contains("player.state.* currently supports only 'int' and 'bool' values"));
}

#[test]
fn rejects_invalid_entity_state_writes() {
    let source = r#"
fn main() -> void:
    let marker = single(selector("@e[type=minecraft:marker,limit=1]"))
    marker.state.name = "bad"
    marker.state.payload = item("minecraft:stick")
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("entity.state.* currently supports only 'int' and 'bool' values"));
}

#[test]
fn rejects_invalid_inventory_slot_usage() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    let pig = single(selector("@e[type=pig,limit=1]"))
    pig.inventory[0].count = 1
    player.hotbar["bad"].count = 1
    player.hotbar[9].count = 1
    player.inventory[27].count = 1
    player.hotbar[0] = "bad"
    player.inventory[0].exists = true
    player.hotbar[0].id = "minecraft:stick"
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(
        rendered.contains(
            "inventory and hotbar are only supported on known player refs; use 'player_ref' to assert a player"
        )
    );
    assert!(rendered.contains("player.hotbar[...] slot index must have type 'int'"));
    assert!(rendered.contains("player.hotbar[...] slot index must be between 0 and 8"));
    assert!(rendered.contains("player.inventory[...] slot index must be between 0 and 26"));
    assert!(rendered.contains("whole-slot inventory assignment requires an 'item_def' value"));
    assert!(rendered.contains("item slot.exists is read-only"));
    assert!(rendered.contains("item slot.id is read-only"));
}

#[test]
fn guards_later_statements_after_nested_return() {
    let source = r#"
fn main() -> void:
    if true:
        return
    else:
        mc "say no"
    mc "say after"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let main = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/main__d0__entry.mcfunction")
        .unwrap();
    assert!(main.contains("execute if score $d0_main__ctrl mcfc matches 0 run say after"));
}

#[test]
fn rejects_recursion() {
    let source = r#"
fn a(x: int) -> int:
    return b(x)
fn b(x: int) -> int:
    return a(x)
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    assert!(error.to_string().contains("recursion is not supported"));
}

#[test]
fn rejects_invalid_macro_placeholders() {
    let source = r#"
fn main() -> void:
    let a = 1
    let player = single(selector("@p"))
    if true:
        let inner = 2
    mcf "say $(missing)"
    mcf "say $(inner)"
    mcf "say $("
    mcf "say $(player.CustomName)"
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("undefined variable 'missing'"));
    assert!(rendered.contains("undefined variable 'inner'"));
    assert!(rendered.contains("unterminated macro placeholder"));
    assert!(rendered.contains("player path access must use 'player.nbt', 'player.state', 'player.tags', 'player.team', 'player.position', 'player.inventory[index]', 'player.hotbar[index]', or an equipment namespace such as 'mainhand'"));
}

#[test]
fn compiles_expression_macro_placeholders() {
    let source = r#"
struct Action:
    kind: string
    duration: int
fn tick(action: Action) -> int:
    return action.duration + 1
fn main() -> void:
    let a = 2
    let x = 3
    let y = 3
    let flag = true
    let ready = false
    let values = [10, 20]
    let key = "npc"
    let store = {"npc": {"value": 7}}
    let action = Action{kind: "idle", duration: 40}
    let player = single(selector("@p"))
    mcf "say $(a + 1)"
    mcf "say $(x == y)"
    mcf "say $(flag and not ready)"
    mcf "say $(tick(action))"
    mcf "say $(values.remove(0))"
    mcf "say $(values.remove_at(0))"
    mcf "say $(store[key][\"value\"])"
    mcf "say $(action.duration)"
    mcf "say $(player.state.quest_complete)"
    mcf "say $(player.team)"
    return
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    let files = result.artifacts.files;
    assert!(files.values().any(|file| file.contains("$(p1)")));
    assert!(
        files
            .values()
            .any(|file| file.contains("arithmetic operators")
                || file.contains("scoreboard players operation")
                || file.contains("scoreboard players set"))
    );
    assert!(
        files
            .values()
            .any(|file| file.contains("data remove storage mcfc:runtime"))
    );
    assert!(
        files
            .values()
            .any(|file| file.contains("function mcfc:generated/"))
    );
}

#[test]
fn rejects_invalid_as_and_at_contexts() {
    let source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    as(block("~ ~ ~")):
        mc "say bad"
    at(block("~ ~ ~")):
        mc "say bad"
    let bad = as(player, 1)
    return
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("as context block requires an 'entity_set' or 'entity_ref' anchor"));
    assert!(rendered.contains("at context block requires an 'entity_set' or 'entity_ref' anchor"));
    assert!(
        rendered.contains("as(...) requires an 'entity_set', 'entity_ref', or 'block_ref' value")
    );
}

#[test]
fn rejects_removed_book_annotations_and_legacy_gameplay_builtins() {
    let book_source = r#"
@book
fn old() -> void:
    return
"#;

    let book_error = compile_source(book_source, &CompileOptions::default()).unwrap_err();
    assert!(
        book_error
            .to_string()
            .contains("unknown annotation '@book'")
    );

    let legacy_source = r#"
fn main() -> void:
    let player = single(selector("@p"))
    tellraw(player, "old")
    return
"#;

    let legacy_error = compile_source(legacy_source, &CompileOptions::default()).unwrap_err();
    assert!(legacy_error.to_string().contains("target.tellraw(message)"));
}

#[test]
fn cli_writes_output_tree() {
    let source = r#"
fn main() -> void:
    let a = 1
    a = a + 2
    return
"#;

    let base = temp_path();
    let input = base.join("program.mcf");
    let out = base.join("out");
    fs::create_dir_all(&base).unwrap();
    fs::write(&input, source).unwrap();

    let status = mcfc::cli::run(vec![
        "mcfc".into(),
        "build".into(),
        input.display().to_string(),
        "--out".into(),
        out.display().to_string(),
        "--emit-ir".into(),
        "--clean".into(),
    ]);

    assert_eq!(status, 0);
    assert!(out.join("pack.mcmeta").exists());
    assert!(out.join("debug").join("ir.txt").exists());
}

#[test]
fn cli_infers_namespace_from_input_filename() {
    let source = r#"
fn main() -> void:
    return
"#;

    let base = temp_path();
    let input = base.join("Test-Pack.mcf");
    let out = base.join("out");
    fs::create_dir_all(&base).unwrap();
    fs::write(&input, source).unwrap();

    let status = mcfc::cli::run(vec![
        "mcfc".into(),
        "build".into(),
        input.display().to_string(),
        "--out".into(),
        out.display().to_string(),
    ]);

    assert_eq!(status, 0);
    assert!(
        out.join("data")
            .join("test-pack")
            .join("function")
            .join("main.mcfunction")
            .exists()
    );
}

#[test]
fn cli_explicit_namespace_overrides_filename_inference() {
    let source = r#"
fn main() -> void:
    return
"#;

    let base = temp_path();
    let input = base.join("test.mcf");
    let out = base.join("out");
    fs::create_dir_all(&base).unwrap();
    fs::write(&input, source).unwrap();

    let status = mcfc::cli::run(vec![
        "mcfc".into(),
        "build".into(),
        input.display().to_string(),
        "--out".into(),
        out.display().to_string(),
        "--namespace".into(),
        "custom_space".into(),
    ]);

    assert_eq!(status, 0);
    assert!(
        out.join("data")
            .join("custom_space")
            .join("function")
            .join("main.mcfunction")
            .exists()
    );
}

#[test]
fn compiles_multi_file_project_with_assets_and_exports() {
    let base = temp_path();
    let project = base.join("sample_project");
    let src_dir = project.join("src");
    let assets_dir = project
        .join("assets")
        .join("data")
        .join("sample")
        .join("predicate");
    fs::create_dir_all(src_dir.join("api")).unwrap();
    fs::create_dir_all(&assets_dir).unwrap();

    fs::write(
        project.join("sample.mcfc.toml"),
        r#"
namespace = "sample"
source_dir = "src"
asset_dir = "assets"
out_dir = "out"

load = ["sample:bootstrap/load"]
tick = ["sample:bootstrap/tick"]

[[export]]
path = "bootstrap/load"
function = "bootstrap_load"

[[export]]
path = "bootstrap/tick"
function = "bootstrap_tick"

[[export]]
path = "api/create"
function = "api_create"
"#,
    )
    .unwrap();

    fs::write(
        src_dir.join("bootstrap.mcf"),
        r#"
fn bootstrap_load() -> void:
    mc "say load"
    return
fn bootstrap_tick() -> void:
    mc "say tick"
    return
"#,
    )
    .unwrap();

    fs::write(
        src_dir.join("api").join("create.mcf"),
        r#"
fn api_create() -> void:
    mc "say create"
    return
"#,
    )
    .unwrap();

    fs::write(
        assets_dir.join("enabled.json"),
        "{\n  \"condition\": \"minecraft:inverted\"\n}\n",
    )
    .unwrap();

    let out = project.join("dist");
    let result = compile_project(
        &project.join("sample.mcfc.toml"),
        &out,
        &CompileOptions {
            clean: true,
            ..CompileOptions::default()
        },
    )
    .expect("project should compile");

    assert!(
        result
            .artifacts
            .files
            .contains_key("data/sample/function/bootstrap/load.mcfunction")
    );
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/sample/function/api/create.mcfunction")
    );
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/sample/predicate/enabled.json")
    );

    let load_tag = result
        .artifacts
        .files
        .get("data/minecraft/tags/function/load.json")
        .unwrap();
    assert!(load_tag.contains("\"sample:bootstrap/load\""));

    let tick_tag = result
        .artifacts
        .files
        .get("data/minecraft/tags/function/tick.json")
        .unwrap();
    assert!(tick_tag.contains("\"sample:bootstrap/tick\""));

    let wrapper = result
        .artifacts
        .files
        .get("data/sample/function/api/create.mcfunction")
        .unwrap();
    assert!(wrapper.contains("function sample:generated/api_create__d0__entry"));
    assert!(
        out.join("data")
            .join("sample")
            .join("predicate")
            .join("enabled.json")
            .exists()
    );
}

#[test]
fn cli_builds_project_directory_using_manifest_default_output() {
    let base = temp_path();
    let project = base.join("cli_project");
    let src_dir = project.join("src");
    fs::create_dir_all(&src_dir).unwrap();

    fs::write(
        project.join("cli.mcfc.toml"),
        r#"
namespace = "cli_demo"
source_dir = "src"
asset_dir = "assets"
out_dir = "out"

[[export]]
path = "bootstrap/load"
function = "bootstrap_load"
"#,
    )
    .unwrap();

    fs::write(
        src_dir.join("bootstrap.mcf"),
        r#"
fn bootstrap_load() -> void:
    mc "say hello"
    return
"#,
    )
    .unwrap();

    let status = mcfc::cli::run(vec![
        "mcfc".into(),
        "build".into(),
        project.display().to_string(),
        "--clean".into(),
    ]);

    assert_eq!(status, 0);
    assert!(
        project
            .join("out")
            .join("data")
            .join("cli_demo")
            .join("function")
            .join("bootstrap")
            .join("load.mcfunction")
            .exists()
    );
}

#[test]
fn anpc_project_builds_expected_public_outputs() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("ANPC_MCF")
        .join("anpc.mcfc.toml");
    if !manifest.exists() {
        return;
    }
    let out = temp_path().join("anpc_out");
    let result = compile_project(
        &manifest,
        &out,
        &CompileOptions {
            clean: true,
            ..CompileOptions::default()
        },
    )
    .expect("ANPC project should compile");

    assert!(
        result
            .artifacts
            .files
            .contains_key("data/anpc/function/internal/init.mcfunction")
    );
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/anpc/function/internal/tick.mcfunction")
    );
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/anpc/function/api/create_default.mcfunction")
    );
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/anpc/function/api/create_dialogue.mcfunction")
    );
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/anpc/function/api/create_guard.mcfunction")
    );
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/anpc/advancement/internal/interacted_with_interaction.json")
    );
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/anpc/predicate/has_vehicle.json")
    );
}

fn temp_path() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mcfc_test_{unique}"))
}
