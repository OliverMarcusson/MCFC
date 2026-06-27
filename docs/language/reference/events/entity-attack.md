# `entity_attack`

```mcfc
event entity_attack(event: entity_attack_event):
```

Agent-backed entity attack event.

Fields: `player`, `target_id`, `cancelled`. Cancellation: yes.

```mcfc
event entity_attack(event: entity_attack_event):
    event.player.state.hits = event.player.state.hits + 1
```

