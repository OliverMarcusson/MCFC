# Task: TP-006 — Create a Versioned Plugin API and Static Rust Extension Loading

**Created:** 2026-04-12
**Size:** L

## Review Level: 3 (Full)

**Assessment:** Defines the first stable-ish extension boundary, introduces workspace/crate changes, and wires project-level enable/disable support.
**Score:** 6/8 — Blast radius: 2, Pattern novelty: 2, Security: 0, Reversibility: 2

## Canonical Task Folder

```
taskplane-tasks/TP-006-create-plugin-api-and-static-loading/
├── PROMPT.md   ← This file (immutable above --- divider)
├── STATUS.md   ← Execution state (worker updates this)
├── .reviews/   ← Reviewer output (task-runner creates this)
└── .DONE       ← Created when complete
```

## Mission

Create the first versioned Rust extension API for MCFC and prove it with statically linked/in-tree extensions before tackling out-of-process plugins. This task should add an explicit plugin API boundary, load built-in/static extensions through the registry/session system, and allow project or CLI configuration to enable and disable known extensions.

## Dependencies

- **Task:** TP-004

## Context to Read First

- `AGENTS.md`
- `docs/compiler-extension-architecture.md`
- `Cargo.toml`
- `src/lib.rs`
- `src/project.rs`
- `src/cli.rs`
- `src/extensions/registry.rs`

## Environment

- **Workspace:** Project root
- **Services required:** None

## File Scope

- `Cargo.toml`
- `src/lib.rs`
- `src/project.rs`
- `src/cli.rs`
- `src/extensions/**/*`
- `crates/mcfc-plugin-api/**/*`
- `tests/plugin_api.rs`

## Steps

### Step 0: Preflight

- [ ] Confirm the registry foundations are sufficient to host a versioned plugin API without reopening core architecture questions.
- [ ] Decide the minimum API surface needed for V1 static extensions: metadata, callable/type registration, and documentation hooks.

### Step 1: Create the Plugin API Boundary

- [ ] Add `crates/mcfc-plugin-api/` and any required Cargo workspace wiring for a versioned extension-facing API.
- [ ] Define a narrow, test-backed registration interface used by the compiler to populate registries from extension providers.

### Step 2: Load Static/In-Tree Extensions

- [ ] Implement static extension loading so built-in or in-tree Rust extensions register through the plugin API instead of bespoke code paths.
- [ ] Add project and/or CLI configuration for enabling and disabling known static extensions during a compilation run.

### Step 3: Regression Coverage

- [ ] Create `tests/plugin_api.rs` using a small in-tree sample extension to verify registration, loading, and configuration behavior.
- [ ] Keep the tests concrete enough that “already works” shortcuts are impossible.

### Step 4: Testing & Verification

- [ ] Run `cargo fmt -- --check`.
- [ ] Run `cargo test -q`.
- [ ] Run `cargo build`.

### Step 5: Delivery

## Documentation Requirements

**Must Update:** `Cargo.toml`, `crates/mcfc-plugin-api/**/*`, `tests/plugin_api.rs`
**Check If Affected:** `docs/compiler-extension-architecture.md`, `LANGUAGE.md`, `editors/vscode-mcfc`

## Completion Criteria

- [ ] A versioned plugin API exists in `crates/mcfc-plugin-api/` and is used by at least one static/in-tree extension path.
- [ ] Project or CLI configuration can enable/disable known static extensions.
- [ ] `tests/plugin_api.rs` exists and `cargo fmt -- --check`, `cargo test -q`, and `cargo build` all pass.

## Git Commit Convention

- **Implementation:** `feat(TP-006): description`
- **Checkpoints:** `checkpoint: TP-006 description`

## Do NOT

- Implement external plugin processes yet.
- Expose unstable compiler internals directly through the public plugin API.
- Accept unversioned extension loading semantics.

---

## Amendments (Added During Execution)

<!-- Workers add amendments here if issues discovered during execution. -->
