# Getting Started

## Install

MCFC is built from source and needs [Rust](https://rustup.rs).

```powershell
git clone https://github.com/OliverMarcusson/MCFC
cargo install --path MCFC
```

This installs two binaries into `~/.cargo/bin`: `mcfc`, the compiler, and `mcfc-lsp`, the language server. Check that the install worked:

```powershell
mcfc --help
```

MCFC targets Minecraft 26.3. Output and syntax still change between commits, so pin a commit for any pack you depend on.

## Build a pack

```powershell
mcfc new my-pack --helper none
mcfc build my-pack --clean
```

`build` writes the datapack to `my-pack/dist`. Copy that folder into `<world>/datapacks/` and run `/reload`. `Main.main()` runs on every load.

To rebuild after every save, run `mcfc watch my-pack`. See [CLI](./cli) for all flags.

## Next

- [Your First Pack](./first-pack) walks through state, tasks, events and commands in about 15 minutes.
- [VS Code](/editor/vscode) sets up diagnostics and completion.
- [Host Bridge](/runtime/host-bridge) covers HTTP, files and databases from a datapack, which need the optional `mcfd` helper.
