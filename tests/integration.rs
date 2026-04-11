use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use mcfc::compiler::{CompileOptions, compile_source};

#[test]
fn compiles_straight_line_program() {
    let source = r#"
fn main() -> void
    let a = 5
    let b = 7
    let text = "done"
    b = a + b
    mc "say done"
    return
end
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
fn main() -> void # signature comment
    let a = 1 # inline comment
    # inside block
    mc "say done"
    return
end
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
fn main() -> void
    let a = 'done'
    mc 'say "done"'
    mcf 'say $(a)'
    return
end
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
    assert!(macro_file.contains("$say $(a)"));
}

#[test]
fn compiles_macro_command_with_storage_call() {
    let source = r#"
fn main() -> void
    let amount = 5
    let label = "hello"
    mcf "xp add @a $(amount) levels"
    mcf "say $(label)"
    mc "say $(amount)"
    return
end
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
    assert!(macro_file.contains("$xp add @a $(amount) levels"));
}

#[test]
fn compiles_entity_queries_and_iteration() {
    let source = r#"
fn main() -> void
    let pigs = selector("@e[type=pig,limit=3]")
    for pig in pigs:
        pig.CustomName = "Hello"
    end
    return
end
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
    assert!(result.artifacts.files.values().any(|file| file.contains("data modify entity $(selector) CustomName set from storage")));
}

