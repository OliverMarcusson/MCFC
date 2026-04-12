# Task: TP-004 — Introduce Registry Foundations for Types, Callables, and Intrinsics

**Created:** 2026-04-12
**Size:** L

## Review Level: 2 (Plan + Code)

**Assessment:** Adds the extension-aware semantic foundation needed to stop hardcoding builtin knowledge across the compiler.
**Score:** 5/8 — Blast radius: 2, Pattern novelty: 2, Security: 0, Reversibility: 1

## Canonical Task Folder

```
taskplane-tasks/TP-004-introduce-registry-foundations/
├── PROMPT.md   ← This file (immutable above --- divider)
├── STATUS.md   ← Execution state (worker updates this)
├── .reviews/   ← Reviewer output (task-runner creates this)
└── .DONE       ← Created when complete
```

## Mission

Introduce registry-based compiler foundations for builtin types, functions, methods, and intrinsic operations. The new model should allow the compiler session to carry a default registry so the type checker, analysis layer, and later lowering/backend stages can resolve language features through data-driven registrations instead of monolithic match trees.

## Dependencies

- **Task:** TP-003

## Context to Read First

- `AGENTS.md`
- `docs/compiler-extension-architecture.md`
- `src/types.rs`
- `src/analysis.rs`
- `src/compiler.rs`
- `src/backend.rs`
- `src/ir.rs`
- `tests/integration.rs`

## Environment

- **Workspace:** Project root
- **Services required:** None

## File Scope

- `src/types.rs`
- `src/analysis.rs`
- `src/lib.rs`
- `src/compiler.rs`
- `src/pipeline.rs`
- `src/extensions/mod.rs`
- `src/extensions/registry.rs`
- `tests/registry_foundation.rs`
- `tests/integration.rs`

## Steps

### Step 0: Preflight

- [ ] Identify the current builtin resolution paths across typing, analysis, and lowering-related code.
- [ ] Choose the smallest registry surface that can support current builtins without changing user-visible behavior.

### Step 1: Define the Registry Model

- [ ] Create extension/registry module(s) for builtin types, callables, methods, and intrinsics.
- [ ] Introduce a compiler session or context object that owns the active registry set for a compilation run.

### Step 2: Route Existing Builtin Resolution Through Registries

- [ ] Refactor type checking and analysis to consult the active registries instead of hardcoding builtin knowledge directly in large match branches.
- [ ] Bootstrap a default core registry so current language behavior remains intact with no external extensions enabled.

### Step 3: Regression Coverage

- [ ] Create `tests/registry_foundation.rs` covering registry-backed builtin lookup and failure cases.
- [ ] Keep or extend integration coverage for representative builtin-heavy programs.

### Step 4: Testing & Verification

- [ ] Run `cargo fmt -- --check`.
- [ ] Run `cargo test -q`.

### Step 5: Delivery

## Documentation Requirements

**Must Update:** `src/lib.rs`, `tests/registry_foundation.rs`
**Check If Affected:** `docs/compiler-extension-architecture.md`, `src/lsp.rs`, `LANGUAGE.md`

## Completion Criteria

- [ ] A compile session/registry foundation exists for types, callables, methods, and intrinsics.
- [ ] Core builtin lookup flows through the registry system without changing current language behavior.
- [ ] `tests/registry_foundation.rs` exists and the full test suite passes.

## Git Commit Convention

- **Implementation:** `feat(TP-004): description`
- **Checkpoints:** `checkpoint: TP-004 description`

## Do NOT

- Expose a broad public plugin API yet; keep the surface internal until proven by follow-up tasks.
- Attempt to migrate every builtin family in one pass.
- Let LSP or analysis create a separate registry model from the compiler pipeline.

---

## Amendments (Added During Execution)

<!-- Workers add amendments here if issues discovered during execution. -->
