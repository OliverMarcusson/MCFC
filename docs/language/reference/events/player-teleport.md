# `player_teleport`

```mcfc
event player_teleport(event: agent_event):
```

Agent-backed player teleport lifecycle event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: no.

```mcfc
event player_teleport(event: agent_event):
    event.player.actionbar("teleported")
```

