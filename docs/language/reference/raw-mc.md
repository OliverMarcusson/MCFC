# Raw Commands: `mc`

`mc` emits a literal Minecraft command into the generated function.

```mcfc
fn load() -> void:
    mc "say MCFC loaded"
    mc "scoreboard objectives add health dummy"
```

## Rules

- The string is emitted as written.
- `$(...)` has no special meaning inside `mc`.
- Use `mc` when the command has no MCFC values to interpolate.
- Use [`mcf`](./macro-mcf) when the command needs runtime placeholders.

## Literal Placeholder Text

Because `mc` is literal-only, placeholder-looking text remains placeholder-looking text.

```mcfc
fn tick() -> void:
    let amount = 5
    mc "say $(amount)"
```

That command emits the text `$(amount)`, not `5`.

## Working With Context

`mc` is often paired with `as` and `at` blocks when the raw command depends on executor or position.

```mcfc
fn mark_nearest() -> void:
    let player = single(selector("@p"))

    as(player):
        mc "tag @s add marked"

    at(player):
        mc "setblock ~ ~-1 ~ minecraft:gold_block"
```

