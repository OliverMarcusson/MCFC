# `player_respawn_request`

```mcfc
event player_respawn_request(event: agent_event):
```

Agent-backed respawn request event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: yes.

```mcfc
event player_respawn_request(event: agent_event):
    event.player.tellraw("respawn requested")
```

