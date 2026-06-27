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

## Under The Hood

Vanilla events lower to generated datapack detectors. Agent events lower to generated `agent/event/<name>.mcfunction` wrappers that copy the current agent payload from storage into the typed event parameter before calling the handler.
