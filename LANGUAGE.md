# MCFC Language Guide

`mcfc` is a statically typed language that compiles to a Minecraft datapack for Minecraft `26.1.2`.

The current language focuses on a compact core:

- functions and typed locals
- integer, boolean, string, array, dictionary, struct, entity, block, bossbar, and NBT values
- `if`, `match`, `while`, range `for`, and selector `for`
- `as(...)` and `at(...)` context composition and blocks
- raw Minecraft commands with `mc` and macro commands with `mcf`
- non-blocking `async:` blocks
- object-style gameplay builtins such as `player.tellraw(...)` and `pos.setblock(...)`

## CLI Usage

Build a source file into a datapack directory:

```powershell
cargo run -- build example.mcf --out build\pack
```

Available flags:

- `--namespace <name>`: override the generated namespace. Default: `mcfc`
- `--emit-ast`: write a typed-program dump to `debug/typed_program.txt`
- `--emit-ir`: write a lowered IR dump to `debug/ir.txt`
- `--clean`: remove the output directory before writing new output

The compiler writes:

- `pack.mcmeta`
- `data/<namespace>/function/main.mcfunction`
- generated helper functions under `data/<namespace>/function/generated/`

`main.mcfunction` runs setup and then calls the generated `main` entrypoint if a `main` function exists.

## Example Program

```text
fn main() -> void
    let player = single(selector("@p"))
    let bb = bossbar("mcfc:demo", "MCFC Bossbar")

    bb.value = 5
    bb.max = 10
    bb.visible = true
    bb.players = player
    bb.name = "MCFC Bossbar Updated"

    player.position.particle("minecraft:happy_villager", 20, player)

    async:
        sleep(5)
        bb.remove()
        player.position.setblock("minecraft:gold_block")
    end

    player.tellraw("Bossbar will disappear soon")
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
- `async: ... end`
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
- block bodies are scoped for new `let` bindings
- loop variables are local to the loop body
- `mc` is literal-only; `mcf` performs runtime interpolation

### `async`

`async:` launches a new execution path immediately and continues the caller without waiting.

```text
async:
    sleep(5)
    debug("later")
