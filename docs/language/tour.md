# Language Tour

The whole language on one page. Each section links to its reference entry. If you've never built a pack with MCFC, start with [Your First Pack](/guide/first-pack).

## Layout

MCFC is written like Java: blocks are `{ ... }`, statements end with `;`, and comments are `//` or `/* ... */`. Code lives in classes. There are no top-level functions or variables: every method and every piece of state belongs to a class, and a pack starts at `public static void main()`.

```mcfc
class Main {
    public static void main() {
        // runs on load and on /reload
        Selector.of("@a").sendMessage("loaded");
    }
}
```

## Classes

```mcfc
class Counter {
    private int count;

    Counter(int start) {
        this.count = start;
    }

    void add(int amount) {
        count = count + amount;
    }

    int get() {
        return count;
    }
}

class Main {
    static int rounds = 0;

    public static void main() {
        var counter = new Counter(10);
        counter.add(5);
        rounds = rounds + 1;
        debug("$(counter.get()) after $(rounds) rounds");
    }

    @Tick
    static void tick() {
        rounds = rounds + 1;
    }
}
```

A class has fields, constructors and methods. Objects are created with `new` and shared by reference, like Java's. A `static` field is one value for the whole world and survives reloads. A `static` method is called on the class, as in `Main.tick()`, or without the class name from inside it. Instance methods see `this`. Classes can extend a parent, implement interfaces and override methods, and calls are virtual. → [`class`](./reference/statements#class)

