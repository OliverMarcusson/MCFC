# CLI

The `mcfc` binary exposes three user-facing commands: `new`, `build`, and `watch`.

## `mcfc new`

```powershell
mcfc new <project-name> [--helper <none|mcfd|mcfd-agent>] [--force]
```

`new` creates:

- `mcfc.toml`
- `src/main.mcf`
- `assets/.gitkeep`
- a project `README.md`
- a project `.gitignore`

Helper choices:

- `none`: plain MCFC with no helper runtime
- `mcfd`: enables the standalone host bridge
- `mcfd-agent`: enables `mcfd` plus an optional agent request

## `mcfc build`

```powershell
mcfc build <input-file|project-dir|manifest> [--out <directory>] [--namespace <name>] [--emit-ast] [--emit-ir] [--no-optimize] [--clean]
```

Build accepts a single `.mcf` file, a project directory, `mcfc.toml`, or `*.mcfc.toml`.

When building a project manifest that defines `out_dir`, `--out` may be omitted. For single-file builds, `--out` is required.

## `mcfc watch`

```powershell
mcfc watch <input-file|project-dir|manifest> [--out <directory>] [--namespace <name>] [--emit-ast] [--emit-ir] [--no-optimize] [--clean]
```

`watch` performs an initial build, polls for `.mcf` source changes, debounces saves, and rebuilds without exiting after compiler diagnostics.

## Shared Flags

- `--out <directory>`: datapack output directory
- `--namespace <name>`: override generated namespace
- `--emit-ast`: write `debug/typed_program.txt`
- `--emit-ir`: write `debug/ir.txt`
- `--no-optimize`: disable conservative IR optimization
- `--clean`: remove the output directory before writing generated files
