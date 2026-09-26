# VS Code

The extension in `editors/vscode-mcfc` adds syntax highlighting and a language server for `.mcf` files and `mcfc.toml`. The language server shows errors as you type, and provides completion, hover, go to definition, rename and formatting. The extension bundles its own `mcfc` and `mcfc-lsp`, so it doesn't need anything on `PATH`.

## Install

Build a VSIX for your platform (Windows x64 or Linux x64; macOS isn't supported) and install it:

```powershell
cd editors/vscode-mcfc
npm install
npm run package:win32-x64
code --install-extension mcfc-syntax-win32-x64-0.1.0.vsix
```

## Commands

Run these from the Command Palette:

| Command | Does |
| --- | --- |
| MCFC: Build Project | `mcfc build` on the current project |
| MCFC: Watch Project / Stop Watch | Rebuilds on save |
| MCFC: Deploy Project | Copies the built pack into a world |
| MCFC: Build and Deploy | Both |
| MCFC: Open Generated Datapack | Opens the output folder |

## Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `mcfc.deploy.datapacksDirectory` | none | A world's `datapacks` folder. Deploy is disabled until you set it. |
| `mcfc.deploy.packName` | namespace | Folder name inside `datapacks` |
| `mcfc.deploy.reloadCommand` | none | Shell command to run after deploying. `${datapackPath}` and `${workspaceFolder}` are replaced. |
| `mcfc.cli.path` | bundled | Use a different `mcfc` binary |

## Developing the extension

```powershell
cargo build --bin mcfc-lsp
cd editors/vscode-mcfc
npm install
npm run compile
```

Press `F5` in VS Code to open an Extension Development Host with the extension loaded.
