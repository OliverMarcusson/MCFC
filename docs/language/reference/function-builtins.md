# Function-style Builtins

Function-style builtins are ordinary calls that create references, builder handles, utility values, or runtime effects.

## Under The Hood

Many builtins do not become runtime function calls. Instead, the compiler recognizes them and lowers them directly:

- `selector`, `single`, `block`, `as`, and `at` build selector/position references used by later `execute` commands.
- `entity`, `item`, `text`, and `block_type` allocate builder payloads in command storage.
- `summon`, `debug`, `sleep`, `sleep_ticks`, `random`, and conversion builtins emit Minecraft commands or generated continuation functions.
- `int`, `bool`, and `string` convert between storage/NBT paths and scoreboard or storage-backed values.

## Signatures

| Builtin | Returns | Notes |
| --- | --- | --- |
| [`selector(value: string)`](./builtins/selector) | `entity_set` | Wraps a Minecraft selector or player name. |
| [`single(value: entity_set)`](./builtins/single) | `entity_ref` | Narrows a selector to one entity. |
| [`exists(value: entity_ref)`](./builtins/exists) | `bool` | Tests whether the reference exists. |
| [`has_data(path)`](./builtins/has-data) | `bool` | Tests whether a storage/NBT path has data. |
| [`block(pos: string)`](./builtins/block) | `block_ref` | Creates a block reference from coordinates. |
| [`entity(id: string)`](./builtins/entity) | `entity_def` | Creates an entity builder. |
| [`item(id: string)`](./builtins/item) | `item_def` | Creates an item builder. |
| [`text()`](./builtins/text) | `text_def` | Creates an empty text component builder. |
| [`text(value: string)`](./builtins/text) | `text_def` | Creates a text component builder with text. |
| [`block_type(id: string)`](./builtins/block-type) | `block_def` | Creates a block builder. |
| [`bossbar(id: string, name: string\|text_def)`](./builtins/bossbar) | `bossbar` | Creates or references a bossbar handle. |
| [`summon(id: string)`](./builtins/summon) | `entity_ref` | Summons an entity by id. |
| [`summon(id: string, data: nbt)`](./builtins/summon) | `entity_ref` | Summons with NBT payload. |
| [`summon(spec: entity_def)`](./builtins/summon) | `entity_ref` | Summons from an entity builder. |
| [`debug(message: string)`](./builtins/debug) | `void` | Emits a debug/log message. |
| [`sleep(seconds: int)`](./builtins/sleep) | `void` | Pauses the current execution path by seconds. |
| [`sleep_ticks(ticks: int)`](./builtins/sleep-ticks) | `void` | Pauses the current execution path by ticks. |
| [`random()`](./builtins/random) | `int` | Returns a random integer. |
| [`random(max: int)`](./builtins/random) | `int` | Returns a bounded random integer. |
| [`random(min: int, max: int)`](./builtins/random) | `int` | Returns a random integer in a range. |
| [`int(value: nbt)`](./builtins/int) | `int` | Converts an NBT value to an `int`. |
| [`bool(value: nbt)`](./builtins/bool) | `bool` | Converts an NBT value to a `bool`. |
| [`string(value: nbt)`](./builtins/string) | `string` | Converts an NBT value to a `string`. |

`as(...)` and `at(...)` also exist as two-argument builtins for selector context composition:

```mcfc
let player = single(selector("@p"))
let nearest_pig = single(at(player, selector("@e[type=minecraft:pig,sort=nearest,limit=1]")))
let self_ref = single(as(player, selector("@s")))
```

For statement blocks, see [`as(entity):` and `at(entity):`](./statements#context-blocks).

## Common Setup

```mcfc
fn tick() -> void:
    let players = selector("@a")
    let player = single(selector("@p"))
    let pos = block("~ ~ ~")
    let pig = entity("minecraft:pig")
    let sword = item("minecraft:diamond_sword")
    let title = text("Hello")
    let bb = bossbar("mcfc:demo", title)

    debug("ready")
```

## Summoning And Existence

```mcfc
fn spawn_pet() -> void:
    let pig = entity("minecraft:pig")
    pig.name = "Pet"
    pig.no_ai = true

    let spawned = summon(pig)
    if exists(spawned):
        spawned.add_tag("pet")
```

## Sleep And Random

```mcfc
fn delayed_roll() -> void:
    async:
        sleep_ticks(20)
        let roll = random(1, 6)
        mcf "say rolled $(roll)"
```