`public static void main()` runs on load, and a `@Tick` method runs every tick. Every other `static void` method with no parameters can also be run in game as `/function <namespace>:<class>/<method>`. → [Entry points](./reference/statements#entry-points)

## Methods

```mcfc
class Main {
    static int add(int a, int b) {
        return a + b;
    }

    static float add(float a, float b) {
        return a + b;
    }

    static <T> T pick(boolean first, T a, T b) {
        return first ? a : b;
    }

    public static void main() {
        var n = add(1, 2);
        var name = pick(true, "Alex", "Sam");
    }
}
```

Parameter and return types are always written out. Methods can be overloaded and [generic](./reference/statements#generic-methods). → [Methods](./reference/statements#methods)

## Values

```mcfc
class Main {
    public static void main() {
        var count = 3;
        var speed = 1.5;
        var ready = true;
        var name = "Alex";
        var line = "$(name) has $(count)";
        var scores = List.of(1, 2, 3);
        var teams = Map.of("red", 0, "blue", 0);
        count = count + 1;
    }
}
```

`var` infers the type, or you can write it: `int count = 3;`. An `int` widens to `float` when needed. Integer `/` rounds down, `%` takes the sign of the divisor, and `(int)` floors a float, following Minecraft number semantics. → [Types](./reference/types)

## Control flow

```mcfc
class Main {
    public static void main() {
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

class Main {
    static void finish(Quest quest, Stage stage) {
        Quest bonus = quest.doubled();
        switch (stage) {
            case NEW -> debug("started $(bonus.name())");
            case DONE -> debug("$(stage.label()): reward $(bonus.reward())");
        }
    }
}
```

Create a record with `new Quest("Mine", 5)`. Records and enums can have methods, including `static` ones, and records get `==`, `equals` and `toString()` from their components. A record's fields can't change after it's made. → [`record`](./reference/statements#record), [`enum`](./reference/statements#enum)

## Missing values

`List.get`, `Map.get` and `findFirst` return an `Optional<T>`:

```mcfc
class Main {
    public static void main() {
        var first = List.of(4, 8).get(5).orElse(0);
        var pig = Selector.of("@e[type=minecraft:pig]").findFirst();
        if (pig.isPresent()) {
            debug("found a pig");
        }
    }
}
```

→ [`Optional<T>`](./reference/types#optional)

## Entities and players

```mcfc
class Main {
    public static void main() {
        var player = Selector.of("@p").getFirst();
        player.sendMessage("Hi");
        player.give("minecraft:bread", 3);
        player.effect("minecraft:speed", 10, 1);
        if (player.getHealth() < 6.0) {
            player.sendTitle("Low health");
        }
        player.position.setBlock("minecraft:torch");
    }
}
```

`Selector.of(...)` can match any number of entities, and `.getFirst()` narrows it to one. The compiler works out from the selector whether a reference is a player, and `(Player) e` asserts it. → [Entities and Players](./reference/methods)

To create customized entities, items, blocks and text before using them, use [builders](./reference/builders):

```mcfc
class Main {
    public static void main() {
        var sword = new ItemStack("minecraft:diamond_sword");
        sword.setName("Quest Blade");
        Selector.of("@p").getFirst().give(sword);
    }
}
```

## Stored state

```mcfc
class Main {
    @PlayerState("Coins")
    static int coins;

    @EntityState
    static String owner;

    static void pay(Player player) {
        player.state.coins = player.state.coins + 1;
    }
}
```

State is stored per player or per entity and survives reloads. → [`@PlayerState`](./reference/statements#playerstate)

## Events, commands and tasks

```mcfc
class Main implements Listener {
    @EventHandler
    void onPlayerJoin(PlayerJoinEvent event) {
        Player player = event.player();
        player.sendMessage("Welcome");
    }

    @Command("spawn")
    static void spawn(Player player) {
        player.teleport(Block.of("0 64 0"));
    }

    @Every(ticks = 6000)
    static void reminder() {
        Selector.of("@a").sendActionBar("Five minutes passed");
    }
}
```

Event handlers are instance methods marked `@EventHandler`, in a class that `implements Listener`. `@Command`, `@Every`, `@After`, `@Menu` and `@Tick` go on `static` methods of any class. A `@Command` is run with `/trigger <name>`. With the optional agent there are 34 events in total, many of them cancellable. → [Events](./reference/events), [`@Command`](./reference/statements#command), [`@Every`](./reference/statements#every-and-after)

## Waiting

```mcfc
class Main {
    static void countdown(Player player) {
        Thread.start(() -> {
            for (int i = 0; i < 3; i++) {
                player.sendTitle("$(3 - i)");
                sleep(1);
            }
            player.sendTitle("Go");
        });
    }
}
```

`sleep` and `sleepTicks` pause the method. `Thread.start(() -> { ... })` runs the lambda without the caller waiting for it. → [`Thread.start`](./reference/statements#thread-start), [Methods that pause](./reference/statements#methods-that-pause)

## Raw commands

```mcfc
class Main {
    public static void main() {
        var n = 5;
        mc("weather clear");
        mcf("xp add @a $(n) levels");
    }
}
```

For commands MCFC has no feature for yet, `mc` emits a command exactly as written and `mcf` fills in `$(...)` values at run time. Use them only as a last resort. → [`mc`](./reference/statements#mc), [`mcf`](./reference/statements#mcf)

## Modules and std

<!-- no-check -->
```mcfc
// src/combat.mcf
public class Combat {
    public static int damage() {
        return 30;
    }
}
```

<!-- no-check -->
```mcfc
// src/main.mcf
import combat.Combat;

class Main {
    public static void main() {
        var hp = Math.clamp(Combat.damage(), 0, 20);
    }
}
```

Each file is a module named by its path (`src/combat.mcf` is `combat`), and `import combat.Combat;` brings its class into scope. Classes and their members are private to their module unless marked `public`. The standard library is a set of classes under `std`, such as `std.timer.Timer`, and `Math` needs no import. → [Modules](./reference/statements#modules-and-public), [`import`](./reference/statements#import), [std](./reference/std)

## Outside the game <Badge type="danger" text="mcfd" title="Needs the mcfd helper running beside the server. Not available on Realms." />

With the optional `mcfd` helper, a pack can make HTTP requests, read and write files, and query SQLite:

```mcfc
class Main {
    static void motd(Player player) {
        var r = http.get("https://api.example.com/motd");
        if (r.ok()) {
            player.sendMessage(r.body());
        }
    }
}
```

→ [Host Bridge](/runtime/host-bridge)

## What's missing

See [Limitations](./limitations).
