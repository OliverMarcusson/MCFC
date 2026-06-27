# `player_connect`

```mcfc
event player_connect(event: agent_event):
```

Agent-backed player connect lifecycle event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: no.

```mcfc
event player_connect(event: agent_event):
    event.player.tellraw("connected")
```

