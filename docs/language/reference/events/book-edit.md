# `book_edit`

```mcfc
event book_edit(event: agent_event):
```

Agent-backed book edit event.

Fields: `player`, `player_name`, `source`, `payload`, `cancelled`. Cancellation: yes.

```mcfc
event book_edit(event: agent_event):
    event.player.tellraw("book edited")
```

