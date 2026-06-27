# `player_abilities`

```mcfc
event player_abilities(event: agent_event):
```

Agent-backed player abilities event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: yes.

```mcfc
event player_abilities(event: agent_event):
    event.player.tellraw("abilities changed")
```

