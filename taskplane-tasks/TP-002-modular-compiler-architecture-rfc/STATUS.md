# TP-002: Modular Compiler & Extension Architecture RFC — Status

**Current Step:** Not Started
**Status:** 🔵 Ready for Execution
**Last Updated:** 2026-04-12
**Review Level:** 1
**Review Counter:** 0
**Iteration:** 0
**Size:** M

---

### Step 0: Preflight
**Status:** ⬜ Not Started

- [ ] Review the current compiler, backend, analysis, and tooling entrypoints to capture real constraints for modularization.
- [ ] Confirm the architecture doc covers both compiler modularization and Rust extension support.

---

### Step 1: Draft the Architecture RFC
**Status:** ⬜ Not Started

- [ ] Create `docs/compiler-extension-architecture.md` with current-state analysis and target module boundaries.
- [ ] Define the extension capability model: builtin registries, first-party modules, static Rust extensions, and external plugin processes.
- [ ] Document non-goals for V1, especially avoiding arbitrary syntax plugins until semantic extension points are stable.

---

### Step 2: Add the Migration Roadmap
**Status:** ⬜ Not Started

- [ ] Add phased implementation waves with ordering, dependencies, risks, and explicit success criteria for each phase.
- [ ] Include a file-ownership matrix that maps current monolithic files to future modules/crates.

---

### Step 3: Testing & Verification
**Status:** ⬜ Not Started

- [ ] Run `cargo test -q` to verify the repository still passes after documentation changes.
- [ ] Proofread the RFC for repo-specific accuracy against the current source tree.

---

### Step 4: Delivery
**Status:** ⬜ Not Started


---
## Reviews

| # | Type | Step | Verdict | File |
|---|------|------|---------|------|

---

## Discoveries

| Discovery | Disposition | Location |
|-----------|-------------|----------|

---

## Execution Log

| Timestamp | Action | Outcome |
|-----------|--------|---------|
| 2026-04-12 | Task staged | PROMPT.md and STATUS.md created |

---

## Blockers

*None*

---

## Notes

*Task staged from the modular compiler and extension-system roadmap.*
