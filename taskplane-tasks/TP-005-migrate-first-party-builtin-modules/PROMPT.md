# Task: TP-005 — Migrate Selected First-Party Builtins into Modular Registrations

**Created:** 2026-04-12
**Size:** M

## Review Level: 2 (Plan + Code)

**Assessment:** Converts a bounded builtin slice into first-party modules to prove the registry architecture works against real features.
**Score:** 4/8 — Blast radius: 2, Pattern novelty: 1, Security: 0, Reversibility: 1

## Canonical Task Folder

```
taskplane-tasks/TP-005-migrate-first-party-builtin-modules/
├── PROMPT.md   ← This file (immutable above --- divider)
├── STATUS.md   ← Execution state (worker updates this)
├── .reviews/   ← Reviewer output (task-runner creates this)
└── .DONE       ← Created when complete
```

## Mission

Prove the modular architecture with real compiler features by extracting a bounded set of first-party builtins out of monolithic logic and registering them through the new registry system. Keep the scope narrow and high-signal: migrate low-risk but meaningful feature families such as `random`/runtime helpers and bossbar or UI helpers, while preserving exact existing behavior.

## Dependencies

- **Task:** TP-004

## Context to Read First

- `AGENTS.md`
- `docs/compiler-extension-architecture.md`
- `src/backend.rs`
- `src/types.rs`
- `src/analysis.rs`
- `src/extensions/registry.rs`
- `tests/integration.rs`

## Environment

- **Workspace:** Project root
- **Services required:** None

## File Scope

- `src/backend.rs`
- `src/types.rs`
- `src/analysis.rs`
- `src/lib.rs`
- `src/builtins/mod.rs`
- `src/builtins/random.rs`
- `src/builtins/ui.rs`
- `tests/builtin_modules.rs`
- `tests/integration.rs`

## Steps

### Step 0: Preflight

- [ ] Choose the exact builtin families to migrate in this slice and verify they are small enough to finish without absorbing unrelated backend work.
- [ ] Map each selected builtin to the typing, lowering, and emission hooks it currently relies on.

### Step 1: Extract First-Party Builtin Modules

- [ ] Create dedicated builtin modules for the selected feature families and register them through the new registry/session system.
- [ ] Move typing/lowering/emission-specific logic out of monolithic branches only after replacement paths are wired and tested.

### Step 2: Remove Monolithic Duplicates

- [ ] Delete or simplify the obsolete hardcoded paths for the migrated builtins once parity is verified.
- [ ] Keep untouched builtin families in place; do not broaden scope to entity/world/block systems in this task.

### Step 3: Regression Coverage

- [ ] Create `tests/builtin_modules.rs` with focused cases for the migrated builtin families.
- [ ] Retain or update broader integration assertions where generated command fragments must stay stable.

### Step 4: Testing & Verification

- [ ] Run `cargo fmt -- --check`.
- [ ] Run `cargo test -q`.

### Step 5: Delivery

## Documentation Requirements

**Must Update:** `src/lib.rs`, `tests/builtin_modules.rs`
**Check If Affected:** `docs/compiler-extension-architecture.md`, `src/lsp.rs`, `editors/vscode-mcfc`

## Completion Criteria

- [ ] At least one meaningful builtin family is implemented as a first-party module registered through the new registry system.
- [ ] Monolithic fallback logic for the migrated families is removed or significantly simplified.
- [ ] `tests/builtin_modules.rs` exists and the full test suite passes.

## Git Commit Convention

- **Implementation:** `feat(TP-005): description`
- **Checkpoints:** `checkpoint: TP-005 description`

## Do NOT

- Expand scope to every builtin family in the compiler.
- Change command output for migrated builtins unless tests are intentionally updated for a documented reason.
- Introduce external plugin loading in this task.

---

## Amendments (Added During Execution)

<!-- Workers add amendments here if issues discovered during execution. -->
