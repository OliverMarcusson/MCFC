# `item_pick`

```mcfc
event item_pick(event: agent_event):
```

Agent-backed creative item-pick event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: yes.

```mcfc
event item_pick(event: agent_event):
    event.player.tellraw("item picked")
```

