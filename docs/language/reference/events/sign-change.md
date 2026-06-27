# `sign_change`

```mcfc
event sign_change(event: sign_change_event):
```

Agent-backed sign edit event.

Fields: `player`, `x`, `y`, `z`, `front`, `line_1`, `line_2`, `line_3`, `line_4`, `cancelled`. Cancellation: yes.

```mcfc
event sign_change(event: sign_change_event):
    event.player.tellraw("line 1: $(event.line_1)")
```

