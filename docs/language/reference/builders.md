# Builders

Builder handles let MCFC assemble NBT-rich entities, blocks, items, and text components before using them in gameplay APIs.

## Under The Hood

Builders are command-storage objects. Field assignments such as `pig.no_ai = true`, `chest.states.facing = "north"`, or `msg.color = "gold"` become `data modify storage ...` writes into generated runtime storage.

When a builder is consumed, MCFC renders that stored data into the relevant Minecraft command sequence. For example, `summon(pig)` uses the entity id and NBT payload, while `block("~ ~ ~").setblock(chest)` emits the block id/states and then merges block-entity NBT.

## Entity Builders

Create an `entity_def` with `entity(id)`.

| Member | Type | Notes |
| --- | --- | --- |
| `id` | `string` | Read-only entity id. |
| `nbt.*` | `nbt` | Reads and writes summon NBT. |
| `name` | `string` | Shorthand for `nbt.CustomName`. |
| `name_visible` | `bool` | Shorthand for `nbt.CustomNameVisible`. |
| `no_ai` | `bool` | Shorthand for `nbt.NoAI`. |
| `silent` | `bool` | Shorthand for `nbt.Silent`. |
| `glowing` | `bool` | Shorthand for `nbt.Glowing`. |
| `tags` | `array<string>` | Shorthand for `nbt.Tags`. |
| `as_nbt()` | `nbt` | Flattened entity compound for passengers and summon payloads. |

```mcfc
fn spawn_pet() -> void:
    let pig = entity("minecraft:pig")
    pig.name = "MCFC"
    pig.no_ai = true
    pig.nbt.Health = 20

    let chicken = entity("minecraft:chicken")
    chicken.name = "Passenger"
    pig.nbt.Passengers[0] = chicken

    summon(pig)
```

## Block Builders

Create a `block_def` with `block_type(id)`.

| Member | Type | Notes |
| --- | --- | --- |
| `id` | `string` | Read-only block id. |
| `states.*` | `string`, `bool`, or `int` | Block-state values. |
| `nbt.*` | `nbt` | Block-entity NBT. |
| `name` | `string` | Shorthand for `nbt.CustomName`. |
| `lock` | `string` | Shorthand for `nbt.Lock`. |
| `loot_table` | `string` | Shorthand for `nbt.LootTable`. |
| `loot_seed` | `int` | Shorthand for `nbt.LootTableSeed`. |
| `as_nbt()` | `nbt` | Block-entity payload, equivalent to `block_def.nbt`. |

`setblock(block_def)` places the block id and states, then merges `block_def.nbt`. `fill(..., block_def)` uses only the block id and states.

```mcfc
fn place_chest() -> void:
    let chest = block_type("minecraft:chest")
    chest.states.facing = "north"
    chest.name = "Loot"
    chest.loot_table = "minecraft:chests/simple_dungeon"

    block("~ ~ ~").setblock(chest)
```

## Item Builders

Create an `item_def` with `item(id)`.

| Member | Type | Notes |
| --- | --- | --- |
| `id` | `string` | Read-only item id. |
| `count` | `int` | Stack size. |
| `nbt.*` | `nbt` | Item NBT. |
| `name` | `string` | Shorthand for `nbt.display.Name`. |
| `as_nbt()` | `nbt` | Item-stack payload compound. |

```mcfc
fn reward(player: player_ref) -> void:
    let sword = item("minecraft:diamond_sword")
    sword.count = 1
    sword.name = "Quest Blade"
    sword.nbt.CustomModelData = 7

    player.give(sword)
```

## Text Builders

Create a `text_def` with `text()` or `text("...")`.

`text_def.*` supports arbitrary nested text-component content, formatting, interactivity, and child fields such as `.color`, `.bold`, `.extra`, `.hover_event.*`, `.click_event.*`, `.with`, `.score.*`, `.separator`, and `.nbt` source fields.

```mcfc
fn send_prompt(player: player_ref) -> void:
    let prompt = text("Open chest")
    prompt.color = "gold"
    prompt.bold = true
    prompt.hover_event.action = "show_text"
    prompt.hover_event.value = text("Contains loot")
    prompt.click_event.action = "run_command"
    prompt.click_event.value = "/trigger mcfcc_status"

    player.tellraw(prompt)
```

Assigning a `text_def` into a nested text-component field stores the nested component object directly.

## Builder-to-NBT Coercion

When an `nbt` value is expected, assigning an `entity_def`, `block_def`, or `item_def` is shorthand for calling `.as_nbt()`.

```mcfc
fn payloads() -> void:
    let pig = entity("minecraft:pig")
    let payload = pig.as_nbt()

    summon("minecraft:pig", payload)
```
