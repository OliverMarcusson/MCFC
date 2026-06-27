# `player_damage`

```mcfc
event player_damage(event: agent_event):
```

Agent-backed player damage lifecycle event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: no.

```mcfc
event player_damage(event: agent_event):
    event.player.tellraw("damage payload=$(event.payload)")
```

