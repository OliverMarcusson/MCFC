# `for` Arrays

Iterates each value in an array.

```mcfc
fn show(values: array<int>) -> void:
    for value in values:
        mcf "say $(value)"
```

The loop variable type is the array element type.

