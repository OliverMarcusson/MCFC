# Standard Library: `std`

`std` is a set of classes that every project can use without declaring them. Each module holds one class of `static` methods named after it, such as `Timer` in `std.timer`. Import the class with [`import`](./statements#import), or write its full path, as in `std.timer.Timer.start(...)`. `Math` needs no import.

```mcfc
import std.timer.Timer;

class Main {
    public static void main() {
        var hp = Math.clamp(150, 0, 100);
        Timer.start("round", 200);
    }
}
```

Only the `std` methods a pack calls are compiled into it. The name `std` is reserved, so a `src/std.mcf` file is an error.

## `std.math`

`min`, `max`, `abs`, `sign` and `clamp` are [generic](./statements#generic-methods): they take `int` or `float`, and mixing the two gives `float`, so `Math.min(1.5, 2)` is `1.5`. They are methods of `Math`, next to the [builtin ones](./types#float) such as `Math.sqrt` and `Math.pow`.

| Method | Returns |
| --- | --- |
| `<T> T min(T a, T b)` | The smaller of `a` and `b`. |
| `<T> T max(T a, T b)` | The larger of `a` and `b`. |
| `<T> T abs(T x)` | `x` without its sign. |
| `<T> T sign(T x)` | `1`, `0`, or `-1`. |
| `<T> T clamp(T x, T low, T high)` | `x` limited to `low` through `high`. |
| `int rem(int a, int b)` | The remainder of `a / b`, with the sign of `b`. For example, `Math.rem(-7, 3)` is `2`. Same as `a % b`. |
| `int gcd(int a, int b)` | The greatest common divisor, never negative. `Math.gcd(12, -18)` is `6`. |
| `float lerp(float a, float b, float t)` | The point `t` of the way from `a` to `b`. `0.0` gives `a` and `1.0` gives `b`. |
| `int isqrt(int n)` | The whole-number square root, rounded down. Negative `n` gives `0`. |
| `float sin(float x)`, `cos`, `tan` | Trig in radians. The compiler inlines these as `/compute` providers, so `Math.sin(x) * 2.0` is still one command. |
| `float atan(float x)` | Arctangent in `(-pi/2, pi/2)`. |
| `float atan2(float y, float x)` | The angle of the point `(x, y)` in `(-pi, pi]`. `Math.atan2(0, 0)` is `0`. |
| `float asin(float x)`, `float acos(float x)` | Arcsine in `[-pi/2, pi/2]` and arccosine in `[0, pi]`. `x` is clamped to `[-1, 1]`. |

`/compute` has no inverse trig, so `atan`, `atan2`, `asin` and `acos` are MCFC code (range reduction and a series), accurate to about `1e-6` radians. Each call costs a few dozen commands.

Integer arithmetic is 32-bit scoreboard math, so results wrap on overflow.

## `std.list`

`Lists` holds these. They are [generic](./statements#generic-methods), so they work on `List<Integer>` and `List<Float>`.

| Method | Returns |
| --- | --- |
| `<T> T sum(List<T> xs)` | The total of all elements, or `0` for an empty list. |
| `<T> T min(List<T> xs)` | The smallest element, or `0` for an empty list. |
| `<T> T max(List<T> xs)` | The largest element, or `0` for an empty list. |

`sortBy` and `top` order a list of anything by a second list of `int` keys, where `keys[i]` belongs to `items[i]`, for example player names and their scores:

```mcfc
import std.list.Lists;

class Main {
    static void leaderboard(List<String> names, List<Integer> kills) {
        var best = Lists.top(names, kills, 3);
        Sidebar.setLine(0, "Top: " + String.join(", ", best));
    }
}
```

| Method | Returns |
| --- | --- |
| `<T> List<T> sortBy(List<T> items, List<Integer> keys)` | A copy of `items` ordered by `keys`, smallest first. Equal keys keep their order. |
| `<T> List<T> top(List<T> items, List<Integer> keys, int n)` | The `n` items with the largest keys, largest first. Equal keys keep their order. |

Both cost about `n²` commands for `n` items, fine for a server's players. To sort plain numbers, use the built-in [`xs.sort()`](./types#list) method.

## `std.function`

The functional interfaces of `java.util.function`, for [lambdas](./statements#lambdas-and-method-references):

```mcfc
import std.function.*;

record Fighter(String name, int kills, int deaths) {}

class Main {
    public static void main() {
        Predicate<Integer> even = number -> number % 2 == 0;
        Comparator<Fighter> ranking = Comparator.comparing(Fighter::kills).reversed()
            .thenComparing(Comparator.comparing(Fighter::deaths));
        List<Fighter> fighters = List.of(new Fighter("alex", 3, 1), new Fighter("sam", 5, 2));
        fighters.sort(ranking);
        fighters.removeIf(fighter -> even.test(fighter.kills()));
    }
}
```

| Interface | Method |
| --- | --- |
| `Function<T, R>` | `R apply(T value)` |
| `BiFunction<T, U, R>` | `R apply(T first, U second)` |
| `UnaryOperator<T>` | `T apply(T value)` |
| `BinaryOperator<T>` | `T apply(T first, T second)` |
| `Predicate<T>` | `boolean test(T value)`, and `negate()` |
| `Consumer<T>` | `void accept(T value)` |
| `Supplier<T>` | `T get()` |
| `Comparator<T>` | `int compare(T first, T second)`: negative when `first` goes first |

`Comparator.comparing(key)` orders by an `int` key, smallest first. `order.reversed()` flips an order, and `order.thenComparing(next)` uses `next` where `order` finds two values equal.

Unlike Java, `UnaryOperator` isn't a `Function`, so one can't be passed where the other is wanted.

## `std.stream`

A small `java.util.stream`. `xs.stream()` on any `List` gives a `Stream`; import `std.stream.Stream` to name the type:

```mcfc
import std.stream.Stream;

class Main {
    public static void main() {
        List<Integer> kills = List.of(5, 2, 8, 1);
        List<String> labels = kills.stream().filter(k -> k > 1).map(k -> "#" + k).toList();
        Stream<Integer> ranked = kills.stream().sorted((a, b) -> b - a).limit(3);
        int best = ranked.findFirst().orElse(0);
        int total = kills.stream().reduce(0, (a, b) -> a + b);
    }
}
```

| Method | Returns |
| --- | --- |
| `filter(Predicate<T>)`, `map(Function<T, R>)` | A new `Stream` |
| `sorted(Comparator<T>)`, `limit(int)`, `skip(int)` | A new `Stream` |
| `forEach(Consumer<T>)` | nothing |
| `toList()`, `count()` | `List<T>`, `int` |
| `anyMatch`, `allMatch`, `noneMatch(Predicate<T>)` | `boolean` |
| `findFirst()`, `min(Comparator<T>)`, `max(Comparator<T>)` | `Optional<T>` |
| `reduce(T identity, BinaryOperator<T>)` | `T` |

Unlike Java, each step runs straight away and builds a new list, so a stream isn't lazy and can be used more than once. A `Stream` is a class object on the [heap](./statements#class), which the collector frees. `sorted` is the same insertion sort as `list.sort(order)`.

## `std.cooldown`

Per-player cooldowns, each with a name, so one player can have several.

```mcfc
import std.cooldown.Cooldown;

class Main {
    @Command("dash")
    static void dash(Player player) {
        if (!Cooldown.ready(player, "dash")) {
            player.sendMessage("Dash is ready in " + Cooldown.remaining(player, "dash") / 20 + "s");
            return;
        }
        Cooldown.start(player, "dash", 60);
        player.addVelocity(player.getLookX(), 0.3, player.getLookZ());
    }
}
```

| Method | Does |
| --- | --- |
| `void start(Player player, String name, int ticks)` | Starts or restarts `name`, lasting `ticks`. |
| `boolean ready(Player player, String name)` | `true` once `name` has run out, or if it was never started. |
| `int remaining(Player player, String name)` | Ticks left, or `0` when ready. |
| `void clear(Player player, String name)` | Ends `name` early. |

Each cooldown stores the [`World.getGameTime()`](./builtins#world) it ends at, in the `std.cooldowns` [`@PlayerState`](./statements#playerstate), so cooldowns keep running while the player is offline and across restarts. Names follow the [map key rules](./types#map).

## `std.random`

`import std.random.Random;` adds these to the [builtin `Random` methods](./builtins#random) such as `Random.nextInt`: `Random.chance(0.25)`.

| Method | Returns |
| --- | --- |
| `boolean chance(float p)` | `true` with chance `p`: `0.25` is `true` about one time in four. |
| `float nextFloat()` | A float from `0.0` up to, but not including, `1.0`. |
| `<T> T pick(List<T> xs)` | A random element. An empty list gives the type's empty value, such as `0`. |
| `<T> List<T> shuffle(List<T> xs)` | A shuffled copy of `xs`. Lists are passed by value, so `xs` itself is unchanged. |

## `std.time`

```mcfc
import std.time.Time;

class Main {
    @Every(ticks = 20)
    static void showUptime() {
        Sidebar.setLine(0, "Uptime " + Time.formatTicks(World.getGameTime()));
    }
}
```

| Method | Returns |
| --- | --- |
| `String formatTicks(int ticks)` | `"m:ss"`, or `"h:mm:ss"` from one hour up, in whole seconds rounded down. `1300` is `"1:05"`. Negative ticks count as `0`. |
| `String padLeft(int value, int width)` | `value` with zeros in front up to `width` characters: `padLeft(5, 2)` is `"05"`. |

## `std.timer`

Named timers for the whole world, such as the time left in a round. For one timer per player, use [`std.cooldown`](#std-cooldown).

```mcfc
import std.timer.Timer;
import std.time.Time;

class Main {
    static void startRound() {
        Timer.start("round", 6000);
    }

    @Every(ticks = 20)
    static void showClock() {
        if (Timer.running("round")) {
            Sidebar.setLine(0, "Time " + Time.formatTicks(Timer.remaining("round")));
        }
    }
}
```

| Method | Does |
| --- | --- |
| `void start(String name, int ticks)` | Starts or restarts `name`. |
| `int remaining(String name)` | Ticks left, or `0` once it ran out or if it never started. |
| `boolean running(String name)` | `true` while ticks remain. |
| `void stop(String name)` | Ends `name` now. |

Timers store the [`World.getGameTime()`](./builtins#world) they end at in the [static field](./statements#static-fields) `Timer.stdTimers`, so they keep counting across reloads.

## `std.region`

Box-shaped areas such as an arena, given by two opposite corner blocks. Both corners are inside.

```mcfc
import std.region.Region;

class Main {
    static void round() {
        var arena = Region.of(Block.of(0, 60, 0), Block.of(40, 80, 40));
        arena.tagPlayers("in_arena");
        Selector.of("@a[tag=!in_arena]").sendActionBar("Get back to the arena");
        Selector.of("@a[tag=in_arena]").teleport(arena.randomBlock());
    }
}
```

| Method | Returns |
| --- | --- |
| `static Region of(Block a, Block b)` | The region between two corners, in any order. `Region` is a record of `minX`, `minY`, `minZ`, `maxX`, `maxY` and `maxZ`, so `new Region(...)` works too. |
| `boolean contains(Entity entity)` | `true` when the entity's feet are in one of the region's blocks. |
| `int countPlayers()` | How many players are inside. |
| `void tagPlayers(String tag)` | Gives `tag` to the players inside and removes it from everyone else. Select them afterwards with `Selector.allPlayers().tag(tag)`. |
| `Block randomBlock()` | A random block inside. |
| `Block center()` | The middle block, rounded down. |
| `void fill(String block)` | Fills the region, within `fill`'s 32768-block limit. |

`countPlayers` and `tagPlayers` check every online player, a few commands each.

## `std.team`

Scoreboard teams. Select a team's players with `Selector.of("@a[team=red]")`.

```mcfc
import std.team.Team;

class Main {
    static void setupTeams() {
        Team.create("red", "red");
        Team.create("blue", "blue");
        Team.setFriendlyFire("red", false);
        Team.setFriendlyFire("blue", false);
        Team.split(Selector.of("@a[sort=random]"), List.of("red", "blue"));
    }
}
```

| Method | Does |
| --- | --- |
| `void create(String name, String color)` | Creates the team if it's missing and sets its color, such as `"red"`. |
| `void remove(String name)` | Removes the team. |
| `<T> void join(T target, String team)`, `<T> void leave(T target)` | `target` is a `Player` or a `Selector`. |
| `void setFriendlyFire(String team, boolean allowed)` | Whether teammates can hurt each other. |
| `void split(Selector players, List<String> teams)` | Deals `players` into `teams` in turn, so sizes differ by at most one. Pass a selector with `sort=random` for random teams. |

## `std.gamemode`

```mcfc
import std.gamemode.GameMode;

class Main {
    static void watch(Player player) {
        if (player.getGameMode() != GameMode.SPECTATOR) {
            player.setGameMode(GameMode.SPECTATOR);
        }
    }
}
```

`GameMode` is `SURVIVAL`, `CREATIVE`, `ADVENTURE` or `SPECTATOR`, the type of [`setGameMode` and `getGameMode`](./methods).

## `std.selector`

```mcfc
import std.selector.Sort;

class Main {
    public static void main() {
        var nearest = Selector.entities().type("minecraft:pig").sort(Sort.NEAREST).limit(1);
        nearest.addTag("picked");
    }
}
```

`Sort` is `NEAREST`, `FURTHEST`, `RANDOM` or `ARBITRARY`, the order [`Selector.sort`](./types#building-selectors) picks entities in.

## `std.inventory`

Costs for shops. `item` is an ID such as `"minecraft:emerald"`. To count items, use [`player.countItem(id)`](./methods).

```mcfc
import std.inventory.Inventory;

class Main {
    @Command("buy")
    static void buy(Player player) {
        if (Inventory.take(player, "minecraft:emerald", 5)) {
            player.give("minecraft:diamond_sword", 1);
        } else {
            player.sendMessage("A sword costs 5 emeralds");
        }
    }
}
```

| Method | Returns |
| --- | --- |
| `boolean has(Player player, String item, int count)` | `true` when the player carries at least `count`. |
| `boolean take(Player player, String item, int count)` | Removes `count` and returns `true`, or removes nothing and returns `false` when the player has fewer. |

## `std.world`

World settings and chunk loading, each one command.

```mcfc
import std.world.World;

class Main {
    @PlayerState("Kills")
    static int kills;

    static void startGame() {
        World.setWeather("clear");
        World.setTimeOfDay(6000);
        World.setDifficulty("hard");
        World.setGameRule("keep_inventory", true);
        World.showState("kills", "sidebar");
    }
}
```

| Method | Does |
| --- | --- |
| `void setWeather(String weather)` | `"clear"`, `"rain"` or `"thunder"`. |
| `void setTimeOfDay(int ticks)` | `0` sunrise, `6000` noon, `13000` night, `18000` midnight. |
| `void setDifficulty(String difficulty)` | `"peaceful"`, `"easy"`, `"normal"` or `"hard"`. |
| `void setGameRule(String rule, boolean value)` | A true/false rule, such as `"keep_inventory"`. |
| `void setGameRuleValue(String rule, int value)` | A number rule, such as `"random_tick_speed"`. |
| `void showState(String state, String slot)` | Shows an `int` [player state](./statements#playerstate) in `"sidebar"`, `"below_name"` or `"list"`. It replaces what the slot showed, including the shared [`Sidebar`](./methods#sidebar). |
| `void hideSlot(String slot)` | Empties a display slot. |
| `void forceload(int x0, int z0, int x1, int z1)` | Keeps the chunks from column `x0, z0` to `x1, z1` loaded with nobody near. They load a few ticks later; check with [`Block.isLoaded()`](./types#block). |
| `void setSpawn(Block block)` | Where players spawn and respawn without a bed. |

To read a game rule, use the builtin [`World.getGameRule(name)`](./builtins#world).

## `std.noise`

Perlin noise: smooth random-looking values for terrain, particle paths and motion. The same inputs always give the same value.

```mcfc
import std.noise.Noise;

class Main {
    public static void main() {
        var height = 64 + (int) (Noise.fractal(10 * 0.05, 20 * 0.05, 4) * 12.0);
    }
}
```

| Method | Returns |
| --- | --- |
| `float perlin(float x, float y)` | 2D Perlin noise, about -1 to 1, and 0 at whole-number points. Multiply the inputs by a small number such as `0.05` for larger features, and add an offset for a different pattern. |
| `float fractal(float x, float y, int octaves)` | `octaves` layers of `perlin` added up, each twice as detailed and half as strong, scaled back to about -1 to 1. |

Each `perlin` call costs a few hundred commands, so sample it once per block or entity, not every tick for many of them.

## `std.shape`

Filled shapes, one `fill` per column, so even large shapes cost a few hundred commands.

```mcfc
import std.shape.Shapes;

class Main {
    static void build() {
        Shapes.sphere(Block.of(0, 80, 0), 6, "minecraft:glass");
        Shapes.cylinder(Block.of(20, 64, 0), 3, 10, "minecraft:stone_bricks");
        Shapes.heightmap(Block.of(40, 60, 0), 16, 16, 8, 0.05, "minecraft:grass_block");
    }
}
```

| Method | Fills |
| --- | --- |
| `sphere(Block center, int radius, String block)` | Every block within `radius` of `center`. |
| `cylinder(Block base, int radius, int height, String block)` | An upright cylinder standing on `base`. |
| `heightmap(Block corner, int width, int depth, int amplitude, float scale, String block)` | Terrain over a `width` by `depth` area: each column rises from `corner` by up to `amplitude` blocks, following [`noise.fractal`](#std-noise). `scale` sets the hill size; try `0.05`. |

Shapes replace what's there, like `fill`.

## `std.str`

`Strings`, as in `Strings.withCommas(1234567)`.

| Method | Returns |
| --- | --- |
| `boolean startsWith(String s, String prefix)` | `true` when `s` begins with `prefix`. |
| `boolean endsWith(String s, String suffix)` | `true` when `s` ends with `suffix`. |
| `int find(String s, String needle)` | The index of the first `needle` in `s`, or `-1`. |
| `boolean contains(String s, String needle)` | `true` when `needle` appears in `s`. |
| `String replace(String s, String target, String replacement)` | `s` with every `target` replaced. |
| `List<String> split(String s, String separator)` | The parts between each `separator`, without trailing empty parts. |
| `String join(String separator, List<String> parts)` | `parts` with `separator` between each. `String.join` calls this. |
| `String formatFloat(float x, int decimals)` | `x` rounded to `decimals` places, always showing them: `Strings.formatFloat(3.14159, 2)` is `"3.14"`, `Strings.formatFloat(2.0, 1)` is `"2.0"`. |
| `String withCommas(int n)` | `n` with commas between groups of three digits: `Strings.withCommas(1234567)` is `"1,234,567"`. |
| `String toUpperCase(String s)`, `String toLowerCase(String s)` | `s` with ASCII letters changed. |

The `String` methods `startsWith`, `endsWith`, `indexOf`, `contains`, `replace`, `split`, `toUpperCase` and `toLowerCase` call these helpers. Call `Strings` yourself for `find`, `formatFloat` and `withCommas`, which have no `String` method. They compare substrings of `s`, so they cost a few commands per character. `startsWith`, `endsWith`, `find` and `contains` never paste the text into a command, so `"` and `\` are safe. The others build new strings by joining, which has the [joining limits](./types#string).

## `std.text`

Where the [Adventure API](./builders#adventure-api) is written: `final class Component { ... }` gives the builtin `Component` its methods (`Component.text(...)`, `c.append(...)`), and `ClickEvent`, `HoverEvent` and `TextColor` hold the static ones. A `final class` named after a builtin type (`Component`, `Entity`, `Player`, `Selector`, `BossBar` or `ItemStack`) only adds methods, whose `this` is the builtin value. `MiniMessage.miniMessage().deserialize(s)` runs the [MiniMessage](./builders#minimessage) parser at run time, for text players type. It applies style tags only.

## `std.vec`

`Vec3` is a record of three floats, for velocities, directions and positions. Its operations are methods, and each returns a new vector:

```mcfc
import std.vec.Vec3;

class Main {
    public static void main() {
        Player player = (Player) Selector.of("@p").getFirst();
        Vec3 look = new Vec3(player.getLookX(), player.getLookY(), player.getLookZ());
        Vec3 push = look.add(new Vec3(0.0, 1.0, 0.0)).normalize().scale(0.8);
        player.addVelocity(push.x(), push.y(), push.z());
    }
}
```

| Method | Returns |
| --- | --- |
| `add(Vec3 other)`, `sub(Vec3 other)` | The component-wise sum or difference. |
| `scale(float factor)` | This vector with every component multiplied by `factor`. |
| `dot(Vec3 other)` | The dot product, a `float`. |
| `cross(Vec3 other)` | The cross product, perpendicular to both. |
| `length()` | The length, a `float`. |
| `normalize()` | This vector scaled to length 1. The zero vector stays zero. |

## `std.bossbar`

Where the [`BossBar`](./types#bossbar) methods are written, as `final class BossBar`. The getters run `bossbar get` and read the result from the `$world_std_bossbar_BossBarRead__value` score.

## `std.item`

Where the `ItemStack` getters (`getCount()`, `getName()`, `getId()`) are written, as `final class ItemStack`. An item stack is a value, so a method only gets a copy of it: its setters stay assignments the compiler writes, and `stack.setCount(2)` changes `stack` itself.

## `std.player`

`Players.isPlayer(Entity)` is what [`e instanceof Player`](./types#entities) calls when the selector doesn't tell.

Where [`setGameMode`, `getGameMode`, `setLevel`, `giveExp`, `giveExpLevels`, `remove`, `spectate`, `stopSpectating`, the facing `teleport` and the longer `sendTitle` forms](./methods) are written, as `final class Entity` and `final class Player`. An `Entity` method also runs on a `Player` and on a `Selector`, where it applies to every match. Call the methods; there's nothing to import.

## `std.attribute`

`Attribute` names the attributes you change most often, for `getAttribute` and `setAttribute`: `MOVEMENT_SPEED`, `JUMP_STRENGTH`, `GRAVITY`, `STEP_HEIGHT`, `SCALE`, `SAFE_FALL_DISTANCE` and `KNOCKBACK_RESISTANCE`. Pass a string ID such as `"minecraft:max_health"` for any other attribute.

## `std.color`

Colors are `0xRRGGBB` ints, the form text components, particles and display entities use.

```mcfc
import std.color.Color;

class Main {
    public static void main() {
        var orange = Color.rgb(255, 128, 0);
        var hex = Color.toHex(orange);
        var back = Color.fromHex("#FF8000");
    }
}
```

| Method | Returns |
| --- | --- |
| `int rgb(int red, int green, int blue)` | The color. Each channel is clamped to 0–255. |
| `int red(int color)`, `green`, `blue` | One channel, 0–255. |
| `String toHex(int color)` | `"#rrggbb"`, lowercase, for a text component's `color`. |
| `int fromHex(String hex)` | Reads `"#rrggbb"` or `"rrggbb"` in either case, or returns `-1` if `hex` isn't a hex color. |

## `std.dialog`

Shows a vanilla dialog to one player. Each button runs `/trigger <command>`, so it reaches the [`@Command`](./statements#command) handler of that name and needs no operator permissions.

```mcfc
import std.dialog.Button;
import std.dialog.Dialog;

class Main {
    @Command("buy")
    static void buy(Player player) {
        player.sendMessage("Bought a sword");
    }

    static void openShop(Player player) {
        Dialog.menu(player, "Shop", Component.text("Pick something"), List.of(new Button("Buy sword", "buy")));
    }
}
```

| Method | Shows |
| --- | --- |
| `void notice(Player player, String title, Component body)` | A message with an OK button. |
| `void menu(Player player, String title, Component body, List<Button> buttons)` | A message with one button per `Button(label, command)`. Needs at least one button. |

Dialogs are sent inline with `/dialog show`, so they need no registry entries and work after a `/reload`.

## Under The Hood

`std` is ordinary MCFC source compiled into the `mcfc` binary. Its methods lower like any other module's, for example `generated/std__math__math__clamp__float__d0__entry`. Unlike your own zero-argument `static void` methods, `std` methods never get public `/function` wrappers.

