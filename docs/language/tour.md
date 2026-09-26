# Language Tour

The whole language on one page. Each section links to its reference entry. If you've never built a pack with MCFC, start with [Your First Pack](/guide/first-pack).

## Layout

MCFC is written like Java: blocks are `{ ... }`, statements end with `;`, and comments are `//` or `/* ... */`.

```mcfc
void main() {
    // runs on load and on /reload
    Selector.of("@a").tellraw("loaded");
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

`var` infers the type, or you can write it: `int count = 3;`. Later assignments must keep the type. Types never convert implicitly: `1.5 + 2` is an error, and `1.5 + (float) 2` is correct. `/` rounds down, the same as scoreboard math. → [Types](./reference/types)

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

There's also `switch`, `break`, `continue` and `return`. → [Statements](./reference/statements)

## Records and enums

```mcfc
record Quest(String name, int reward) {}

enum Stage { NEW, DONE }

void finish(Quest quest, Stage stage) {
    switch (stage) {
        case NEW -> debug("started $(quest.name)");
        case DONE -> debug("reward $(quest.reward)");
    }
}
```

Create a record with `new Quest("Mine", 5)`. → [`record`](./reference/statements#record), [`enum`](./reference/statements#enum)

## Missing values

`List.get`, `Map.get` and `findFirst` return an `Optional<T>`:

```mcfc
void main() {
    var first = List.of(4, 8).get(5).orElse(0);
    var pig = findFirst(Selector.of("@e[type=minecraft:pig]"));
    if (pig.isPresent()) {
        debug("found a pig");
    }
}
```

→ [`Optional<T>`](./reference/types#optional)

## Entities and players

```mcfc
void main() {
    var player = single(Selector.of("@p"));
    player.tellraw("Hi");
    player.give("minecraft:bread", 3);
    player.effect("minecraft:speed", 10, 1);
    if (player.health() < 6.0) {
        player.title("Low health");
    }
    player.position.setblock("minecraft:torch");
}
```

`Selector.of(...)` can match any number of entities, and `single(...)` narrows it to one. The compiler works out from the selector whether a reference is a player, and `(Player) e` asserts it. → [Entities and Players](./reference/methods)

To create customized entities, items, blocks and text before using them, use [builders](./reference/builders):

```mcfc
void main() {
    var sword = new ItemStack("minecraft:diamond_sword");
    sword.name = "Quest Blade";
    single(Selector.of("@p")).give(sword);
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
    Player player = event.player;
    player.tellraw("Welcome");
}

@Command("spawn")
void spawn(Player player) {
    player.teleport(Block.of("0 64 0"));
}

@Every(ticks = 6000)
void reminder() {
    Selector.of("@a").actionbar("Five minutes passed");
}
```

Handlers are ordinary functions with an annotation. A `@Command` is run with `/trigger <name>`. With the optional agent there are 34 events in total, many of them cancellable. → [Events](./reference/events), [`@Command`](./reference/statements#command), [`@Every`](./reference/statements#every-and-after)

## Waiting

```mcfc
void countdown(Player player) {
    async {
        for (int i = 0; i < 3; i++) {
            player.title("$(3 - i)");
            sleep(1);
        }
        player.title("Go");
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
    if (r.ok) {
        player.tellraw(r.body);
    }
}
```

→ [Host Bridge](/runtime/host-bridge)

## What's missing

See [Limitations](./limitations).
