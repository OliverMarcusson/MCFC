# Entity And Player Methods

`entity_ref` and `player_ref` expose gameplay operations for moving, damaging, messaging, inventory work, tags, effects, and sound.

## Under The Hood

Entity and player methods lower to vanilla commands targeted at the receiver selector:

- movement and combat methods emit commands such as `teleport` and `damage`
- messaging methods emit `tellraw`, `title`, or actionbar/title commands
- sound methods emit `playsound` and `stopsound`
- tag methods emit scoreboard tag commands
- inventory and hotbar surfaces use generated item-slot storage/NBT commands

When the receiver is context-sensitive, MCFC preserves executor and position by wrapping generated method commands in the necessary `execute as` or `execute at` context.

## Methods

| Method | Returns | Notes |
| --- | --- | --- |
| [`teleport(destination: entity_ref\|block_ref)`](./methods/teleport) | `void` | Teleports the receiver. |
| [`damage(amount: int)`](./methods/damage) | `void` | Applies damage. |
| [`heal(amount: int)`](./methods/heal) | `void` | Restores health. |
| [`give(item_id: string, count: int)`](./methods/give) | `void` | Gives an item by id and count. |
| [`give(stack: item_def)`](./methods/give) | `void` | Gives a built item stack. |
| [`clear(item_id: string, count: int)`](./methods/clear) | `void` | Clears matching items. |
| [`loot_give(table: string)`](./methods/loot-give) | `void` | Gives loot from a loot table. |
| [`tellraw(message: string\|text_def)`](./methods/tellraw) | `void` | Sends chat text. |
| [`title(message: string\|text_def)`](./methods/title) | `void` | Sends a title. |
| [`actionbar(message: string\|text_def)`](./methods/actionbar) | `void` | Sends an actionbar message. |
| [`playsound(sound: string, category: string)`](./methods/playsound) | `void` | Plays a sound. |
| [`stopsound(category: string, sound: string)`](./methods/stopsound) | `void` | Stops a sound. |
| [`debug_entity(label: string)`](./methods/debug-entity) | `void` | Emits debug info for the receiver. |
| [`effect(name: string, duration: int, amplifier: int)`](./methods/effect) | `void` | Applies an effect. |
| [`add_tag(name: string)`](./methods/add-tag) | `void` | Adds a scoreboard tag. |
| [`remove_tag(name: string)`](./methods/remove-tag) | `void` | Removes a scoreboard tag. |
| [`has_tag(name: string)`](./methods/has-tag) | `bool` | Tests for a scoreboard tag. |

Bukkit-style aliases are also available for display and sound calls: [`send_message`](./methods/send-message), [`send_title`](./methods/send-title), [`send_actionbar`](./methods/send-actionbar), [`play_sound`](./methods/play-sound), and [`stop_sound`](./methods/stop-sound).

## Fields And Surfaces

| Field | Type | Notes |
| --- | --- | --- |
| `team` | `string` | Writable team name. |
| `position` | `block_ref` | Read-only current block position. |
| `nbt.*` | `nbt` | Entity NBT read/write namespace. |
| `state.*` | scoreboard-backed values | Player/entity state namespace. |
| `mainhand.*` / `offhand.*` | item slot surface | Writable held item namespaces. |
| `inventory[index]` | `item_slot` | Player inventory slot surface. |
| `hotbar[index]` | `item_slot` | Player hotbar slot surface. |

`inventory` and `hotbar` are player surfaces. Use them on `player_ref` values.

## Movement And Combat

```mcfc
fn rescue() -> void:
    let player = single(selector("@p"))
    let pig = single(selector("@e[type=minecraft:pig,limit=1]"))

    pig.teleport(player)
    pig.heal(4)
    player.damage(1)
```

## Messaging And Sound

```mcfc
fn celebrate(player: player_ref) -> void:
    let message = text("Quest complete")
    message.color = "gold"
    message.bold = true

    player.tellraw(message)
    player.title("Victory")
    player.actionbar("Reward delivered")
    player.playsound("minecraft:entity.player.levelup", "master")
```

## Tags And Effects

```mcfc
fn prepare(player: player_ref) -> void:
    if not player.has_tag("prepared"):
        player.add_tag("prepared")
        player.effect("minecraft:speed", 200, 1)
    else:
        player.remove_tag("prepared")
```

## Inventory And Hotbar

```mcfc
fn equip() -> void:
    let player = single(selector("@p"))
    let sword = item("minecraft:diamond_sword")
    sword.name = "Quest Blade"
    sword.count = 1

    player.give(sword)
    player.hotbar[0] = sword

    if player.inventory[3].exists:
        player.tellraw(player.inventory[3].id)
```

## Position

`entity.position` can be passed anywhere a `block_ref` is accepted.

```mcfc
fn mark_position() -> void:
    let player = single(selector("@p"))
    player.position.particle("minecraft:happy_villager", 8, player)
    player.position.setblock("minecraft:gold_block")
```
