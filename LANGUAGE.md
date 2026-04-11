# MCFC Language Guide

`mcfc` is a small statically typed language that compiles to a Minecraft datapack for Minecraft `26.1.2`.

The compiler is currently focused on a compact core language:

- functions
- optional `@book` function exposure
- typed parameters and return values
- local variables with inferred types
- integer arithmetic
- boolean conditions
- `if`, `else`, `while`, and range `for`
- function calls
- raw Minecraft commands
- storage-backed strings
- storage-backed arrays and dictionaries

## CLI Usage

Build a source file into a datapack directory:

```powershell
cargo run -- build example.mcf --out build\pack
```

Available flags:

- `--namespace <name>`: override the generated namespace. Default: `mcfc`
- `--emit-ast`: write a debug dump of the typed program to `debug/typed_program.txt`
- `--emit-ir`: write a debug dump of the lowered IR to `debug/ir.txt`
- `--clean`: remove the output directory before writing new output

The compiler writes:

- `pack.mcmeta`
- `data/<namespace>/function/main.mcfunction`
- generated functions under `data/<namespace>/function/generated/`

`main.mcfunction` runs setup and then calls the generated `main` entrypoint if a `main` function exists.

## Example Program

```text
fn add(x: int, y: int) -> int
    return x + y
end

fn main() -> void
    let a = 1
    let b = 2
    let label = "done"

    while a < 4:
        if a == 2:
            b = add(a, b)
        end
        a = a + 1
    end

    mc "say done"
    return
end
```

## Syntax

### Functions

Functions begin with `fn` and end with `end`.

```text
fn name(param: type, other: type) -> return_type
    ...
end
```

Rules:

- parameter types are required
- return types are required
- duplicate function names are rejected
- duplicate parameter names are rejected
- `#` starts a line comment and may also appear after code on a line

Book-exposed functions may be annotated with `@book`:

```text
@book
fn fibb(n: int) -> void
    ...
end
```

Current `@book` restrictions:

- the function must return `void`
- all parameters must be `int`
- the function name becomes the book command name

### Statements

Supported statements:

- `struct Name: field: type ... end`
- `let name = expr`
- `name = expr`
- `if condition: ... end`
- `if condition: ... else: ... end`
- `match value: "a" => stmt else => stmt end`
- `while condition: ... end`
- `for name in start..end: ... end`
- `for name in start..=end: ... end`
- `for name in selector_expr: ... end`
- `for name in array_expr: ... end`
- `break`
- `continue`
- `return`
- `return expr`
- `mc "raw minecraft command"`
- `mcf "macro command with $(placeholders)"`
- `as(entity): ... end`
- `at(entity): ... end`
- a bare function call as a statement, for example `do_work()`

Notes:

- only function calls may be used as bare expression statements
- `if`, `while`, and `for` bodies are block-scoped for new `let` bindings
- `for` loop variables are loop-local and do not exist after the loop ends
- `mc` is literal-only; `mcf` is the runtime interpolation form

### Expressions

Supported expressions:

- integer literals, for example `42`
- boolean literals: `true`, `false`
- string literals, for example `"hello"` or `'hello'`
- variables
- function calls
- path access, for example `pig.CustomName` or `pig.HandItems[0]`
- array literals, for example `[1, 2, 3]`
- dictionary literals, for example `{"wood": 12, "stone": 4}`
- collection indexing, for example `values[i]` or `counts["wood"]`
- unary `not`
- binary operators

Binary operators:

- arithmetic: `+`, `-`, `*`, `/`
- logical: `and`, `or`
- comparison: `==`, `!=`, `<`, `<=`, `>`, `>=`

Precedence:

1. `not`
2. `*`, `/`
3. `+`, `-`
4. comparisons
5. `and`
6. `or`

Parentheses may be used to group expressions.

## Types

Built-in types:

- `int`
- `bool`
- `string`
- `array<T>`
- `dict<T>`
- `entity_set`
- `entity_ref`
- `block_ref`
- `nbt`
- `void`
- named `struct` types

