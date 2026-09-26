# Statements

MCFC uses Java syntax: blocks are `{ ... }`, statements end with `;`, and `//` and `/* ... */` are comments. Indentation has no meaning.

**Top level:** [functions](#functions) · [`record`](#record) · [`enum`](#enum) · [modules and `public`](#modules-and-public) · [`import`](#import) · [`@PlayerState`](#playerstate) · [`@EntityState`](#entitystate) · [`@EventHandler`](./events) · [`@Command`](#command) · [`@Every` / `@After`](#every-and-after)

**In a function:** [variables](#variables) · [assignment](#assignment) · [`if`](#if) · [conditional expressions](#conditional-expressions) · [`switch`](#switch) · [`while`](#while) · [`do` / `while`](#do-while) · [`for`](#for) · [`break` / `continue` / `return`](#break-continue-return) · [`async`](#async) · [`as` / `at`](#as-and-at) · [`mc`](#mc) · [`mcf`](#mcf) · [calls](#calls)

## Declarations

### Functions

```mcfc
void greet(Player player, String message) {
    player.sendMessage(message);
}
```

The return type comes first and every parameter has a type. Duplicate function or parameter names are errors. Recursion isn't supported.

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
    var quest = new Quest("Mine", 5);
    quest = new Quest(quest.name(), quest.reward() + 1);
    debug(quest.name());
}
```

`new Quest(...)` takes one argument per component, in declaration order. Records are top-level, and their body is always `{}`. Read components with accessor calls such as `quest.reward()`. To change a value, construct a new record. Record values live in command storage.

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

An enum needs at least one constant. Outside a `case` you refer to a constant as `Mode.SURVIVAL`. Constants are stored as integers starting at 0, in declaration order, so reordering them changes stored values.

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
| `import a.b.name;` | the function, record or enum `name` |
| `import a.b;` | the module `b`, so `b.name(...)` works |

- Imports are private to their module. There are no `*` imports and no renaming.
- Functions, records and modules have separate namespaces.
- `$(...)` placeholders don't apply imports, so inside them write the full path from the root, such as `$(util.twice(x))`.

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

Allowed types are `int`, `boolean`, `String`, `float` and records. The optional string is the display name of the scoreboard objective that holds `int` and `boolean` state; it defaults to the state's name. For other types it's ignored.

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

### `@Command`

```mcfc
@Command("status")
void status(Player player) {
    player.sendMessage("Ready");
}
```

Players run the command with `/trigger status`, which needs no operator permissions. The trigger objective is named after the command, so two packs with the same command name share it. The handler runs as that player, and the optional `Player` parameter is that player. Without a string, the command is named after the function. With the agent attached, `/status` also works as a real command. Commands take no arguments, and there's no tab completion.

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

The target is a local variable or a writable path, and the new value must have the same type. `+=`, `-=`, `*=`, `/=`, `%=`, `++` and `--` are statements, not expressions: `x = y++;` is an error.

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
