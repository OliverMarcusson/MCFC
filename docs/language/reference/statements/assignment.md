# Assignment

Assigns a new value to an existing local or writable path.

```mcfc
fn tick() -> void:
    let amount = 1
    amount = amount + 1

    let player = single(selector("@p"))
    player.state.score = amount
```

Assignments must preserve the original type of the target.