Type rules:

- locals infer their type from the initializer
- assignments must keep the original variable type
- function arguments must match declared parameter types
- return expressions must match the declared return type
- there is no implicit conversion between types

Current operator support:

- arithmetic requires `int`
- `and`, `or`, and `not` require `bool`
- ordering comparisons currently support `int` and `bool`
- string equality supports only `==` and `!=`

Minecraft query builtins:

- `selector("...") -> entity_set`
- `single(entity_set) -> entity_ref`
- `exists(entity_ref) -> bool`
- `has_data(storage_path) -> bool`
- `block("...") -> block_ref`
- `at(entity_ref, entity_set|entity_ref|block_ref)`
- `as(entity_set|entity_ref, entity_set|entity_ref|block_ref)`
- `int(nbt)`, `bool(nbt)`, `string(nbt)`

Gameplay builtins:

- `summon(entity_id: string) -> entity_ref`
- `summon(entity_id: string, data: nbt) -> entity_ref`
- `teleport(target: entity_ref|entity_set, destination: entity_ref|block_ref) -> void`
- `damage(target: entity_ref|entity_set, amount: int) -> void`
- `heal(target: entity_ref, amount: int) -> void`
- `give(target: entity_ref|entity_set, item_id: string, count: int) -> void`
- `clear(target: entity_ref|entity_set, item_id: string, count: int) -> void`
- `loot_give(target: entity_ref|entity_set, table: string) -> void`
- `loot_insert(container: block_ref, table: string) -> void`
- `loot_spawn(position: block_ref, table: string) -> void`
- `tellraw(target: entity_ref|entity_set, message: string) -> void`
- `title(target: entity_ref|entity_set, message: string) -> void`
- `actionbar(target: entity_ref|entity_set, message: string) -> void`
- `debug(message: string) -> void`
- `debug_marker(position: block_ref, label: string) -> void`
- `debug_marker(position: block_ref, label: string, marker_block: string) -> void`
- `debug_entity(target: entity_ref|entity_set, label: string) -> void`
- `sleep(seconds: int) -> void`
- `random() -> int`
- `random(max: int) -> int`
- `random(min: int, max: int) -> int`
- `bossbar_add`, `bossbar_remove`, `bossbar_name`, `bossbar_value`, `bossbar_max`, `bossbar_visible`, `bossbar_players`
- `playsound(sound: string, category: string, target: entity_ref|entity_set) -> void`
- `stopsound(target: entity_ref|entity_set, category: string, sound: string) -> void`
- `particle(name: string, position: block_ref[, count: int[, viewers: entity_ref|entity_set]]) -> void`
- `setblock(position: block_ref, block_id: string) -> void`
- `fill(from: block_ref, to: block_ref, block_id: string) -> void`

Collection methods:

- `array<T>.len() -> int`
- `array<T>.push(value: T) -> void`
- `array<T>.pop() -> T`
- `array<T>.remove_at(index: int) -> T`
- `dict<T>.has(key: string) -> bool`
- `dict<T>.remove(key: string) -> void`

Collection notes:

- arrays are backed by Minecraft storage lists
- array `for-each` iterates over a snapshot of the source array
- dictionaries are backed by Minecraft storage compounds
- dictionary keys are strings and use bracket syntax: `counts["wood"]` or `counts[key]`
- dictionary key values must be storage-path-safe: letters, digits, and `_`, with a non-digit first character
- empty collection literals currently require type context; otherwise they are rejected

Struct notes:

- structs are named storage-backed compounds
- define them at top level with `struct Name: ... end`
- construct them with `Name{field: value, ...}`
- access fields with existing path syntax, for example `action.duration`

Player-safe surfaces:

