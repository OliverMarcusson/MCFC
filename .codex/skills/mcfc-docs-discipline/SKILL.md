---
name: mcfc-docs-discipline
description: Keep MCFC documentation continuously synchronized with language, compiler, runtime, LSP, examples, and generated datapack behavior. Use when changing MCFC syntax, types, builtins, methods, events, lowering, diagnostics, examples, VitePress docs, LANGUAGE.md, README language sections, src/language_catalog.rs, src/types.rs, src/backend.rs, src/parser.rs, src/analysis.rs, src/lsp.rs, tests, or any behavior that affects how users write .mcf code or how MCFC translates to mcfunction.
---

# MCFC Docs Discipline

Use this skill to make documentation part of language development, not a cleanup task after the fact.

## Core Rule

For every MCFC language, compiler, runtime, LSP, or example change, decide explicitly whether documentation must change. If behavior changed and docs did not, state why in the final response.

## Source Of Truth

Before editing docs, inspect the implementation source that owns the behavior:

- Syntax and declarations: `src/parser.rs`, `src/ast.rs`, `src/lexer.rs`
- Types, methods, builtins, payload structs: `src/types.rs`, `src/analysis.rs`
- Events and public names: `src/language_catalog.rs`, `src/backend.rs`, `src/compiler.rs`
- Lowering and generated mcfunction behavior: `src/backend.rs`
- Editor-visible completions, hovers, snippets: `src/lsp.rs`
- Expected behavior examples: `tests/integration.rs`, `examples/**`

Do not document aspirational behavior unless the user explicitly asks for future-facing docs.

## Documentation Targets

Update all relevant surfaces:

- `docs/language/reference/**`: canonical reference pages. Add or update a dedicated page for each keyword, type, builtin, method, event, or runtime concept touched.
- `docs/language/reference/lowering.md`: update when generated mcfunction, scoreboard/storage representation, scheduling, macros, events, or helper transport changes.
- `docs/language/*.md`: keep overview pages accurate and link to detailed reference pages.
- `LANGUAGE.md`: keep the root language reference aligned with implemented behavior.
- `README.md`: update only when the user-facing summary, quick examples, or advertised capability changed.
- `examples/**` and `docs/examples/**`: update when examples demonstrate changed or newly preferred syntax.

When adding a public language item, create both the aggregate entry and the leaf page.

## Workflow

1. Identify whether the change affects user-facing language behavior, generated datapack behavior, editor behavior, or examples.
2. Read the implementation source of truth before writing docs.
3. Update the narrowest complete set of docs in the same change.
4. Include "Under The Hood" notes where generated mcfunction behavior changes or where users must understand scoreboards, storage, macros, schedules, selectors, or agent dispatch.
5. Keep examples concise and valid `mcfc` fences.
6. Run `npm run docs:build` after docs edits.
7. If compiler behavior changed, also run the relevant Rust tests or explain why they were not run.

## Reference Page Expectations

Each dedicated reference page should include:

- syntax or signature
- what it does
- one minimal example
- constraints or type rules when relevant
- an `Under The Hood` section when the lowering model affects user understanding

Aggregate pages should link to leaf pages rather than duplicating all detail.

## Final Response Checklist

Mention:

- which docs were updated
- whether a new reference page was added
- validation run, especially `npm run docs:build`
- any known documentation gap intentionally left out
