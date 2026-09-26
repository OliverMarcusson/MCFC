# Java-likeness fixes: progress

Started 2026-09-26. You asked for every item from the Java-likeness rating (groups A, C, D, E).
Numbers keep Minecraft semantics: `/` floors, `%` takes the sign of the divisor, `(int)` floors.
Those should be documented, not changed. The checklist is in `TASKS.md` under "Java-likeness fixes".

## State

The Java-likeness batch is implemented and validated. The old syntax tests, examples,
documentation, LSP, and VS Code support have been migrated to the new names.

## Done

### A. Bugs and quick fixes
- **Error messages** now use Java names. `language_catalog::display_call(internal)` maps
  internal builtin names to source form: `bool` → `(boolean) x`, `selector` → `Selector.of(...)`,
  `single` → `getFirst(...)`. Every `format!("{}(...)", function|method)` in `types.rs` goes through it.
  Cast errors read "cannot cast 'X' to boolean".
- **String joining:** `"s" + n` works for `int`, `float`, `boolean` (gives `true`/`false`) and enums
  (gives the constant name). The helper is `string_operand` in `types.rs`. `$(b)` inside a String
  literal also prints `true`/`false` and constant names. `mcf("...")` keeps raw values.
- **Boxed type names:** `Integer`/`Float`/`Boolean` are accepted anywhere. Inside `<...>`, a primitive
  is an error ("use 'Integer' inside '<...>'"). `Type::as_type_arg` prints `List<Integer>`.
- **int → float widening** happens in `coerce_expr_to_expected_type`, which covers declarations,
  assignments, arguments and record fields. Binary arithmetic promotes to `float`.
- **Lexer:** `0xFF`, `1_000`, `1.5f`, `2f`, and the `?` and `do` tokens.
- **Parser:**
  - `do { } while (c);` desugars to `{ var __do_L_C = true; while (__do || c) { __do = false; body } }`.
  - `final` works on locals and params, tracked in `Parser::final_scopes`. Assigning to one is an error.
  - `static` is now accepted silently.

### E. Syntax
- `c ? a : b` is `ExprKind::Conditional`, which becomes `TypedExprKind::Conditional` and then
  `IrExprKind::Conditional`. The backend compiles the condition into a temp, and each branch into
  its own generated function (`cond_then` / `cond_else`).
- **Switch expressions** (`ExprKind::Switch`) support expression arms and `{ yield e; }` blocks.
  Typing is in `type_check_switch_expr`, which lowers to nested conditionals via `switch_chain`.
  A value that isn't a variable or literal is bound once through the new `TypedExprKind::Bind` /
  `IrExprKind::Bind`. A switch expression must be exhaustive: it needs a `default` or every enum constant.
- `unify_branches` joins `int` with `float`, and `Player` with `Entity`.
- The optimizer folds conditionals whose condition is constant. `calls_in_expr` and the cancel
  detection walk the new nodes.

### C. Library names
- **Static calls** are rewritten in the parser (`Parser::static_call`):
  - `Math.f(x, ...)` becomes a method named `"Math.f"` on `x`. The dotted name can't be written in source.
  - `Integer.parseInt(s)` becomes `"Integer.parseInt"`.
  - `String.valueOf(x)` / `Integer.toString(x)` / `Float.toString(x)` become `"" + x`.
- **`Math` typing** is in `type_check_math_call`. When every argument is `int`, `abs`/`min`/`max`/`clamp`/`signum`
  call `std::math::*`. Otherwise the call widens to `float`. `Math.round` returns `int`.
  Old `x.sqrt()` and `s.parseInt()` give "use Math.sqrt / Integer.parseInt".
- **Strings:**
  - `equals` becomes `==` (an AST rewrite).
  - `contains`/`startsWith`/`endsWith`/`indexOf` call `std::str::*`.
  - `charAt(i)` becomes `slice(i, i+1)` and returns a String, since there is no char type.
  - `isEmpty()` works on String, List, Map and Optional.
