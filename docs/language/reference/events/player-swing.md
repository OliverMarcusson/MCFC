# `player_swing`

```mcfc
event player_swing(event: player_swing_event):
```

Agent-backed swing event.

Fields: `player`, `hand`, `cancelled`. Cancellation: yes. Since 26.3 the client only sends main-hand punches, so `hand` is always `"MAIN_HAND"`.

```mcfc
event player_swing(event: player_swing_event):
    event.player.actionbar("swing $(event.hand)")
```

