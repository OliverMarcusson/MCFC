# MCFC Language Support

VS Code syntax highlighting, editor basics, and language server support for
MCFC `.mcf` files.

This extension registers the `mcfc` language id, associates it with `.mcf`
files, and provides a TextMate grammar, snippets, project workflow commands,
and a bundled Rust language server. `mcfc.toml` is also validated and completed
without replacing your installed TOML syntax theme.

## Language Server

The bundled `mcfc-lsp` server provides:

- compiler-backed diagnostics, including `@EventHandler`, `@Command`, `@Every`,
  `@After`, `@PlayerState`, and `@EntityState` declarations
- symbols, semantic highlighting, folding ranges, selection ranges, formatting,
  document highlights, definitions, references, rename, and signature help
- hovers and completions for functions, locals, types, methods, host modules,
  payload structs, vanilla events, and the full experimental agent event catalog
- manifest diagnostics, symbols, and completions for `mcfc.toml`, including
  helper capabilities, agent events, and agent commands

That includes the builder-oriented gameplay surface, such as:

- `new EntityData("minecraft:pig")`, `new BlockData("minecraft:chest")`, and `new ItemStack("minecraft:apple")`
- `summon(entityData)` plus explicit-position `Block.of("~ ~ ~").summon(...)`
- `.asNbt()` on `EntityData`, `BlockData`, and `ItemStack`
- implicit builder-to-`Nbt` coercion in NBT contexts such as
  `pig.nbt.Passengers[0] = chicken`
- player inventory completions for `player.inventory[0].*`, `player.hotbar[0].*`,
  and explicit `Player` values
- member completions for `EntityData.nbt.*`, `BlockData.states.*`, `ItemStack.nbt.*`,
  and curated aliases like `name`, `noAi`, `lock`, and `lootTable`

## Local Testing

1. Open `editors/vscode-mcfc` in VS Code.
2. Run `cargo build --bin mcfc-lsp` from the repository root.
3. Run `npm install` and `npm run compile` from `editors/vscode-mcfc`.
4. Press `F5` to launch an Extension Development Host.
5. Open `syntaxes/test-cases/sample.mcf`, or an existing `.mcf` file from this
   repository.
6. Confirm VS Code detects the file as `MCFC`, starts `mcfc-lsp`, highlights the
   file, and reports diagnostics as you edit. Also open an `mcfc.toml` project
   manifest and verify its MCFC completions.

## Build, Watch, and Deploy

The extension contributes these commands: **MCFC: Build Project**, **Watch
Project**, **Stop Watch**, **Deploy Project**, **Build and Deploy**, and **Open
Generated Datapack**. The packaged extension includes both `mcfc` and
`mcfc-lsp`; set `mcfc.cli.path` only to override the bundled compiler.

Deployment is deliberately opt-in. Set `mcfc.deploy.datapacksDirectory` to a
Minecraft world's `datapacks` directory. `mcfc.deploy.packName` defaults to the
manifest namespace. An optional `mcfc.deploy.reloadCommand` runs after copying
and can use `${datapackPath}` and `${workspaceFolder}`. It is intentionally an
external command: the extension never pretends it can inject `/reload` into a
single-player game.

## Packaging

Run one of:

```bash
npm run package
npm run package:linux-x64
npm run package:win32-x64
```

The packaging flow builds `mcfc` and `mcfc-lsp` in release mode, clears any
previously staged payload, and copies exactly one matching CLI/server pair into
a platform-specific server directory before creating the VSIX.

Current packaged targets:

- Linux x64: `server/linux-x64/mcfc` and `server/linux-x64/mcfc-lsp`
- Windows x64: `server/win32-x64/mcfc.exe` and `server/win32-x64/mcfc-lsp.exe`

Important packaging/runtime expectations:

- VSIX artifacts are **platform-specific**.
- `npm run package` packages for the current host by default.
- `npm run package:linux-x64` and `npm run package:win32-x64` force a specific
  target and can be used for cross-packaging when the matching Rust target
  toolchain is installed.
- macOS is not currently supported.
- If you install a mismatched VSIX, activation now fails with an explicit error
  instead of silently missing the language server binary.

## Install in VSCodium

On Linux, you can build and install the extension in one command:

```bash
./scripts/install-vscodium-extension.sh
```

That script:

1. runs `npm install`
2. runs `npm run package`
3. installs the generated VSIX with `codium --install-extension`
