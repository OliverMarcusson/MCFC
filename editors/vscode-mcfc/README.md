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

```powershell
npm run package
```

The packaging script builds `mcfc-lsp.exe` in release mode and copies it to
`server/win32-x64` before creating the VSIX. The initial packaged server target
is Windows only.
