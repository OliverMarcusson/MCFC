# `text`

```mcfc
text() -> text_def
text(value: string) -> text_def
```

Creates a text component builder.

```mcfc
fn announce(player: player_ref) -> void:
    let msg = text("Quest complete")
    msg.color = "gold"
    msg.bold = true
    player.tellraw(msg)
```

