# Builders

A builder holds an entity, block, item or text component that you configure, then pass to `summon`, `setBlock`, `give` or `sendMessage`.

## Entity Builders

Create an `EntityData` with `new EntityData(id)`.

| Member | Type | Notes |
| --- | --- | --- |
| `id` | `String` | Read-only entity id. |
| `nbt.*` | `Nbt` | Reads and writes summon NBT. |
| `getName()` / `setName(String)` | `String` | Shorthand for `nbt.CustomName`. |
| `getNameVisible()` / `setNameVisible(boolean)` | `boolean` | Shorthand for `nbt.CustomNameVisible`. |
| `getNoAi()` / `setNoAi(boolean)` | `boolean` | Shorthand for `nbt.NoAI`. |
| `getSilent()` / `setSilent(boolean)` | `boolean` | Shorthand for `nbt.Silent`. |
| `getGlowing()` / `setGlowing(boolean)` | `boolean` | Shorthand for `nbt.Glowing`. |
| `getTags()` / `setTags(List<String>)` | `List<String>` | Shorthand for `nbt.Tags`. |
| `asNbt()` | `Nbt` | Flattened entity compound for passengers and summon payloads. |

```mcfc
void spawnPet() {
    var pig = new EntityData("minecraft:pig");
    pig.setName("MCFC");
    pig.setNoAi(true);
    pig.nbt.Health = 20;

    var chicken = new EntityData("minecraft:chicken");
    chicken.setName("Passenger");
    pig.nbt.Passengers[0] = chicken;

    summon(pig);
}
```

## Block Builders

Create a `BlockData` with `new BlockData(id)`.

| Member | Type | Notes |
| --- | --- | --- |
| `id` | `String` | Read-only block id. |
| `states.*` | `String`, `boolean`, or `int` | Block-state values. |
| `nbt.*` | `Nbt` | Block-entity NBT. |
| `getName()` / `setName(String)` | `String` | Shorthand for `nbt.CustomName`. |
| `getLock()` / `setLock(String)` | `String` | Shorthand for `nbt.Lock`. |
| `getLootTable()` / `setLootTable(String)` | `String` | Shorthand for `nbt.LootTable`. |
| `getLootSeed()` / `setLootSeed(int)` | `int` | Shorthand for `nbt.LootTableSeed`. |
| `asNbt()` | `Nbt` | Block-entity payload, equivalent to `BlockData.nbt`. |

`setBlock(BlockData)` places the block id and states, then merges `BlockData.nbt`. `fill(..., BlockData)` uses only the block id and states.

```mcfc
void placeChest() {
    var chest = new BlockData("minecraft:chest");
    chest.states.facing = "north";
    chest.setName("Loot");
    chest.setLootTable("minecraft:chests/simple_dungeon");

    Block.of("~ ~ ~").setBlock(chest);
}
```

## Item Builders

Create an `ItemStack` with `new ItemStack(id)`.

| Member | Type | Notes |
| --- | --- | --- |
| `id` | `String` | Read-only item id. |
| `getCount()` / `setCount(int)` | `int` | Stack size. |
| `nbt.*` | `Nbt` | Item NBT. |
| `getName()` / `setName(String)` | `Nbt` / `String` | Shorthand for `nbt.display.Name`. Cast the getter to `String` if needed. |
| `asNbt()` | `Nbt` | Item-stack payload compound. |

```mcfc
void reward(Player player) {
    var sword = new ItemStack("minecraft:diamond_sword");
    sword.setCount(1);
    sword.setName("Quest Blade");
    sword.nbt.CustomModelData = 7;

    player.give(sword);
}
```

## Text Builders

Create a `Component` with `new Component()` or `new Component("...")`.

`Component.*` supports arbitrary nested text-component content, formatting, interactivity, and child fields such as `.color`, `.bold`, `.extra`, `.hover_event.*`, `.click_event.*`, `.with`, `.score.*`, `.separator`, and `.nbt` source fields.

```mcfc
void sendPrompt(Player player) {
    var prompt = new Component("Open chest");
    prompt.color = "gold";
    prompt.bold = true;
    prompt.hover_event.action = "show_text";
    prompt.hover_event.value = new Component("Contains loot");
    prompt.click_event.action = "run_command";
    prompt.click_event.command = "/trigger status";

    player.sendMessage(prompt);
}
```

Assigning a `Component` into a nested text-component field stores the nested component object directly.

## Builder-to-NBT Coercion

When an `Nbt` value is expected, assigning an `EntityData`, `BlockData`, or `ItemStack` is shorthand for calling `.asNbt()`.

```mcfc
void payloads() {
    var pig = new EntityData("minecraft:pig");
    var payload = pig.asNbt();

    summon("minecraft:pig", payload);
}
```

## Under the hood

Builders are command-storage objects. Setters such as `pig.setNoAi(true)` and raw-data assignments such as `chest.states.facing = "north"` or `msg.color = "gold"` become `data modify storage ...` writes into generated runtime storage.

When a builder is consumed, MCFC renders that stored data into the relevant Minecraft command sequence. For example, `summon(pig)` uses the entity id and NBT payload, while `Block.of("~ ~ ~").setBlock(chest)` emits the block id/states and then merges block-entity NBT.
