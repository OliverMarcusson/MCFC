# `inventory_open`

```mcfc
event inventory_open(event: agent_event):
```

Agent-backed inventory open lifecycle event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: no.

```mcfc
event inventory_open(event: agent_event):
    event.player.tellraw("inventory opened")
```

