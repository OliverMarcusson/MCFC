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
| `xs.sort()` | Sorts an `array<int>` or `array<float>` in place, smallest first. |

`push`, `pop`, `remove`, `insert`, `clear`, `reverse`, and `sort` change the array, so they need a variable or collection element such as `teams["red"]`, not a function result.

## Under The Hood

`insert`, `clear`, `first`, and `last` are single `data` commands. `contains`, `index_of`, and `reverse` loop over the elements in a generated function that calls itself once per element. Two elements are equal when copying one onto the other changes nothing, so the check works for every element type, including structs.

`sort` is a merge sort. It splits the array into already ascending stretches, then merges them two at a time, only ever reading the first element of each, so it needs no macros. An `array<float>` compares through one `/compute` command per step. Each step moves one element, and a sort does at most 1,000 steps per tick, about 10,000 commands or roughly 10 ms of a 50 ms tick. Small arrays finish right away. Bigger ones pause the function, like `sleep`, and it carries on once the array is sorted: 5,000 random elements take about 3 seconds and 10,000 about 7. Any function that calls one that sorts pauses with it, so call such functions on their own line (see [`sleep`](../builtins/sleep)).
