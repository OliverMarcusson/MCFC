# Limitations

What MCFC doesn't do yet, with workarounds where there are any.

## Language

- **No recursion.** A function can't call itself, directly or indirectly. Use a `while` loop.
- **No implicit conversions.** Use `(float) n`, `(int) x` and `toString()`.
- **No generic records.** Only functions can be generic.
- **Imports:** no re-exports, no `*` imports and no renaming.
- **`$(...)` in `mcf`** doesn't apply imports. Write `$(util.twice(x))` with the full path.

## Runtime values

- **Strings aren't escaped** when they're joined or inserted with `$(...)`. A value containing `"` or `\` breaks the command. See [string limits](./reference/types#string).
- **`block(...)` needs a literal string.** Positions can't be computed at run time yet.
- **`findFirst`** needs a literal `selector(...)`, not a variable.
- **`hasData`** only works on storage values (lists, maps, records), not on entity NBT.
- **`heal`** only works on references known to be non-players, for example `@e[type=minecraft:pig]`. For players, use `effect("minecraft:instant_health", 1, 0)`.
- **`Selector.position`** isn't supported. Loop over the set and use each entity's `position`.
- **`orElse(x)`** always evaluates `x`, even when the Optional has a value.
- **Map keys** may only use letters, digits and `_`, and can't start with a digit.
- **Undeclared `player.state.*`** only holds `int` or `boolean`. Declare other types with `@PlayerState`.

## Statements

- `sleep`, `sleepTicks`, host calls and calls to functions that pause have to be statements of their own. See [Functions that pause](./reference/statements#functions-that-pause).

## Platform

- Output targets Minecraft 26.3 only.
- Vanilla events are limited to `player_join` and `player_death`. Other events need the [agent](/runtime/mcfd-agent).
