# `int`

Integer type used for arithmetic, counts, ranges, scores, and numeric event fields.

```mcfc
fn add(a: int, b: int) -> int:
    return a + b
```

`n.to_string()` gives the number as text, and `"42".parse_int()` goes the other way.

`/` rounds down and `%` takes the sign of the right side, matching Minecraft's scoreboard `/=` and `%=`. `-7 / 2` is `-4`, and `-7 % 3` is `2`. Dividing by `0` makes the scoreboard command fail, so the result is the left side unchanged.

