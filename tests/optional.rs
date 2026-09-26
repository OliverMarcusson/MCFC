use mcfc::compiler::{CompileOptions, compile_source};

#[test]
fn safe_collection_lookups_lower_presence_and_fallback() {
    let source = r#"
struct Item:
    name: string

fn lookup(items: array<Item>, index: int) -> Optional<Item>:
    return items.get(index)

fn main() -> void:
    let items = [Item{name: "apple"}]
    let index = 0
    let found = lookup(items, index)
    let name = found.orElse(Item{name: "missing"}).name
    let exists = found.isPresent()
    let counts = {"apple": 2}
    let key = "banana"
    let maybe_count = counts.get(key)
    let count = maybe_count.orElse(5)
    let unusual = {"unused": "value"}.get("unused").orElse("missing")
    let spaced = {"two_words": "value"}
    let dynamic_key = "two_words"
    let spaced_value = spaced.get(dynamic_key).orElse("missing")
    let prices = [1.5]
    let price = prices.get(3).orElse(2.0)
    mcf "say $(name) $(exists) $(count)"
"#;
    let result = compile_source(source, &CompileOptions::default()).expect("safe lookups compile");
    let files: Vec<&str> = result
        .artifacts
        .files
        .values()
        .map(String::as_str)
        .collect();
    let generated = files.join("\n");
    assert!(generated.contains("set value {present:0b}"));
    assert!(generated.contains(".present set value 1b"));
    assert!(generated.contains(".value set from storage"));
    assert!(generated.contains("[$(index)]"));
    assert!(generated.contains(".\"$(key)\""));
    assert!(generated.contains("matches 1 run"));
    assert!(generated.contains(".\"unused\""));
}

#[test]
fn optional_values_can_be_nested_in_collections() {
    let source = r#"
fn nested(values: array<Optional<int>>) -> Optional<Optional<int>>:
    return values.get(0)

fn main() -> void:
    let numbers = [1]
    let values = [numbers.get(0)]
    let outer = nested(values)
    let inner = outer.orElse(numbers.get(1))
    let result = inner.orElse(0)
    mcf "say $(result)"
"#;
    compile_source(source, &CompileOptions::default()).expect("nested Optional values compile");
}

#[test]
fn safe_entity_lookup_returns_optional_reference() {
    let source = r#"
fn main() -> void:
    let maybe = find_first(selector("@e[type=minecraft:pig]"))
    let present = maybe.isPresent()
    let pig = maybe.orElse(single(selector("@s")))
    if present:
        pig.add_tag("found")
"#;
    let result =
        compile_source(source, &CompileOptions::default()).expect("entity lookup compiles");
    let generated = result
        .artifacts
        .files
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(generated.contains("if entity $(selector)"));
    assert!(generated.contains("@e[type=minecraft:pig,limit=1]"));
    assert!(generated.contains("{present:1b}"));
}

#[test]
fn safe_lookup_rejects_wrong_key_and_fallback_types() {
    for source in [
        "fn main() -> void:\n    let xs = [1]\n    let value = xs.get(\"x\")\n",
        "fn main() -> void:\n    let xs = [1]\n    let value = xs.get(0).orElse(\"x\")\n",
        "fn first(xs: entity_set) -> Optional<entity_ref>:\n    return find_first(xs)\n",
    ] {
        assert!(compile_source(source, &CompileOptions::default()).is_err());
    }
}
