# `while`

Repeats a block while a `bool` expression remains true.

```mcfc
fn count() -> void:
    let i = 0
    while i < 3:
        mcf "say $(i)"
        i = i + 1
```

Use `break` to exit early and `continue` to skip to the next iteration.

