use mcfc::compiler::{CompileOptions, compile_source};

#[test]
fn safe_collection_lookups_lower_presence_and_fallback() {
    let source = r#"
record Item(String name) {}

class Main {
    static Optional<Item> lookup(List<Item> items, int index) {
        return items.get(index);
    }

    public static void main() {
        var items = List.of(new Item("apple"));
        var index = 0;
        var found = lookup(items, index);
        var name = found.orElse(new Item("missing")).name();
        var exists = found.isPresent();
        var counts = Map.of("apple", 2);
        var key = "banana";
        var maybe_count = counts.get(key);
        var count = maybe_count.orElse(5);
        var unusual = Map.of("unused", "value").get("unused").orElse("missing");
        var spaced = Map.of("two_words", "value");
        var dynamic_key = "two_words";
        var spaced_value = spaced.get(dynamic_key).orElse("missing");
        var prices = List.of(1.5);
        var price = prices.get(3).orElse(2.0);
        Commands.run("say $(name) $(exists) $(count)");
    }
}
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
    let source = r#"class Main {
    static Optional<Optional<Integer>> nested(List<Optional<Integer>> values) {
        return values.get(0);
    }

    public static void main() {
        var numbers = List.of(1);
        var values = List.of(numbers.get(0));
        var outer = nested(values);
        var inner = outer.orElse(numbers.get(1));
        var result = inner.orElse(0);
        Commands.run("say $(result)");
    }
}
"#;
    compile_source(source, &CompileOptions::default()).expect("nested Optional values compile");
}

#[test]
fn safe_entity_lookup_returns_optional_reference() {
    let source = r#"class Main {
    public static void main() {
        var maybe = Selector.of("@e[type=minecraft:pig]").findFirst();
        var present = maybe.isPresent();
        var pig = maybe.orElse(Selector.of("@s").getFirst());
        if (present) {
            pig.addTag("found");
        }
    }
}
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
        "class Main {\n    public static void main() {\n        var xs = List.of(1);\n        var value = xs.get(\"x\");\n    }\n}\n",
        "class Main {\n    public static void main() {\n        var xs = List.of(1);\n        var value = xs.get(0).orElse(\"x\");\n    }\n}\n",
        "class Main {\n    static Optional<Entity> first(Selector xs) {\n        return xs.findFirst();\n    }\n}\n",
    ] {
        assert!(compile_source(source, &CompileOptions::default()).is_err());
    }
}
