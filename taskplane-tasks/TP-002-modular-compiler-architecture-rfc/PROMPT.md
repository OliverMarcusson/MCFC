# Task: TP-002 — Modular Compiler & Extension Architecture RFC

**Created:** 2026-04-12
**Size:** M

## Review Level: 1 (Plan Only)

**Assessment:** Architecture-planning task that defines module boundaries, extension tiers, and migration constraints before code changes land.
**Score:** 3/8 — Blast radius: 1, Pattern novelty: 1, Security: 0, Reversibility: 1

## Canonical Task Folder

```
taskplane-tasks/TP-002-modular-compiler-architecture-rfc/
├── PROMPT.md   ← This file (immutable above --- divider)
├── STATUS.md   ← Execution state (worker updates this)
├── .reviews/   ← Reviewer output (task-runner creates this)
└── .DONE       ← Created when complete
```

## Mission

Produce a concrete architecture document for making MCFC modular and Rust-extension-friendly. The document should translate the current repo state into a target design: frontend/sema/IR/backend/driver/tooling boundaries, builtin registries, in-process vs out-of-process extension tiers, migration phases, risks, and success criteria. This task is the contract for the implementation tasks that follow.

## Dependencies

- **None**

## Context to Read First

- `AGENTS.md`
- `Cargo.toml`
- `src/compiler.rs`
- `src/backend.rs`
- `src/analysis.rs`
- `src/lsp.rs`
- `tests/integration.rs`

## Environment

- **Workspace:** Project root
- **Services required:** None

## File Scope

- `docs/compiler-extension-architecture.md`

## Steps

### Step 0: Preflight

- [ ] Review the current compiler, backend, analysis, and tooling entrypoints to capture real constraints for modularization.
- [ ] Confirm the architecture doc covers both compiler modularization and Rust extension support.

### Step 1: Draft the Architecture RFC

- [ ] Create `docs/compiler-extension-architecture.md` with current-state analysis and target module boundaries.
- [ ] Define the extension capability model: builtin registries, first-party modules, static Rust extensions, and external plugin processes.
- [ ] Document non-goals for V1, especially avoiding arbitrary syntax plugins until semantic extension points are stable.

### Step 2: Add the Migration Roadmap

- [ ] Add phased implementation waves with ordering, dependencies, risks, and explicit success criteria for each phase.
- [ ] Include a file-ownership matrix that maps current monolithic files to future modules/crates.

### Step 3: Testing & Verification

- [ ] Run `cargo test -q` to verify the repository still passes after documentation changes.
- [ ] Proofread the RFC for repo-specific accuracy against the current source tree.

### Step 4: Delivery

## Documentation Requirements

**Must Update:** `docs/compiler-extension-architecture.md`
**Check If Affected:** `AGENTS.md`, `LANGUAGE.md`

## Completion Criteria

- [ ] `docs/compiler-extension-architecture.md` exists and describes target compiler boundaries, extension tiers, and migration phases.
- [ ] The RFC includes risks, non-goals, and concrete success criteria that later implementation tasks can follow.

## Git Commit Convention

- **Implementation:** `feat(TP-002): description`
- **Checkpoints:** `checkpoint: TP-002 description`

## Do NOT

- Change compiler behavior in this task.
- Introduce code refactors beyond lightweight doc-support edits needed for the RFC.
- Promise a public plugin API surface that is not explicitly marked provisional.

---

## Amendments (Added During Execution)

<!-- Workers add amendments here if issues discovered during execution. -->
