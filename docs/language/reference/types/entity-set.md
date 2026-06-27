# `entity_set`

Selector result that may contain multiple entities.

```mcfc
fn tick() -> void:
    let players = selector("@a")
    for player in players:
        player.add_tag("seen")
```

