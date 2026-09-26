# Limitations

MCFC is early-stage software. The backend prioritizes correctness, inspectable output, and deterministic code generation over aggressive optimization.

Currently not supported:

- recursion
- implicit conversions, except builder-to-`nbt` coercions in NBT contexts
- `entity_set.position`
- richer object systems beyond structs and built-in handle types

Additional notes:

- `match` currently supports only `string` scrutinees.
- each `match` arm currently contains exactly one statement.
- `switch` accepts enum, `int`, and `string` values; enum switches require every constant or a `default` arm.
- `sleep(...)` and `sleep_ticks(...)` are statement-only.
- host calls are statement-only because they suspend execution.
- modules have no `pub use` re-exports, `*` glob imports, or inline `mod name:` bodies.
- `$(...)` placeholders in `mcf` need full paths from the root module, such as `$(util::double(x))`.
- String, float, and struct `entity.state.*` and `player.state.*` paths require declarations; undeclared paths support `int` and `bool`.
- `Optional<void>` is not valid. `orElse(...)` evaluates its fallback even when a value is present.
- `find_first` needs a direct `selector(...)` expression, optionally wrapped in `at(...)` or `as(...)`, so MCFC can enforce `limit=1`.
- Dictionary keys must use letters, digits, and `_`, with a non-digit first character. A dynamic key that violates this rule cannot name a stored dictionary entry.

::: tip
Use helper functions when a `match` arm needs more than one operation.
:::
