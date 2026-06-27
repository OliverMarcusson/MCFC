# `player_respawn`

```mcfc
event player_respawn(event: agent_event):
```

Agent-backed player respawn lifecycle event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: no.

```mcfc
event player_respawn(event: agent_event):
    event.player.tellraw("respawned")
```

