# `remove_tag`

```mcfc
entity.remove_tag(name: string) -> void
```

Removes a scoreboard tag from the receiver.

```mcfc
fn unmark(player: player_ref) -> void:
    player.remove_tag("quest_ready")
```