- `player.nbt.*` reads vanilla player NBT
- `player.state.*` stores MCFC-managed integer and boolean player state
- `player.tags.*` reads and writes entity tags as booleans
- `entity.add_tag("name")`, `entity.remove_tag("name")`, and `entity.has_tag("name")` work on any `entity_ref`
- `entity.team = "name"` assigns a team for any `entity_ref`
- `entity.mainhand.*`, `entity.offhand.*`, `entity.head.*`, `entity.chest.*`, `entity.legs.*`, and `entity.feet.*` modify equipped items
- `entity.effect("name", duration, amplifier)` applies an effect for any `entity_ref`
- `heal(...)` is a synthetic helper in v1 and only accepts known non-player `entity_ref` targets

Debugging helpers:

- `debug(message)` prints a gold `[MCFC debug]` chat line to all players
- `debug_marker(position, label)` prints a marker line, emits a visible particle burst, and plays a note-block ping at the position
- `debug_marker(position, label, marker_block)` also places `marker_block` at the position, which is intentionally destructive and should be used only for temporary checks
- `debug_entity(target, label)` reports whether the selector/entity resolves and briefly gives matching entities the glowing effect
- relative block positions such as `block("~ ~ ~")` follow the current execution position; use `at(player):` for player-position world effects, while `as(player):` only changes the executing entity

Timing and randomness:

- `sleep(seconds)` pauses the current MCFC execution path and resumes the following statements later with Minecraft `schedule function`
- `sleep(...)` is statement-only; it cannot be used as a value in `let`, `return`, function arguments, or macro placeholders
- `random()` returns an `int` from `0..2147483647`
- `random(max)` returns an `int` from `0..max`
- `random(min, max)` returns an `int` from `min..max`
- random bounds are inclusive because they map directly to Minecraft `random value` ranges
- string literals support `$(expr)` interpolation with the same expression rules as `mcf` placeholders

```text
fn main() -> void
    let roll = random(1, 20)
    let demo_title = "MCFC Demo $(random(100))"
    if roll == 20:
        debug("critical")
    end

    sleep(3)
    debug(demo_title)
end
```

String literal notes:

- both double-quoted and single-quoted strings are supported
- use `\"` inside `"..."`
- use `\'` inside `'...'`

## Scope and Name Rules

Variables are function-local.

Important behavior:

- parameters are available throughout the function
- `let` introduces a new binding
- reusing an existing local or parameter name with `let` is rejected
- `let` bindings created inside an `if`, `while`, or `for` body do not become visible outside that block
- `for` loop variables follow the same block-scoping rule

Example:

```text
fn main() -> void
    if true:
        let x = 1
    end

    x = 2
end
```

This fails because `x` does not exist outside the `if` block.

## Control Flow

### `if`

`if` conditions must have type `bool`.

```text
if a == 5:
    mc "say five"
end
```

### `while`

`while` conditions must have type `bool`.

```text
while counter < 10:
    counter = counter + 1
end
```

The compiler lowers loops into generated datapack functions. It does not try to protect you from infinite loops, so loop termination is your responsibility.

### `for`

`for` loops are range-based and require `int` bounds.

```text
for i in 0..10:
    mc "say half open"
end

for i in 0..=10:
    mc "say inclusive"
end
```

Rules:

- `start..end` is half-open and iterates while `i < end`
- `start..=end` is inclusive and iterates while `i <= end`
- range bounds are evaluated once before the loop starts
- the loop variable is local to the loop body

### `break` and `continue`

`break` exits the nearest enclosing loop.

`continue` skips the rest of the current iteration and starts the next one.

Both statements are only valid inside `while` and `for`.

## Functions and Calls

Function calls may appear:

- in expressions
- on the right-hand side of assignments
- in returns
- as standalone call statements

Example:

```text
fn inc(x: int) -> int
    return x + 1
end

fn main() -> void
    let a = 1
    a = inc(a)
    return
end
```

Current call model:

- nested non-recursive calls are supported
- recursion is rejected at compile time
- unknown functions are rejected
- wrong arity is rejected

The backend uses depth-indexed frame slots so nested calls do not overwrite caller state.