#[test]
fn compiles_single_exists_and_context_composition() {
    let source = r#"
fn main() -> void
    let player = single(selector("@a[tag=hunter]"))
    if exists(player):
        let nearest = single(at(player, selector("@e[type=pig,sort=nearest]")))
        if exists(nearest):
            nearest.CustomName = "Target"
        end
    end
    return
end
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
fn compiles_as_value_context_composition() {
    let source = r#"
fn main() -> void
    let player = single(selector("@p"))
    if exists(player):
        let self_ref = single(as(player, selector("@s")))
        self_ref.tags.welcomed = true
    end
    return
end
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
fn main() -> void
    let player = single(selector("@p"))
    as(player):
        mcf 'tellraw @s "welcome @s"'
        mc 'title @s actionbar "title @s"'
        mc "say hello @s"
    end
    at(player):
        mc "say here"
    end
    return
end
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
fn main() -> void
    let player = single(selector("@p"))
    at(player):
        as(selector("@e[type=pig,limit=1]")):
            mc "say @s"
        end
    end
    return
end
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
fn compiles_block_paths_and_nbt_casts() {
    let source = r#"
fn main() -> void
    let chest = block("~ ~ ~")
    chest.CustomName = "Loot"
    let name = string(chest.CustomName)
    return
end
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
fn compiles_player_safe_api_surfaces() {
    let source = r#"
fn main() -> void
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
    end
    return
end
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
fn compiles_book_runtime_for_annotated_functions() {
    let source = r#"
@book
fn fibb(n: int) -> void
    mcf "tellraw @s \"$(n)\""
    return
end

fn main() -> void
    return
end
"#;

    let result = compile_source(source, &CompileOptions::default()).expect("source should compile");
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/minecraft/tags/function/tick.json")
    );
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/mcfc/function/generated/book/tick.mcfunction")
    );
    assert!(
        result
            .artifacts
            .files
            .contains_key("data/mcfc/function/generated/book/dispatch_fibb.mcfunction")
    );
    let dispatch = result
        .artifacts
        .files
        .get("data/mcfc/function/generated/book/dispatch_fibb.mcfunction")
        .unwrap();
    assert!(dispatch.contains("Wrong argument count for fibb"));
}

#[test]
fn compiles_if_and_while_blocks() {
    let source = r#"
fn inc(x: int) -> int
    return x + 1
end

fn main() -> void
    let a = 0
    while a < 3:
        if a == 1:
            a = inc(a)
        end
        a = a + 1
    end
    return
end
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
fn main() -> void
    for i in 0..=5:
        if i == 0 or not false:
            continue
        else if i == 3:
            break
        else:
            mc "say loop"
        end
    end
    return
end
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
fn main() -> void
    let a = "done"
    let b = "done"
    if a == b:
        mc "say equal"
    end
    if a != "other":
        mc "say diff"
    end
    return
end
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
fn compiles_storage_backed_arrays() {
    let source = r#"
fn pick(xs: array<int>, index: int) -> int
    return xs[index]
end

fn main() -> void
    let values = [1, 2, 3]
    let i = 1
    values.push(4)
    let popped = values.pop()
    let size = values.len()
    values[i] = popped + size
    let selected = pick(values, i)
    mcf "say $(selected)"
    return
end
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
fn compiles_storage_backed_dictionaries() {
    let source = r#"
fn main() -> void
    let counts = {"wood": 12, "stone": 4}
    let key = "wood"
    counts[key] = 13
    let has_wood = counts.has(key)
    counts.remove("stone")
    let amount = counts[key]
    if has_wood:
        mcf "say $(amount)"
    end
    return
end
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
fn rejects_invalid_collection_usage() {
    let source = r#"
fn bad_param(xs: array<entity_ref>) -> void
    return
end

fn main() -> void
    let arr = [1, 2]
    let dict = {"wood": 1}
    let empty = []
    let bad_mix = [1, "two"]
    let bad_index = arr["x"]
    let bad_key = dict[1]
    let bad_refs = [selector("@a")]
    arr.push("bad")
    dict["bad-key"] = 2
    return
end
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("empty array literals require type context"));
    assert!(rendered.contains("array literals must contain values of one type"));
    assert!(rendered.contains("array index must have type 'int'"));
    assert!(rendered.contains("dictionary key must have type 'string'"));
    assert!(rendered.contains("push(...) value must be 'int', found 'string'"));
    assert!(rendered.contains("dictionary key 'bad-key' is not storage-path-safe"));
    assert!(rendered.contains("collection values may not have unsupported type 'entity_ref'"));
    assert!(rendered.contains("collection values may not have unsupported type 'entity_set'"));
}

#[test]
fn for_bounds_are_evaluated_once() {
    let source = r#"
fn start() -> int
    return 1
end

fn finish() -> int
    return 3
end

fn main() -> void
    for i in start()..=finish():
        mc "say loop"
    end
    return
end
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
fn main() -> void
    break
    continue
    let i = 0
    for i in 0.."bad":
        return
    end
    if 1 and true:
        return
    end
    if "a" < "b":
        return
    end
    return
end
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("'break' may only appear inside a loop"));
    assert!(rendered.contains("'continue' may only appear inside a loop"));
    assert!(rendered.contains("variable 'i' is already defined"));
    assert!(rendered.contains("for range end must have type 'int'"));
    assert!(rendered.contains("logical operators require 'bool' operands"));
    assert!(rendered.contains("strings only support '==' and '!=' comparisons"));
}

#[test]
fn rejects_invalid_query_usage() {
    let source = r#"
fn main() -> void
    let bad = single(selector("@e[type=pig,limit=2]"))
    let also_bad = selector("@e[type=pig]")
    also_bad.CustomName = "Nope"
    return
end
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("single(selector(...)) requires no limit or 'limit=1'"));
    assert!(rendered.contains("path assignment requires an 'entity_ref' or 'block_ref' base"));
}

#[test]
fn rejects_unsafe_player_writes() {
    let source = r#"
fn main() -> void
    let player = single(selector("@p"))
    player.CustomName = "Nope"
    player.nbt.SelectedItem = "bad"
    player.state.story = "hello"
    return
end
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("player path access must use 'player.nbt', 'player.state', 'player.tags', 'player.team', or 'player.mainhand'"));
    assert!(rendered.contains("player.nbt.* is read-only"));
    assert!(rendered.contains("player.state.* currently supports only 'int' and 'bool' values"));
}

#[test]
fn guards_later_statements_after_nested_return() {
    let source = r#"
fn main() -> void
    if true:
        return
    else:
        mc "say no"
    end
    mc "say after"
    return
end
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
fn a(x: int) -> int
    return b(x)
end

fn b(x: int) -> int
    return a(x)
end
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    assert!(error.to_string().contains("recursion is not supported"));
}

#[test]
fn rejects_invalid_macro_placeholders() {
    let source = r#"
fn main() -> void
    let a = 1
    if true:
        let inner = 2
    end
    mcf "say $(missing)"
    mcf "say $(inner)"
    mcf "say $(a + 1)"
    mcf "say $("
    return
end
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("unknown macro placeholder 'missing'"));
    assert!(rendered.contains("unknown macro placeholder 'inner'"));
    assert!(rendered.contains("invalid macro placeholder character ' '"));
    assert!(rendered.contains("unterminated macro placeholder"));
}

#[test]
fn rejects_invalid_as_and_at_contexts() {
    let source = r#"
fn main() -> void
    let player = single(selector("@p"))
    as(block("~ ~ ~")):
        mc "say bad"
    end
    at(block("~ ~ ~")):
        mc "say bad"
    end
    let bad = as(player, 1)
    return
end
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
fn rejects_invalid_book_annotations() {
    let source = r#"
@book
fn bad_return(n: int) -> int
    return n
end

@book
fn bad_param(label: string) -> void
    return
end
"#;

    let error = compile_source(source, &CompileOptions::default()).unwrap_err();
    let rendered = error.to_string();
    assert!(rendered.contains("@book function 'bad_return' must return 'void'"));
    assert!(rendered.contains("@book function 'bad_param' may only have 'int' parameters"));
}

#[test]
fn cli_writes_output_tree() {
    let source = r#"
fn main() -> void
    let a = 1
    a = a + 2
    return
end
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
fn main() -> void
    return
end
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
fn main() -> void
    return
end
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

fn temp_path() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mcfc_test_{unique}"))
}
