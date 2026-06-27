# Contributing

MCFC is early-stage, so keep changes scoped and verify compiler behavior with tests and smoke builds.

## Repository Layout

- `src/`: compiler, CLI, library, and LSP
- `tests/integration.rs`: main regression suite
- `examples/`: runnable packs and smoke-test projects
- `mcfd/`: host-bridge helper daemon
- `mcfd-agent/`: optional Java instrumentation agent
- `editors/vscode-mcfc/`: VS Code extension
- `docs/`: VitePress documentation site

## Rust Checks

```powershell
cargo fmt -- --check
cargo test -q
cargo build
cargo build --bin mcfc-lsp
```

Manual compiler smoke test:

```powershell
cargo run --bin mcfc -- build npc.mcf --out build/pack --clean
```

## VS Code Extension Checks

```powershell
cd editors/vscode-mcfc
npm install
npm run compile
npm run package
```

There is currently no `npm test` script for the VS Code extension.

## Docs Checks

```powershell
npm install
npm run docs:build
npm run docs:dev -- --host 127.0.0.1
```

The production docs output is generated under `docs/.vitepress/dist/`.
