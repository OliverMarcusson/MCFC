# Getting Started

MCFC is built from this repository with Cargo. Run commands from the repository root unless a page says otherwise.

## Build The Tools

```powershell
cargo build
cargo build --bin mcfc-lsp
```

For day-to-day compiler use during development, `cargo run --bin mcfc -- ...` works without installing a binary globally.

## Create A Project

Create a manifest-based project:

```powershell
cargo run --bin mcfc -- new my-pack --helper none
```

The project creator can also scaffold helper-enabled projects:

```powershell
cargo run --bin mcfc -- new plain-pack --helper none
cargo run --bin mcfc -- new helper-pack --helper mcfd
cargo run --bin mcfc -- new agent-pack --helper mcfd-agent
```

Use `--force` only with an existing empty target directory.

## Build The Datapack

For a project with `out_dir = "dist"` in `mcfc.toml`:

```powershell
cargo run --bin mcfc -- build my-pack --clean
```

For a single file:

```powershell
cargo run --bin mcfc -- build npc.mcf --out build/pack --clean
```

## Load In Minecraft

1. Copy the generated datapack directory into your world's `datapacks/` folder.
2. Run `/reload` in Minecraft.
3. Run a generated public wrapper if your pack exposes one:

```text
/function my_namespace:main
```

::: tip Helper projects
If the project uses `mcfd`, install or start the helper with `mcfd service install` before testing host calls.
:::
