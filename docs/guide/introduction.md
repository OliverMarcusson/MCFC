# Introduction

MCFC is a statically typed language, compiler, and language server for building Minecraft datapacks from `.mcf` source files. It targets Minecraft `26.2` and focuses on a compact typed core that still produces inspectable vanilla datapack output.

The repository currently provides:

- `mcfc`, a command-line datapack compiler
- `mcfc`, a Rust library crate
- `mcfc-lsp`, a language server for editor integrations
- a VS Code extension under `editors/vscode-mcfc`
- `mcfd`, an optional host-bridge helper service
- `mcfd-agent`, an optional experimental Java instrumentation agent

::: warning Early-stage software
MCFC is actively evolving. Language behavior, generated output, and helper runtime surfaces may change between commits.
:::

## What MCFC Compiles

The compiler writes a normal datapack:

- `pack.mcmeta`
- generated functions under `data/<namespace>/function/`
- load and tick tags when needed
- public wrappers for zero-argument `void` functions other than `main` and special `tick`
- optional `mcfd.pack.toml` descriptors when helper capabilities are enabled

The generated code is designed to remain vanilla-first. The host bridge and agent are opt-in additions, not requirements for loading a datapack.

## Where To Go Next

- New to the project: start with [Getting Started](./getting-started.md).
- Writing code: read the [Language Overview](/language/overview.md).
- Calling outside services: read the [Host Bridge](/runtime/host-bridge.md).
- Editor setup: read [VS Code](/editor/vscode.md).
