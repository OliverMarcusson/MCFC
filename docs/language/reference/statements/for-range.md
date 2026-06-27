# `for` Ranges

Iterates an integer range. `..` is exclusive; `..=` is inclusive.

```mcfc
fn count() -> void:
    for i in 0..3:
        mcf "say exclusive $(i)"

    for i in 1..=3:
        mcf "say inclusive $(i)"
```

The loop variable is local to the loop body.

