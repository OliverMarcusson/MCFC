# TP-003: Extract Compiler Driver and Named Pipeline Passes — Status

**Current Step:** Not Started
**Status:** 🔵 Ready for Execution
**Last Updated:** 2026-04-12
**Review Level:** 2
**Review Counter:** 0
**Iteration:** 0
**Size:** L

---

### Step 0: Preflight
**Status:** ⬜ Not Started

- [ ] Read the architecture RFC and identify the minimum refactor needed to separate pure pipeline code from driver/file-system code.
- [ ] Inventory all current callers of compile/project helper functions so behavior stays aligned.

---

### Step 1: Introduce the Shared Pipeline Surface
**Status:** ⬜ Not Started

- [ ] Create named pass-oriented APIs in new pipeline/driver modules so `compile_source` and related helpers have clear, testable boundaries.
- [ ] Move special-function normalization into an explicit pass rather than an inline helper hidden inside the compile path.

---

### Step 2: Move IO and Project Concerns into the Driver
**Status:** ⬜ Not Started

- [ ] Relocate manifest loading, source merging, asset copying, and output writing away from the pure pipeline layer.
- [ ] Update CLI and LSP entrypoints to use the shared driver/session surface without changing user-facing behavior.

---

### Step 3: Regression Coverage
**Status:** ⬜ Not Started

- [ ] Create `tests/pipeline_driver.rs` to cover pure pipeline use and project/file-driver use separately.
- [ ] Update existing integration tests only where internal refactoring changes helper boundaries, not public behavior.

---

### Step 4: Testing & Verification
**Status:** ⬜ Not Started

- [ ] Run `cargo fmt -- --check`.
- [ ] Run `cargo test -q`.

---

### Step 5: Delivery
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
