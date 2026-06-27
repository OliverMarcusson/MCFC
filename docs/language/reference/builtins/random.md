# `random`

```mcfc
random() -> int
random(max: int) -> int
random(min: int, max: int) -> int
```

Returns a random integer.

```mcfc
fn roll() -> void:
    let value = random(1, 6)
    mcf "say rolled $(value)"
```

## Under The Hood

`random` lowers to Minecraft scoreboard randomization commands where possible, with the result stored in the generated scoreboard slot for the returned `int`.
