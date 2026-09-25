# `dict<T>`

String-keyed map of values with one value type.

```mcfc
fn show() -> void:
    let counts = {"wood": 2, "stone": 4}
    mcf "say wood=$(counts[\"wood\"])"
```

| Method | Returns | Notes |
| --- | --- | --- |
| `d.has(key)` | `bool` | |
| `d.remove(key)` | `void` | |
| `d.len()` | `int` | Number of keys |
| `d.keys()` | `array<string>` | In storage order |

`keys()` prints the dict through a macro and reads the keys out of the text, one
command chain per character. A string value anywhere in the dict that contains
`'` or `"` breaks the macro, and `keys()` then returns `[]`.

```mcfc
fn list() -> void:
    let counts = {"wood": 2, "stone": 4}
    for key in counts.keys():
        mcf "say $(key)"
```

