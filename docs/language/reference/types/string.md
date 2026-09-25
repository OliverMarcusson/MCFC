# `string`

Text type used for messages, selectors, ids, tags, and event fields.

```mcfc
fn say(message: string) -> void:
    debug(message)
```


## Joining

`+` joins strings. Convert numbers first with `to_string()`:

```mcfc
fn main() -> void:
    let name = "Steve"
    let score = 42
    let line = "Hi " + name + ", you have " + score.to_string() + " points"
```

A chain of `+` becomes one macro command, the same way `"$(name)"` interpolation does. Joining two literals, such as `"a" + "b"`, happens at compile time.

## Methods

| Method | Returns |
| --- | --- |
| `s.len()` | The number of characters. |
| `s.slice(start)` | The text from `start` to the end. |
| `s.slice(start, end)` | The text from `start` up to, not including, `end`. Negative indices count from the end, so `s.slice(-3)` is the last three characters. |
| `s.parse_int()` | The whole number in `s`, or `0` when `s` is not one. |
| `x.to_string()` | Available on `int`, `float`, and `string`. Floats print without a suffix, such as `1.5`. |

## Limits

Joined values and `to_string()` go through a Minecraft macro, so a value containing `"` or `\` breaks the generated command. `$(...)` interpolation has the same limit.

`slice` with literal indices is one `data modify ... set string` command. Indices computed at run time use a macro.
