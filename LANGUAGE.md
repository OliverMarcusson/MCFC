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
- `--no-optimize`: disable the conservative IR optimisation pass
- `--clean`: remove the output directory before writing new output

The compiler writes:

- `pack.mcmeta`
- `data/<namespace>/function/main.mcfunction`
- generated helper functions under `data/<namespace>/function/generated/`

`main.mcfunction` runs setup and then calls the generated `main` entrypoint if a `main` function exists.
Every zero-argument `void` function except `main` and special `tick` also gets
a public wrapper at `data/<namespace>/function/<name>.mcfunction`, so it can be
run with `/function <namespace>:<name>`.

## Example Program

```text
fn main() -> void:
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
    player.tellraw("Bossbar will disappear soon")
```

## Syntax

### Functions

Functions begin with `fn`, use `:` after the signature, and use indentation for
their body.

```text
fn name(param: type, other: type) -> return_type:
    ...
```

Rules:

- parameter types are required
- return types are required
- duplicate function names are rejected
- duplicate parameter names are rejected
- `#` starts a line comment and may also appear after code on a line

Special functions:

- `fn tick() -> void:` maps to the datapack tick function and runs once every
  game tick.
- if multiple source files define `tick() -> void`, their bodies are merged in
  deterministic source order.
- parameterized functions named `tick`, such as `fn tick(action: Action) -> int:`,
  remain ordinary helpers unless a zero-argument `tick() -> void` is also
  present.

### Statements

Supported statements:

- `player_state name: int|bool = "Display Name"`
- `struct Name:` followed by indented fields
- `let name = expr`
- `name = expr`
- `if condition:` followed by an indented body
- `if condition:` / `else:` with indented bodies
- `match value:` with indented `"a" => stmt` and `else => stmt` arms
- `while condition:` followed by an indented body
- `for name in start..end:` followed by an indented body
- `for name in start..=end:` followed by an indented body
- `for name in selector_expr:` followed by an indented body
- `for name in array_expr:` followed by an indented body
- `async:` followed by an indented body
- `break`
- `continue`
- `return`
- `return expr`
- `mc "raw minecraft command"`
- `mcf "macro command with $(placeholders)"`
- `as(entity):` followed by an indented body
- `at(entity):` followed by an indented body
- a bare function call as a statement, for example `do_work()`

Notes:

- only function calls may be used as bare expression statements
- block bodies are scoped for new `let` bindings
- loop variables are local to the loop body
- `mc` is literal-only; `mcf` performs runtime interpolation
- tabs are rejected for indentation; use spaces

### `async`

`async:` launches a new execution path immediately and continues the caller without waiting.

