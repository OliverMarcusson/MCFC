# `game_mode_request`

```mcfc
event game_mode_request(event: game_mode_request_event):
```

Agent-backed game-mode request event.

Fields: `player`, `mode`, `cancelled`. Cancellation: yes.

```mcfc
event game_mode_request(event: game_mode_request_event):
    event.player.tellraw("requested $(event.mode)")
```

