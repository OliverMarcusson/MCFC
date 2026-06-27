# `entity_teleport`

```mcfc
event entity_teleport(event: agent_event):
```

Agent-backed entity teleport event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: yes.

```mcfc
event entity_teleport(event: agent_event):
    event.player.tellraw("entity teleport payload=$(event.payload)")
```

