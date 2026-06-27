# `for` Arrays

Iterates each value in an array.

```mcfc
fn show(values: array<int>) -> void:
    for value in values:
        mcf "say $(value)"
```

The loop variable type is the array element type.

## Under The Hood

Array loops keep the array in command storage and use a scoreboard index. Each iteration copies the current storage element into the loop variable slot before calling the body.
