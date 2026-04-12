# TP-006: Create a Versioned Plugin API and Static Rust Extension Loading — Status

**Current Step:** Not Started
**Status:** 🔵 Ready for Execution
**Last Updated:** 2026-04-12
**Review Level:** 3
**Review Counter:** 0
**Iteration:** 0
**Size:** L

---

### Step 0: Preflight
**Status:** ⬜ Not Started

- [ ] Confirm the registry foundations are sufficient to host a versioned plugin API without reopening core architecture questions.
- [ ] Decide the minimum API surface needed for V1 static extensions: metadata, callable/type registration, and documentation hooks.

---

### Step 1: Create the Plugin API Boundary
**Status:** ⬜ Not Started

- [ ] Add `crates/mcfc-plugin-api/` and any required Cargo workspace wiring for a versioned extension-facing API.
- [ ] Define a narrow, test-backed registration interface used by the compiler to populate registries from extension providers.

---

### Step 2: Load Static/In-Tree Extensions
**Status:** ⬜ Not Started

- [ ] Implement static extension loading so built-in or in-tree Rust extensions register through the plugin API instead of bespoke code paths.
- [ ] Add project and/or CLI configuration for enabling and disabling known static extensions during a compilation run.

---

### Step 3: Regression Coverage
**Status:** ⬜ Not Started

- [ ] Create `tests/plugin_api.rs` using a small in-tree sample extension to verify registration, loading, and configuration behavior.
- [ ] Keep the tests concrete enough that “already works” shortcuts are impossible.

---

### Step 4: Testing & Verification
**Status:** ⬜ Not Started

- [ ] Run `cargo fmt -- --check`.
- [ ] Run `cargo test -q`.
- [ ] Run `cargo build`.

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
