# `send_message`

```mcfc
entity.send_message(message: string|text_def) -> void
```

Bukkit-style alias for [`tellraw`](./tellraw).

```mcfc
fn greet(player: player_ref) -> void:
    player.send_message("Welcome")
```

