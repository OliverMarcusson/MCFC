# Statements

Statements perform work, control flow, or introduce declarations. MCFC uses `:` and indentation for block bodies; use spaces for indentation because tabs are rejected.

## Forms

| Statement | Purpose |
| --- | --- |
| [`fn name(...) -> type:`](./statements/fn) | Declare a function. |
| [`data player.name: type = value`](./statements/data) | Declare scoreboard-backed player data. |
| [`event name:`](./statements/event) | Declare an event handler. |
| [`command name:`](./statements/command) | Declare a command handler. |
| [`task name every_ticks(n):`](./statements/task) | Declare a scheduled task. |
| [`player_state name: type = "Display"`](./statements/player-state) | Declare scoreboard-backed player state. |
| [`struct Name:`](./statements/struct) | Declare a named struct type with indented fields. |
| [`let name = expr`](./statements/let) | Create a local binding inferred from the initializer. |
| [`name = expr`](./statements/assignment) | Assign a new value to an existing local or writable path. |
| [`if condition:` / `else:`](./statements/if) | Branch on a `bool` expression. |
| [`match value:`](./statements/match) | Branch on string arms and an optional `else` arm. |
| [`while condition:`](./statements/while) | Repeat while a `bool` expression is true. |
| [`for name in start..end:`](./statements/for-range) | Iterate an integer range. |
| [`for name in start..=end:`](./statements/for-range) | Iterate an inclusive integer range. |
| [`for name in selector_expr:`](./statements/for-selector) | Iterate over matching entities. |
| [`for name in array_expr:`](./statements/for-array) | Iterate over array values. |
| [`async:`](./statements/async) | Launch a non-blocking execution path. |
| [`break` / `continue`](./statements/control-flow) | Control the nearest enclosing loop. |
| [`return` / `return expr`](./statements/control-flow) | Exit the current function. |
| [`mc "..."`](./statements/mc) | Emit a literal Minecraft command. |
| [`mcf "..."`](./statements/mcf) | Emit a Minecraft function macro with `$(...)` placeholders. |
| [`as(entity):`](./statements/as) / [`at(entity):`](./statements/at) | Run the body with changed executor or position context. |
| [`do_work()`](./statements/call) | Call a function for side effects. |

Only function calls may be used as bare expression statements. For example, `debug("ok")` is valid, but `amount + 1` is not a statement.

## Under The Hood

Statements lower into generated `.mcfunction` files. Straight-line statements become command lines in the current generated function; nested blocks usually become generated helper functions guarded by scoreboard state.

- `if`, `match`, `while`, and `for` use generated scoreboard guards and `execute if/unless score ... run function ...`.
- `break`, `continue`, and `return` set generated control slots that later commands check before continuing.
- `async`, `sleep`, and host calls split code into continuation functions so the current execution path can pause or branch.
- `as:` and `at:` wrap the generated body call with `execute as ...` or `execute at ...`.

For the full runtime model, see [How MCFC Lowers To mcfunction](./lowering).

## Structs

Struct declarations are top-level and contain typed fields.

```mcfc
struct Quest:
    name: string
    reward: int

fn describe(quest: Quest) -> void:
    debug(quest.name)
```

## Locals And Assignment

`let` introduces a new local. Reusing an existing local or parameter name with `let` is rejected, and later assignments must keep the original type.

```mcfc
fn tick() -> void:
    let amount = 5
    amount = amount + 1

    let player = single(selector("@p"))
    player.state.quest_steps = amount
```

Bindings created inside a block are scoped to that block. Loop variables are local to the loop body.

## Branching

`if` and `else` bodies are indented blocks.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))

    if player.has_tag("ready"):
        player.tellraw("Starting")
    else:
        player.tellraw("Waiting")
```

`match` compares a value against string arms. Use `else` for the fallback arm.

```mcfc
fn handle(action: string) -> void:
    match action:
        "jump" => debug("leap")
        "pathfind" => debug("move")
        else => debug("idle")
```

## Loops

Integer ranges use `..` for an exclusive end and `..=` for an inclusive end.

```mcfc
fn countdown() -> void:
    for step in 1..4:
        mcf "say step $(step)"

    for step in 1..=3:
        mcf "say inclusive $(step)"
```

Selectors and arrays can also be iterated.

```mcfc
fn tag_players() -> void:
    for player in selector("@a"):
        player.add_tag("seen")

fn show_scores(scores: array<int>) -> void:
    for score in scores:
        mcf "say score $(score)"
```

`break` exits the nearest loop and `continue` skips to its next iteration.

```mcfc
fn find_ready() -> void:
    for player in selector("@a"):
        if not player.has_tag("ready"):
            continue

        player.tellraw("Found ready player")
        break
```

## Async

`async:` starts a new execution path immediately and continues the caller without waiting.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))

    async:
        sleep_ticks(20)
        player.actionbar("One second later")

    player.actionbar("Now")
```

Async blocks are statement-only. Locals and parameters are snapshotted when the block starts, later parent mutations do not change the async copy, and `return` is not allowed inside an async block.

## Context Blocks

`as` changes the executor for the indented body. `at` changes the execution position.

```mcfc
fn tick() -> void:
    let player = single(selector("@p"))

    as(player):
        mc "say running as @s"

    at(player):
        mc "particle minecraft:happy_villager ~ ~1 ~ 0.2 0.2 0.2 0 8"
```

## Return

Use `return` without a value from `void` functions. Use `return expr` when the function has a non-`void` return type.

```mcfc
fn clamp_health(value: int) -> int:
    if value < 0:
        return 0

    return value

fn announce() -> void:
    debug("done")
    return
```
