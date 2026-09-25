# `array<T>`

Ordered collection of values with one element type.

```mcfc
fn show(values: array<int>) -> void:
    for value in values:
        mcf "say $(value)"
```


## Methods

| Method | Returns |
| --- | --- |
| `xs.len()` | The number of elements. |
| `xs.push(value)` | Adds `value` at the end. |
| `xs.pop()` | Removes and returns the last element. |
| `xs.remove(index)` | Removes and returns the element at `index`. |
| `xs.insert(index, value)` | Puts `value` at `index`, shifting later elements up. |
| `xs.clear()` | Removes every element. |
| `xs.first()`, `xs.last()` | The first or last element. On an empty array you get the type's empty value, such as `0` for `int`. |
| `xs.contains(value)` | `true` when an element equals `value`. |
| `xs.index_of(value)` | The index of the first element equal to `value`, or `-1`. |
| `xs.reverse()` | Reverses the array in place. |

`push`, `pop`, `remove`, `insert`, `clear`, and `reverse` change the array, so they need a variable or collection element such as `teams["red"]`, not a function result.

## Under The Hood

`insert`, `clear`, `first`, and `last` are single `data` commands. `contains`, `index_of`, and `reverse` loop over the elements in a generated function that calls itself once per element. Two elements are equal when copying one onto the other changes nothing, so the check works for every element type, including structs.
