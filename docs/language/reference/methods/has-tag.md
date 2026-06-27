# `has_tag`

```mcfc
entity.has_tag(name: string) -> bool
```

Tests whether the receiver has a scoreboard tag.

```mcfc
fn check(player: player_ref) -> void:
    if player.has_tag("quest_ready"):
        player.tellraw("Ready")
```

