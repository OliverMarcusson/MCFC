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

