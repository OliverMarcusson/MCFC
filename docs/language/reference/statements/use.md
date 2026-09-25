# `use`

`use` brings a function, struct, or module from another module into scope under a short name.

```mcfc
mod game
use game::score::{add, Points as P}
use game::score

fn main() -> void:
    let p = P { value: add(1, 2) }
    score::reset(p)
```

## Forms

| Form | Imports |
| --- | --- |
| `use a::b::name` | `name` |
| `use a::b::name as other` | `name`, called `other` here |
| `use a::b::{x, y as z}` | `x` and `y`, with `y` called `z` |
| `use a::b` | the module `b`, so `b::name(...)` works |

## Path Rules

Paths are written with `::` and work the same in `use` declarations, calls, struct literals, and type annotations.

- The first segment is looked up in the current module first, then in the root module. From anywhere in the project, `util::double` therefore reaches the root's `util` module.
- `self::` starts at the current module, and `super::` starts at the parent module. `super::super::` goes up two levels.
- A child module does not see root-module items automatically. Write `use helper` or `super::helper()` to call a root function `helper` from `src/util.mcf`.
- Functions, structs, and modules are separate namespaces, so `use helper::helper` can import a function with the same name as its module.

## Constraints

- `use` is a top-level declaration. `pub use` re-exports and `*` globs are not supported.
- Imports are private to the module that declares them.
- Inside `mcf` `$(...)` placeholders, write functions with their full path from the root module, such as `$(util::double(x))`. Imports and `self`/`super` are not applied there.