```text
async:
    sleep(5)
    debug("later")
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
- `player_ref`
- `block_ref`
- `entity_def`
- `block_def`
- `item_def`
- `text_def`
- `item_slot`
- `bossbar`
- `nbt`
- `void`
- named `struct` types

Type rules:

- locals infer their type from the initializer
- assignments must keep the original variable type
- function arguments must match declared parameter types
- return expressions must match the declared return type
- there is no implicit conversion between types, except `entity_def` and `block_def`
  automatically coerce to `nbt` in NBT contexts
- `item_def` also automatically coerces to `nbt` in NBT contexts through
  `item_def.as_nbt()`
- `text_def` is storage-backed and can be assigned anywhere an `nbt` text component
  payload is expected

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
- `entity(entity_id: string) -> entity_def`
- `item(item_id: string) -> item_def`
- `text() -> text_def`
- `text(value: string) -> text_def`
- `block("...") -> block_ref`
- `block_type(block_id: string) -> block_def`
- `at(entity_ref, entity_set|entity_ref|block_ref)`
- `as(entity_set|entity_ref, entity_set|entity_ref|block_ref)`
- `summon(entity_id: string) -> entity_ref`
- `summon(entity_id: string, data: nbt) -> entity_ref`
- `summon(spec: entity_def) -> entity_ref`
- `debug(message: string) -> void`
- `sleep(seconds: int) -> void`
- `sleep_ticks(ticks: int) -> void`
- `random() -> int`
- `random(max: int) -> int`
- `random(min: int, max: int) -> int`
- `int(nbt) -> int`
- `bool(nbt) -> bool`
- `string(nbt) -> string`
- `bossbar(id: string, name: string|text_def) -> bossbar`

### Entity methods

- `entity.teleport(destination: entity_ref|block_ref) -> void`
- `entity.damage(amount: int) -> void`
- `entity.heal(amount: int) -> void`
- `entity.give(item_id: string, count: int) -> void`
- `entity.give(stack: item_def) -> void`
- `entity.clear(item_id: string, count: int) -> void`
- `entity.loot_give(table: string) -> void`
- `entity.tellraw(message: string|text_def) -> void`
- `entity.title(message: string|text_def) -> void`
- `entity.actionbar(message: string|text_def) -> void`
- `entity.playsound(sound: string, category: string) -> void`
- `entity.stopsound(category: string, sound: string) -> void`
- `entity.debug_entity(label: string) -> void`
- `entity.effect(name: string, duration: int, amplifier: int) -> void`
- `entity.add_tag(name: string) -> void`
- `entity.remove_tag(name: string) -> void`
- `entity.has_tag(name: string) -> bool`

### Block methods

- `block.is(block_id: string) -> bool`
- `block.loot_insert(table: string) -> void`
- `block.loot_spawn(table: string) -> void`
- `block.debug_marker(label: string) -> void`
- `block.debug_marker(label: string, marker_block: string) -> void`
- `block.particle(name: string) -> void`
- `block.particle(name: string, count: int) -> void`
- `block.particle(name: string, count: int, viewers: entity_ref|entity_set) -> void`
- `block.setblock(block_id: string|block_def) -> void`
- `block.fill(to: block_ref, block_id: string|block_def) -> void`
- `block.summon(entity_id: string) -> entity_ref`
- `block.summon(entity_id: string, data: nbt) -> entity_ref`
- `block.summon(spec: entity_def) -> entity_ref`
- `block.spawn_item(stack: item_def) -> entity_ref`

### Builder handles

Create an entity builder with `entity(id)` and mutate it before summoning:

- `entity_def.id` is a read-only `string`
- `entity_def.nbt.*` reads and writes summon NBT
- `entity_def.name` is shorthand for `entity_def.nbt.CustomName`
- `entity_def.name_visible` is shorthand for `entity_def.nbt.CustomNameVisible`
- `entity_def.no_ai` is shorthand for `entity_def.nbt.NoAI`
- `entity_def.silent` is shorthand for `entity_def.nbt.Silent`
- `entity_def.glowing` is shorthand for `entity_def.nbt.Glowing`
- `entity_def.tags` is shorthand for `entity_def.nbt.Tags`
- `entity_def.as_nbt() -> nbt` returns a flattened entity compound suitable for
  passengers and summon payloads

Create a block builder with `block_type(id)` and mutate it before placement:

- `block_def.id` is a read-only `string`
- `block_def.states.*` writes block-state values as `string`, `bool`, or `int`
- `block_def.nbt.*` reads and writes block-entity NBT
- `block_def.name` is shorthand for `block_def.nbt.CustomName`
- `block_def.lock` is shorthand for `block_def.nbt.Lock`
- `block_def.loot_table` is shorthand for `block_def.nbt.LootTable`
- `block_def.loot_seed` is shorthand for `block_def.nbt.LootTableSeed`
- `block_def.as_nbt() -> nbt` returns the block-entity payload, equivalent to
  `block_def.nbt`

Create an item builder with `item(id)` and mutate it before giving, storing, or
spawning it:

- `item_def.id` is a read-only `string`
- `item_def.count` reads and writes the stack size
- `item_def.nbt.*` reads and writes item NBT
- `item_def.name` is shorthand for `item_def.nbt.display.Name`
- `item_def.as_nbt() -> nbt` returns an item-stack payload compound

Create a text component builder with `text()` or `text("...")` and mutate any
text component field path before sending it to display APIs:

- `text_def.*` supports arbitrary nested text-component content, formatting,
  interactivity, and child fields such as `.color`, `.bold`, `.extra`,
  `.hover_event.*`, `.click_event.*`, `.with`, `.score.*`, `.separator`, and
  `.nbt` source fields
- assigning a `text_def` into a nested text-component field stores the nested
  component object directly

`setblock(block_def)` places the block id and states, then merges `block_def.nbt`.
`fill(..., block_def)` uses only the block id and states.
When `nbt` is expected, assigning an `entity_def`, `block_def`, or `item_def`
is shorthand for calling `.as_nbt()`.

Example:

```text
let pig = entity("minecraft:pig")
pig.name = "MCFC"
pig.no_ai = true
let chicken = entity("minecraft:chicken")
chicken.name = "Passenger"
pig.nbt.Passengers[0] = chicken
let spawned = summon(pig)
let payload = summon("minecraft:pig", chicken.as_nbt())

