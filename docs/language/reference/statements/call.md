# Function Call Statements

A bare function call may be used as a statement when only the side effect matters.

```mcfc
fn greet(player: player_ref) -> void:
    player.tellraw("hello")

fn tick() -> void:
    greet(single(selector("@p")))
```

Only function calls may be used as bare expression statements. Plain expressions such as `amount + 1` are rejected as statements.

