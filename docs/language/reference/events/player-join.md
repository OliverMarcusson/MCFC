# `player_join`

```mcfc
event player_join:
```

Vanilla-safe event. Runs once as every player first seen by the pack.

Payload: none. Cancellation: no.

```mcfc
event player_join:
    let player = single(selector("@s"))
    player.tellraw("Welcome")
```

