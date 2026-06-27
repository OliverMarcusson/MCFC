# `entity_interact`

```mcfc
event entity_interact(event: entity_interact_event):
```

Agent-backed entity interaction event.

Fields: `player`, `target_id`, `hand`, `secondary`, `cancelled`. Cancellation: yes.

```mcfc
event entity_interact(event: entity_interact_event):
    if event.secondary:
        event.player.tellraw("secondary interact")
```

