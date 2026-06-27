# `inventory_close`

```mcfc
event inventory_close(event: inventory_close_event):
```

Agent-backed inventory close event.

Fields: `player`, `container_id`, `cancelled`. Cancellation: yes.

```mcfc
event inventory_close(event: inventory_close_event):
    event.player.tellraw("closed $(event.container_id)")
```

