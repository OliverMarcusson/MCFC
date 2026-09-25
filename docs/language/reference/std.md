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
| `gcd(a: int, b: int) -> int` | The greatest common divisor, never negative. `gcd(12, -18)` is `6`. |
| `lerp(a: float, b: float, t: float) -> float` | The point `t` of the way from `a` to `b`. `0.0` gives `a` and `1.0` gives `b`. |

Integer arithmetic is 32-bit scoreboard math, so results wrap on overflow.

## `std::array`

These take `array<int>`. Other element types wait for generics.

| Function | Returns |
| --- | --- |
| `sum(xs: array<int>) -> int` | The total of all elements. |
| `min(xs: array<int>) -> int` | The smallest element, or `0` for an empty array. |
| `max(xs: array<int>) -> int` | The largest element, or `0` for an empty array. |
| `sort(xs: array<int>) -> array<int>` | A sorted copy, smallest first. |

`sort` returns a new array, so write `xs = sort(xs)`. It is a quicksort with a median-of-three pivot that stops splitting at 16 elements, followed by one insertion sort pass over the whole array. That is the same approach as Rust's `sort_unstable`, and equal elements may change places. Every comparison and swap reads or writes storage, so sorting 1,000 elements takes tens of thousands of commands; sort large arrays across several ticks.

## `std::str`

| Function | Returns |
| --- | --- |
| `starts_with(s: string, prefix: string) -> bool` | `true` when `s` begins with `prefix`. |
| `ends_with(s: string, suffix: string) -> bool` | `true` when `s` ends with `suffix`. |
| `find(s: string, needle: string) -> int` | The index of the first `needle` in `s`, or `-1`. |
| `contains(s: string, needle: string) -> bool` | `true` when `needle` appears in `s`. |

These compare slices of `s`, so `find` and `contains` cost a few commands per character. They are fine for names, ids, and short messages. Unlike joining, they never paste the text into a command, so `"` and `\` are safe.

## Under The Hood

`std` is ordinary MCFC source compiled into the `mcfc` binary. Its functions lower like any other module function, for example `generated/std__math__clamp__d0__entry`. Unlike your own zero-argument `void` functions, `std` functions never get public `/function` wrappers.

`pow` runs a `while` loop, so its cost grows with `exponent`.
