# `player_state`

Declares a scoreboard-backed player state value.

```mcfc
player_state quest_steps: int = "Quest Steps"

fn tick() -> void:
    let player = single(selector("@p"))
    player.state.quest_steps = player.state.quest_steps + 1
```

Supported state types are `int` and `bool`.

## Under The Hood

Player state values are scoreboard-backed. MCFC creates the objective during setup and compiles `player.state.name` reads and writes to scoreboard commands for the selected player.
