# `float`

```mcfc
float(value: int) -> float
float(value: nbt) -> float
```

Converts an `int` or an NBT value to a [`float`](../types/float).

```mcfc
fn half(n: int) -> float:
    return float(n) / 2.0
```
