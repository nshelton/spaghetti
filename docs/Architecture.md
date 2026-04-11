# Architecture

## Overview

spaghetti is a cross-platform desktop visualizer for code structure and dataflow. It parses C++ projects via `compile_commands.json` and renders an interactive, zoomable graph of symbols and their relationships.

Built in Rust with [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) (egui + wgpu + winit).

## Data Flow

```
compile_commands.json
        |
        v
  frontend-clang ──> core-ir <── query
                        |
                        v
                     layout (LayoutState — incremental)
                        |
                        v
                       viz  (eframe app, binary = spaghetti)
```

## Crates

### core-ir

The stable contract of the system. All other crates depend on it; it depends on nothing in the workspace.

- Language-agnostic graph types: `Graph`, `Symbol`, `Edge`, `SymbolId`
- Symbol kinds: `Class`, `Struct`, `Function`, `Method`, `Field`, `Namespace`, `TemplateInstantiation`, `TranslationUnit`
- Edge kinds: `Calls`, `Inherits`, `Contains`, `Overrides`, `ReadsField`, `WritesField`, `Includes`, `Instantiates`, `HasType`
- Stable `SymbolId` hashing with input normalization
- Serde support for JSON serialization/deserialization
- Interned file paths via `FileTable`

### frontend-clang

Indexes C++ projects using libclang and emits a `core-ir::Graph`.

- Reads `compile_commands.json` and drives libclang to extract AST information
- Emits `Class`, `Method`, and `Function` symbol kinds
- Emits `Calls`, `Inherits`, `Contains`, and `Overrides` edge kinds
- Automatically discovers system C++ include paths (via `LIBCLANG_PATH` or `clang++`)
- Filters to project-local symbols only (skips standard library internals)
- Deduplicates edges across translation units
- Integration tests gated behind the `clang-tests` feature

### layout

Force-directed graph layout engine with two modes:

- **`ForceDirected`** — batch layout: `compute()` runs all iterations and returns final `Positions`
- **`LayoutState`** — incremental simulation driven frame-by-frame:
  - `new(graph, seed, params)` — hash-scatter initial positions, build edge index
  - `step(n)` — run `n` iterations of repulsion + attraction + velocity damping
  - `positions()` — snapshot current state
  - `energy()` — convergence metric (total kinetic energy)
  - `pin(id, pos)` / `unpin(id)` / `set_position(id, pos)` — support for interactive node dragging
- `ForceParams` — tuneable constants (repulsion, attraction, damping, ideal_length)
- Connected component packing for disconnected graphs

### query

Graph query and subgraph extraction.

- Subgraph extraction by symbol set
- Search by name (substring match)
- Callers-of / callees-of traversal
- Neighbor traversal filtered by edge kind

### viz

The eframe desktop application (binary name: `spaghetti`).

- **`main.rs`** — CLI entry, indexes via `frontend-clang`, creates `LayoutState`, launches eframe
- **`app.rs`** — `SpaghettiApp`: drives layout each frame, renders graph, handles interaction
- **`camera.rs`** — `Camera2D` (pan/zoom/coordinate transforms) and `hit_test()`, with unit tests
- Node dragging with pin/unpin during drag
- Left panel: search bar, edge kind filters, scrollable symbol list
- Right panel: selected symbol details (name, qualified name, kind, location, attributes, neighbors)
- Layout auto-settles: simulation runs each frame until kinetic energy drops below threshold

## Design Principles

- **core-ir is upstream**: always extend `core-ir` first, then frontends, then query/layout, then viz. Never reverse this order.
- **No frontend leakage**: frontends must not expose language-specific types in their public API.
- **Workspace deps**: all shared dependencies live in root `[workspace.dependencies]`; member crates inherit with `workspace = true`.
- **Error handling**: no `unwrap()`/`expect()` in library crates — use `thiserror` and `Result`. `anyhow` is allowed only in the `viz` binary.

## Code Health Summary

*Last audited: 2025-04-10*

### Per-Crate Grades

| Crate | Quality | Tests | Notes |
|-------|---------|-------|-------|
| core-ir | A | 25 tests | Exemplary API design, strong type safety, comprehensive golden-file tests |
| frontend-clang | B | 1 integration test | Clean API boundary, but thin test coverage and a panic risk in `extract_args()` |
| layout | A- | 17 tests | Solid physics simulation with deterministic seeding; 3 `expect()` calls violate convention |
| query | A | 18 tests | Minimal, correct, panic-free; 100% public surface coverage |
| viz | B+ | ~15 tests | Good camera tests, clean panel architecture; UI panels and settings untested |

### Known Issues

**High priority**

- `frontend-clang/src/lib.rs:148,151` — `extract_args()` indexes `[1..]` without bounds check. An empty `arguments` array or single-word `command` string will panic. Fix: use `.get(1..)`.
- `viz/src/app.rs:179,197,204` — channel send errors silently dropped (`let _ = tx.send(...)`). Should log on failure.

**Medium priority**

- `layout/src/lib.rs:551,592,599` — three `expect()` calls on infallible invariants. Safe in practice but violates the no-unwrap convention. Replace with `unwrap_or_else(|| unreachable!(...))` or restructure.
- `frontend-clang/src/lib.rs:50-51` — doc comment claims "parallelizes per TU using rayon" but code uses sequential `.iter()`.
- `viz/src/panels/left.rs:41-63` — O(n) symbol filtering runs every frame; will lag at 10k+ symbols. Cache results, invalidate on search change.
- `viz/src/panels/right.rs:62-77` — iterates all edges per edge-kind per frame. Pre-group edges by kind.
- `viz/src/camera.rs:114-143` — hit-test is O(n) over all nodes. Fine for <1k nodes, bottleneck at 10k+.

**Low priority**

- `frontend-clang/src/lib.rs:199` — non-UTF-8 file paths silently fall back to empty string.
- `frontend-clang` cache invalidation uses only `compile_commands.json` mtime, not header mtimes.
- `viz/src/settings.rs:17-21` — settings path derived from `current_exe()`, fragile on non-standard platforms.
- Hardcoded constants in viz (zoom factor 0.002, 8ms budget, node dimensions) are undocumented.

### Test Coverage Gaps

- **frontend-clang**: only 1 integration test; no tests for malformed JSON, empty commands, cache corruption, or system-include discovery.
- **viz panels**: canvas rendering, left-panel search, right-panel connections, console, FPS counter, and settings persistence are all untested.
- **layout**: no large-graph (1k+) stress tests.
