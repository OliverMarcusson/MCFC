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

