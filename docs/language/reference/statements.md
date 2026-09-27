# Statements

MCFC uses Java syntax: blocks are `{ ... }`, statements end with `;`, and `//` and `/* ... */` are comments. Indentation has no meaning.

**Top level:** [functions](#functions) · [overloading](#overloading) · [`record`](#record) · [`enum`](#enum) · [`class`](#class) · [modules and `public`](#modules-and-public) · [`import`](#import) · [`@PlayerState`](#playerstate) · [`@EntityState`](#entitystate) · [`@EventHandler`](./events) · [`@Command`](#command) · [`@Every` / `@After`](#every-and-after)

**In a function:** [variables](#variables) · [assignment](#assignment) · [`if`](#if) · [conditional expressions](#conditional-expressions) · [`switch`](#switch) · [`while`](#while) · [`do` / `while`](#do-while) · [`for`](#for) · [`break` / `continue` / `return`](#break-continue-return) · [`async`](#async) · [`as` / `at`](#as-and-at) · [`mc`](#mc) · [`mcf`](#mcf) · [calls](#calls)

## Declarations

### Functions

```mcfc
void greet(Player player, String message) {
    player.sendMessage(message);
}
```

The return type comes first and every parameter has a type. Duplicate function or parameter names are errors. Functions may call themselves or each other recursively, but a recursive function can't pause (see [Functions that pause](#functions-that-pause)).

Two names are special:

- `void main()` in the root module runs every time the datapack loads, including on `/reload`. It's added to the Lantern Load `load:load` tag, which `minecraft:load` runs. Each load also sets the score `<namespace>` in `load.status` to 1, so other packs can check that yours loaded.
- `void tick()` runs every game tick through the `minecraft:tick` tag. If several modules define `tick`, their bodies all run, in module order. A `tick` that takes parameters is an ordinary function.

Every other zero-argument `void` function is also exported as `/function <namespace>:<name>`, so you can call it from chat. Function paths are lowercase: `resetArena` is exported as `<namespace>:reset_arena`.

#### Generic functions

Type parameters go in `<...>` before the return type. A call infers them from its arguments:

```mcfc
<T> T biggest(List<T> values) {
    var best = values.getFirst();
    for (var value : values) {
        if (value > best) {
            best = value;
        }
    }
    return best;
}

void main() {
    var a = biggest(List.of(3, 9, 2));
    var b = biggest(List.of(1.5, 0.25));
}
```

Every type parameter must appear in a parameter's type, because there's no `f<int>(...)` call syntax. Arguments bound to the same parameter must agree: for `<T> boolean same(T a, T b)`, `same(1, "x")` is an error. There are no bounds. Each combination of types compiles to its own copy (`biggest__int`, `biggest__float`), and each copy is type-checked on its own. So `biggest(List.of("a", "b"))` reports that `>` needs numbers, plus "'biggest' does not work with T = String" at the call. Records can't be generic.

#### Overloading

Functions and methods can share a name when their parameter types differ:

```mcfc
int area(int side) {
    return side * side;
}

float area(float width, float height) {
    return width * height;
}

void main() {
    int square = area(3);
    float rect = area(2, 1.5);
}
```

A call picks the overload whose parameters match the argument types exactly, then one the arguments convert to (`int` to `float`), then a generic one. Two matches at the same step are an ambiguous call. Each overload compiles to its own function, `area__int` and `area__float__float`. Only a zero-parameter overload keeps the plain name, so it's the one `/function` exports.

#### Functions that pause

A function that calls `sleep`, `sleepTicks`, `sort()` or a host call pauses, and so does any function that calls it. The caller continues once the callee is done. Because of that, a call to a pausing function has to be a statement of its own: `f();`, `var x = f();`, `x = f();` or `return f();`. Using it inside a condition or a larger expression is an error.

```mcfc
int waitThenDouble(int n) {
    sleepTicks(20);
    return n * 2;
}

void main() {
    var x = waitThenDouble(4);
    debug("one second later, x is $(x)");
}
```

### `record`

```mcfc
record Quest(String name, int reward) {}

void main() {
    Quest quest = new Quest("Mine", 5);
    quest = new Quest(quest.name(), quest.reward() + 1);
    debug(quest.name());
}
```

`new Quest(...)` takes one argument per component, in declaration order. Records are top-level. Read components with accessor calls such as `quest.reward()`. To change a value, construct a new record. Record values live in command storage and are copied when assigned or passed.

A record body can declare methods:

```mcfc
record Point(int x, int y) {
    static Point origin() {
        return new Point(0, 0);
    }

    Point add(Point other) {
        return new Point(x + other.x(), y + other.y());
    }

    int manhattan() {
        return abs(x) + abs(this.y);
    }

    private int abs(int value) {
        return value < 0 ? -value : value;
    }
}

void main() {
    Point moved = Point.origin().add(new Point(3, -4));
    debug("$(moved) is $(moved.manhattan()) away");
}
```

- Inside a method, `this` is the record. A component reads as `x`, `this.x` or `x()`, and other methods can be called without `this.`.
- A `static` method has no `this` and is called on the type: `Point.origin()`.
- Methods follow the same visibility rule as functions: without `public`, only the record's module and the modules below it can call them. A method of a private record is private.
- Methods can be generic and overloaded, and a method can't be named like a component, since that name is the accessor.
- Records can't declare fields or constructors.

`==`, `!=` and `equals(other)` compare every component. `toString()`, `+` and `$(...)` give Java's record text, `Point[x=3, y=-4]`. Declare `toString()` or `equals(Point other)` in the body to replace them; `@Override` is accepted on these two.

### `enum`

```mcfc
enum Mode { SURVIVAL, CREATIVE }

void describe(Mode mode) {
    switch (mode) {
        case SURVIVAL -> debug("Survival");
        case CREATIVE -> debug("Creative");
    }
}
```

An enum needs at least one constant. Outside a `case` and outside the enum's own methods, you refer to a constant as `Mode.SURVIVAL`. Constants are stored as integers starting at 0, in declaration order, so reordering them changes stored values. `mode.name()` is the constant's name, `mode.ordinal()` its index, and `Mode.values()` lists every constant.

After the constants and a `;`, an enum can declare `final` fields, one constructor and methods:

```mcfc
enum Planet {
    MERCURY(3, 2),
    EARTH(6, 5);

    private final int mass;
    private final int radius;

    Planet(int mass, int radius) {
        this.mass = mass;
        this.radius = radius;
    }

    int density() {
        return mass * 10 / radius;
    }

    boolean isHome() {
        return this == EARTH;
    }
}

void main() {
    for (Planet planet : Planet.values()) {
        debug("$(planet) $(planet.density()) $(planet.mass)");
    }
}
```

Each constant passes one argument per constructor parameter. The constructor can only assign parameters to fields (`this.mass = mass;`), and every field must be assigned. Nothing is stored for a field: reading `planet.mass` compiles to a switch over the constants, so the arguments are best kept to literals.

### `class`

```mcfc
class Counter {
    static int created;
    private int count;
    String label = "hits";

    Counter(int start) {
        count = start;
        created += 1;
    }

    Counter add(int amount) {
        count += amount;
        return this;
    }

    int get() {
        return count;
    }
}

class Node {
    int value;
    Node next;

    Node(int value) {
        this.value = value;
    }
}

void main() {
    Counter hits = new Counter(5);
    Counter same = hits;
    same.add(1).add(2);
    debug("$(hits.get()) $(Counter.created)");

    Node head = new Node(1);
    head.next = new Node(2);
    head.next.value = 20;
    if (head.next.next == null) {
        debug("two nodes");
    }
}
```

Unlike records, class objects are shared: assigning or passing one copies a reference, so `same.add(1)` above changes `hits` too. `==` and `!=` compare references.

- Fields can have an initializer. A field without one starts as `0`, `false`, `""`, `0.0`, `null`, or an empty list or map. Initializers run before the constructor body.
- A class without a constructor gets one that takes no arguments. Constructors can be overloaded like methods.
- Inside a class, `this` is the object. A field reads and writes as `count` or `this.count`, and methods can be called without `this.`.
- `static` fields belong to the class: `Counter.created`, or `created` inside it. They are world state, so they keep their value across reloads. Their initializers run once per world.
- A `final` field can only be set by its initializer or a constructor, and a `static final` field only by its initializer.
- Fields, constructors and methods follow the same visibility rule as functions: without `public`, only the class's module and the modules below it can use them.
- `null` is a reference to no object, and fits any class type. The default `toString()` gives `Counter@3`, or `null`. Declare `toString()` or `equals(...)` to replace them.
- Objects can be kept in locals, parameters, lists, maps, records, and all three kinds of state.

Objects live in one storage list, and a reference is the object's index there. Each field read or write is a macro call, so keep an object's values in locals inside hot loops. Objects nothing refers to anymore are freed at the end of a tick, once more objects were made since the last collection than survived it. A collection walks every live object and all player and entity state holding objects, so its cost grows with how much of both is kept.

### Modules and `public`

In a project, every `.mcf` file under the source directory is a module named by its path: `src/util.mcf` is `util`, and `src/game/score.mcf` is `game.score`. The root file (`src/main.mcf`) is the root module. There is no module declaration.

<!-- no-check -->
```mcfc
// src/main.mcf
void main() {
    var n = util.twice(21);
}
```

<!-- no-check -->
```mcfc
// src/util.mcf
public int twice(int x) {
    return helper(x) * 2;
}

int helper(int x) {
    return x;
}
```

- Items are private unless marked `public`. A private item can be used by its own module and the modules below it: `game.score` can use private items of `game`, but not the other way round.
- `public` works on functions, records and enums.
- Every module can reach every other module by path, like Java packages. A path's first segment is looked up in the current module, then in the root module.
- `tick`, `@EventHandler`, `@Command`, `@Every` and `@After` handlers work in any module and ignore `public`. `main` is only special in the root module.
- The name `std` is reserved for the [standard library](./std).

A function in a module compiles under its full path. A zero-argument `void` function `util.announce` is exported as `/function <namespace>:util/announce`.

### `import`

```mcfc
import std.math.clamp;
import std.math;

void main() {
    var hp = clamp(150, 0, 100);
    var bits = math.pow(2, 10);
}
```

| Form | Imports |
| --- | --- |
| `import a.b.name;` | the function, record, enum or class `name` |
| `import a.b;` | the module `b`, so `b.name(...)` works |
| `import a.b.*;` | every public function, record, enum and class of `a.b` |

- Imports are private to their module. There's no renaming.
- As in Java, a name defined in the module or imported by name wins over a `*` import.
- Functions, records and modules have separate namespaces.
- Calls inside `$(...)` placeholders use the same imports as calls outside them.

### `@PlayerState`

Declares a value stored per player, read and written as `player.state.<name>`:

```mcfc
record Profile(int level, String title) {}

@PlayerState("Coins")
int coins;

@PlayerState
Profile profile;

void update(Player player) {
    player.state.coins = player.state.coins + 1;
    player.state.profile = new Profile(4, "Scout");
}
```

Allowed types are `int`, `boolean`, `String`, `float`, `Entity`, `Player`, records, maps and class objects. An `Entity` is a handle, as with [`@WorldState`](#worldstate), so each player can own a camera: `player.state.camera = Block.of(0, 70, 0).summon("minecraft:item_display");`. A map in state can only be indexed by a literal key; to use a variable key, copy it, change the copy, and assign it back: `var m = player.state.kills; m.put(name, 1); player.state.kills = m;`. The optional string is the display name of the scoreboard objective that holds `int` and `boolean` state; it defaults to the state's name. For other types it's ignored.

Values persist across reloads and restarts. A value that was never set reads as `0`, `false`, `""`, `0.0` or an empty record.

You can also use `player.state.<name>` for `int` and `boolean` without declaring it. Other types have to be declared. A name can have dots (`@PlayerState int stats.kills;`), but two declarations can't overlap, for example `a` and `a.b`.

`int` and `boolean` state is stored in scoreboard objectives named `mcfs_*`. Other types are stored in `<namespace>:state` storage, keyed by the player's UUID.

### `@EntityState`

The same as `@PlayerState`, but for any `Entity` and without a display name:

```mcfc
record MarkerInfo(String label, float weight) {}

@EntityState
MarkerInfo info;

void mark(Entity entity) {
    entity.state.info = new MarkerInfo("Target", 1.5);
    debug(entity.state.info.label());
}
```

Scoreboard objectives are named `mcfe_*`. Stored values aren't removed when the entity despawns.

### `@WorldState`

One value for the whole world, such as the current round. Read and write it by name, like a variable:

```mcfc
@WorldState
int round;

@WorldState
List<Integer> topScores;

void endRound(int best) {
    round = round + 1;
    topScores.add(best);
}
```

Allowed types are `int`, `boolean`, `String`, `float`, `Entity`, `Player`, enums, records, class objects, lists and maps. Values persist across reloads and restarts. A value that was never set reads as `0`, `false`, `""`, `0.0`, an empty list, map or record. Names can't have dots, and a local variable or parameter with the same name hides the world state inside its function.

`int` and `boolean` values are the scores `$world_<name>` in the `mcfc` objective; other types are in `<namespace>:runtime` storage at `world.<name>`.

An `Entity` or `Player` world state is a handle the pack keeps to one entity, so you summon it once and don't look it up by selector later:

```mcfc
@WorldState
Entity token;

void setup() {
    token = Block.of(0, 65, 0).summon("minecraft:armor_stand");
}

void hop() {
    token.teleport(Block.of(4, 65, 0));
}
```

Assigning gives the entity a unique `mcfc_id` score and stores a selector for it. Before the first assignment, or after the entity is gone, `token.isValid()` is `false` and commands on it do nothing. Player and entity state can hold handles too.

### `@Command`

```mcfc
@Command("status")
void status(Player player) {
    player.sendMessage("Ready");
}
```

Players run the command with `/trigger status`, which needs no operator permissions. The trigger objective is named after the command, so two packs with the same command name share it. The handler runs as that player, and the optional `Player` parameter is that player. Without a string, the command is named after the function. With the agent attached, `/status` also works as a real command. Commands take no arguments, and there's no tab completion.

### `@Menu`

```mcfc
@Menu("Settings")
void settings(Player player) {
    player.sendMessage("Settings");
}
```

`@Menu("label")` is a `@Command` named after the function that is also a button in your pack's page of the pause-screen data pack menu. The menu follows the [Smithed Data Pack Menu](https://docs.smithed.dev/conventions/data-pack-menu/) convention, so every pack using it shares one list. The page is titled with the namespace.

Data pack dialogs are registry entries, so a new or changed `@Menu` needs a world or server restart, not `/reload`. For dialogs you open from code, use [`std.dialog`](./std#std-dialog), which has neither limit.

### `@Every` and `@After`

```mcfc
@Every(ticks = 20)
void heartbeat() {
    debug("heartbeat");
}

@After(seconds = 1)
void setup() {
    debug("setup");
}
```

`@Every` repeats every `n` ticks. `@After` runs once, `n` ticks after load. Both take `ticks = n` or `seconds = n` (20 ticks each). Tasks take no parameters and run as the server, not as a player. Use `for (Player player : Selector.of("@a")) { ... }` to act on each player.

### `@Test` and `assert`

```mcfc
int triple(int x) {
    return x * 3;
}

@Test
void triplesNumbers() {
    assert triple(2) == 6;
    assert triple(-1) == -3 : "negatives";
}
```

`/function <namespace>:test` runs every `@Test` function and prints `[ns TEST] 1 passed, 0 failed`. A false `assert` prints `assertion failed at line N: message` and marks the test failed; the test keeps running. Tests take no parameters and must finish in the tick they start, so they can't `sleep`.

`assert` works in any function. Outside a test it still prints the failure.

## In a function

### Variables

```mcfc
void main() {
    var amount = 5;
    List<String> names = List.of("a", "b");
    final int maximum = 10;
}
```

`var` takes its type from the initializer. With a written type, the initializer must match or widen from `int` to `float`. Every variable needs an initializer. `final` prevents later assignment and also works on parameters. Declaring a name that's already a local or parameter is an error, and a variable declared inside a block isn't visible outside it. `static` is accepted on declarations but does not change storage lifetime.

### Assignment

```mcfc
void main() {
    var amount = 1;
    amount = amount + 1;
    amount += 2;
    amount++;

    var player = Selector.of("@p").getFirst();
    player.state.score = amount;
}
```

The target is a local variable or a writable path, and the new value must have the same type. `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`, `++` and `--` are statements, not expressions: `x = y++;` is an error.

### `if`

```mcfc
void check(Player player) {
    if (player.hasTag("ready")) {
        player.sendMessage("Ready");
    } else if (player.hasTag("waiting")) {
        player.sendMessage("Waiting");
    } else {
        player.sendMessage("Not ready");
    }
}
```

The condition must be a `boolean`. Braces are required.

### Conditional expressions

```mcfc
int fee(boolean member, int price) {
    return member ? price / 2 : price;
}
```

`condition ? whenTrue : whenFalse` chooses one branch. `int` and `float` branches combine as `float`; `Player` and `Entity` branches combine as `Entity`.

### `switch`

```mcfc
void describe(int level) {
    switch (level) {
        case 1, 2 -> debug("low");
        case 3 -> {
            debug("medium");
            debug("still medium");
        }
        default -> debug("high");
    }
}
```

`switch` works on enum, `int` and `String` values. Cases are constants of that type, and a case can list several. Each case is a single statement or a `{ ... }` block. Cases don't fall through, so there's no `break`. On an enum, cases name the bare constant (`case SURVIVAL`), and the switch must either list every constant or have a `default`. The value is evaluated once, and the first matching case runs.

A switch can also produce a value. Expression arms end with `;`; a block arm returns a value with `yield`. A switch expression needs a `default` or every enum constant.

```mcfc
enum Rank { LOW, HIGH }

String label(Rank rank) {
    return switch (rank) {
        case LOW -> "low";
        case HIGH -> { yield "high"; }
    };
}
```

### `while`

```mcfc
void count() {
    var i = 0;
    while (i < 3) {
        debug("$(i)");
        i++;
    }
}
```

A loop runs entirely within one tick unless its body sleeps. A long loop with no `sleep` can hit Minecraft's command limit (`maxCommandChainLength`), and the rest of the function then doesn't run.

### `do` / `while`

```mcfc
void retry() {
    var attempts = 0;
    do {
        attempts++;
    } while (attempts < 3);
}
```

The body runs at least once. `continue` proceeds to the condition check.

### `for`

```mcfc
void loops(List<Integer> values) {
    for (int i = 0; i < 3; i++) {
        debug("$(i)");
    }
    for (Player player : Selector.of("@a")) {
        player.addTag("seen");
    }
    for (var value : values) {
        debug("$(value)");
    }
}
```

| Form | Iterates |
| --- | --- |
| `for (int i = 0; i < n; i++)` | a counting loop: the condition is checked before every iteration and the update runs after it |
| `for (var e : Selector.of(...))` | each matching entity, as an `Entity`. Runs through `execute as`, so `@s` is the current entity. Write `Player e` to get a `Player`; the selector must be able to match players. |
| `for (var x : list)` | each element |

The loop variable exists only inside the loop. In a counting loop, `continue` runs the update before the next check, like Java.

### `break`, `continue`, `return`

```mcfc
void firstReady() {
    for (Player player : Selector.of("@a")) {
        if (!player.hasTag("ready")) {
            continue;
        }
        player.sendMessage("Ready");
        break;
    }
}

int clampZero(int value) {
    if (value < 0) {
        return 0;
    }
    return value;
}
```

`break` and `continue` apply to the innermost loop. In a `void` function, use `return;` on its own. Otherwise, `return expr;` must match the declared return type.

### `async`

```mcfc
void greetLater(Player player) {
    async {
        sleepTicks(20);
        player.sendActionBar("later");
    }
    player.sendActionBar("now");
}
```

The body starts running right away, and the statement after the block runs without waiting for it. Local variables are copied when the block starts, so later changes in the parent don't reach the copy. `return` isn't allowed inside `async`.

### `as` and `at`

```mcfc
void sparkle(Player player) {
    as (player) {
        Selector.of("@s").getFirst().addTag("marked");
    }
    at (player) {
        Block.of("~ ~1 ~").spawnParticle("minecraft:happy_villager", 8);
        Block.of("~ ~-1 ~").setBlock("minecraft:gold_block");
    }
}
```

`as` changes who `@s` is, and `at` changes where `~ ~ ~` is. The anchor can be an `Entity` or a `Selector`, in which case the body runs once per entity. These compile to `execute as` and `execute at`. To build a selector relative to an entity, use the function forms [`as(...)` and `at(...)`](./builtins#as-and-at).

### `mc`

`mc` and `mcf` are for Minecraft commands MCFC has no feature for yet. Prefer a method or builtin when one exists; missing features are tracked in [`TODO.md`](https://github.com/OliverMarcusson/MCFC/blob/main/TODO.md).

```mcfc
void setup() {
    mc("weather clear");
}
```

Emits a Minecraft command exactly as written. The argument must be a string literal. `$(...)` isn't interpreted, so `mc("say $(x)");` prints `$(x)` literally.

### `mcf`

```mcfc
void reward(int amount) {
    mcf("xp add @a $(amount) levels");
}
```

Emits a command with each `$(expr)` replaced by its value at run time. It compiles to a Minecraft function macro. Values are copied into storage, and then `function ... with storage ...` is called. Use `mc` when there's nothing to substitute.

Values are inserted without escaping. A string containing `"` breaks the command, as described under [string limits](./types#string).

### Calls

A function or method call can be a statement on its own, such as `debug("ok");` or `player.heal(2);`. Other expressions can't: `amount + 1;` is an error.
