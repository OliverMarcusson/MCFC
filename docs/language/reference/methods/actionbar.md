# `actionbar`

```mcfc
entity.actionbar(message: string|text_def) -> void
```

Sends an actionbar message to the receiver.

```mcfc
fn status(player: player_ref) -> void:
    player.actionbar("Ready")
```

Alias: [`send_actionbar`](./send-actionbar).