let chest = block_type("minecraft:chest")
chest.states.facing = "north"
chest.name = "Loot"
let chest_payload = chest.as_nbt()
block("~ ~ ~").setblock(chest)

let player = single(selector("@p"))
let sword = item("minecraft:diamond_sword")
sword.count = 1
sword.name = "Quest Blade"
sword.nbt.CustomModelData = 7
player.give(sword)
```

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

You can also test a block at any position:

```text
let below = block("~ ~-1 ~")
if below.is("minecraft:air"):
    below.setblock("minecraft:purple_concrete")
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
- `entity.state.*` stores MCFC-managed integer and boolean state on any `entity_ref`
- `player.tags.*` reads and writes player tags as booleans
- `entity.team = "name"` assigns a team for any `entity_ref`
- `player.hotbar[0..8] -> item_slot` reads and writes live player hotbar slots
- `player.inventory[0..26] -> item_slot` reads and writes the main inventory
- `player_ref(entity)` asserts that an `entity_ref` is a player so player-only surfaces are available
- `entity.mainhand.*`, `entity.offhand.*`, `entity.head.*`, `entity.chest.*`, `entity.legs.*`, and `entity.feet.*` modify equipped items
- `heal(...)` is currently limited to known non-player `entity_ref` targets

`entity.state.*` and `player.state.*` currently support only `int` and `bool`
values. MCFC creates the scoreboard objectives automatically when a state path is
used. Player state uses the internal `mcfs_*` objective prefix and generic
entity state uses `mcfe_*`.

Example:

```text
fn tick() -> void:
    for marker in selector("@e[type=minecraft:marker,tag=skyrunner_decay]"):
        marker.state.decay = marker.state.decay + 1
        if marker.state.decay >= 20:
            as(marker):
                mc "kill @s"
```

Declare player state display metadata at the top level when you want a clean
scoreboard display name:

```text
player_state money: int = "Money"

fn main() -> void:
    let player = single(selector("@p"))
    player.state.money = 10
```

The generated objective remains MCFC-managed internally, but the sidebar label
uses the declared display name. Undeclared `player.state.*` and `entity.state.*`
paths still work and use generated objective names.

Equipment `.item` assignments accept either a string item id or an `item_def`:

```text
let crown = item("minecraft:golden_helmet")
crown.name = "Crown"
player.head.item = crown
```

`item_slot` exposes:

- `exists: bool` read-only
- `id: string` read-only
- `count: int` read and write
- `nbt.*` read and write
- `name` as shorthand for the live display name
- `clear() -> void`

Examples:

```text
player.hotbar[0] = item("minecraft:stick")

let sword = item("minecraft:diamond_sword")
sword.name = "Blade"
player.inventory[5] = sword
player.inventory[5].count = 16

let idx = 7
player.hotbar[idx] = sword

let known_player = player_ref(single(selector("@e[limit=1]")))
known_player.inventory[idx] = sword

if player.inventory[3].exists:
    player.tellraw(player.inventory[3].id)
player.hotbar[2].clear()
```

Position-aware summon APIs live on `block_ref` and keep the global `summon(...)`
shorthand unchanged:

```text
block("1 64 1").summon(entity("minecraft:pig"))
block("~ ~ ~").spawn_item(item("minecraft:apple"))

let player = single(selector("@p"))
let rel = at(player, block("~1 ~ ~"))
rel.summon("minecraft:pig")
```

## Timing and Randomness

`sleep(seconds)` pauses the current execution path and resumes it later with Minecraft `schedule function`.
`sleep_ticks(ticks)` does the same thing with Minecraft tick units.

That means:

- a plain `sleep(...)` in normal code pauses the current path
- a `sleep(...)` inside `async:` pauses only that async branch
- `sleep(...)` and `sleep_ticks(...)` are statement-only and cannot be used as values

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

Use `as(anchor):` with an indented body to run commands as an entity set or entity ref.
Use `at(anchor):` with an indented body to run commands at an entity set, entity ref, or block ref.

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

## Optimisation

By default, MCFC runs a conservative IR optimisation pass before backend
generation. The pass folds pure literal expressions, removes simple no-op
self-assignments, drops `while false:` bodies, and simplifies literal `if`
branches only when doing so cannot change later control-flow guarding.

Use `--no-optimize` to inspect the unoptimised lowered output.

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

Notes:

- `match` currently supports only `string` scrutinees
- each `match` arm currently contains exactly one statement; use helper functions if an arm needs more work

The backend prioritizes correctness, inspectable output, and deterministic code generation over aggressive optimization.
