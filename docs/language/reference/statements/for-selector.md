# `for` Selectors

Iterates each entity in an `entity_set`.

```mcfc
fn tag_all() -> void:
    for player in selector("@a"):
        player.add_tag("seen")
```

The loop variable behaves like a single entity reference for the body.

