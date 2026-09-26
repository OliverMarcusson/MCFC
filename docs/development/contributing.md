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

Optimizer check. It builds the programs in `scripts/pack-sim/programs` with and without optimization, runs them in a small mcfunction simulator, and fails when output differs from the `.expected` file or the optimized build runs more commands than its budget in `check.py`:

```powershell
python scripts/pack-sim/check.py
```

When the optimizer gets faster, lower the budgets. `PROFILE=raw python scripts/pack-sim/mcsim.py <pack-dir>` prints the most executed lines.

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
cargo build --release
npm run docs:check   # compiles every mcfc code block in docs/
npm run docs:build
npm run docs:dev -- --host 127.0.0.1
```

`docs:check` compiles each ```` ```mcfc ```` block as its own project, with every capability and the agent enabled. Put `<!-- no-check -->` on the line before a block that isn't a complete program, such as a multi-file example. The production docs output is generated under `docs/.vitepress/dist/`.