## Raw Minecraft Commands

Use `mc "..."` to emit raw commands directly:

```text
mc "say hello"
mc "scoreboard players set @s health 20"
```

The string is copied directly into the generated `.mcfunction` output.

## Macro Commands

Use `mcf "..."` for runtime interpolation through Minecraft function macros.

```text
let amount = 5
mcf "xp add @a $(amount + 1) levels"
```

Rules:

- placeholders use `$(expr)` syntax
- placeholders may use the current MCFC expression language, including paths, indexing, arithmetic, comparisons, logical operators, casts, function calls, and method calls
- placeholders are type-checked using the same rules as ordinary expressions
- supported interpolated result types are `int`, `bool`, `string`, `nbt`, `entity_set`, `entity_ref`, `block_ref`, `array`, `dict`, and `struct`
- malformed placeholders are rejected during compilation

Implementation model:

- the compiler generates a macro `.mcfunction` containing a `$`-prefixed command
- it writes placeholder values into storage
- it invokes the macro with `function ... with storage ...`

`mc` and `mcf` are intentionally different:

- `mc "say $(a)"` emits the literal text `say $(a)`
- `mcf "say $(a)"` performs runtime substitution through Minecraft macros

## Execution Context Blocks

Use `as(anchor): ... end` to run one or more generated commands as an entity set or entity ref.
Use `at(anchor): ... end` to run one or more generated commands at an entity set or entity ref.

```text
let player = single(selector("@p"))

as(player):
    mcf 'tellraw @s "welcome @s"'
    mc "say another command as the same entity"
end

at(player):
    mc "particle minecraft:happy_villager ~ ~1 ~"
end
```

The value-level forms `as(anchor, value)` and `at(anchor, value)` compose execution context onto entity and block reference values. Context blocks are better when several commands should run under the same `execute as` or `execute at` wrapper.

## Runtime Representation

The current backend maps values like this:

- `int`: scoreboard-backed
- `bool`: scoreboard-backed using `0` and `1`
- `string`: Minecraft data storage-backed
- `array<T>`: Minecraft data storage-backed
- `dict<T>`: Minecraft data storage-backed

Generated files are deterministic and use a reserved generated namespace layout.

## Book Commands

The compiler can generate a writable-book command runtime for functions marked with `@book`.

Current runtime behavior:

- players are polled each tick
- a managed writable book is given to players if they do not already have one
- when the managed book is in the player's main hand, the runtime reads page 1
- if the command text changed since the last processed edit, it dispatches the command once

Current command format:

- page 1, first command text only
- command grammar: `<name> <int> <int> ...`
- example: `fibb 8`

Current dispatch behavior:

- only `@book` functions are callable
- dispatch runs as the player holding the book
- bad command names or bad integer arguments produce a `tellraw` error

This is implemented as tick-based polling, which approximates “on close” as “on the next tick after the book contents changed.”

## Diagnostics

The compiler reports line and column information for parse and type errors.

Common errors include:

- undefined variables
- duplicate function names
- duplicate parameter names
- duplicate local names
- wrong argument count
- wrong argument type
- invalid return type
- non-boolean `if`/`while` conditions
- non-boolean logical operands
- invalid `for` range bounds
- `break` or `continue` outside a loop
- unsupported recursion

## Current Limitations

This is the current implemented language, not the final design.

Not supported yet:

- recursion
- implicit conversions
- `@book` string arguments
- `@book` function calls with non-`int` parameters or non-`void` return types
- rich book parsing such as `fibb(8)`, quoted arguments, or multiple commands
- arbitrary object/struct types beyond arrays, dictionaries, and raw `nbt`
- modules/imports
- optimization passes

Notes:

- `match` currently supports only `string` scrutinees
- each `match` arm currently contains exactly one statement; use helper functions if an arm needs more work

The backend is intentionally simple and prioritizes correctness, inspectable output, and deterministic code generation over optimization.
