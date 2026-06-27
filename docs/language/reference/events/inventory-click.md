# `inventory_click`

```mcfc
event inventory_click(event: inventory_click_event):
```

Agent-backed inventory click event.

Fields: `player`, `container_id`, `state_id`, `slot`, `button`, `cancelled`. Cancellation: yes.

```mcfc
event inventory_click(event: inventory_click_event):
    event.player.state.last_slot = event.slot
    event.player.actionbar("slot=$(event.slot)")
```

