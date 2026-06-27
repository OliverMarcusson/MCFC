# `chat`

```mcfc
event chat(event: chat_event):
```

Agent-backed chat event.

Fields: `player: player_ref`, `message: string`, `cancelled: bool`. Cancellation: yes.

```mcfc
event chat(event: chat_event):
    if event.message == "spark":
        event.cancel()
        event.player.tellraw("Spark accepted")
```

