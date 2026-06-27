# Bukkit-style API

MCFC includes a vanilla-safe surface inspired by Bukkit and Paper. These declarations are top-level and compile to ordinary datapack functions.

```mcfc
data player.coins: int = 0

event player_join:
    let player = single(selector("@s"))
    player.send_message("Welcome!")

command status:
    let player = single(selector("@s"))
    player.send_message("coins=$(player.data.coins)")

task heartbeat every_ticks(20):
    debug("heartbeat")
```

## Supported Vanilla Declarations

- `event player_join:` runs once as every player first seen by the pack.
- `event player_death:` runs as a player when their `deathCount` score changes.
- `command name:` enables `/trigger mcfcc_name` for players.
- `task name every_ticks(n):` runs repeatedly.
- `task name after_ticks(n):` runs once after datapack load.
- `data player.name: int = 0` and `data player.name: bool = false` alias scoreboard-backed `player.state.name`.

Handlers execute as the affected player. Use `single(selector("@s"))` to get a player reference.

## Aliases

These Bukkit-inspired aliases map to existing MCFC display and sound methods:

- `send_message` -> `tellraw`
- `send_title` -> `title`
- `send_actionbar` -> `actionbar`
- `play_sound` -> `playsound`
- `stop_sound` -> `stopsound`

::: warning Vanilla limits
Vanilla event handlers do not expose synthetic event objects and are not cancellable. Real command registration, event metadata, and cancellation require the experimental agent path.
:::
