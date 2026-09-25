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
- `sleep(...)` and `sleep_ticks(...)` are statement-only.
- host calls are statement-only because they suspend execution.
- modules have no `pub use` re-exports, `*` glob imports, or inline `mod name:` bodies.
- `$(...)` placeholders in `mcf` need full paths from the root module, such as `$(util::double(x))`.
- `entity.state.*` and `player.state.*` currently support only `int` and `bool`.

::: tip
Use helper functions when a `match` arm needs more than one operation.
:::
