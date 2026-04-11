# MCFC Language Guide

`mcfc` is a small statically typed language that compiles to a Minecraft datapack for Minecraft `26.1.2`.

The compiler is currently focused on a compact core language:

- functions
- optional `@book` function exposure
- typed parameters and return values
- local variables with inferred types
- integer arithmetic
- boolean conditions
- `if` and `while`
- function calls
- raw Minecraft commands
- storage-backed strings

## CLI Usage

Build a source file into a datapack directory:

```powershell
cargo run -- build example.mcf --out build\pack
```

Available flags:

- `--namespace <name>`: override the generated namespace. Default: `mcfc`
- `--emit-ast`: write a debug dump of the typed program to `debug/typed_program.txt`
- `--emit-ir`: write a debug dump of the lowered IR to `debug/ir.txt`
- `--clean`: remove the output directory before writing new output

The compiler writes:

- `pack.mcmeta`
- `data/<namespace>/function/main.mcfunction`
- generated functions under `data/<namespace>/function/generated/`

`main.mcfunction` runs setup and then calls the generated `main` entrypoint if a `main` function exists.

## Example Program

```text
fn add(x: int, y: int) -> int
    return x + y
end

fn main() -> void
    let a = 1
    let b = 2
    let label = "done"

    while a < 4:
        if a == 2:
            b = add(a, b)
        end
        a = a + 1
    end

    mc "say done"
    return
end
```

## Syntax

### Functions

Functions begin with `fn` and end with `end`.

```text
fn name(param: type, other: type) -> return_type
    ...
end
```

Rules:

- parameter types are required
- return types are required
- duplicate function names are rejected
- duplicate parameter names are rejected
- `#` starts a line comment and may also appear after code on a line

Book-exposed functions may be annotated with `@book`:

```text
@book
fn fibb(n: int) -> void
    ...
end
```

Current `@book` restrictions:

- the function must return `void`
- all parameters must be `int`
- the function name becomes the book command name

### Statements

Supported statements:

- `let name = expr`
- `name = expr`
- `if condition: ... end`
- `while condition: ... end`
- `return`
- `return expr`
- `mc "raw minecraft command"`
- `mcf "macro command with $(placeholders)"`
- a bare function call as a statement, for example `do_work()`

Notes:

- only function calls may be used as bare expression statements
- `if` and `while` bodies are block-scoped for new `let` bindings
- there is currently no `else`
- `mc` is literal-only; `mcf` is the runtime interpolation form

### Expressions

Supported expressions:

- integer literals, for example `42`
- boolean literals: `true`, `false`
- string literals, for example `"hello"` or `'hello'`
- variables
- function calls
- binary operators

Binary operators:

- arithmetic: `+`, `-`, `*`, `/`
- comparison: `==`, `!=`, `<`, `<=`, `>`, `>=`

Precedence:

1. `*`, `/`
2. `+`, `-`
3. comparisons

Parentheses may be used to group expressions.

## Types

Built-in types:

- `int`
- `bool`
- `string`
- `void`

Type rules:

- locals infer their type from the initializer
- assignments must keep the original variable type
- function arguments must match declared parameter types
- return expressions must match the declared return type
- there is no implicit conversion between types

Current operator support:

- arithmetic requires `int`
- comparisons currently support `int` and `bool`
- strings can be stored, passed, assigned, and returned, but string comparison/operators are not supported yet

String literal notes:

- both double-quoted and single-quoted strings are supported
- use `\"` inside `"..."`
- use `\'` inside `'...'`

## Scope and Name Rules

Variables are function-local.

Important behavior:

- parameters are available throughout the function
- `let` introduces a new binding
- reusing an existing local or parameter name with `let` is rejected
- `let` bindings created inside an `if` or `while` body do not become visible outside that block

Example:

```text
fn main() -> void
    if true:
        let x = 1
    end

    x = 2
end
```

This fails because `x` does not exist outside the `if` block.

## Control Flow

### `if`

`if` conditions must have type `bool`.

```text
if a == 5:
    mc "say five"
end
```

### `while`

`while` conditions must have type `bool`.

```text
while counter < 10:
    counter = counter + 1
end
```

The compiler lowers loops into generated datapack functions. It does not try to protect you from infinite loops, so loop termination is your responsibility.

## Functions and Calls

Function calls may appear:

- in expressions
- on the right-hand side of assignments
- in returns
- as standalone call statements

Example:

```text
fn inc(x: int) -> int
    return x + 1
end

fn main() -> void
    let a = 1
    a = inc(a)
    return
end
```

Current call model:

- nested non-recursive calls are supported
- recursion is rejected at compile time
- unknown functions are rejected
- wrong arity is rejected

The backend uses depth-indexed frame slots so nested calls do not overwrite caller state.

## Raw Minecraft Commands

Use `mc "..."` to emit raw commands directly:

```text
mc "say hello"
mc "scoreboard players set @s health 20"
```

The string is copied directly into the generated `.mcfunction` output.

## Macro Commands

Use `mcf "..."` for runtime interpolation through Minecraft function macros.

```text
let amount = 5
mcf "xp add @a $(amount) levels"
```

Rules:

- placeholders use `$(name)` syntax
- placeholders may reference in-scope local variables and parameters only
- supported placeholder types are `int`, `bool`, and `string`
- placeholders are not full expressions, so `$(a + 1)` is invalid
- malformed placeholders are rejected during compilation

Implementation model:

- the compiler generates a macro `.mcfunction` containing a `$`-prefixed command
- it writes placeholder values into storage
- it invokes the macro with `function ... with storage ...`

`mc` and `mcf` are intentionally different:

- `mc "say $(a)"` emits the literal text `say $(a)`
- `mcf "say $(a)"` performs runtime substitution through Minecraft macros

## Runtime Representation

The current backend maps values like this:

- `int`: scoreboard-backed
- `bool`: scoreboard-backed using `0` and `1`
- `string`: Minecraft data storage-backed

Generated files are deterministic and use a reserved generated namespace layout.

## Book Commands

The compiler can generate a writable-book command runtime for functions marked with `@book`.

Current runtime behavior:

- players are polled each tick
- a managed writable book is given to players if they do not already have one
- when the managed book is in the player's main hand, the runtime reads page 1
- if the command text changed since the last processed edit, it dispatches the command once

Current command format:

- page 1, first command text only
- command grammar: `<name> <int> <int> ...`
- example: `fibb 8`

Current dispatch behavior:

- only `@book` functions are callable
- dispatch runs as the player holding the book
- bad command names or bad integer arguments produce a `tellraw` error

This is implemented as tick-based polling, which approximates “on close” as “on the next tick after the book contents changed.”

## Diagnostics

The compiler reports line and column information for parse and type errors.

Common errors include:

- undefined variables
- duplicate function names
- duplicate parameter names
- duplicate local names
- wrong argument count
- wrong argument type
- invalid return type
- non-boolean `if`/`while` conditions
- unsupported recursion

## Current Limitations

This is the current implemented language, not the final design.

Not supported yet:

- `else`
- recursion
- implicit conversions
- string operators or string comparison
- placeholder expressions such as `$(a + 1)`
- direct entity, block, or arbitrary storage-path placeholders in `mcf`
- `@book` string arguments
- `@book` function calls with non-`int` parameters or non-`void` return types
- rich book parsing such as `fibb(8)`, quoted arguments, or multiple commands
- compound data types
- pattern matching
- modules/imports
- optimization passes

The backend is intentionally simple and prioritizes correctness, inspectable output, and deterministic code generation over optimization.
