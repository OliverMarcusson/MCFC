# `event`

Declares a vanilla-safe or agent-backed event handler.

```mcfc
event player_join:
    let player = single(selector("@s"))
    player.tellraw("Welcome")

event chat(event: chat_event):
    event.player.tellraw("You said: $(event.message)")
```

See the [event catalog](../event-catalog) for every supported event.

