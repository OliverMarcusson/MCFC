# `Optional<T>`

An `Optional<T>` holds a value of type `T` when present, or records that no value was found. Safe collection lookups and entity selection return it.

```mcfc
fn lookup(values: array<int>, index: int) -> Optional<int>:
    return values.get(index)

fn main() -> void:
    let maybe_count = lookup([4, 8], 3)
    if maybe_count.isPresent():
        mcf "say found"
    let count = maybe_count.orElse(0)
    mcf "say $(count)"
```

## Methods

| Method | Returns | Meaning |
| --- | --- | --- |
| `value.isPresent()` | `bool` | `true` when a value was found. |
| `value.orElse(fallback: T)` | `T` | The contained value when present, otherwise the fallback. |

The fallback must have type `T`. It is evaluated even when the Optional is present, so avoid side effects in fallback expressions. `Optional<void>` is not valid. Nested optionals can arise when looking up a collection of optional values.

Producers are [`array<T>.get(index)`](./array), [`dict<T>.get(key)`](./dict), and [`find_first(entity_set)`](../builtins/find-first).

## Under The Hood

The compiler stores an Optional as a command-storage compound with a `present` byte and, when present, a `value` field. `isPresent()` reads the flag into a scoreboard boolean. `orElse(...)` writes the fallback to the result first, then copies the stored value over it when `present` is `1`.
