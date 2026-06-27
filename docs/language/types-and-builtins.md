# Types And Builtins

## Built-in Types

MCFC includes primitive values, collections, Minecraft reference types, builder handles, `bossbar`, `nbt`, `void`, and named `struct` types.

See the [built-in types reference](./reference/builtin-types) for the complete list and type rules. Locals infer their type from the initializer. Assignments must keep the original variable type, function arguments must match declared parameter types, and return expressions must match declared return types.

## Function-style Builtins

Frequently used builtins include selectors, single-entity narrowing, block/entity/item/text builders, bossbars, debugging, sleep helpers, random numbers, summoning, and NBT conversion helpers.

```mcfc
let players = selector("@a")
let player = single(selector("@p"))
let pos = block("~ ~ ~")
let pig = entity("minecraft:pig")
let sword = item("minecraft:diamond_sword")
let title = text("Hello")
let bb = bossbar("mcfc:demo", title)
```

See the [function-style builtins reference](./reference/function-builtins) for signatures and examples.

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

See the [entity and player methods reference](./reference/entity-player-methods) for signatures, fields, player inventory surfaces, aliases, and examples.

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

See the [builders reference](./reference/builders) for supported builder fields and NBT behavior.

## Bossbars

```mcfc
let bb = bossbar("mcfc:demo", "Demo")
bb.value = 5
bb.max = 10
bb.visible = true
bb.players = selector("@a")
bb.remove()
```
