# `tellraw`

```mcfc
entity.tellraw(message: string|text_def) -> void
```

Sends chat text to the receiver.

```mcfc
fn greet(player: player_ref) -> void:
    let msg = text("Hello")
    msg.color = "green"
    player.tellraw(msg)
```

Alias: [`send_message`](./send-message).

