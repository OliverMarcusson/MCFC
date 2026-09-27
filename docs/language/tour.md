# Language Tour

The whole language on one page. Each section links to its reference entry. If you've never built a pack with MCFC, start with [Your First Pack](/guide/first-pack).

## Layout

MCFC is written like Java: blocks are `{ ... }`, statements end with `;`, and comments are `//` or `/* ... */`.

```mcfc
void main() {
    // runs on load and on /reload
    Selector.of("@a").sendMessage("loaded");
}
```

## Functions

```mcfc
int add(int a, int b) {
    return a + b;
}

void tick() {
    var n = add(1, 2);
}
```

Parameter and return types are always written out. `main` runs on load and `tick` runs every tick. Every other zero-argument `void` function can be called in game as `/function <namespace>:<name>`. Functions can be [generic](./reference/statements#generic-functions). → [Functions](./reference/statements#functions)

## Values

```mcfc
void main() {
    var count = 3;
    var speed = 1.5;
    var ready = true;
    var name = "Alex";
    var line = "$(name) has $(count)";
    var scores = List.of(1, 2, 3);
    var teams = Map.of("red", 0, "blue", 0);
    count = count + 1;
}
```

`var` infers the type, or you can write it: `int count = 3;`. An `int` widens to `float` when needed. Integer `/` rounds down, `%` takes the sign of the divisor, and `(int)` floors a float, following Minecraft number semantics. → [Types](./reference/types)

## Control flow

```mcfc
void main() {
    var hp = 12;
    if (hp < 5) {
        debug("low");
    } else if (hp < 10) {
        debug("mid");
    } else {
        debug("ok");
    }

    for (int i = 0; i < 3; i++) {
        debug("$(i)");
    }

    for (Player player : Selector.of("@a")) {
        player.addTag("seen");
    }

    while (hp > 0) {
        hp = hp - 5;
    }
}
```

There's also `switch`, `break`, `continue` and `return`. Conditional expressions (`ready ? "go" : "wait"`), value-producing `switch` expressions, and `do { ... } while (condition);` are available. → [Statements](./reference/statements)

## Records and enums

```mcfc
record Quest(String name, int reward) {
    Quest doubled() {
        return new Quest(name, reward * 2);
    }
}

enum Stage {
    NEW("New"),
    DONE("Done");

    private final String label;

    Stage(String label) {
        this.label = label;
    }

    String label() {
        return label;
    }
}

void finish(Quest quest, Stage stage) {
    Quest bonus = quest.doubled();
    switch (stage) {
        case NEW -> debug("started $(bonus.name())");
        case DONE -> debug("$(stage.label()): reward $(bonus.reward())");
    }
}
```

Create a record with `new Quest("Mine", 5)`. Records and enums can have methods, including `static` ones, and records get `==`, `equals` and `toString()` from their components. Functions and methods can be overloaded. A `class` has fields that can change, and its objects are shared by reference like Java's. Classes can extend a parent and implement interfaces, and method calls are virtual. → [`record`](./reference/statements#record), [`enum`](./reference/statements#enum), [`class`](./reference/statements#class), [overloading](./reference/statements#overloading)

## Missing values

`List.get`, `Map.get` and `findFirst` return an `Optional<T>`:

```mcfc
void main() {
    var first = List.of(4, 8).get(5).orElse(0);
    var pig = Selector.of("@e[type=minecraft:pig]").findFirst();
    if (pig.isPresent()) {
        debug("found a pig");
    }
}
```

→ [`Optional<T>`](./reference/types#optional)

## Entities and players

```mcfc
void main() {
    var player = Selector.of("@p").getFirst();
    player.sendMessage("Hi");
    player.give("minecraft:bread", 3);
    player.effect("minecraft:speed", 10, 1);
    if (player.getHealth() < 6.0) {
        player.sendTitle("Low health");
    }
    player.position.setBlock("minecraft:torch");
}
```

`Selector.of(...)` can match any number of entities, and `.getFirst()` narrows it to one. The compiler works out from the selector whether a reference is a player, and `(Player) e` asserts it. → [Entities and Players](./reference/methods)

To create customized entities, items, blocks and text before using them, use [builders](./reference/builders):

```mcfc
void main() {
    var sword = new ItemStack("minecraft:diamond_sword");
    sword.setName("Quest Blade");
    Selector.of("@p").getFirst().give(sword);
}
```

## Stored state

```mcfc
@PlayerState("Coins")
int coins;

@EntityState
String owner;

void pay(Player player) {
    player.state.coins = player.state.coins + 1;
}
```

State is stored per player or per entity and survives reloads. → [`@PlayerState`](./reference/statements#playerstate)

## Events, commands and tasks

```mcfc
@EventHandler
void onPlayerJoin(PlayerJoinEvent event) {
    Player player = event.player();
    player.sendMessage("Welcome");
}

@Command("spawn")
void spawn(Player player) {
    player.teleport(Block.of("0 64 0"));
}

@Every(ticks = 6000)
void reminder() {
    Selector.of("@a").sendActionBar("Five minutes passed");
}
```

Handlers are ordinary functions with an annotation. A `@Command` is run with `/trigger <name>`. With the optional agent there are 34 events in total, many of them cancellable. → [Events](./reference/events), [`@Command`](./reference/statements#command), [`@Every`](./reference/statements#every-and-after)

## Waiting

```mcfc
void countdown(Player player) {
    async {
        for (int i = 0; i < 3; i++) {
            player.sendTitle("$(3 - i)");
            sleep(1);
        }
        player.sendTitle("Go");
    }
}
```

`sleep` and `sleepTicks` pause the function. `async { ... }` runs its body without the caller waiting for it. → [`async`](./reference/statements#async), [Functions that pause](./reference/statements#functions-that-pause)

## Raw commands

```mcfc
void main() {
    var n = 5;
    mc("weather clear");
    mcf("xp add @a $(n) levels");
}
```

For commands MCFC has no feature for yet, `mc` emits a command exactly as written and `mcf` fills in `$(...)` values at run time. Use them only as a last resort. → [`mc`](./reference/statements#mc), [`mcf`](./reference/statements#mcf)

## Modules and std

<!-- no-check -->
```mcfc
import std.math.clamp;

void main() {
    var hp = clamp(combat.damage(), 0, 20);
}
```

Each file is a module named by its path (`src/combat.mcf` is `combat`), and `combat.damage()` calls into it. Items are private unless marked `public`. `std` is always available. → [Modules](./reference/statements#modules-and-public), [`import`](./reference/statements#import), [std](./reference/std)

## Outside the game <Badge type="danger" text="mcfd" title="Needs the mcfd helper running beside the server. Not available on Realms." />

With the optional `mcfd` helper, a pack can make HTTP requests, read and write files, and query SQLite:

```mcfc
void motd(Player player) {
    var r = http.get("https://api.example.com/motd");
    if (r.ok()) {
        player.sendMessage(r.body());
    }
}
```

→ [Host Bridge](/runtime/host-bridge)

## What's missing

See [Limitations](./limitations).
