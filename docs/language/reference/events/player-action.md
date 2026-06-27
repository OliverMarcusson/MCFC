# `player_action`

```mcfc
event player_action(event: player_action_event):
```

Agent-backed player action packet event.

Fields: `player`, `action`, `face`, `x`, `y`, `z`, `cancelled`. Cancellation: yes.

```mcfc
event player_action(event: player_action_event):
    event.player.actionbar("action=$(event.action)")
```

