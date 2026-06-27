# `text_def`

Mutable text component builder created with `text(...)`.

```mcfc
fn tell(player: player_ref) -> void:
    let msg = text("Hello")
    msg.color = "gold"
    player.tellraw(msg)
```

