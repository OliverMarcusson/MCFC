# MCFC Syntax Highlighting

VS Code syntax highlighting and editor basics for MCFC `.mcf` files.

This extension registers the `mcfc` language id, associates it with `.mcf`
files, and provides a static TextMate grammar plus basic editor behavior for
comments, brackets, auto-closing pairs, and indentation.

## Local Testing

1. Open `editors/vscode-mcfc` in VS Code.
2. Press `F5` to launch an Extension Development Host.
3. Open `syntaxes/test-cases/sample.mcf`, or an existing `.mcf` file from this
   repository.
4. Confirm VS Code detects the file as `MCFC` and highlights comments, strings,
   keywords, types, built-ins, operators, ranges, and `mcf` placeholders.

## Scope

This is a v1 grammar-only extension. It does not include diagnostics, compiler
integration, formatting, completion, semantic highlighting, or a language
server.
