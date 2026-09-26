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
| `d.get(key)` | `Optional<T>` | Returns absent when the `string` key is missing. |
| `d.remove(key)` | `void` | |
| `d.len()` | `int` | Number of keys |
| `d.keys()` | `array<string>` | In storage order |

```mcfc
fn main() -> void:
    let counts = {"wood": 2}
    let count = counts.get("stone").orElse(0)
    mcf "say $(count)"
```

See [`Optional<T>`](./optional) for presence and fallback methods.

## Under The Hood

`get` checks the dictionary's command-storage path and copies an existing value into an Optional storage compound. A dynamic key is quoted in a generated command macro. Dictionary keys must follow MCFC's storage-path-safe rule: letters, digits, and `_`, with a non-digit first character.

`keys()` prints the dict through a macro and reads the keys out of the text, one
command chain per character. A string value anywhere in the dict that contains
`'` or `"` breaks the macro, and `keys()` then returns `[]`.

```mcfc
fn list() -> void:
    let counts = {"wood": 2, "stone": 4}
    for key in counts.keys():
        mcf "say $(key)"
```

