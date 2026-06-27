# `trade_select`

```mcfc
event trade_select(event: trade_select_event):
```

Agent-backed villager trade selection event.

Fields: `player`, `trade_index`, `cancelled`. Cancellation: yes.

```mcfc
event trade_select(event: trade_select_event):
    event.player.actionbar("trade $(event.trade_index)")
```

