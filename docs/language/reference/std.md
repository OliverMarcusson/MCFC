# Standard Library: `std`

Every file and project build can use the `std` module without declaring it. Import from it with [`import`](./statements#import) or call it by path:

```mcfc
import std.math.clamp;

void main() {
    var hp = clamp(150, 0, 100);
    var bits = std.math.pow(2, 10);
}
```

Only the `std` functions a pack calls are compiled into it. The name `std` is reserved, so a `src/std.mcf` file is an error.

## `std.math`

| Function | Returns |
| --- | --- |
| `int min(int a, int b)` | The smaller of `a` and `b`. |
| `int max(int a, int b)` | The larger of `a` and `b`. |
| `int abs(int x)` | `x` without its sign. |
| `int sign(int x)` | `1`, `0`, or `-1`. |
| `int clamp(int x, int low, int high)` | `x` limited to `low` through `high`. |
| `int rem(int a, int b)` | The remainder of `a / b`, with the sign of `b`. For example, `rem(-7, 3)` is `2`. Same as `a % b`. |
| `int pow(int base, int exponent)` | `base` multiplied by itself `exponent` times. Negative exponents return `0`. |
| `int gcd(int a, int b)` | The greatest common divisor, never negative. `gcd(12, -18)` is `6`. |
| `float lerp(float a, float b, float t)` | The point `t` of the way from `a` to `b`. `0.0` gives `a` and `1.0` gives `b`. |

Integer arithmetic is 32-bit scoreboard math, so results wrap on overflow.

## `std.list`

These are [generic](./statements#generic-functions), so they work on `List<Integer>` and `List<Float>`.

| Function | Returns |
| --- | --- |
| `<T> T sum(List<T> xs)` | The total of all elements, or `0` for an empty list. |
| `<T> T min(List<T> xs)` | The smallest element, or `0` for an empty list. |
| `<T> T max(List<T> xs)` | The largest element, or `0` for an empty list. |

To sort, use the built-in [`xs.sort()`](./types#list) method.

## `std.str`

| Function | Returns |
| --- | --- |
| `boolean startsWith(String s, String prefix)` | `true` when `s` begins with `prefix`. |
| `boolean endsWith(String s, String suffix)` | `true` when `s` ends with `suffix`. |
| `int find(String s, String needle)` | The index of the first `needle` in `s`, or `-1`. |
| `boolean contains(String s, String needle)` | `true` when `needle` appears in `s`. |

The `String` methods `startsWith`, `endsWith`, `indexOf` and `contains` call these helpers. Import `std.str` functions only when you need the free-function form. They compare substrings of `s`, so `find` and `contains` cost a few commands per character. Unlike joining, they never paste the text into a command, so `"` and `\` are safe.

## Under The Hood

`std` is ordinary MCFC source compiled into the `mcfc` binary. Its functions lower like any other module function, for example `generated/std__math__clamp__d0__entry`. Unlike your own zero-argument `void` functions, `std` functions never get public `/function` wrappers.

`pow` runs a `while` loop, so its cost grows with `exponent`.