- **Collections and Optional:**
  - `xs.set(i, v);` and `m.put(k, v);` are statements that lower to index assignment. This is in the
    `type_check_block` Expr arm.
  - `m.getOrDefault(k, d)` becomes `get(k).orElse(d)`.
  - `Optional.get()` has a new backend arm, `"get"` on Optional.
- **Enums:** `E.values()` gives a `List<E>` literal. `e.ordinal()` gives an `int`, and `e.name()` gives
  a conditional chain (`enum_name_expr`).
- **Records:**
  - `p.x()` reads a component (`record_component`).
  - Field syntax `p.x`, including assignment, is an error on user records and payload structs.
    Names starting with `@` (state maps) are exempt.
  - As a result, event and host payloads are now `event.player()` and `r.body()`.

### D. Minecraft API
- **`ENTITY_METHOD_NAMES`** (Java name → internal name) and **`OLD_ENTITY_METHOD_NAMES`** (old name →
  hint) are in `language_catalog.rs`. The type checker maps them, but only when the receiver is a
  Selector, Entity, Player or Block, so record components such as `title()` don't collide:
  - `sendMessage`, `sendTitle`, `sendActionBar`, `playSound`, `stopSound`, `setBlock`, `spawnParticle`
  - `getLightLevel`, `getBiome`, `getEnvironment`
  - `getX`/`getY`/`getZ`, `getYaw`/`getPitch`, `getLookX`/`getLookY`/`getLookZ`
  - `getHealth`, `getFoodLevel`, `getLevel`, `getGameMode`, `getSelectedSlot`, `getDimension`
- **Selector and entity methods:** `sel.getFirst()` replaces `single(sel)`, `sel.findFirst()` replaces
  `findFirst(sel)`, and `e.isValid()` replaces `exists(e)`. The parser rejects the old free forms.
  Internally it still uses `call("single", ...)` for handler prologues.
- **Properties:** `property_names(ty)` covers bossbar (name, value, max, visible, players),
  ItemStack (count, name), EntityData (name, nameVisible, noAi, silent, glowing, tags) and
  BlockData (name, lock, lootTable, lootSeed).
  - `x.getFoo()` reads a property, and `x.setFoo(v);` writes it. The setter only works when the
    receiver is a plain variable.
  - Direct field syntax for these properties is an error (`check_property_syntax`).
  - The `LOWERING_SETTER` thread-local stops the lowered assignment from tripping that check.
- **Kept as field syntax** because it is raw Minecraft data: `nbt.*`, `states.*`, Component text fields,
  entity `state.*`/`tags.*`/equipment/inventory, and ItemSlot fields.

### Simulator programs (all pass with and without optimization)
- `scripts/pack-sim/programs/javaish.mcf` covers `?:`, switch expressions, do-while, String `+` and number literals.
  Budget 119.
- `scripts/pack-sim/programs/javaapi.mcf` covers String/Integer/Math/List/Map/Optional/enum/record methods.
  Budget 383.
- The budgets are in `check.py`. Run `python scripts/pack-sim/check.py`. The simulator can't run floats (`/compute`).

## Validation

- `cargo test --quiet`: 83 library, 107 integration, and 4 optional tests pass.
- `cargo clippy --all-targets --quiet`: passes with style warnings.
- `cargo fmt --check`: passes.
- `cargo build --release`: passes.
- `python scripts/pack-sim/check.py`: all eight programs pass with and without optimization.
- All seven example projects build.
- `npm run docs:check`: all 80 snippets compile.
- `npm run docs:build`: passes.
- VS Code extension `npm run compile` and grammar/snippet JSON parsing: pass.

The `javaapi` program probes world method compilation, and the integration test checks
the generated Minecraft commands. The simulator cannot execute those commands or
floating-point `/compute`; live Minecraft behavior has not been tested.