end
```

Rules:

- `async` is statement-only
- locals and parameters are snapshotted when the async block starts
- later parent mutations do not affect the async copy
- `return` is not allowed inside an async block
- `break` and `continue` keep their usual loop-only rules

### Expressions

Supported expressions:

- integer literals, for example `42`
- boolean literals: `true`, `false`
- string literals, for example `"hello"` or `'hello'`
- variables
- function calls
- method calls, for example `player.tellraw("hi")`
- path access, for example `pig.CustomName` or `player.position`
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
- `bossbar`
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

## Builtins and Methods

### Function-style builtins

These remain ordinary functions:

- `selector("...") -> entity_set`
- `single(entity_set) -> entity_ref`
- `exists(entity_ref) -> bool`
- `has_data(storage_path) -> bool`
- `block("...") -> block_ref`
- `at(entity_ref, entity_set|entity_ref|block_ref)`
- `as(entity_set|entity_ref, entity_set|entity_ref|block_ref)`
- `summon(entity_id: string) -> entity_ref`
- `summon(entity_id: string, data: nbt) -> entity_ref`
- `debug(message: string) -> void`
- `sleep(seconds: int) -> void`
- `random() -> int`
- `random(max: int) -> int`
- `random(min: int, max: int) -> int`
- `int(nbt) -> int`
- `bool(nbt) -> bool`
- `string(nbt) -> string`
- `bossbar(id: string, name: string) -> bossbar`

### Entity methods

- `entity.teleport(destination: entity_ref|block_ref) -> void`
- `entity.damage(amount: int) -> void`
- `entity.heal(amount: int) -> void`
- `entity.give(item_id: string, count: int) -> void`
- `entity.clear(item_id: string, count: int) -> void`
- `entity.loot_give(table: string) -> void`
- `entity.tellraw(message: string) -> void`
- `entity.title(message: string) -> void`
- `entity.actionbar(message: string) -> void`
- `entity.playsound(sound: string, category: string) -> void`
- `entity.stopsound(category: string, sound: string) -> void`
- `entity.debug_entity(label: string) -> void`
- `entity.effect(name: string, duration: int, amplifier: int) -> void`
- `entity.add_tag(name: string) -> void`
- `entity.remove_tag(name: string) -> void`
- `entity.has_tag(name: string) -> bool`

### Block methods

- `block.loot_insert(table: string) -> void`
- `block.loot_spawn(table: string) -> void`
- `block.debug_marker(label: string) -> void`
- `block.debug_marker(label: string, marker_block: string) -> void`
- `block.particle(name: string) -> void`
- `block.particle(name: string, count: int) -> void`
- `block.particle(name: string, count: int, viewers: entity_ref|entity_set) -> void`
- `block.setblock(block_id: string) -> void`
- `block.fill(to: block_ref, block_id: string) -> void`

### Bossbar handles

Create a bossbar with `bossbar(id, name)` and then mutate it through fields:

- `bb.name = string`
- `bb.value = int`
- `bb.max = int`
- `bb.visible = bool`
- `bb.players = entity_ref|entity_set`
- `bb.remove()`

### `entity.position`

`entity.position` is a read-only `block_ref` representing the entity's current block position.

Use it anywhere a `block_ref` is accepted:

```text
player.position.particle("minecraft:happy_villager", 8, player)
player.position.setblock("minecraft:gold_block")
```

`entity_set.position` is not supported. Iterate the set and use each `entity_ref.position`.

### Collections

- `array<T>.len() -> int`
- `array<T>.push(value: T) -> void`
- `array<T>.pop() -> T`
- `array<T>.remove_at(index: int) -> T`
- `dict<T>.has(key: string) -> bool`
- `dict<T>.remove(key: string) -> void`

## Player and Entity Surfaces

- `player.nbt.*` reads vanilla player NBT
- `player.state.*` stores MCFC-managed integer and boolean player state
- `player.tags.*` reads and writes player tags as booleans
- `entity.team = "name"` assigns a team for any `entity_ref`
- `entity.mainhand.*`, `entity.offhand.*`, `entity.head.*`, `entity.chest.*`, `entity.legs.*`, and `entity.feet.*` modify equipped items
- `heal(...)` is currently limited to known non-player `entity_ref` targets

## Timing and Randomness

`sleep(seconds)` pauses the current execution path and resumes it later with Minecraft `schedule function`.

That means:

- a plain `sleep(...)` in normal code pauses the current path
- a `sleep(...)` inside `async:` pauses only that async branch
- `sleep(...)` is statement-only and cannot be used as a value

`random()` helpers return inclusive integer ranges because they map directly to Minecraft `random value`.

String literals support `$(expr)` interpolation with the same expression rules as `mcf` placeholders.

## Scope and Name Rules

Variables are function-local.

Important behavior:

- parameters are available throughout the function
- `let` introduces a new binding
- reusing an existing local or parameter name with `let` is rejected
- `let` bindings created inside a block do not become visible outside that block
- `for` loop variables follow the same block-scoping rule

## Control Flow

### `if`

`if` conditions must have type `bool`.

### `while`

`while` conditions must have type `bool`.

The compiler lowers loops into generated datapack functions. Infinite loops are your responsibility.

### `for`

Range loops require `int` bounds.

Rules:

- `start..end` is half-open and iterates while `i < end`
- `start..=end` is inclusive and iterates while `i <= end`
- range bounds are evaluated once before the loop starts
- the loop variable is local to the loop body

### `break` and `continue`

- `break` exits the nearest enclosing loop
- `continue` skips to the next iteration
- both are only valid inside `while` and `for`

## Raw Minecraft Commands

Use `mc "..."` to emit raw commands directly:

```text
mc "say hello"
mc "scoreboard players set @s health 20"
```

Use `mcf "..."` for runtime interpolation through Minecraft function macros:

```text
let amount = 5
mcf "xp add @a $(amount + 1) levels"
```

Rules:

- placeholders use `$(expr)` syntax
- placeholders are type-checked using normal MCFC expression rules
- malformed placeholders are rejected during compilation

`mc "say $(a)"` emits the literal text. `mcf "say $(a)"` performs runtime substitution.

## Execution Context

Use `as(anchor): ... end` to run commands as an entity set or entity ref.
Use `at(anchor): ... end` to run commands at an entity set, entity ref, or block ref.

The value-level forms `as(anchor, value)` and `at(anchor, value)` compose execution context onto entity and block reference values.

## Runtime Representation

The current backend maps values like this:

- `int`: scoreboard-backed
- `bool`: scoreboard-backed using `0` and `1`
- `string`: Minecraft data storage-backed
- `array<T>`: Minecraft data storage-backed
- `dict<T>`: Minecraft data storage-backed
- `bossbar`: Minecraft data storage-backed handle containing the bossbar id

Generated files are deterministic and use a reserved generated namespace layout.

## Diagnostics

The compiler reports line and column information for parse and type errors.

Common errors include:

- undefined variables
- duplicate names
- wrong argument count
- wrong argument type
- invalid return type
- non-boolean conditions
- invalid `for` bounds
- `break` or `continue` outside a loop
- `return` inside `async`
- use of removed legacy gameplay builtins such as `tellraw(target, ...)`

## Current Limitations

Not supported yet:

- recursion
- implicit conversions
- modules/imports
- `entity_set.position`
- richer object systems beyond structs plus the built-in handle types
- optimization passes

Notes:

- `match` currently supports only `string` scrutinees
- each `match` arm currently contains exactly one statement; use helper functions if an arm needs more work

The backend prioritizes correctness, inspectable output, and deterministic code generation over optimization.
