use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use mcfc::compiler::{compile_source, CompileOptions};

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
    assert!(result
        .artifacts
        .files
        .contains_key("data/mcfc/function/main.mcfunction"));
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
    assert!(result
        .artifacts
        .files
        .contains_key("data/minecraft/tags/function/tick.json"));
    assert!(result
        .artifacts
        .files
        .contains_key("data/mcfc/function/generated/book/tick.mcfunction"));
    assert!(result
        .artifacts
        .files
        .contains_key("data/mcfc/function/generated/book/dispatch_fibb.mcfunction"));
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
    assert!(!generated_files.is_empty(), "expected generated block files");
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

fn temp_path() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("mcfc_test_{unique}"))
}
