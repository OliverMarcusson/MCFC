# `player_action_toggle`

```mcfc
event player_action_toggle(event: player_action_toggle_event):
```

Agent-backed player action toggle event.

Fields: `player`, `action`, `entity_id`, `data`, `cancelled`. Cancellation: yes.

```mcfc
event player_action_toggle(event: player_action_toggle_event):
    event.player.actionbar("toggle $(event.action)")
```

