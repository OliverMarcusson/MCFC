# Syntax

## Functions

Functions begin with `fn`, use `:` after the signature, and use indentation for the body.

```mcfc
fn name(param: int, label: string) -> void:
    debug(label)
```

Rules:

- parameter types are required
- return types are required
- duplicate function names are rejected
- duplicate parameter names are rejected

`fn tick() -> void:` is special: it maps to the datapack tick function and runs once every game tick. If multiple source files define zero-argument `tick() -> void`, their bodies are merged in deterministic source order.

## Statements

MCFC supports declarations, local bindings, assignment, branching, loops, async blocks, context blocks, raw commands, macro commands, control flow, and function-call statements.

See the [statements reference](./reference/statements) for the full list, examples, and rules such as the bare-expression restriction: only function calls may be used as bare expression statements.

## Expressions

Supported expressions include literals, variables, calls, method calls, path access, arrays, dictionaries, indexing, unary `not`, and binary operators.

Binary operators:

- arithmetic: `+`, `-`, `*`, `/`
- logical: `and`, `or`
- comparison: `==`, `!=`, `<`, `<=`, `>`, `>=`

Precedence:

1. `not`
2. `*`, `/`
3. `+`, `-`
4. comparisons
5. `and`
6. `or`

## Raw And Macro Commands

Use [`mc`](./reference/raw-mc) for literal Minecraft commands:

```mcfc
mc "say hello"
mc "scoreboard players set @s health 20"
```

Use [`mcf`](./reference/macro-mcf) for runtime interpolation:

```mcfc
let amount = 5
mcf "xp add @a $(amount + 1) levels"
```

`mc "say $(amount)"` emits the literal text. `mcf "say $(amount)"` performs runtime substitution.

## Scope

Variables are function-local. `let` introduces a new binding, and reusing an existing local or parameter name with `let` is rejected.

Bindings created inside a block are not visible outside that block. Loop variables are local to the loop body.
