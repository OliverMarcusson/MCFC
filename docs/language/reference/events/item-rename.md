# `item_rename`

```mcfc
event item_rename(event: item_rename_event):
```

Agent-backed item rename event.

Fields: `player`, `name`, `cancelled`. Cancellation: yes.

```mcfc
event item_rename(event: item_rename_event):
    event.player.tellraw("renamed to $(event.name)")
```

