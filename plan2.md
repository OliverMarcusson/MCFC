# Full MCDoc Parser Tightening Plan

## Goal
Tighten MCFC's build-time MCDoc ingestion so it can successfully ingest **all available `.mcdoc` files** from the pinned upstream `vanilla-mcdoc` tarball, not just the current NBT-focused subset.

This plan is about two things:
- **structural coverage**: the parser should accept the full upstream corpus without panicking
- **semantic preservation**: unsupported constructs should be represented explicitly instead of being silently dropped whenever practical

## Success Criteria
At the end of this work, we should have:
- tarball scanning over **all** `.mcdoc` files
- a parser that can parse the pinned corpus without hard failure
- coverage reporting that tells us which constructs are still degraded or unresolved
- reducer behavior that preserves more information for dynamic keys/dispatches instead of collapsing to `None` / `"nbt"`
- tests and fixtures for the syntax patterns that previously broke ingestion

## Current Gaps
Today the implementation still has several deliberate shortcuts:
- only `java/world/**` plus `java/util/avatar.mcdoc` are read from the tarball
- dynamic dispatch refs are parsed but not reduced
- dynamic struct entries are parsed but ignored in reduction
- enums are reduced to only their underlying scalar type
- generic arguments are skipped rather than modeled
- annotations are mostly skipped semantically
- unsupported constructs are often degraded to `Unknown(())` or `None`
- parser coverage is not measured corpus-wide yet

That means the current implementation is good enough for the current NBT completion slice, but not yet robust enough for full-corpus ingestion.

## Non-Goals
Not required for the first tightening pass:
- full MCDoc validation semantics
- runtime use of every upstream feature
- perfect preservation of every annotation/value constraint on day one
- building a general-purpose standalone MCDoc engine

## High-Level Strategy
Do this in three layers:

1. **Measure the real corpus**
   - stop guessing which syntax matters
   - gather concrete failure modes from the pinned tarball

2. **Broaden parser acceptance**
   - make the lexer/parser accept all top-level declarations and type forms in the corpus
   - prefer preserving unsupported structure over skipping it

3. **Tighten reduction semantics**
   - once parsing is stable, improve how dynamic keys, dispatches, enums, generics, and annotations feed the reduced schema

## Implementation Phases

### Phase 1: Corpus-wide coverage harness
Before broadening behavior, add instrumentation so we can see exactly what the pinned corpus needs.

Tasks:
- remove the current file filter in `build_mcdoc.rs` and enumerate all `.mcdoc` files in the tarball
- add a corpus scan mode inside the build-time parser module that records:
  - file count discovered
  - file count parsed successfully
  - file count failed
  - first parse error per failed file
  - counts of degraded constructs encountered
- keep normal build output behavior unchanged for production generation, but make the parser capable of reporting coverage in tests or debug runs
- add a small summary structure, e.g. `ParseCoverageReport`

Deliverable:
- a reproducible report showing which upstream files and grammar features still fail

### Phase 2: Top-level grammar completeness
Expand the file parser so it can accept every declaration kind used in the pinned corpus.

Tasks:
- audit upstream top-level declarations actually present in the tarball
- extend `FileParser::parse()` to explicitly handle each supported top-level form instead of falling back to token-by-token skipping
- preserve unknown top-level declarations as explicit placeholders rather than silently discarding them
- improve error messages so failures include file path, declaration kind, and nearby token context

Likely additions:
- richer `use` forms
- any top-level aliases/dispatch variants not currently modeled
- explicit placeholder nodes for unsupported declaration kinds

Deliverable:
- parser no longer relies on ad hoc skipping for top-level unsupported syntax

### Phase 3: Import and name-resolution support
Strengthen symbol resolution so all referenced upstream definitions can be resolved correctly.

Tasks:
- extend `use` parsing to support:
  - explicit aliases (`as`-style forms if present upstream)
  - grouped imports if present
  - wildcard-style imports if present
- preserve import metadata in `DefinitionContext`
- improve `resolve_reference(...)` so it handles all import forms deterministically
- add targeted tests for cross-file symbol resolution

Why this matters:
- full-corpus parsing is only useful if cross-file references resolve consistently

Deliverable:
- import coverage sufficient for all pinned upstream reference patterns

### Phase 4: Type grammar tightening
Refactor type parsing so bracket-heavy and nested forms are handled by clear helpers instead of ad hoc branching.

Tasks:
- split `parse_type_atom()` into smaller helpers for:
  - primary type parsing
  - suffix parsing
  - list/array-like suffix parsing
  - dispatch-key parsing
  - generic argument parsing
- preserve richer type information in `TypeExpr` instead of collapsing early
- add dedicated handling for:
  - nested bracket expressions
  - tuple/group forms
  - constrained scalars/ranges
  - literal/string forms used in unions
  - nested generic parameter lists
- make annotation skipping safe in all nested delimiter contexts

Suggested AST expansion:
- keep existing variants, but consider adding:
  - `LiteralString`
  - `LiteralNumber`
  - `Constrained`
  - `GenericInstance`
  - `MapLike` or more explicit dynamic-key representation

Deliverable:
- parser accepts the full range of type expressions used upstream

### Phase 5: Dynamic fields and dispatch semantics
Stop erasing the most important dynamic schema constructs.

Tasks:
- preserve actual dynamic-key expressions in `StructEntry::Dynamic`
- preserve actual dynamic dispatch expressions in `DispatchRefKey`
- teach the reducer to represent unresolved dynamic structure explicitly instead of dropping it
- add a reduced-schema representation for map-like/dynamic compounds
- where exact reduction is impossible, surface a meaningful detail string instead of `"nbt"`

