# `item_held_change`

```mcfc
event item_held_change(event: item_held_change_event):
```

Agent-backed held-item slot change event.

Fields: `player`, `slot`, `cancelled`. Cancellation: yes.

```mcfc
event item_held_change(event: item_held_change_event):
    event.player.state.last_slot = event.slot
```

