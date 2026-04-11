# Project Description: A Small Statically Typed Language That Compiles to Minecraft Datapacks

## Overview

This project is a compiler for a small, statically typed programming language designed specifically to improve the development experience of Minecraft datapacks and `mcfunction`-based logic.

The goal is not to emulate a full JVM, nor to build a general-purpose programming language. The goal is to provide the programming fundamentals that `mcfunction` lacks out of the box, such as:

- named variables
- function calls with parameters
- return values
- control flow
- simpler arithmetic and comparisons
- cleaner code organization
- optional raw access to native Minecraft commands

The compiler translates a small high-level source language into standard Minecraft datapack functions.

Initial target platform:

- Minecraft version `26.1.2`
- datapack output compatible with that version's command and storage model

---

## Motivation

`mcfunction` is powerful, but it is not ergonomic as a programming language. Even simple logic quickly becomes awkward because there is no direct support for:

- local variables in the normal sense
- structured function calling conventions
- typed parameters
- return values
- high-level `if` / `while` syntax
- straightforward expression handling

This project adds those missing fundamentals through a compilation step.

Instead of writing raw command flow everywhere, the user writes code in a small purpose-built language, and the compiler lowers it into valid Minecraft function files.

---

## Project Goals

The compiler should:

1. provide a small readable source language for datapack logic
2. use simple static typing to keep semantics clear
3. compile structured control flow into valid `mcfunction` execution flow
4. support function parameters and return values
5. support integer variables cleanly using scoreboards
6. support strings through Minecraft data storage, and structured data later through storage
7. preserve an escape hatch for raw Minecraft commands

The project is primarily about making datapack logic feel more like real programming, while still targeting Minecraft's command system faithfully.

---

## Scope

This is a Minecraft-oriented compiler, not a JVM backend and not a general-purpose language runtime.

The language is intentionally small and practical. It should be designed around what compiles well into `mcfunction`, rather than around traditional language completeness.

The first versions should focus on:

- integers
- booleans
- strings backed by Minecraft data storage
- basic control flow
- function calls
- simple expressions
- raw embedded Minecraft commands

More advanced features such as richer structured data, storage-backed compound values, and structured builtins can be added later.

---

## Source Language Design

The source language should be small, readable, and imperative.

A representative example:

```text
fn add(x: int, y: int) -> int
    return x + y
end

fn main() -> void
    let a = 5
    let b = 7

    if a == 5:
        b = add(a, b)
    end

    mc "say done"
end
```

This language provides:

- `fn` for function definitions
- typed function parameters
- typed return values
- `let` for local variable bindings
- reassignment with `=`
- `if` / `end`
- function calls
- `return`
- `mc "..."` for raw Minecraft commands

The syntax should remain simple and easy to parse.

---

## Static Typing

The language should use simple static typing.

Recommended initial types:

- `int`
- `bool`
- `string`
- `void`

### Why static typing?

Static typing increases frontend work slightly, but makes the compiler significantly simpler and safer overall.

It helps by:

- making variable representation explicit
- reducing ambiguity in expressions and assignments
- simplifying code generation
- enabling better compiler diagnostics
- cleanly separating scoreboard-backed and storage-backed values

### Recommended typing model

Use a hybrid approach:

- function parameters must have explicit types
- function return types must be explicit
- local variables may use type inference from the initializer

Example:

```text
fn test(a: int) -> void
    let b = 5
    let c = a + b

    if c == 10:
        mc "say ten"
    end
end
```

Here:

- `a` is explicitly typed as `int`
- `b` is inferred as `int`
- `c` is inferred as `int`
- `c == 10` produces a `bool`

### Initial type rules

- arithmetic operators work on `int`
- comparisons produce `bool`
- assignments must preserve type
- function arguments must match parameter types
- return statements must match the declared return type
- no implicit conversion between `int` and `string`
- string values are represented in Minecraft data storage rather than scoreboards

This keeps the language predictable and the compiler manageable.

---

## Runtime Model

The language is compiled, not interpreted.

The generated datapack should not emulate a virtual machine with a program counter and operand stack. Instead, the compiler should directly lower the meaning of the source program into Minecraft command flow.

### Value representation

Recommended initial representation:

- `int` values: scoreboards
- `bool` values: scoreboards using `0` / `1`
- `string` values: Minecraft data storage
- structured data: storage

This aligns with Minecraft's strengths.

### Target runtime assumptions

The first production target is Minecraft `26.1.2`, and backend decisions should be evaluated against that exact version rather than against Minecraft in general.

In particular:

- scoreboard-based integer execution is the default numeric model
- strings live in Minecraft data storage
- raw emitted commands through `mc "..."` are assumed to target the command syntax of `26.1.2`

---

## Compilation Strategy

The compiler should have at least three layers:

### 1. Source language frontend
Parses the high-level language into an abstract syntax tree.

### 2. Internal IR
Lower the AST into a simple explicit intermediate representation.

This IR should be register-based or temp-based, not stack-based.

Example IR:

```text
func main() -> void
entry:
    const a, 5
    const b, 7
    eq t0, a, 5
    br t0, then0, end0

then0:
    call t1, add, a, b
    mov b, t1
    jump end0

end0:
    rawcmd "say done"
    ret
```

A temp-based IR is easier to analyze and compile than a JVM-like stack machine.

### 3. `mcfunction` backend
Translate the IR into datapack files and function calls.

The backend should be designed around explicit storage locations and explicit control-flow blocks, because those are the real execution primitives available in the target datapack environment.

