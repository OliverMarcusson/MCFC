# `player_death`

```mcfc
event player_death:
```

Vanilla-safe event. Runs as a player when their `deathCount` score changes.

Payload: none. Cancellation: no.

```mcfc
event player_death:
    let player = single(selector("@s"))
    player.state.deaths = player.state.deaths + 1
```

