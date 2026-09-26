# Statements

Blocks start with `:` and are indented with spaces. Tabs are an error. `#` starts a comment.

**Top level:** [`fn`](#fn) · [`struct`](#struct) · [`enum`](#enum) · [`mod` / `pub`](#mod-and-pub) · [`use`](#use) · [`player_state`](#player-state) · [`entity_state`](#entity-state) · [`event`](./events) · [`command`](#command) · [`task`](#task)

**In a function:** [`let`](#let) · [assignment](#assignment) · [`if`](#if) · [`switch`](#switch) · [`match`](#match) · [`while`](#while) · [`for`](#for) · [`break` / `continue` / `return`](#break-continue-return) · [`async`](#async) · [`as` / `at`](#as-and-at) · [`mc`](#mc) · [`mcf`](#mcf) · [calls](#calls)

## Declarations

### `fn`

```mcfc
fn greet(player: player_ref, message: string) -> void:
    player.tellraw(message)
```

Every parameter type and the return type are required. Duplicate function or parameter names are errors. Recursion isn't supported.

Two names are special:

- `fn main() -> void` in the root module runs every time the datapack loads, including on `/reload`. It's added to the `minecraft:load` tag.
- `fn tick() -> void` runs every game tick through the `minecraft:tick` tag. If several modules define `tick`, their bodies all run, in module-tree order. A `tick` that takes parameters is an ordinary function.

Every other zero-argument `void` function is also exported as `/function <namespace>:<name>`, so you can call it from chat.

#### Generic functions

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

Every type parameter must appear in a parameter's type, because there's no `f::<int>(...)` syntax. Arguments bound to the same parameter must agree: for `fn same<T>(a: T, b: T)`, `same(1, "x")` is an error. There are no bounds. Each combination of types compiles to its own copy (`biggest__int`, `biggest__float`), and each copy is type-checked on its own. So `biggest(["a", "b"])` reports that `>` needs numbers, plus "'biggest' does not work with T = string" at the call. Structs can't be generic.

#### Functions that pause

A function that calls `sleep`, `sleep_ticks`, `sort()` or a host call pauses, and so does any function that calls it. The caller continues once the callee is done. Because of that, a call to a pausing function has to be a statement of its own: `f()`, `let x = f()`, `x = f()` or `return f()`. Using it inside a condition or a larger expression is an error.

```mcfc
fn wait_then_double(n: int) -> int:
    sleep_ticks(20)
    return n * 2

fn main() -> void:
    let x = wait_then_double(4)
    debug("one second later, x is $(x)")
```

### `struct`

```mcfc
struct Quest:
    name: string
    reward: int

fn main() -> void:
    let quest = Quest { name: "Mine", reward: 5 }
    quest.reward = quest.reward + 1
    debug(quest.name)
```

Structs are top-level. Fields are read and written with `.`. Struct values live in command storage.

### `enum`

```mcfc
enum Mode:
    SURVIVAL
    CREATIVE

fn describe(mode: Mode) -> void:
    switch mode:
        case Mode.SURVIVAL:
            debug("Survival")
        case Mode.CREATIVE:
            debug("Creative")
```

Enums have one constant per line and need at least one constant. You refer to a constant as `Mode.SURVIVAL`. Constants are stored as integers starting at 0, in declaration order. Reordering them changes stored values.

### `mod` and `pub`

`mod name` loads a child module from another file. Items are private unless marked `pub`.

<!-- no-check -->
```mcfc
# src/main.mcf
mod util

fn main() -> void:
    let n = util::double(21)
```

<!-- no-check -->
```mcfc
# src/util.mcf
pub fn double(x: int) -> int:
    return helper(x) * 2

fn helper(x: int) -> int:
    return x
```

Module files are found the same way Rust finds them:

| Declared in | `mod util` loads |
| --- | --- |
| `src/main.mcf` (the root) | `src/util.mcf` or `src/util/mod.mcf` |
| `src/game/mod.mcf` or `src/game.mcf` | `src/game/util.mcf` or `src/game/util/mod.mcf` |

It's an error if neither file exists or if both do. A `.mcf` file that no `mod` reaches isn't compiled, and the build warns about it.

- `pub` works on `fn`, `struct` and `mod`. A private item can be used by its own module and that module's children.
- `mod` is top-level only. Inline `mod name:` bodies aren't supported.
- `mod`, `use` and `pub` are only keywords at the start of a top-level line. Inside a function you can use them as ordinary names.
- `tick`, `event`, `command` and `task` work in any module and ignore `pub`. `main` is only special in the root module.

A function in a child module compiles under its full path. A zero-argument `void` function `util::announce` is exported as `/function <namespace>:util/announce`.

### `use`

```mcfc
use std::math::{clamp, pow as power}

fn main() -> void:
    let hp = clamp(150, 0, 100)
    let bits = power(2, 10)
```

| Form | Imports |
| --- | --- |
| `use a::b::name` | `name` |
| `use a::b::name as other` | `name`, under the name `other` |
| `use a::b::{x, y as z}` | `x`, and `y` under the name `z` |
| `use a::b` | the module `b`, so `b::name(...)` works |

- A path's first segment is looked up in the current module, then in the root module.
- `self::` starts at the current module. `super::` starts at the parent, and `super::super::` goes up two levels.
- A child module doesn't see root items automatically. Write `use helper` or `super::helper()`.
- Functions, structs and modules have separate namespaces.
- `pub use` and `*` globs aren't supported. Imports are private to their module.
- `$(...)` placeholders don't apply imports, so inside them write the full path from the root, such as `$(util::double(x))`.

### `player_state`

Declares a value stored per player, read and written as `player.state.<name>`:

```mcfc
struct Profile:
    level: int
    title: string

player_state coins: int = "Coins"
player_state profile: Profile = "Profile"

fn update(player: player_ref) -> void:
    player.state.coins = player.state.coins + 1
    player.state.profile = Profile { level: 3, title: "Scout" }
    player.state.profile.level = 4
```

Allowed types are `int`, `bool`, `string`, `float` and structs. The string after `=` is the display name of the scoreboard objective that holds `int` and `bool` state. For other types it's ignored.

Values persist across reloads and restarts. A value that was never set reads as `0`, `false`, `""`, `0.0` or `{}`.

You can also use `player.state.<name>` for `int` and `bool` without declaring it. Other types have to be declared. Two declarations can't overlap, for example `a` and `a.b`.

`int` and `bool` state is stored in scoreboard objectives named `mcfs_*`. Other types are stored in `<namespace>:state` storage, keyed by the player's UUID.

### `entity_state`

This is the same as `player_state`, but for any `entity_ref` and without a display name:

```mcfc
struct MarkerInfo:
    label: string
    weight: float

entity_state info: MarkerInfo

fn mark(entity: entity_ref) -> void:
    entity.state.info = MarkerInfo { label: "Target", weight: 1.5 }
    debug(entity.state.info.label)
```

Scoreboard objectives are named `mcfe_*`. Stored values aren't removed when the entity despawns.

### `command`

```mcfc
command status:
    let player = single(selector("@s"))
    player.tellraw("Ready")
```

Players run the command with `/trigger mcfcc_status`, which needs no operator permissions. The handler runs as that player. With the agent attached, `/status` also works as a real command. Commands take no arguments, and there's no tab completion.

Put `async` and `sleep` in a function you call from the handler, not directly in the handler body.

### `task`

```mcfc
task heartbeat every_ticks(20):
    debug("heartbeat")

task setup after_ticks(1):
    debug("setup")
```

`every_ticks(n)` repeats every `n` ticks. `after_ticks(n)` runs once, `n` ticks after load. Tasks run as the server, not as a player. Use `for player in selector("@a"):` to act on each player.

## In a function

### `let`

```mcfc
fn main() -> void:
    let amount = 5
    let names = ["a", "b"]
```

`let` creates a local variable, and its type comes from the initializer. Declaring a name that's already a local or parameter is an error. A variable declared inside a block isn't visible outside it.

### Assignment

```mcfc
fn main() -> void:
    let amount = 1
    amount = amount + 1

    let player = single(selector("@p"))
    player.state.score = amount
```

The target is a local variable or a writable path, and the new value must have the same type.

### `if`

```mcfc
fn check(player: player_ref) -> void:
    if player.has_tag("ready"):
        player.tellraw("Ready")
    else:
        player.tellraw("Waiting")
```

The condition must be a `bool`. `else` is optional, and `else if condition:` chains conditions.

### `switch`

```mcfc
fn describe(level: int) -> void:
    switch level:
        case 1:
            debug("low")
        case 2:
            debug("medium")
            debug("still medium")
        default:
            debug("high")
```

`switch` works on enum, `int` or `string` values. Cases are constants of that type, and each case body can have several statements. A switch on an enum must either list every constant or have a `default`. The value is evaluated once, and the first matching case runs.

### `match`

```mcfc
fn handle(action: string) -> void:
    match action:
        "jump" => debug("leap")
        "pathfind" => debug("move")
        else => debug("idle")
```

A shorter form for strings, with exactly one statement per arm. Use `switch` when an arm needs more than one statement.

### `while`

```mcfc
fn count() -> void:
    let i = 0
    while i < 3:
        mcf "say $(i)"
        i = i + 1
```

A loop runs entirely within one tick unless its body sleeps. A long loop with no `sleep` can hit Minecraft's command limit (`maxCommandChainLength`), and the rest of the function then doesn't run.

### `for`

```mcfc
fn loops(values: array<int>) -> void:
    for i in 0..3:
        mcf "say $(i)"
    for i in 1..=3:
        mcf "say $(i)"
    for player in selector("@a"):
        player.add_tag("seen")
    for value in values:
        mcf "say $(value)"
```

| Form | Iterates |
| --- | --- |
| `for i in a..b` | `a` up to `b - 1` |
| `for i in a..=b` | `a` up to `b` |
| `for e in selector(...)` | each matching entity, as an `entity_ref`. Runs through `execute as`, so `@s` is the current entity. |
| `for x in array` | each element |

The loop variable exists only inside the loop body. Range bounds are evaluated once, before the loop starts.

### `break`, `continue`, `return`

```mcfc
fn first_ready() -> void:
    for player in selector("@a"):
        if not player.has_tag("ready"):
            continue
        player.tellraw("Ready")
        break

fn clamp_zero(value: int) -> int:
    if value < 0:
        return 0
    return value
```

`break` and `continue` apply to the innermost loop. In a `void` function, use `return` on its own. Otherwise, `return expr` must match the declared return type.

### `async`

```mcfc
fn greet_later(player: player_ref) -> void:
    async:
        sleep_ticks(20)
        player.actionbar("later")
    player.actionbar("now")
```

The body starts running right away, and the statement after the block runs without waiting for it. Local variables are copied when the block starts, so later changes in the parent don't reach the copy. `return` isn't allowed inside `async`.

### `as` and `at`

```mcfc
fn sparkle(player: player_ref) -> void:
    as(player):
        mc "say I am @s"
    at(player):
        mc "particle minecraft:happy_villager ~ ~1 ~ 0.2 0.2 0.2 0 8"
```

`as` changes who `@s` is, and `at` changes where `~ ~ ~` is. The anchor can be an `entity_ref` or an `entity_set`, in which case the body runs once per entity. These compile to `execute as` and `execute at`. To build a selector relative to an entity, use the function forms [`as(...)` and `at(...)`](./builtins#as-and-at).

### `mc`

```mcfc
fn setup() -> void:
    mc "scoreboard objectives add health dummy"
```

Emits a Minecraft command exactly as written. `$(...)` isn't interpreted, so `mc "say $(x)"` prints `$(x)` literally.

### `mcf`

```mcfc
fn reward(amount: int) -> void:
    mcf "xp add @a $(amount) levels"
    mcf "say next reward is $(amount + 1)"
```

Emits a command with each `$(expr)` replaced by its value at run time. It compiles to a Minecraft function macro. Values are copied into storage, and then `function ... with storage ...` is called. Use `mc` when there's nothing to substitute.

Values are inserted without escaping. A string containing `"` breaks the command, as described under [string limits](./types#string).

### Calls

A function or method call can be a statement on its own, such as `debug("ok")` or `player.heal(2)`. Other expressions can't: `amount + 1` on its own line is an error.
