# Types And Builtins

## Built-in Types

MCFC supports:

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

Locals infer their type from the initializer. Assignments must keep the original variable type, function arguments must match declared parameter types, and return expressions must match declared return types.

## Function-style Builtins

Frequently used builtins:

```mcfc
let players = selector("@a")
let player = single(selector("@p"))
let pos = block("~ ~ ~")
let pig = entity("minecraft:pig")
let sword = item("minecraft:diamond_sword")
let title = text("Hello")
let bb = bossbar("mcfc:demo", title)
```

Other host-independent builtins include `exists`, `has_data`, `block_type`, `summon`, `debug`, `sleep`, `sleep_ticks`, and `random`.

## Entity And Player Methods

Entities expose gameplay operations such as:

- `teleport`
- `damage`
- `heal`
- `give`
- `clear`
- `tellraw`
- `title`
- `actionbar`
- `playsound`
- `stopsound`
- `effect`
- `add_tag`
- `remove_tag`
- `has_tag`

Players additionally expose inventory and hotbar surfaces:

```mcfc
let player = single(selector("@p"))
let sword = item("minecraft:diamond_sword")
sword.name = "Quest Blade"
player.hotbar[0] = sword

if player.inventory[3].exists:
    player.tellraw(player.inventory[3].id)
```

## Builders

Builder handles let code assemble NBT-rich entities, blocks, items, and text components before using them.

```mcfc
let pig = entity("minecraft:pig")
pig.name = "MCFC"
pig.no_ai = true

let chest = block_type("minecraft:chest")
chest.states.facing = "north"
chest.name = "Loot"
block("~ ~ ~").setblock(chest)

let msg = text("Open chest")
msg.color = "gold"
single(selector("@p")).tellraw(msg)
```

When an `nbt` value is expected, `entity_def`, `block_def`, and `item_def` can be assigned as shorthand for `.as_nbt()`.

## Bossbars

```mcfc
let bb = bossbar("mcfc:demo", "Demo")
bb.value = 5
bb.max = 10
bb.visible = true
bb.players = selector("@a")
bb.remove()
```
