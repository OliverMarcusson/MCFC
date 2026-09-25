# `mod` and `pub`

`mod name` declares a child module whose code lives in another file. Items are private to their module unless marked `pub`.

```mcfc
# src/main.mcf
mod util

fn main() -> void:
    let n = util::double(21)
```

```mcfc
# src/util.mcf
pub fn double(x: int) -> int:
    return helper(x) * 2

fn helper(x: int) -> int:
    return x
```

## Where Module Files Live

The rules match Rust's:

| Declared in | `mod util` loads |
| --- | --- |
| `src/main.mcf` (the root) | `src/util.mcf` or `src/util/mod.mcf` |
| `src/game/mod.mcf` | `src/game/util.mcf` or `src/game/util/mod.mcf` |
| `src/game.mcf` | `src/game/util.mcf` or `src/game/util/mod.mcf` |

It is an error when neither file exists, or when both do. In a project build, a `.mcf` file that no `mod` declaration reaches is not compiled, and the build prints a warning naming it.

## Visibility

`pub` can precede `fn`, `struct`, and `mod`. Without it, an item is private: only its own module and that module's descendants can use it.

```mcfc
# src/game/mod.mcf
pub mod score      # other modules can reach game::score
mod internals      # only game and its children can reach game::internals

fn base() -> int:  # game::base is private; game::score can call super::base()
    return 10
```

## Constraints

- `mod` is a top-level declaration. Inline module bodies are not supported.
- `mod`, `use`, and `pub` are only reserved at the start of a top-level line, so they still work as ordinary names inside functions.
- `tick()` and `event`, `command`, and `task` handlers are hooks rather than callable items. They work in any module and ignore `pub`.
- `main()` is the entry point only in the root module. In a child module it is an ordinary function.

## Under The Hood

Modules only affect names. A function in a child module is compiled under its full path, so `util::double` in namespace `my_pack` gets its entry function at `data/my_pack/function/generated/util__double__d0__entry.mcfunction`. A zero-argument `void` function is also exported at its module path, for example `function my_pack:util/announce`. To choose that path yourself, name the function in an `[[export]]` entry as `function = "util::announce"`. Items in the root module keep their bare names, so single-file programs compile exactly as before.
