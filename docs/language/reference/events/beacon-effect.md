# `beacon_effect`

```mcfc
event beacon_effect(event: agent_event):
```

Agent-backed beacon effect selection event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: yes.

```mcfc
event beacon_effect(event: agent_event):
    event.player.tellraw("beacon changed")
```

