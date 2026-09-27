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

## Handoff: datapack systems backlog (2026-09-27)

Last commit: `2442148` (Tier 3 + sidebars). Everything below is **uncommitted** on
`feat/datapack-systems`. The checklist is in untracked `TASKS.md`.

### State at handoff

All checks green, recursion included:
- `cargo test`: 83 unit + 123 integration + 4
- `python scripts/pack-sim/check.py`: 21 programs pass, plain and optimized
- `bun run docs:check`: 92/92 (run `cargo build --release` first); `docs:build` passes
- `cargo fmt --check`: clean
- clippy: 31 warnings (baseline 32)

### Done since `2442148`

**Carried over from earlier in the session**
- Sidebar Component text
- `EntityHurtPlayerEvent`
- `Block.of(x, y, z)`
- Bitwise operators and shifts
- `String.replace/split/toUpperCase/toLowerCase`
- `std.color`, `std.noise`, `std.shape`
- `Block.getX/Y/Z`, `getType`, `copyTo`, `getState`
- `Log.*`
- `pack_opt` early-return fix

**`@Test` + `assert`**
- `assert c [: msg];` desugars in the parser (`parse_stmt`) to `if (!c) assert_fail("line N: " + msg)`.
- `@Test` renames the function to `__mcfc_test_<name>`.
- `assert_fail` is lowered in `compile_log`: it sets `#test_failed` and prints a red tellraw.
- `emit_test_runner` writes `data/<ns>/function/test.mcfunction`, which prints pass/fail counts.
- Covered by sim program `tests`.

**`std.dialog`** (`std/dialog.mcf`)
- `dialog.notice(player, title, body)` and `dialog.menu(player, title, body, List<Button>)`.
- `Button(label, command)` runs `/trigger <command>`, which reaches `@Command` handlers.
- The dialog is built from records, so the game writes and escapes the SNBT.
- Covered by sim program `dialogs`.

**`@Menu("Label")`**
- Works as `@Command`, plus a button in the generated `data/<ns>/dialog/about.json`.
- Also emits the Smithed Data Pack Menu files: `smithed:data_packs` dialog, `#smithed:data_packs`, `#minecraft:pause_screen_additions`.
- The label is hex-encoded into the handler name (`__mcfc_command_<cmd>__menu_<hex>`) and decoded in `discover_bukkit_runtime`.
- Code: `emit_pack_menu`. Test: `menu_commands_join_the_smithed_data_pack_menu`.

**Actionbar coordination**
- `sendActionBar(msg[, "override"|"notification"|"conditional"|"persistent"])`. The default is `"notification"`.
- Call sites go through `actionbar_command` → `generated/actionbar/show {json:[…],priority:"…"}`.
- If the Smithed Actionbar library is loaded (`$default.freeze smithed.actionbar.const` set), messages go to `#smithed.actionbar:message`.
- Otherwise a port of its algorithm runs on the same objectives. Its tick has a gametime guard, so several MCFC packs only tick once.
- An empty `#smithed.actionbar:message` tag keeps the call valid without the library.
- Covered by sim program `actionbar` and test `actionbar_priorities_are_checked`.

**Wildcard imports** (`import a.b.*;`)
- The parser sets the alias to `"*"`.
- The resolver's `add_import` expands it to every public function and record.
- Wildcards are applied after named imports. Clashes are skipped, as in Java, where local and named imports win.
- Test: `wildcard_imports_bring_in_public_names`.

**Imports inside `$(...)`**
- `Resolver::resolve_placeholders` (modules.rs) lexes each placeholder.
- It resolves `a.b(`-style call chains that don't start at a local, and rewrites them to their full name `util::twice(`.
- The parser now accepts `a::b` as one identifier.
- Covered by sim program `placeholders`.

**Pack simulator** (`mcsim.py`)
- tellraw and actionbar lines render as plain text.
- One pretend player: `execute as @a/@s`, and `@a[scores={o=r}]` treats the pretend player as `@s`.
- `function id {inline args}`.
- Compound path filters `a{k:v}`.
- SNBT escapes `\n` and `\t`.
- `dialog` lines are traced.

**Recursion** (direct and mutual)
- `types::analyze_calls`: Tarjan SCCs. A group is 2+ functions in a cycle, or one that calls itself. `recursion_groups` (function to group id) rides on `TypedProgram`/`IrProgram`; call depths are longest paths with groups condensed.
- Calls inside a group stay at the caller's depth. Args go into fresh temps first, then a per-site `__call_N` function runs: save frame, set params, call, copy return to `$rec_ret`/`rec_ret`, restore frame. It is its own function because the callee shares the caller's `__ctrl`, which would skip guarded lines after the call.
- `emit_frame_stack` (end of `generate`) writes `<fn>__d<d>__save/restore`: every `$d<d>_<fn>_*` score in the pack plus `frames.d<d>.<fn>`, pushed onto `<ns>:runtime stack`.
- `validate_suspending_calls` rejects recursive functions that can pause. Test: `rejects_sleep_in_recursion`.
- Sim program `recursion`: factorial, fibonacci, isEven/isOdd, gcd (swapped params), a String and a List function.
- Docs: limitations.md and statements.md; also removed the stale "no `*` imports" and "`$(...)` ignores imports" limitations.

**String escaping in `+`**
- `call_macro(..., escape_strings)`: string-building macros (`compile_interpolated_string`) pass each `String` placeholder through `generated/escape_string` first.
- It copies the value into `escape.c.v` and reads `escape.c` back with `set string`: the game's own SNBT printer escapes it. If the value's first quote is `"`, the printer uses `'...'`, so `escape_string_single` puts a `'` in front and prints again.
- Costs about 6 commands per String operand; budgets rose for colors, strings, strtools and others.
- `mcsim.py` now prints SNBT like the game (quote choice, control-character escapes) and reads non-string `set string` sources as SNBT. The control-character escapes are assumed from 1.21.5, unverified in 26.3.
- Sim program `strescape`. `$(...)` in `mcf(...)` is still pasted raw, by design.

### Still open in TASKS.md
- Live-test the per-player sidebar on a 26.3 server with the agent attached. This needs the user.
- Unverified in-game:
  - dialogs, the `@Menu` pause-screen entry, and actionbar priorities
  - `nbt` text components
  - block predicate state matching
  - marker coordinates
  - string escaping (`set string` on a compound, SNBT quote choice)
