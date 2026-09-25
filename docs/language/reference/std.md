# Standard Library: `std`

Every file and project build can use the `std` module without declaring it. Import from it with [`use`](./statements/use) or call it by path:

```mcfc
use std::math::clamp

fn main() -> void:
    let hp = clamp(150, 0, 100)
    let bits = std::math::pow(2, 10)
```

Only the `std` functions a pack calls are compiled into it. The name `std` is reserved, so `mod std` in `main.mcf` is an error.

## `std::math`

| Function | Returns |
| --- | --- |
| `min(a: int, b: int) -> int` | The smaller of `a` and `b`. |
| `max(a: int, b: int) -> int` | The larger of `a` and `b`. |
| `abs(x: int) -> int` | `x` without its sign. |
| `sign(x: int) -> int` | `1`, `0`, or `-1`. |
| `clamp(x: int, low: int, high: int) -> int` | `x` limited to `low..=high`. |
| `rem(a: int, b: int) -> int` | The remainder of `a / b`, with the sign of `b`. For example, `rem(-7, 3)` is `2`. Same as `a % b`. |
| `pow(base: int, exponent: int) -> int` | `base` multiplied by itself `exponent` times. Negative exponents return `0`. |

All arithmetic is 32-bit scoreboard math, so results wrap on overflow.

## Under The Hood

`std` is ordinary MCFC source compiled into the `mcfc` binary. Its functions lower like any other module function, for example `generated/std__math__clamp__d0__entry`. Unlike your own zero-argument `void` functions, `std` functions never get public `/function` wrappers.

`pow` runs a `while` loop, so its cost grows with `exponent`.
