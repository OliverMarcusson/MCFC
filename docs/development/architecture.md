# Architecture

MCFC is a Rust workspace with the compiler, CLI, language server, helper daemon, Java agent, examples, and editor integration in one repository.

## Compiler Pipeline

```text
.mcf source
  -> lexer
  -> parser
  -> AST
  -> module resolution and hook normalization (entry points, handlers, @Tick)
  -> type analysis
  -> IR lowering
  -> conservative optimization
  -> datapack backend
  -> whole-pack optimization
```

Key modules:

- `src/lexer.rs`, `src/parser.rs`, `src/ast.rs`: frontend syntax
- `src/types.rs`, `src/analysis.rs`: type checking and editor-facing analysis
- `src/ir.rs`, `src/optimizer.rs`: lowering and optimization
- `src/backend.rs`: datapack generation
- `src/pack_opt.rs`: whole-pack optimizer over the emitted commands
- `src/project.rs`: manifest discovery and project file collection
- `src/cli.rs`: command-line workflow
- `src/lsp.rs`: language server implementation
- `src/language_catalog.rs`: shared public language metadata

## Runtime Pieces

`mcfd` discovers generated pack descriptors, tails Minecraft logs, dispatches host capability calls, and writes results back through generated inbox functions.

`mcfd-agent` is a version-pinned optional adapter that can emit structured Minecraft event records and route no-argument root commands.

## Editor Integration

The VS Code extension bundles platform-specific `mcfc` and `mcfc-lsp` binaries, contributes commands, and uses the Rust language server for compiler-backed editing.
