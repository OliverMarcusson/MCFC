# `for` Selectors

Iterates each entity in an `entity_set`.

```mcfc
fn tag_all() -> void:
    for player in selector("@a"):
        player.add_tag("seen")
```

The loop variable behaves like a single entity reference for the body.

## Under The Hood

Selector loops lower to `execute as <selector> run function ...`. Inside the loop body, the loop variable is represented by the current executor selector, so methods on it naturally target `@s` under the generated context.
