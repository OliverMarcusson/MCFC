# Statements

MCFC uses Java syntax: blocks are `{ ... }`, statements end with `;`, and `//` and `/* ... */` are comments. Indentation has no meaning.

**Declarations:** [entry points](#entry-points) · [`class`](#class) · [methods](#methods) · [overloading](#overloading) · [`record`](#record) · [`enum`](#enum) · [`import`](#import) · [modules and `public`](#modules-and-public) · [static fields](#static-fields) · [`@PlayerState`](#playerstate) · [`@EntityState`](#entitystate) · [`@EventHandler`](./events) · [`@Command`](#command) · [`@Every` / `@After`](#every-and-after) · [`@Test`](#test-and-assert)

**In a method:** [variables](#variables) · [assignment](#assignment) · [`if`](#if) · [conditional expressions](#conditional-expressions) · [`switch`](#switch) · [`while`](#while) · [`do` / `while`](#do-while) · [`for`](#for) · [`break` / `continue` / `return`](#break-continue-return) · [`Thread.start`](#thread-start) · [`Execute.as` / `Execute.at`](#execute-as-and-execute-at) · [`Commands.run`](#commands-run) · [calls](#calls)

## Declarations

### Entry points

A pack is a set of classes. There are no top-level functions or variables, so every method and every piece of state is declared inside a class.

```mcfc
class Main {
    static int ticks;

    public static void main() {
        Selector.of("@a").sendMessage("loaded");
    }

    @Tick
    static void tick() {
        ticks = ticks + 1;
    }

    static void resetArena() {
        ticks = 0;
    }
}
```

- `public static void main()` in a class of the root module runs every time the datapack loads, including on `/reload`. It's added to the Lantern Load `load:load` tag, which `minecraft:load` runs. Each load also sets the score `<namespace>` in `load.status` to 1, so other packs can check that yours loaded. Only one class may declare `main`.
- A `static void` method marked `@Tick` runs every game tick through the `minecraft:tick` tag. Any class in any module can have one, and they all run.
- Every other `static void` method with no parameters is exported as `/function <namespace>:<class>/<method>`, so you can call it from chat. Paths are lowercase: `Main.resetArena` is exported as `<namespace>:main/reset_arena`.

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

class Main {
    public static void main() {
        Counter hits = new Counter(5);
        Counter same = hits;
        same.add(1).add(2);
        System.out.println("$(hits.get()) $(Counter.created)");

        Node head = new Node(1);
        head.next = new Node(2);
        head.next.value = 20;
        if (head.next.next == null) {
            System.out.println("two nodes");
        }
    }
}
```

Unlike records, class objects are shared: assigning or passing one copies a reference, so `same.add(1)` above changes `hits` too. `==` and `!=` compare references.

- Fields can have an initializer. A field without one starts as `0`, `false`, `""`, `0.0`, `null`, or an empty list or map. Initializers run before the constructor body.
- A class without a constructor gets one that takes no arguments. Constructors can be overloaded like methods.
- Inside a class, `this` is the object. A field reads and writes as `count` or `this.count`, and methods can be called without `this.`.
- `static` fields belong to the class: `Counter.created`, or `created` inside it. They are world state, so they keep their value across reloads. Their initializers run once per world. See [static fields](#static-fields).
- A `final` field can only be set by its initializer or a constructor, and a `static final` field only by its initializer.
- Without `public`, a class, field, constructor or method can only be used by its own module and the modules below it.
- `null` is a reference to no object, and fits any class type. The default `toString()` gives `Counter@3`, or `null`. Declare `toString()` or `equals(...)` to replace them.
- Objects can be kept in locals, parameters, lists, maps, records, and all three kinds of state.

Objects live in one storage list, and a reference is the object's index there. Each field read or write is a macro call, so keep an object's values in locals inside hot loops. Objects nothing refers to anymore are freed at the end of a tick, once more objects were made since the last collection than survived it. A collection walks every live object and all player and entity state holding objects, so its cost grows with how much of both is kept.

#### Inheritance and interfaces

```mcfc
interface Named {
    String name();

    default String greet() {
        return "hi " + name();
    }
}

abstract class Animal implements Named {
    int legs;

    Animal(int legs) {
        this.legs = legs;
    }

    abstract int speed();
}

class Dog extends Animal {
    Dog() {
        super(4);
    }

    int speed() {
        return 30;
    }

    public String name() {
        return "dog";
    }
}

class Puppy extends Dog {
    @Override
    int speed() {
        return super.speed() / 2;
    }
}

class Main {
    public static void main() {
        Animal pet = new Puppy();
        System.out.println("$(pet.speed()) $(pet.greet())");
        if (pet instanceof Dog dog && dog.legs == 4) {
            Dog same = (Dog) pet;
        }
    }
}
```

- A class `extends` one parent and `implements` any number of interfaces. It has its parent's fields and methods, and a `Dog` fits wherever an `Animal` or a `Named` is expected.
- A constructor calls `super(...)` first to run the parent's constructor. Without one, the parent's constructor without arguments runs.
- Calls are virtual, like Java's: `pet.speed()` runs `Puppy`'s `speed`. `super.speed()` runs the parent's.
- An `abstract` class can't be created with `new`, and its `abstract` methods have no body. Every class that isn't abstract must implement them.
- Interface methods are abstract and `public` unless they are `default`, `static` or `private`. Interface fields are constants.
- `@Override` checks that a method overrides one from a parent class or interface. An override returns the same type as the method it overrides.
- A `final` class can't be extended. A `sealed` class or interface lists what may extend it: `sealed interface Shape permits Circle, Square`.
- `x instanceof Dog` is `false` for `null`. `x instanceof Dog dog` also declares `dog`, in an `if` condition or in an `&&` chain inside one; it matches a variable or a field.
- A cast `(Dog) pet` isn't checked while the pack runs, so test with `instanceof` first.

A call to a method that some subclass overrides costs one extra function call, plus one compare for each class whose version differs. Other calls cost the same as before.

#### Generic classes

```mcfc
class Pair<A, B> {
    A first;
    B second;

    Pair(A first, B second) {
        this.first = first;
        this.second = second;
    }

    Pair<B, A> swap() {
        return new Pair<>(second, first);
    }
}

interface Shape {
    int area();
}

class Holder<T extends Shape> {
    List<T> shapes = List.of();

    int total() {
        int sum = 0;
        for (T shape : shapes) {
            sum += shape.area();
        }
        return sum;
    }
}

class Main {
    public static void main() {
        Pair<String, Integer> pair = new Pair<>("a", 1);
        Pair<Integer, String> swapped = pair.swap();
    }
}
```

- Type parameters go after the class or interface name, and a bound after `extends`: `class Holder<T extends Shape>`. `Holder<Integer>` is then an error.
- A generic class is always written with its type arguments, `Pair<String, Integer>`. `new Pair<>(...)` takes them from the declared type of a variable, a `return`, or the parameter it's passed to.
- Classes can extend or implement generic types: `class Doubler implements Function<Integer, Integer>`, or `class Counted<T> extends Cell<T>`. A generic method takes them too, and infers `T` from a class that implements its parameter's type.
- Like generic methods, each set of type arguments compiles its own copy of the class (`Pair__string__int`). The copies are unrelated classes: a `Pair<String, Integer>` isn't a `Pair<Integer, String>`, and there are no wildcards (`Pair<?, ?>`).
- A generic class can't have `static` fields, since each copy would get its own. Static methods are fine.

#### Lambdas and method references

```mcfc
interface IntOp {
    int apply(int x);
}

class Counter {
    int count = 0;

    IntOp adder() {
        return x -> x + count;
    }

    int twice(int x) {
        return x * 2;
    }
}

class Main {
    static int applyTwice(IntOp op, int x) {
        return op.apply(op.apply(x));
    }

    public static void main() {
        int offset = 10;
        IntOp addOffset = x -> x + offset;
        IntOp block = (int x) -> {
            int tripled = x * 3;
            return tripled - 1;
        };
        Counter counter = new Counter();
        IntOp doubler = counter::twice;
        int result = applyTwice(x -> x * 10, 3) + addOffset.apply(5) + block.apply(2) + doubler.apply(4);
    }
}
```

- A lambda's type is a functional interface: an interface with exactly one abstract method. It takes that type from the variable it's assigned to, the `return` it's in, or the parameter it's passed to. `(x -> x).apply(1)` has no type and is an error.
- Parameter types can be written, `(int x) -> ...`, or left out. A body is an expression, an assignment such as `() -> count += 1`, or a block.
- A lambda captures the local variables it uses by copying them when it's made, so it can't assign to them. It can change the fields of `this`, which it keeps a reference to.
- Method references: `Tools::square` (a static method), `String::length` (called on the first argument), `this::twice` or `counter::twice` (called on that object), and `Point::new` (a constructor).
- A generic method infers its type arguments from a lambda's result: with `<T, R> List<R> mapAll(List<T> values, Mapper<T, R> mapper)`, `mapAll(numbers, n -> "n" + n)` is a `List<String>`.
- Each lambda compiles to its own class implementing the interface, so calling one is a virtual call like any other.

### Methods

```mcfc
class Main {
    static void greet(Player player, String message) {
        player.sendMessage(message);
    }
}
```

Methods are declared in a class, record, enum or interface. The return type comes first and every parameter has a type. A `static` method belongs to the class and is called as `Main.greet(...)`, or as `greet(...)` from inside the class. A method without `static` is called on an object and sees it as `this`. Duplicate parameter names are errors. Methods may call themselves or each other recursively, but a recursive method can't pause (see [Methods that pause](#methods-that-pause)).

#### Generic methods

Type parameters go in `<...>` before the return type. A call infers them from its arguments:

```mcfc
class Main {
    static <T> T biggest(List<T> values) {
        var best = values.getFirst();
        for (var value : values) {
            if (value > best) {
                best = value;
            }
        }
        return best;
    }

    public static void main() {
        var a = biggest(List.of(3, 9, 2));
        var b = biggest(List.of(1.5, 0.25));
    }
}
```

When the arguments can't tell, write the type arguments out, as in Java: `none<String>()` for `<T> List<T> none()`, or `Util.<Integer>pick(2)` for a static or instance method. Written type arguments replace inference, so each argument must then fit them. Arguments bound to the same parameter must agree: for `<T> boolean same(T a, T b)`, `same(1, "x")` is an error. A bound limits a parameter to a class and its subtypes: `<T extends Animal> T fastest(List<T> animals)`. Each combination of types compiles to its own copy (`Main__biggest__int`, `Main__biggest__float`), and each copy is type-checked on its own. So `biggest(List.of("a", "b"))` reports that `>` needs numbers, plus "'Main.biggest' does not work with T = String" at the call. Records can't be generic.

#### Varargs

```mcfc
class Main {
    static int sum(int... values) {
        int total = 0;
        for (int value : values) {
            total += value;
        }
        return total;
    }

    public static void main() {
        int none = sum();
        int some = sum(1, 2, 3);
        int listed = sum(List.of(4, 5));
    }
}
```

The last parameter can take any number of arguments, as a `List`. Passing a `List` there passes it as it is.

#### Overloading

Methods can share a name when their parameter types differ:

```mcfc
class Main {
    static int area(int side) {
        return side * side;
    }

    static float area(float width, float height) {
        return width * height;
    }

    public static void main() {
        int square = area(3);
        float rect = area(2, 1.5);
    }
}
```

A call picks the overload whose parameters match the argument types exactly, then one the arguments convert to (`int` to `float`), then a generic one. Two matches at the same step are an ambiguous call. Each overload compiles to its own function, `Main__area__int` and `Main__area__float__float`. Only a zero-parameter overload keeps the plain name, so it's the one `/function` exports.

#### Methods that pause

A method that calls `Thread.sleep`, `Thread.sleepTicks`, `sort()` or a host call pauses, and so does any method that calls it. The caller continues once the callee is done. Because of that, a call to a pausing method has to be a statement of its own: `f();`, `var x = f();`, `x = f();` or `return f();`. Using it inside a condition or a larger expression is an error.

```mcfc
class Main {
    static int waitThenDouble(int n) {
        Thread.sleepTicks(20);
        return n * 2;
    }

    public static void main() {
        var x = waitThenDouble(4);
        System.out.println("one second later, x is $(x)");
    }
}
```

### `record`

```mcfc
record Quest(String name, int reward) {}

class Main {
    public static void main() {
        Quest quest = new Quest("Mine", 5);
        quest = new Quest(quest.name(), quest.reward() + 1);
        System.out.println(quest.name());
    }
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

class Main {
    public static void main() {
        Point moved = Point.origin().add(new Point(3, -4));
        System.out.println("$(moved) is $(moved.manhattan()) away");
    }
}
```

- Inside a method, `this` is the record. A component reads as `x`, `this.x` or `x()`, and other methods can be called without `this.`.
- A `static` method has no `this` and is called on the type: `Point.origin()`.
- Without `public`, only the record's module and the modules below it can call them. A method of a private record is private.
- Methods can be generic and overloaded, and a method can't be named like a component, since that name is the accessor.
- Records can't declare fields or constructors.

`==`, `!=` and `equals(other)` compare every component. `toString()`, `+` and `$(...)` give Java's record text, `Point[x=3, y=-4]`. Declare `toString()` or `equals(Point other)` in the body to replace them; `@Override` is accepted on these two.

### `enum`

```mcfc
enum Mode { SURVIVAL, CREATIVE }

class Main {
    static void describe(Mode mode) {
        switch (mode) {
            case SURVIVAL -> System.out.println("Survival");
            case CREATIVE -> System.out.println("Creative");
        }
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

class Main {
    public static void main() {
        for (Planet planet : Planet.values()) {
            System.out.println("$(planet) $(planet.density()) $(planet.mass)");
        }
    }
}
```

Each constant passes one argument per constructor parameter. The constructor can only assign parameters to fields (`this.mass = mass;`), and every field must be assigned. Nothing is stored for a field: reading `planet.mass` compiles to a switch over the constants, so the arguments are best kept to literals.

### `import`

```mcfc
import std.timer.Timer;

class Main {
    public static void main() {
        var hp = Math.clamp(150, 0, 100);
        Timer.start("round", 200);
    }
}
```

`Math` and the other [builtin classes](./builtins) need no import.

| Form | Imports |
| --- | --- |
| `import a.b.Name;` | the class, record, enum or interface `Name` |
| `import a.b.*;` | every public class, record, enum and interface of `a.b` |

- Imports are private to their module. There's no renaming.
- As in Java, a name defined in the module or imported by name wins over a `*` import.
- Calls inside `$(...)` placeholders use the same imports as calls outside them.

### Modules and `public`

In a project, every `.mcf` file under the source directory is a module named by its path: `src/util.mcf` is `util`, and `src/game/score.mcf` is `game.score`. The root file (`src/main.mcf`) is the root module. There is no package declaration.

<!-- no-check -->
```mcfc
// src/main.mcf
import util.Util;

class Main {
    public static void main() {
        var n = Util.twice(21);
    }
}
```

<!-- no-check -->
```mcfc
// src/util.mcf
public class Util {
    public static int twice(int x) {
        return helper(x) * 2;
    }

    static int helper(int x) {
        return x;
    }
}
```

- Classes, records, enums, interfaces and their members are private unless marked `public`. A private one can be used by its own module and the modules below it: `game.score` can use private items of `game`, but not the other way round.
- Every module can reach every other module by path, like Java packages: `util.Util.twice(21)` works without an import. A path's first segment is looked up in the current module, then in the root module.
- `@Tick`, `@EventHandler`, `@Command`, `@Every` and `@After` methods work in any module and ignore `public`. `main` is only special in the root module.
- The name `std` is reserved for the [standard library](./std).

A method compiles under its module path and class. A zero-argument `static void` method `util.Util.announce` is exported as `/function <namespace>:util/util/announce`.

### Static fields

A `static` field is one value for the whole world, such as the current round. There is no separate world-state annotation. Inside the class, read and write it by name; elsewhere, as `Main.round`:

```mcfc
class Main {
    static int round;

    static List<Integer> topScores;

    static void endRound(int best) {
        round = round + 1;
        topScores.add(best);
    }
}
```

Allowed types are `int`, `boolean`, `String`, `float`, `Entity`, `Player`, enums, records, class objects, lists and maps. Values persist across reloads and restarts. A value that was never set reads as `0`, `false`, `""`, `0.0`, an empty list, map or record. A local variable or parameter with the same name hides the field inside its method. An initializer, as in `static int lives = 3;`, runs once per world, before `main`.

`int` and `boolean` values are the scores `$world_<Class>__<name>` in the `mcfc` objective; other types are in `<namespace>:runtime` storage at `world.<Class>__<name>`. A class in a module adds the module path, as in `world.game_Arena__round`.

An `Entity` or `Player` static field is a handle the pack keeps to one entity, so you summon it once and don't look it up by selector later:

```mcfc
class Main {
    static Entity token;

    static void setup() {
        token = Block.of(0, 65, 0).summon("minecraft:armor_stand");
    }

    static void hop() {
        token.teleport(Block.of(4, 65, 0));
    }
}
```

Assigning gives the entity a unique `mcfc_id` score and stores a selector for it. Before the first assignment, or after the entity is gone, `token.isValid()` is `false` and commands on it do nothing. Player and entity state can hold handles too.

### `@PlayerState`

A `static` field marked `@PlayerState` declares a value stored per player, read and written as `player.state.<name>`:

```mcfc
record Profile(int level, String title) {}

class Main {
    @PlayerState("Coins")
    static int coins;

    @PlayerState
    static Profile profile;

    static void update(Player player) {
        player.state.coins = player.state.coins + 1;
        player.state.profile = new Profile(4, "Scout");
    }
}
```

Allowed types are `int`, `boolean`, `String`, `float`, `Entity`, `Player`, records, maps and class objects. An `Entity` is a handle, as with a [static field](#static-fields), so each player can own a camera: `player.state.camera = Block.of(0, 70, 0).summon("minecraft:item_display");`. A map in state can only be indexed by a literal key; to use a variable key, copy it, change the copy, and assign it back: `var m = player.state.kills; m.put(name, 1); player.state.kills = m;`. The field's name is the state's name, and it's shared by every class: two classes can't declare the same state. The optional string is the display name of the scoreboard objective that holds `int` and `boolean` state; it defaults to the state's name. For other types it's ignored.

Values persist across reloads and restarts. A value that was never set reads as `0`, `false`, `""`, `0.0` or an empty record.

You can also use `player.state.<name>` for `int` and `boolean` without declaring it. Other types have to be declared. A name can have dots (`@PlayerState static int stats.kills;`), but two declarations can't overlap, for example `a` and `a.b`.

`int` and `boolean` state is stored in scoreboard objectives named `mcfs_*`. Other types are stored in `<namespace>:state` storage, keyed by the player's UUID.

### `@EntityState`

The same as `@PlayerState`, but for any `Entity` and without a display name:

```mcfc
record MarkerInfo(String label, float weight) {}

class Main {
    @EntityState
    static MarkerInfo info;

    static void mark(Entity entity) {
        entity.state.info = new MarkerInfo("Target", 1.5);
        System.out.println(entity.state.info.label());
    }
}
```

Scoreboard objectives are named `mcfe_*`. Stored values aren't removed when the entity despawns.

### `@Command`

```mcfc
class Main {
    @Command("status")
    static void status(Player player) {
        player.sendMessage("Ready");
    }
}
```

Players run the command with `/trigger status`, which needs no operator permissions. The trigger objective is named after the command, so two packs with the same command name share it. The handler runs as that player, and the optional `Player` parameter is that player. Without a string, the command is named after the method. With the agent attached, `/status` also works as a real command. Commands take no arguments, and there's no tab completion.

### `@Menu`

```mcfc
class Main {
    @Menu("Settings")
    static void settings(Player player) {
        player.sendMessage("Settings");
    }
}
```

`@Menu("label")` is a `@Command` named after the method that is also a button in your pack's page of the pause-screen data pack menu. The menu follows the [Smithed Data Pack Menu](https://docs.smithed.dev/conventions/data-pack-menu/) convention, so every pack using it shares one list. The page is titled with the namespace.

Data pack dialogs are registry entries, so a new or changed `@Menu` needs a world or server restart, not `/reload`. For dialogs you open from code, use [`std.dialog`](./std#std-dialog), which has neither limit.

### `@Every` and `@After`

```mcfc
class Main {
    @Every(ticks = 20)
    static void heartbeat() {
        System.out.println("heartbeat");
    }

    @After(seconds = 1)
    static void setup() {
        System.out.println("setup");
    }
}
```

`@Every` repeats every `n` ticks. `@After` runs once, `n` ticks after load. Both take `ticks = n` or `seconds = n` (20 ticks each). Tasks take no parameters and run as the server, not as a player. Use `for (Player player : Selector.of("@a")) { ... }` to act on each player.

### `@Test` and `assert`

```mcfc
class Main {
    static int triple(int x) {
        return x * 3;
    }

    @Test
    static void triplesNumbers() {
        assert triple(2) == 6;
        assert triple(-1) == -3 : "negatives";
    }
}
```

`/function <namespace>:test` runs every `@Test` method and prints `[ns TEST] 1 passed, 0 failed`. A false `assert` prints `assertion failed at line N: message` and marks the test failed; the test keeps running. Tests take no parameters and must finish in the tick they start, so they can't `Thread.sleep`.

`assert` works in any method. Outside a test it still prints the failure.

## In a method

### Variables

```mcfc
class Main {
    public static void main() {
        var amount = 5;
        List<String> names = List.of("a", "b");
        final int maximum = 10;
    }
}
```

`var` takes its type from the initializer. With a written type, the initializer must match or widen from `int` to `float`. Every variable needs an initializer. `final` prevents later assignment and also works on parameters. Declaring a name that's already a local or parameter is an error, and a variable declared inside a block isn't visible outside it. `static` is accepted on declarations but does not change storage lifetime.

### Assignment

```mcfc
class Main {
    public static void main() {
        var amount = 1;
        amount = amount + 1;
        amount += 2;
        amount++;

        var player = Selector.of("@p").getFirst();
        player.state.score = amount;
    }
}
```

The target is a local variable or a writable path, and the new value must have the same type. `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`, `<<=`, `>>=`, `++` and `--` are statements, not expressions: `x = y++;` is an error.

### `if`

```mcfc
class Main {
    static void check(Player player) {
        if (player.hasTag("ready")) {
            player.sendMessage("Ready");
        } else if (player.hasTag("waiting")) {
            player.sendMessage("Waiting");
        } else {
            player.sendMessage("Not ready");
        }
    }
}
```

The condition must be a `boolean`. Braces are required.

### Conditional expressions

```mcfc
class Main {
    static int fee(boolean member, int price) {
        return member ? price / 2 : price;
    }
}
```

`condition ? whenTrue : whenFalse` chooses one branch. `int` and `float` branches combine as `float`; `Player` and `Entity` branches combine as `Entity`.

### `switch`

```mcfc
class Main {
    static void describe(int level) {
        switch (level) {
            case 1, 2 -> System.out.println("low");
            case 3 -> {
                System.out.println("medium");
                System.out.println("still medium");
            }
            default -> System.out.println("high");
        }
    }
}
```

`switch` works on enum, `int` and `String` values. Cases are constants of that type, and a case can list several. Each case is a single statement or a `{ ... }` block. Cases don't fall through, so there's no `break`. On an enum, cases name the bare constant (`case SURVIVAL`), and the switch must either list every constant or have a `default`. The value is evaluated once, and the first matching case runs.

A switch can also produce a value. Expression arms end with `;`; a block arm returns a value with `yield`. A switch expression needs a `default` or every enum constant.

```mcfc
enum Rank { LOW, HIGH }

class Main {
    static String label(Rank rank) {
        return switch (rank) {
            case LOW -> "low";
            case HIGH -> { yield "high"; }
        };
    }
}
```

#### Switching on types

```mcfc
sealed interface Shape permits Circle, Square {}

final class Circle implements Shape {
    int radius = 2;
}

final class Square implements Shape {
    int side = 3;
}

class Main {
    static int area(Shape shape) {
        return switch (shape) {
            case Circle circle -> 3 * circle.radius * circle.radius;
            case Square square -> square.side * square.side;
        };
    }
}
```

A case can be a class with a variable name, and the first case whose class the object has runs. Without a `default`, the cases must cover every class the value can be. A switch on types works as a statement, a variable's value, an assignment or a `return`, but not inside a bigger expression.

### `while`

```mcfc
class Main {
    static void count() {
        var i = 0;
        while (i < 3) {
            System.out.println("$(i)");
            i++;
        }
    }
}
```

A loop runs entirely within one tick unless its body sleeps. A long loop with no `Thread.sleep` can hit Minecraft's command limit (`maxCommandChainLength`), and the rest of the method then doesn't run.

### `do` / `while`

```mcfc
class Main {
    static void retry() {
        var attempts = 0;
        do {
            attempts++;
        } while (attempts < 3);
    }
}
```

The body runs at least once. `continue` proceeds to the condition check.

### `for`

```mcfc
class Main {
    static void loops(List<Integer> values) {
        for (int i = 0; i < 3; i++) {
            System.out.println("$(i)");
        }
        for (Player player : Selector.of("@a")) {
            player.addTag("seen");
        }
        for (var value : values) {
            System.out.println("$(value)");
        }
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
class Main {
    static void firstReady() {
        for (Player player : Selector.of("@a")) {
            if (!player.hasTag("ready")) {
                continue;
            }
            player.sendMessage("Ready");
            break;
        }
    }

    static int clampZero(int value) {
        if (value < 0) {
            return 0;
        }
        return value;
    }
}
```

`break` and `continue` apply to the innermost loop, or to the loop with that label:

```mcfc
class Main {
    static void findPair(List<Integer> values) {
        search:
        for (int first : values) {
            for (int second : values) {
                if (first + second == 10) {
                    break search;
                }
            }
        }
    }
}
```

In a `void` method, use `return;` on its own. Otherwise, `return expr;` must match the declared return type.

### `throw`, `try`, `catch`, `finally`

```mcfc
class NotEnough extends RuntimeException {
    NotEnough(int missing) {
        super("missing " + missing);
    }
}

class Main {
    static int spend(int coins, int amount) {
        if (amount < 0) {
            throw new IllegalArgumentException("negative amount");
        }
        if (amount > coins) {
            throw new NotEnough(amount - coins);
        }
        return coins - amount;
    }

    static void buy() {
        int coins = 10;
        try {
            coins = spend(coins, 15);
        } catch (NotEnough error) {
            Log.warn(error.getMessage());
        } catch (IllegalArgumentException | IllegalStateException error) {
            Log.error("bad purchase");
        } finally {
            Log.info("coins left: " + coins);
        }
    }
}
```

- Exceptions are classes that extend `Exception`. `RuntimeException`, `IllegalArgumentException`, `IllegalStateException` and `UnsupportedOperationException` come with MCFC and need no import. Each takes a message, which `getMessage()` returns.
- The first `catch` whose type matches runs. `catch (A | B e)` catches either, and `e` is then an `Exception`. An exception no `catch` matches goes on to the caller after `finally` runs.
- Every exception is unchecked: `throws` after the parameters is allowed but only documents.
- An exception nothing catches is logged with [`Log.error`](./builtins#logging) where it leaves a method no code calls, such as `main`, a `@Tick` method or an event handler. In a `@Test`, it fails the test.
- After each call that can throw, the caller checks for an exception, so a program without `throw` pays nothing. See [limitations](../limitations) for what differs from Java.

### `Thread.start`

```mcfc
class Main {
    static void greetLater(Player player) {
        Thread.start(() -> {
            Thread.sleepTicks(20);
            player.sendActionBar("later");
        });
        player.sendActionBar("now");
    }
}
```

The lambda starts running right away, and the statement after `Thread.start` runs without waiting for it. Local variables are copied when it starts, so later changes in the parent don't reach the copy. The lambda takes no parameters, and `return` isn't allowed inside it.

### `Execute.as` and `Execute.at`

```mcfc
class Main {
    static void sparkle(Player player) {
        Execute.as(player, () -> {
            Selector.of("@s").getFirst().addTag("marked");
        });
        Execute.at(player, () -> {
            Block.of("~ ~1 ~").spawnParticle("minecraft:happy_villager", 8);
            Block.of("~ ~-1 ~").setBlock("minecraft:gold_block");
        });
    }
}
```

`Execute.as` changes who `@s` is, and `Execute.at` changes where `~ ~ ~` is. The anchor can be an `Entity` or a `Selector`, in which case the lambda runs once per entity. These compile to `execute as` and `execute at`, so the lambda runs in place and `return` isn't allowed inside it. Passing `() -> value` instead of a block gives a [selector or position relative to the entity](./builtins#execute-as-and-execute-at).

### `Commands.run`

`Commands.run` is for Minecraft commands MCFC has no feature for yet. Prefer a method or builtin when one exists; missing features are tracked in [issues](https://github.com/OliverMarcusson/MCFC/issues).

```mcfc
class Main {
    static void setup() {
        Commands.run("weather clear");
    }

    static void reward(int amount) {
        Commands.run("xp add @a $(amount) levels");
    }
}
```

The argument must be a string literal. Without `$(...)`, the command is emitted exactly as written. With `$(expr)`, each placeholder is replaced by its value at run time: the command compiles to a Minecraft function macro, its values are copied into storage, and then `function ... with storage ...` is called.

Values are inserted without escaping. A string containing `"` breaks the command, as described under [string limits](./types#string).

### Calls

A method call can be a statement on its own, such as `System.out.println("ok");` or `player.heal(2);`. Other expressions can't: `amount + 1;` is an error.