---

## Control Flow Compilation

Minecraft functions do not support arbitrary jumps within a file, so structured control flow must be lowered into function-based basic blocks.

For example, an `if` statement should compile into separate generated functions for each block, with branching performed via `execute if` / `execute unless`.

Source:

```text
if a == 5:
    b = add(a, b)
end
```

Conceptual lowering:

- evaluate condition
- branch into `then` or `end`
- compile each branch as its own block function

This is one of the core transformations of the compiler.

### Loops and runtime constraints

Loops need stricter rules than simple branching.

For a first version, `while` should be supported only when it can be lowered into a safe, explicit control-flow structure using generated block functions.

The design should acknowledge the following runtime realities:

- Minecraft does not provide arbitrary in-function jumps
- long-running or non-terminating loops can exceed practical command execution limits
- some looping behavior may need to be split across generated functions or future tick-based scheduling mechanisms

For the MVP, the safest recommendation is:

- support structured `if`
- support `while`
- define `while` lowering explicitly in the backend design
- do not promise automatic protection from user-written infinite loops
- do not introduce general unstructured jumps

In other words, loops are part of the language, but they must compile through explicit generated control-flow blocks rather than through any hidden VM-like mechanism.

---

## Function Calls, Parameters, and Return Values

One of the main project goals is to add proper function semantics on top of `mcfunction`.

The compiler should introduce a calling convention, for example:

- each call frame has isolated argument, local, temporary, and return locations
- integer and boolean frame values are stored in scoreboards
- string frame values are stored in Minecraft data storage
- callers write arguments into the callee frame before transfer of control
- callees write return values into the current frame's return slot
- callers read back the return value after the function call completes

This allows code like:

```text
b = add(a, b)
```

to compile into ordinary datapack commands without the user manually managing temporary scoreboard slots.

### MVP calling convention recommendation

The first version should not rely on a single shared global argument area or a single shared return slot.

Instead, the spec should assume a frame-isolated calling convention, such as:

- a compile-time-known frame layout for each function
- storage paths or scoreboard names parameterized by call depth, frame id, or another explicit frame handle
- nested calls must not overwrite the caller's live arguments, locals, temporaries, or return values

This keeps ordinary nested expressions such as `f(g(x))` well-defined.

Recursion may still be excluded from the first implementation if desired, but nested non-recursive calls should be semantically safe by design.

---

## Raw Minecraft Commands

The language should include a low-level escape hatch for direct command emission.

Recommended syntax:

```text
mc "say hello"
```

This allows users to interleave high-level language constructs with raw Minecraft commands when needed.

This is especially important early in the project, before richer Minecraft-specific builtins are introduced.

Later, the language may add structured builtins such as:

- `say "hello"`
- `summon "pig"`
- scoreboard helpers
- storage helpers
- tellraw/text helpers

But `mc "..."` should remain available as a universal fallback.

---

## Strings

Strings should be supported in a limited, practical way rather than by attempting to reproduce full Java-style string semantics.

Recommended early support:

- string literals
- string variables
- passing strings to builtins or raw output systems
- Minecraft data storage-backed representation

Strings should primarily be viewed as data for Minecraft-facing effects, text output, and storage operations, rather than as a fully featured standard-library type from the start.

For the MVP, strings are in scope, but only with simple semantics:

- no implicit conversion to numeric types
- no full standard-library string API
- no requirement for advanced mutation semantics beyond what compiles cleanly to storage operations

---

## Non-Goals

The project should explicitly avoid the following, at least initially:

- full Java compatibility
- JVM bytecode parsing as a core requirement
- object-oriented features
- heap allocation
- exceptions
- threads
- a runtime interpreter or virtual machine inside Minecraft
- full standard library semantics

This is a compiler for a small Minecraft-oriented language, not a general VM implementation.

---

## Recommended Implementation Language

### Recommendation: Rust

Rust is the recommended language for implementing the compiler.

### Why Rust?

Rust fits this project very well because it offers:

- strong type safety
- excellent support for enums and AST/IR modeling
- good performance
- great ergonomics for compiler-style code
- strong pattern matching for parsers and IR transforms
- easy creation of a single standalone binary
- no runtime dependency burden for users

Compiler projects benefit a lot from algebraic data types and explicit modeling of program states, and Rust is particularly good at that.

### Why not Python as the main implementation language?

Python would be faster for initial prototyping and experimentation, and it is a valid choice for an early proof of concept. However, for the full compiler, Rust is likely the better long-term choice because it provides:

- stricter structure
- better maintainability as the compiler grows
- clearer typed representations of AST, IR, and diagnostics
- stronger guarantees against accidental state bugs

### Practical recommendation

A very sensible approach would be:

- prototype the parser and codegen ideas quickly in Python if desired
- implement the real compiler in Rust once the language design stabilizes

If choosing only one language from the start, Rust is the best recommendation.

---

## Summary

This project is a compiler for a small statically typed language designed to add real programming fundamentals to Minecraft datapack development.

It aims to provide:

- clean function definitions
- typed parameters and returns
- variables
- arithmetic
- comparisons
- storage-backed strings
- structured control flow
- raw Minecraft command embedding
- a direct compilation path into `mcfunction`

The language should stay intentionally small, practical, and Minecraft-oriented.

The recommended implementation language is Rust, with a temp-based internal IR and a backend that compiles structured logic into scoreboard- and Minecraft-data-storage-backed datapack functions targeting Minecraft `26.1.2`.