Examples of desired behavior:
- `minecraft:block_entity[[id]]` should remain visibly dynamic even if not fully resolved
- `[#[id="..."] string]: T` should reduce to something map-like instead of disappearing

Deliverable:
- structurally important dynamic constructs survive ingestion and can be inspected downstream

### Phase 6: Enum, metadata, and annotation preservation
Increase semantic fidelity once structural parsing is stable.

Tasks:
- parse enum bodies instead of skipping them
- preserve enum variants, docs, and literal values
- widen metadata support beyond only `since`, `until`, and `canonical`
- record parsed annotations in AST form instead of only skipping them
- distinguish between:
  - annotations required for shape
  - annotations that are validation-only
  - annotations that can safely remain opaque for now

Design rule:
- preserve first, interpret second

Deliverable:
- semantically rich upstream constructs are retained for future tooling use

### Phase 7: Generic handling
Move from “skip generics” to “track generics enough to preserve structure”.

Tasks:
- parse generic parameter declarations on types if present
- store generic arguments in the type AST
- support basic substitution when reducing aliases or generic structs where shape depends on parameters
- if full substitution is too broad initially, at least retain generic instances in the reduced detail text

Why this matters:
- many upstream helper types become more faithful once parameterization is preserved

Deliverable:
- generic container/helper types no longer lose their shape immediately

### Phase 8: Error recovery and unknown-node strategy
Make the parser more resilient for future upstream changes.

Tasks:
- replace hard-fail-only behavior with limited local recovery where safe
- introduce explicit `UnknownDeclaration`, `UnknownType`, or `OpaqueNode` placeholders
- keep enough source context on unknown nodes to diagnose future upstream syntax drift
- ensure parser recovery does not silently corrupt later declarations in the same file

Deliverable:
- one unsupported construct does not necessarily abort the whole file parse

### Phase 9: Reducer coverage reporting
Track how much of the parsed corpus is actually useful to MCFC.

Tasks:
- add reducer metrics for:
  - resolved symbols
  - unresolved references
  - resolved dispatches
  - dynamic dispatch fallbacks
  - dynamic fields preserved vs dropped
  - ids using exact schema vs default-only fallback
- report coverage for the real MCFC targets:
  - entity roots
  - block roots
  - item roots
- make it easy to compare reports before/after parser changes

Deliverable:
- we can tell whether broader parsing improves actual schema quality, not just acceptance rate

## Testing Plan

### 1. Corpus regression tests
Add tests that parse the full pinned tarball and assert:
- all files are attempted
- parse success count meets expectations
- no hard failures remain for the pinned corpus

### 2. Syntax fixture tests
Add small focused fixtures for previously troublesome syntax:
- nested `[[...]]` dispatch refs
- dynamic keyed fields `[...]: ...` and `[... ]?: ...`
- attributes containing nested brackets/arrays
- multi-line dispatch key lists with trailing commas
- enum declarations with metadata
- generic aliases and generic struct references
- grouped/aliased imports if present upstream

### 3. Reduction tests
Verify that:
- dynamic fields are preserved in some form
- dynamic dispatch refs produce meaningful reduced output
- enums preserve values/docs where intended
- generic helper types reduce consistently

### 4. Output quality tests
Keep existing entity/block/item schema tests and add broader spot checks on exact ids from upstream.

## Recommended Delivery Order
1. Corpus-wide coverage harness
2. Remove tarball file filter and measure failures
3. Top-level grammar + import support
4. Type grammar refactor
5. Dynamic field/dispatch preservation
6. Enum/annotation/generic preservation
7. Reducer coverage reporting
8. Error recovery polish

## Concrete Code Areas
Most of the work will land in:
- `build_mcdoc.rs`
  - `read_mcdoc_files_from_tarball`
  - `Lexer::tokenize`
  - `FileParser::parse`
  - `FileParser::parse_type_expr`
  - `FileParser::parse_type_atom`
  - `FileParser::parse_dispatch_keys_definition`
  - `FileParser::parse_struct_after_name`
  - `DefinitionContext` and import resolution
  - `TypeExpr`, `StructEntry`, and related AST types
  - `SchemaReducer::reduce_type`
  - `SchemaReducer::reduce_struct`
  - `SchemaReducer::detail_for_type`
- `build.rs`
  - only if we expose coverage reporting or debug toggles for corpus scans

## Risks
- broadening the parser without a coverage harness may create silent degradation
- adding too much semantic interpretation too early may slow progress
- error recovery can hide bugs if it is introduced before syntax coverage is well tested
- upstream grammar drift may still happen later, so preserving opaque nodes is safer than assuming exhaustive support

## Practical Design Principles

### 1. Preserve, don’t discard
If we cannot fully interpret a construct, keep it in the AST/reduced model in an opaque form.

### 2. Separate acceptance from interpretation
The parser should first accept the corpus reliably; reducer fidelity can improve iteratively.

### 3. Let the corpus drive the work
Only add semantics for constructs proven to exist in the pinned upstream set or required by MCFC outputs.

### 4. Measure output quality, not just parse success
The important question is not only “did every file parse?” but also “did parsing improve entity/block/item schema quality?”

## End State
The desired end state is a build-time MCDoc ingestion pipeline that:
- reads the entire pinned `vanilla-mcdoc` tarball
- parses every `.mcdoc` file without crashing
- preserves unresolved advanced constructs explicitly
- resolves enough structure to keep improving MCFC’s generated schema quality over time
- gives us clear metrics when upstream syntax changes or coverage regresses
