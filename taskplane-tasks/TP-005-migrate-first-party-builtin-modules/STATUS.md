# TP-005: Migrate Selected First-Party Builtins into Modular Registrations — Status

**Current Step:** Not Started
**Status:** 🔵 Ready for Execution
**Last Updated:** 2026-04-12
**Review Level:** 2
**Review Counter:** 0
**Iteration:** 0
**Size:** M

---

### Step 0: Preflight
**Status:** ⬜ Not Started

- [ ] Choose the exact builtin families to migrate in this slice and verify they are small enough to finish without absorbing unrelated backend work.
- [ ] Map each selected builtin to the typing, lowering, and emission hooks it currently relies on.

---

### Step 1: Extract First-Party Builtin Modules
**Status:** ⬜ Not Started

- [ ] Create dedicated builtin modules for the selected feature families and register them through the new registry/session system.
- [ ] Move typing/lowering/emission-specific logic out of monolithic branches only after replacement paths are wired and tested.

---

### Step 2: Remove Monolithic Duplicates
**Status:** ⬜ Not Started

- [ ] Delete or simplify the obsolete hardcoded paths for the migrated builtins once parity is verified.
- [ ] Keep untouched builtin families in place; do not broaden scope to entity/world/block systems in this task.

---

### Step 3: Regression Coverage
**Status:** ⬜ Not Started

- [ ] Create `tests/builtin_modules.rs` with focused cases for the migrated builtin families.
- [ ] Retain or update broader integration assertions where generated command fragments must stay stable.

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
