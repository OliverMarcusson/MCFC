# `find_first`

```mcfc
find_first(value: entity_set) -> Optional<entity_ref>
```

Looks for one matching entity and returns a present `Optional<entity_ref>` when one exists.

```mcfc
fn tick() -> void:
    let maybe_pig = find_first(selector("@e[type=minecraft:pig]"))
    if maybe_pig.isPresent():
        let pig = maybe_pig.orElse(single(selector("@s")))
        pig.add_tag("found")
```

The argument must be a direct `selector(...)` expression, optionally wrapped in `at(...)` or `as(...)`. A selector without a limit is narrowed to `limit=1`; selectors with another limit are rejected. MCFC rejects an `entity_set` variable because its selector cannot be narrowed at compile time. See [`Optional<T>`](../types/optional) for handling an absent result.

## Under The Hood

MCFC tests the narrowed selector with `execute if entity`. It stores a presence flag and copies the selector reference into the Optional value only when a match exists. The reference is resolved by Minecraft when used, so this check does not snapshot the entity.
