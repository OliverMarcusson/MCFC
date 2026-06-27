# `player_swing`

```mcfc
event player_swing(event: player_swing_event):
```

Agent-backed swing event.

Fields: `player`, `hand`, `cancelled`. Cancellation: yes.

```mcfc
event player_swing(event: player_swing_event):
    event.player.actionbar("swing $(event.hand)")
```

