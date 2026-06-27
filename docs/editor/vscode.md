# VS Code

The VS Code extension in `editors/vscode-mcfc` provides syntax highlighting, editor commands, manifest tooling, and a bundled Rust language server.

## Language Server Features

The bundled `mcfc-lsp` server provides:

- compiler-backed diagnostics for `.mcf` source
- diagnostics and completions for Bukkit-style `data`, `event`, `command`, and `task` declarations
- symbols, semantic highlighting, folding ranges, selection ranges, formatting, document highlights, definitions, references, rename, and signature help
- hovers and completions for functions, locals, types, methods, host modules, payload structs, vanilla events, and agent events
- manifest diagnostics, symbols, and completions for `mcfc.toml`

## Local Testing

```powershell
cargo build --bin mcfc-lsp
cd editors/vscode-mcfc
npm install
npm run compile
```

Then press `F5` in VS Code to launch an Extension Development Host. Open a `.mcf` source file or `mcfc.toml` manifest and confirm diagnostics, highlighting, and completions.

## Project Commands

The extension contributes:

- MCFC: Build Project
- MCFC: Watch Project
- MCFC: Stop Watch
- MCFC: Deploy Project
- MCFC: Build and Deploy
- MCFC: Open Generated Datapack

Deployment is opt-in. Set `mcfc.deploy.datapacksDirectory` to a Minecraft world's `datapacks` directory. `mcfc.deploy.packName` defaults to the manifest namespace.

## Packaging

```bash
npm run package
npm run package:linux-x64
npm run package:win32-x64
```

VSIX artifacts are platform-specific. Current packaged targets are Linux x64 and Windows x64. macOS is not currently supported.
