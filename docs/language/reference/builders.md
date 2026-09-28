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
class Main {
    static void spawnPet() {
        var pig = new EntityData("minecraft:pig");
        pig.setName("MCFC");
        pig.setNoAi(true);
        pig.nbt.Health = 20;

        var chicken = new EntityData("minecraft:chicken");
        chicken.setName("Passenger");
        pig.nbt.Passengers[0] = chicken;

        summon(pig);
    }
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
class Main {
    static void placeChest() {
        var chest = new BlockData("minecraft:chest");
        chest.states.facing = "north";
        chest.setName("Loot");
        chest.setLootTable("minecraft:chests/simple_dungeon");

        Block.of("~ ~ ~").setBlock(chest);
    }
}
```

## Item Builders

Create an `ItemStack` with `new ItemStack(id)`.

| Member | Type | Notes |
| --- | --- | --- |
| `id`, `getId()` | `String` | Read-only item id. |
| `getCount()` / `setCount(int)` | `int` | Stack size. |
| `nbt.*` | `Nbt` | Item NBT. |
| `getName()` / `setName(String)` | `Nbt` / `String` | Shorthand for `nbt.display.Name`. Cast the getter to `String` if needed. |
| `asNbt()` | `Nbt` | Item-stack payload compound. |

```mcfc
class Main {
    static void reward(Player player) {
        var sword = new ItemStack("minecraft:diamond_sword");
        sword.setCount(1);
        sword.setName("Quest Blade");
        sword.nbt.CustomModelData = 7;

        player.give(sword);
    }
}
```

## Text Builders

Create a `Component` with `new Component()` or `new Component("...")`.

`Component.*` supports arbitrary nested text-component content, formatting, interactivity, and child fields such as `.color`, `.bold`, `.extra`, `.hover_event.*`, `.click_event.*`, `.with`, `.score.*`, `.separator`, and `.nbt` source fields.

```mcfc
class Main {
    static void sendPrompt(Player player) {
        var prompt = new Component("Open chest");
        prompt.color = "gold";
        prompt.bold = true;
        prompt.hover_event.action = "show_text";
        prompt.hover_event.value = new Component("Contains loot");
        prompt.click_event.action = "run_command";
        prompt.click_event.command = "/trigger status";

        player.sendMessage(prompt);
    }
}
```

Assigning a `Component` into a nested text-component field stores the nested component object directly. Reading a field such as `prompt.extra` gives `Nbt`; declare it as `List<Component>` or `Component` to use it as one.

### Adventure API

Paper's Adventure calls work too. Each method returns a changed copy and leaves the component it's called on alone, as in Adventure.

```mcfc
class Main {
    static void greet(Player player, String name) {
        var message = Component.text("Welcome, ", NamedTextColor.GOLD)
            .append(Component.text(name).decorate(TextDecoration.BOLD))
            .clickEvent(ClickEvent.suggestCommand("/msg " + name))
            .hoverEvent(HoverEvent.showText(Component.text("Click to message")));
        player.sendMessage(message);
    }
}
```

| Call | Result |
|---|---|
| `Component.text(s)`, `Component.text(s, color)` | A text component |
| `Component.empty()`, `newline()`, `space()` | `""`, `"
"`, `" "` |
| `Component.translatable(key)`, `translatable(key, args)` | A translated component; `args` is a `List<Component>` |
| `c.color(color)` | `color` is a `NamedTextColor.RED`-style constant, `TextColor.color(0xff8800)`, `TextColor.color(r, g, b)`, `TextColor.fromHexString("#ff8800")`, or a string such as `"red"` |
| `c.decorate(d)`, `c.decoration(d, on)` | `d` is `TextDecoration.BOLD`, `ITALIC`, `UNDERLINED`, `STRIKETHROUGH` or `OBFUSCATED` |
| `c.append(child)`, `c.appendNewline()`, `c.appendSpace()` | Adds a child, which takes `c`'s style |
| `c.children()`, `c.children(list)` | Reads or replaces the children |
| `c.clickEvent(e)` | `e` is `ClickEvent.runCommand(cmd)`, `suggestCommand(cmd)`, `openUrl(url)` or `copyToClipboard(text)` |
| `c.hoverEvent(HoverEvent.showText(text))` | Shows `text` on hover |
| `c.insertion(text)`, `c.font(id)` | Shift-click insertion text, and the font |

Serializers (`LegacyComponentSerializer`, `PlainTextComponentSerializer`, Gson), `ClickEvent.callback` and `replaceText` aren't supported. They need Java on the server.

### MiniMessage

`MiniMessage.miniMessage().deserialize(text)` turns [MiniMessage](https://docs.advntr.dev/minimessage/format.html) markup into a `Component`.

```mcfc
class Main {
    static void announce(Player player) {
        player.sendMessage(MiniMessage.miniMessage().deserialize(
            "<gradient:gold:red>Arena open</gradient> <click:run_command:'/trigger join'><u>join</u>"));
    }
}
```

A string literal is parsed by the compiler into one constant component, so it costs one command. It supports colors (`<red>`, `<#ff8800>`, `<color:red>`), decorations and `<!bold>`, `<reset>`, `<newline>`/`<br>`, `<click:...>`, `<hover:show_text:'...'>`, `<gradient:...>`, `<rainbow>`, `<lang:key:args...>`, `<key:...>`, `<insert:...>` and `<font:...>`. `"\\<red>"` writes a literal `<red>`. Unknown tags stay as text. A literal can't use `$(...)`; append the value with `.append(Component.text(x))`.

Any other string, such as text a player typed, is parsed while the pack runs by `MiniMessage.miniMessage().deserialize(...)`. It only applies colors, decorations, `<reset>` and `<newline>`. Every other tag, including `<click>` and `<hover>`, stays as plain text, so a player's message can't make someone else run a command. It costs a few hundred commands per tag and a few per character, so keep it to chat-length text.

## Builder-to-NBT Coercion

When an `Nbt` value is expected, assigning an `EntityData`, `BlockData`, or `ItemStack` is shorthand for calling `.asNbt()`.

```mcfc
class Main {
    static void payloads() {
        var pig = new EntityData("minecraft:pig");
        var payload = pig.asNbt();

        summon("minecraft:pig", payload);
    }
}
```

## Under the hood

Builders are command-storage objects. Setters such as `pig.setNoAi(true)` and raw-data assignments such as `chest.states.facing = "north"` or `msg.color = "gold"` become `data modify storage ...` writes into generated runtime storage.

When a builder is consumed, MCFC renders that stored data into the relevant Minecraft command sequence. For example, `summon(pig)` uses the entity id and NBT payload, while `Block.of("~ ~ ~").setBlock(chest)` emits the block id/states and then merges block-entity NBT.
