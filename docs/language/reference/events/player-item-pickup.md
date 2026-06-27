# `player_item_pickup`

```mcfc
event player_item_pickup(event: agent_event):
```

Agent-backed player item pickup lifecycle event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: no.

```mcfc
event player_item_pickup(event: agent_event):
    event.player.tellraw("picked up item")
```

