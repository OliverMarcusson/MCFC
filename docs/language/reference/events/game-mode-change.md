# `game_mode_change`

```mcfc
event game_mode_change(event: agent_event):
```

Agent-backed game-mode change lifecycle event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: no.

```mcfc
event game_mode_change(event: agent_event):
    event.player.tellraw("game mode changed")
```

