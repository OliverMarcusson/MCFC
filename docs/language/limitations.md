# Limitations

What MCFC doesn't do yet, with workarounds where there are any.

## Language

- **Recursive methods can't pause.** A method that calls itself, directly or indirectly, can't `sleep`, sort or wait on a host call. Deep recursion is also bounded by the game's command chain limit (`maxCommandChainLength`), since every call saves and restores its frame.
- **`char`, `short` and `byte` are aliases.** A `char` is a one-character `String` and has no number value; `short` and `byte` are full 32-bit `int`s and don't wrap. There is no `long` or `double`.
- **Limited implicit conversions.** `int` widens to `float`, and `String + value` converts numbers, booleans, enums and records to text. Other conversions need a cast or `toString()`.
- **Unchecked casts.** `(Dog) animal` doesn't check the object's class while the pack runs, and there is no `ClassCastException`. Test with `instanceof` first.
- **Records don't implement interfaces**, and generic methods aren't virtual.
- **Lists and maps are values.** Assigning or passing a `List` or `Map` copies it, unlike Java, where both names would see the same list. This is on purpose: a copy is one storage command, while a shared list would make every element read and write a macro call through the object heap. Class objects are shared, so wrap a list in a class when several places must see one list.
- **Lambdas copy what they capture.** Since lists and maps are values, `x -> seen.add(x)` adds to the lambda's own copy of `seen`. Capture a class object instead. A generic call only infers type arguments from an expression lambda's result, not from a block's `return`.
- **Exceptions differ from Java.** `finally` doesn't run when the `try` or `catch` body leaves with `return`, `break` or `continue`. When a call in the middle of an expression throws, the rest of the expression still runs; the check comes after the statement. The runtime never throws on its own: dividing by zero or reading past the end of a list gives a value, not an exception.
- **No generic records.** Generic type parameters work on methods and classes, but record declarations are not generic.
- **Enum constructors** can only assign parameters to fields (`this.mass = mass;`), and fields are `final`.
- **Only `toString()` and `equals(other)`** can be marked `@Override`.
- **Imports:** no re-exports and no renaming.
- **Hook methods can't be called directly.** A method marked `@Tick`, `@Command`, `@Menu`, `@Every`, `@After`, `@Test` or `@EventHandler` is only run by its trigger. Put shared code in an ordinary method and call that from the hook.
- **One `main`.** Only one class in the root module can declare `public static void main()`.
- **A few builtins are still free calls.** `debug`, `random`, `sleep`, `sleepTicks`, `gameTime`, `mc` and `mcf` are called without a class name.

## Runtime values

- **`$(...)` in `mcf(...)` isn't escaped.** The value is pasted into the command as it is. Joining strings with `+` is safe. See [string limits](./reference/types#string).
- **Selector methods** such as `findFirst()`, `tag(...)` and `matches(...)` need a selector built in the same expression, not a variable or a runtime `String`.
- **`hasData`** only works on storage values (lists, maps, records), not on entity NBT.
- **`heal`** only works on references known to be non-players, for example `@e[type=minecraft:pig]`. For players, use `effect("minecraft:instant_health", 1, 0)`.
- **`Selector.position`** isn't supported. Loop over the set and use each entity's `position`.
- **`orElse(x)`** always evaluates `x`, even when the Optional has a value.
- **Map keys** may only use letters, digits and `_`, and can't start with a digit.
- **Undeclared `player.state.*`** only holds `int` or `boolean`. Declare other types with `@PlayerState`.

## Statements

- `sleep`, `sleepTicks`, host calls and calls to methods that pause have to be statements of their own. See [Methods that pause](./reference/statements#methods-that-pause).

## Platform

- Output targets Minecraft 26.3 only.
- Vanilla events are limited to `PlayerJoinEvent` and `PlayerDeathEvent`. Other events need the [agent](/runtime/mcfd-agent).
- Sections marked <Badge type="danger" text="Agent" /> or <Badge type="danger" text="mcfd" /> need a process running beside the server, so they don't work on Realms or most shared hosts. Everything else is a plain datapack.
