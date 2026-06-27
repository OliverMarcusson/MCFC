# `player_quit`

```mcfc
event player_quit(event: agent_event):
```

Agent-backed player quit lifecycle event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: no.

```mcfc
event player_quit(event: agent_event):
    debug("player quit")
```

