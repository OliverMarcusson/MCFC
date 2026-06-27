# `player_interact_block`

```mcfc
event player_interact_block(event: player_interact_block_event):
```

Agent-backed block interaction event.

Fields: `player`, `hand`, `face`, `x`, `y`, `z`, `cancelled`. Cancellation: yes.

```mcfc
event player_interact_block(event: player_interact_block_event):
    event.player.actionbar("$(event.hand) $(event.face)")
```

