# `player_interact_item`

```mcfc
event player_interact_item(event: player_interact_item_event):
```

Agent-backed item interaction event.

Fields: `player`, `hand`, `cancelled`. Cancellation: yes.

```mcfc
event player_interact_item(event: player_interact_item_event):
    event.player.actionbar("used $(event.hand)")
```

