# `player_item_drop`

```mcfc
event player_item_drop(event: agent_event):
```

Agent-backed player item drop lifecycle event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: no.

```mcfc
event player_item_drop(event: agent_event):
    event.player.tellraw("dropped item")
```

