# Task: TP-003 — Extract Compiler Driver and Named Pipeline Passes

**Created:** 2026-04-12
**Size:** L

## Review Level: 2 (Plan + Code)

**Assessment:** Core refactor of compiler orchestration that separates pure compilation from project/file IO while preserving behavior.
**Score:** 5/8 — Blast radius: 2, Pattern novelty: 1, Security: 0, Reversibility: 2

## Canonical Task Folder

```
taskplane-tasks/TP-003-extract-compiler-driver-pipeline/
├── PROMPT.md   ← This file (immutable above --- divider)
├── STATUS.md   ← Execution state (worker updates this)
├── .reviews/   ← Reviewer output (task-runner creates this)
└── .DONE       ← Created when complete
```

## Mission

Refactor the current compile entrypoints so the core pipeline is explicit and reusable: parse, normalize, type-check, lower, optimize, emit. Project loading, source merging, path resolution, and output writing should move into driver-oriented modules. CLI and LSP should both call shared driver APIs instead of embedding compiler orchestration details.

## Dependencies

- **Task:** TP-002

## Context to Read First

- `AGENTS.md`
- `docs/compiler-extension-architecture.md`
- `src/compiler.rs`
- `src/cli.rs`
- `src/project.rs`
- `src/analysis.rs`
- `src/lsp.rs`
- `tests/integration.rs`

## Environment

- **Workspace:** Project root
- **Services required:** None

## File Scope

- `src/compiler.rs`
- `src/cli.rs`
- `src/project.rs`
- `src/lib.rs`
- `src/lsp.rs`
- `src/analysis.rs`
- `src/driver.rs`
- `src/pipeline.rs`
- `tests/pipeline_driver.rs`
- `tests/integration.rs`

## Steps

### Step 0: Preflight

- [ ] Read the architecture RFC and identify the minimum refactor needed to separate pure pipeline code from driver/file-system code.
- [ ] Inventory all current callers of compile/project helper functions so behavior stays aligned.

### Step 1: Introduce the Shared Pipeline Surface

- [ ] Create named pass-oriented APIs in new pipeline/driver modules so `compile_source` and related helpers have clear, testable boundaries.
- [ ] Move special-function normalization into an explicit pass rather than an inline helper hidden inside the compile path.

### Step 2: Move IO and Project Concerns into the Driver

- [ ] Relocate manifest loading, source merging, asset copying, and output writing away from the pure pipeline layer.
- [ ] Update CLI and LSP entrypoints to use the shared driver/session surface without changing user-facing behavior.

### Step 3: Regression Coverage

- [ ] Create `tests/pipeline_driver.rs` to cover pure pipeline use and project/file-driver use separately.
- [ ] Update existing integration tests only where internal refactoring changes helper boundaries, not public behavior.

### Step 4: Testing & Verification

- [ ] Run `cargo fmt -- --check`.
- [ ] Run `cargo test -q`.

### Step 5: Delivery

## Documentation Requirements

**Must Update:** `src/lib.rs`, `tests/pipeline_driver.rs`
**Check If Affected:** `src/lsp.rs`, `docs/compiler-extension-architecture.md`, `LANGUAGE.md`

## Completion Criteria

- [ ] Pure compilation stages are isolated from project/file-system concerns behind explicit driver and pipeline modules.
- [ ] CLI and LSP reuse shared compiler-driver code instead of duplicating orchestration logic.
- [ ] `tests/pipeline_driver.rs` covers the new boundaries and the full test suite passes.

## Git Commit Convention

- **Implementation:** `feat(TP-003): description`
- **Checkpoints:** `checkpoint: TP-003 description`

## Do NOT

- Change language semantics or diagnostic wording unless absolutely required by the refactor.
- Rename generated datapack files or alter output layout.
- Mix registry/plugin work into this refactor; keep the task focused on architecture extraction.

---

## Amendments (Added During Execution)

<!-- Workers add amendments here if issues discovered during execution. -->
