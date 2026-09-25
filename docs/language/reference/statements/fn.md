# `fn`

Declares a function with typed parameters and an explicit return type.

```mcfc
fn greet(player: player_ref, message: string) -> void:
    player.tellraw(message)
```

`fn tick() -> void:` is special: it maps to the datapack tick function and runs every game tick.

## Generic Functions

Type parameters go in `<...>` after the name. A call infers them from its arguments:

```mcfc
fn biggest<T>(values: array<T>) -> T:
    let best = values[0]
    for value in values:
        if value > best:
            best = value
    return best

fn main() -> void:
    let a = biggest([3, 9, 2])
    let b = biggest([1.5, 0.25])
```

Every type parameter must appear in a parameter's type. There are no bounds: each set of types a call uses compiles its own copy, such as `biggest__int` and `biggest__float`, and that copy is type-checked on its own, so `biggest(["a", "b"])` reports that `>` needs numbers, plus "'biggest' does not work with T = string" at the call. Structs cannot be generic.

## Under The Hood

Each function lowers to a generated `.mcfunction` body plus, when exported, a public wrapper under `data/<namespace>/function/`. The wrapper resets the generated control slot before calling the lowered body. `tick` is wired into the datapack tick entrypoint.
