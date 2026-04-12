# TP-004: Introduce Registry Foundations for Types, Callables, and Intrinsics — Status

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

- [ ] Identify the current builtin resolution paths across typing, analysis, and lowering-related code.
- [ ] Choose the smallest registry surface that can support current builtins without changing user-visible behavior.

---

### Step 1: Define the Registry Model
**Status:** ⬜ Not Started

- [ ] Create extension/registry module(s) for builtin types, callables, methods, and intrinsics.
- [ ] Introduce a compiler session or context object that owns the active registry set for a compilation run.

---

### Step 2: Route Existing Builtin Resolution Through Registries
**Status:** ⬜ Not Started

- [ ] Refactor type checking and analysis to consult the active registries instead of hardcoding builtin knowledge directly in large match branches.
- [ ] Bootstrap a default core registry so current language behavior remains intact with no external extensions enabled.

---

### Step 3: Regression Coverage
**Status:** ⬜ Not Started

- [ ] Create `tests/registry_foundation.rs` covering registry-backed builtin lookup and failure cases.
- [ ] Keep or extend integration coverage for representative builtin-heavy programs.

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
