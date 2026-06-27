# Macro Commands: `mcf`

`mcf` emits a Minecraft function macro command and evaluates `$(...)` placeholders at runtime.

```mcfc
fn reward(amount: int, label: string) -> void:
    mcf "xp add @a $(amount) levels"
    mcf "say awarded $(amount) levels for $(label)"
```

## Rules

- Placeholder expressions use `$(expr)`.
- Placeholder expressions can reference locals, parameters, path reads, calls, indexing, and supported operators.
- The generated command is macro-backed, so use it only when interpolation is needed.
- Use [`mc`](./raw-mc) for literal commands.

## Under The Hood

`mcf` lowers to a generated Minecraft function macro. MCFC evaluates each placeholder expression into command storage under the generated runtime namespace, then calls the macro with `function <namespace>:<path> with storage <namespace>:runtime <slot>`.

Scoreboard-backed values are copied into storage first with `execute store result storage ... run scoreboard players get ...`. Storage-backed values are copied with `data modify storage ... set from storage ...`.

## Expressions In Placeholders

```mcfc
fn show_status(player: player_ref, amount: int) -> void:
    let ready = player.has_tag("ready")
    mcf "say next=$(amount + 1), ready=$(ready)"
    mcf "say team=$(player.team)"
```

## `mc` Compared With `mcf`

```mcfc
fn compare() -> void:
    let amount = 5

    mc "say $(amount)"
    mcf "say $(amount)"
```

The first command emits literal `$(amount)`. The second command substitutes the current value of `amount`.
