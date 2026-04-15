# MCFC Language Support

VS Code syntax highlighting, editor basics, and language server support for
MCFC `.mcf` files.

This extension registers the `mcfc` language id, associates it with `.mcf`
files, and provides a static TextMate grammar, basic editor behavior, and a
Rust language server.

## Language Server

The bundled `mcfc-lsp` server provides:

- compiler-backed diagnostics
- function document symbols
- hovers for functions, locals, built-ins, and methods
- completions for keywords, types, built-ins, functions, locals, and common
  member-access surfaces
- indentation-based block syntax with `:` headers, including `player_state`
  declarations

That includes the builder-oriented gameplay surface, such as:

- `entity("minecraft:pig")`, `block_type("minecraft:chest")`, and `item("minecraft:apple")`
- `summon(entity_def)` plus explicit-position `block("~ ~ ~").summon(...)`
- `entity_def.as_nbt()`, `block_def.as_nbt()`, and `item_def.as_nbt()`
- implicit builder-to-`nbt` coercion in NBT contexts such as
  `pig.nbt.Passengers[0] = chicken`
- player inventory completions for `player.inventory[0].*`, `player.hotbar[0].*`,
  and explicit `player_ref` values
- member completions for `entity_def.nbt.*`, `block_def.states.*`, `item_def.nbt.*`,
  and curated aliases like `name`, `no_ai`, `lock`, and `loot_table`

## Local Testing

1. Open `editors/vscode-mcfc` in VS Code.
2. Run `cargo build --bin mcfc-lsp` from the repository root.
3. Run `npm install` and `npm run compile` from `editors/vscode-mcfc`.
4. Press `F5` to launch an Extension Development Host.
5. Open `syntaxes/test-cases/sample.mcf`, or an existing `.mcf` file from this
   repository.
6. Confirm VS Code detects the file as `MCFC`, starts `mcfc-lsp`, highlights the
   file, and reports diagnostics as you edit.

## Packaging

Run:

```bash
npm run package
```

The packaging script builds `mcfc-lsp` in release mode, clears any previously
staged server payload, and copies exactly one native server binary into a
platform-specific server directory before creating the VSIX.

Current packaged targets:

- Linux x64: `server/linux-x64/mcfc-lsp`
- Windows x64: `server/win32-x64/mcfc-lsp.exe`

Important packaging/runtime expectations:

- VSIX artifacts are **platform-specific**.
- Build the VSIX on the same target platform you intend to install it on.
- A Linux-built VSIX contains only the Linux server; a Windows-built VSIX
  contains only the Windows server.
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
