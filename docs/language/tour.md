# Language Tour

The whole language on one page. Each section links to its reference entry. If you've never built a pack with MCFC, start with [Your First Pack](/guide/first-pack).

## Layout

Blocks open with `:` and are indented with spaces. Tabs are an error. `#` starts a comment.

```mcfc
fn main() -> void:
    # runs on load and on /reload
    selector("@a").tellraw("loaded")
```

## Functions

```mcfc
fn add(a: int, b: int) -> int:
    return a + b

fn tick() -> void:
    let n = add(1, 2)
```

Parameter and return types are always written out. `main` runs on load and `tick` runs every tick. Every other zero-argument `void` function can be called in game as `/function <namespace>:<name>`. Functions can be [generic](./reference/statements#generic-functions). → [`fn`](./reference/statements#fn)

## Values

```mcfc
fn main() -> void:
    let count = 3
    let speed = 1.5
    let ready = true
    let name = "Alex"
    let line = "$(name) has $(count)"
    let scores = [1, 2, 3]
    let teams = {"red": 0, "blue": 0}
    count = count + 1
```

`let` infers the type, and later assignments must keep it. Types never convert implicitly: `1.5 + 2` is an error, and `1.5 + float(2)` is correct. `/` rounds down, the same as scoreboard math. → [Types](./reference/types)

## Control flow

```mcfc
fn main() -> void:
    let hp = 12
    if hp < 5:
        debug("low")
    else if hp < 10:
        debug("mid")
    else:
        debug("ok")

    for i in 0..3:
        debug("$(i)")

    for player in selector("@a"):
        player.add_tag("seen")

    while hp > 0:
        hp = hp - 5
```

There's also `switch`, `match`, `break`, `continue` and `return`. → [Statements](./reference/statements)

## Structs and enums

```mcfc
struct Quest:
    name: string
    reward: int

enum Stage:
    NEW
    DONE

fn finish(quest: Quest, stage: Stage) -> void:
    switch stage:
        case Stage.NEW:
            debug("started $(quest.name)")
        case Stage.DONE:
            debug("reward $(quest.reward)")
```

## Missing values

`array.get`, `dict.get` and `find_first` return an `Optional<T>`:

```mcfc
fn main() -> void:
    let first = [4, 8].get(5).orElse(0)
    let pig = find_first(selector("@e[type=minecraft:pig]"))
    if pig.isPresent():
        debug("found a pig")
```

→ [`Optional<T>`](./reference/types#optional)

## Entities and players

```mcfc
fn main() -> void:
    let player = single(selector("@p"))
    player.tellraw("Hi")
    player.give("minecraft:bread", 3)
    player.effect("minecraft:speed", 10, 1)
    if player.health() < 6.0:
        player.title("Low health")
    player.position.setblock("minecraft:torch")
```

`selector(...)` can match any number of entities, and `single(...)` narrows it to one. The compiler works out from the selector whether a reference is a player. → [Entities and Players](./reference/methods)

To create customized entities, items, blocks and text before using them, use [builders](./reference/builders):

```mcfc
fn main() -> void:
    let sword = item("minecraft:diamond_sword")
    sword.name = "Quest Blade"
    single(selector("@p")).give(sword)
```

## Stored state

```mcfc
player_state coins: int = "Coins"
entity_state owner: string

fn pay(player: player_ref) -> void:
    player.state.coins = player.state.coins + 1
```

State is stored per player or per entity and survives reloads. → [`player_state`](./reference/statements#player-state)

## Events, commands and tasks

```mcfc
event player_join:
    single(selector("@s")).tellraw("Welcome")

command spawn:
    single(selector("@s")).teleport(block("0 64 0"))

task reminder every_ticks(6000):
    selector("@a").actionbar("Five minutes passed")
```

A `command` is run with `/trigger mcfcc_<name>`. With the optional agent there are 34 events in total, many of them cancellable. → [Events](./reference/events), [`command`](./reference/statements#command), [`task`](./reference/statements#task)

## Waiting

```mcfc
fn countdown(player: player_ref) -> void:
    async:
        for i in 0..3:
            player.title("$(3 - i)")
            sleep(1)
        player.title("Go")
```

`sleep` and `sleep_ticks` pause the function. `async:` runs its body without the caller waiting for it. → [`async`](./reference/statements#async), [Functions that pause](./reference/statements#functions-that-pause)

## Raw commands

```mcfc
fn main() -> void:
    let n = 5
    mc "weather clear"
    mcf "xp add @a $(n) levels"
```

For commands MCFC has no feature for yet, `mc` emits a command exactly as written and `mcf` fills in `$(...)` values at run time. Use them only as a last resort. → [`mc`](./reference/statements#mc), [`mcf`](./reference/statements#mcf)

## Modules and std

<!-- no-check -->
```mcfc
mod combat              # loads src/combat.mcf
use std::math::clamp

fn main() -> void:
    let hp = clamp(combat::damage(), 0, 20)
```

Items are private unless marked `pub`. `std` is always available. → [`mod`](./reference/statements#mod-and-pub), [`use`](./reference/statements#use), [std](./reference/std)

## Outside the game

With the optional `mcfd` helper, a pack can make HTTP requests, read and write files, and query SQLite:

```mcfc
fn motd(player: player_ref) -> void:
    let r = http.get("https://api.example.com/motd")
    if r.ok:
        player.tellraw(r.body)
```

→ [Host Bridge](/runtime/host-bridge)

## What's missing

See [Limitations](./limitations).
