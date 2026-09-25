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

Joining, `to_string()`, and `$(...)` interpolation go through a Minecraft macro, which pastes each value into a quoted string without escaping it. Vanilla has no command that escapes a string, so:

- A value containing `"` makes the macro line invalid, and the result is `""`.
- A value containing `\` is read as an escape, so `\n` becomes a newline.

Player names and ids never contain these characters. Text players type, such as item names or chat, can.

`slice` also gives `""` when an index is out of range.

`slice` with literal indices is one `data modify ... set string` command. Indices computed at run time use a macro.
