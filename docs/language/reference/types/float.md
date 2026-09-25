# `float`

Decimal numbers, for distances, angles, and anything that needs square roots or trigonometry. Needs Minecraft 26.3.

```mcfc
fn distance(x: float, z: float) -> float:
    return (x * x + z * z).sqrt()

fn main() -> void:
    let d = distance(3.0, 4.0)
    if d > 4.5:
        mc "say far away"
```

A float literal needs a digit on both sides of the point: `1.0`, `0.5`. Write `-2.5` for a negative value.

## Operators

| Operator | Result |
| --- | --- |
| `+`, `-`, `*`, `/`, `%` | `float` |
| unary `-` | `float` |
| `==`, `!=`, `<`, `<=`, `>`, `>=` | `bool` |

Both sides must be `float`. `1.5 + 2` is an error; write `1.5 + float(2)` or `int(1.5) + 2`.

## Methods

| Method | Returns |
| --- | --- |
| `x.sqrt()` | Square root. |
| `x.sin()`, `x.cos()`, `x.tan()` | Trigonometry. |
| `x.abs()` | `x` without its sign. |
| `x.floor()`, `x.ceil()` | Round down or up. |
| `x.round()` | Round to the nearest whole number. |
| `x.trunc()` | Drop the fraction, rounding toward zero. |
| `x.pow(e)` | `x` raised to `e`. |
| `x.min(y)`, `x.max(y)` | The smaller or larger value. |
| `x.clamp(low, high)` | `x` limited to `low..=high`. |
| `x.hypot(y)` | `sqrt(x*x + y*y)`. |

All arguments are `float`. The trigonometry methods follow Minecraft's `/compute` providers; whether they take radians has not been checked in game yet.

## Converting

- `float(n)` turns an `int` into a `float`.
- `int(x)` turns a `float` into an `int`, rounding down: `int(2.7)` is `2`, `int(-2.7)` is `-3`.
- `float(value)` also reads an `nbt` value.

## Limits

Floats are 32-bit, like Minecraft's float NBT tag, so they hold about 7 significant digits. `x.pow(y)` stops the command when both are `0.0`.

## Under The Hood

A `float` lives in command storage. Each float expression becomes one `/compute` command, however many operators it has, so `(x * x + z * z).sqrt()` costs one command. Comparisons compute `floor(a - b)` and `floor(b - a)` and check their signs. See [lowering](../lowering).
