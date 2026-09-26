---
name: mcfc-docs-discipline
description: Keep MCFC documentation continuously synchronized with language, compiler, runtime, LSP, examples, and generated datapack behavior. Use when changing MCFC syntax, types, builtins, methods, events, lowering, diagnostics, examples, VitePress docs, README, src/language_catalog.rs, src/types.rs, src/backend.rs, src/parser.rs, src/analysis.rs, src/lsp.rs, tests, or any behavior that affects how users write .mcf code or how MCFC translates to mcfunction.
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

The reference is one page per area. Add a section (with an anchor) to the right page; do not create per-item pages.

| Change | Page |
| --- | --- |
| Statement or declaration | `docs/language/reference/statements.md` |
| Type, type method, operator | `docs/language/reference/types.md` |
| Free function builtin | `docs/language/reference/builtins.md` |
| Entity/player method or field | `docs/language/reference/methods.md` |
| Builder field | `docs/language/reference/builders.md` |
| Event or payload | `docs/language/reference/events.md` |
| `std` function | `docs/language/reference/std.md` |
| Generated mcfunction layout | `docs/language/reference/lowering.md` |
| Something unsupported or with a sharp edge | `docs/language/limitations.md` |
| Host call or capability | `docs/runtime/capabilities.md` |

Also update, when affected:

- `docs/language/tour.md` when a user-visible feature is added (one short section, link to the reference).
- `docs/guide/cookbook.md` when a feature makes a common task easier.
- `docs/guide/first-pack.md` only if its code stops compiling or a simpler form exists.
- `README.md` only for install steps or the headline example.
- `examples/**`; `docs/examples/index.md` imports their source, so it updates itself.

`LANGUAGE.md` is only a pointer to the docs. Do not add content to it.

## Workflow

1. Identify whether the change affects user-facing language behavior, generated datapack behavior, editor behavior, or examples.
2. Read the implementation source of truth before writing docs. Test claims by compiling a small file; do not copy claims from older docs.
3. Update the narrowest complete set of docs in the same change.
4. Every ` ```mcfc ` block must be a complete program. Mark the rare exception (multi-file examples) with `<!-- no-check -->` on the line before. Signatures go in tables, not code blocks.
5. Run `cargo build --release && npm run docs:check && npm run docs:build`.
6. If compiler behavior changed, also run the relevant Rust tests or explain why they were not run.

## Writing Style

- Plain technical prose: say what it does, what it takes, what it returns, and what breaks it. No marketing adjectives, no "powerful", no "seamless".
- Lead with the example or table; keep prose to what the example does not show.
- Mention generated commands only where a user needs them to predict cost or behavior.
- Do not use `mc` or `mcf` in examples when a method, builtin or `debug(...)` does the job. They are a last resort. If an example genuinely needs one because the language lacks the feature, add the gap to `TODO.md` under LANGUAGE GAPS.

## Final Response Checklist

Mention:

- which docs were updated
- validation run: `npm run docs:check` and `npm run docs:build`
- any known documentation gap intentionally left out
