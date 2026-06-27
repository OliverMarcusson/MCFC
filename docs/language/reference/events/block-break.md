# `block_break`

```mcfc
event block_break(event: block_break_event):
```

Agent-backed block break event.

Fields: `player`, `x`, `y`, `z`, `cancelled`. Cancellation: yes.

```mcfc
event block_break(event: block_break_event):
    if event.player.has_tag("protected"):
        event.cancel()
```

