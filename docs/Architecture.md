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
- Emits `Class`, `Method`, `Function`, and `Field` symbol kinds
- Emits `Calls`, `Inherits`, `Contains`, `Overrides`, `ReadsField`, and `WritesField` edge kinds
- Per-TU disk cache (`.spaghetti-cache/`, seahash-keyed) skips re-parsing unchanged translation units
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
- `ForceParams` — tuneable constants (repulsion, attraction, damping, ideal_length) with per-edge-kind overrides and location-affinity forces
- `step_budgeted(duration)` — runs iterations within a per-frame time budget
- Grid-based spatial bucketing for repulsion optimisation; rayon parallelism at 500+ nodes
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
- **`camera.rs`** — `Camera2D` (pan/zoom/coordinate transforms/auto-fit) and `hit_test()`, with unit tests
- **`file_tree.rs`** — directory-based visibility filtering; toggle entire directories on/off
- **`settings.rs`** — `AppSettings` saves/loads layout params, render settings, and view state to JSON
- **`fps.rs`** — rolling average FPS counter overlay
- **`log_capture.rs`** — tracing layer that bridges log events to the UI console panel
- **`progress.rs`** — background indexing progress overlay with cancellation
- **Modular panels** (`panels/`): canvas, left (file tree + edge filters + search), right (details + render controls), console (log viewer)
- Node dragging with pin/unpin during drag
- Zoom-based LOD: labeled rectangles → plain rectangles → circles at low zoom
- Layout auto-settles: simulation runs each frame until kinetic energy drops below threshold

## Design Principles

- **core-ir is upstream**: always extend `core-ir` first, then frontends, then query/layout, then viz. Never reverse this order.
- **No frontend leakage**: frontends must not expose language-specific types in their public API.
- **Workspace deps**: all shared dependencies live in root `[workspace.dependencies]`; member crates inherit with `workspace = true`.
- **Error handling**: no `unwrap()`/`expect()` in library crates — use `thiserror` and `Result`. `anyhow` is allowed only in the `viz` binary.

## Code Health Summary

*Last audited: 2026-04-11*

### Per-Crate Grades

| Crate | Quality | Tests | Notes |
|-------|---------|-------|-------|
| core-ir | A | 25 tests | Exemplary API design, strong type safety, comprehensive golden-file tests |
| frontend-clang | B | 1 integration test | Clean API boundary, but thin test coverage |
| layout | A | 17 tests | Solid physics simulation with deterministic seeding; no convention violations |
| query | A | 18 tests | Minimal, correct, panic-free; 100% public surface coverage |
| viz | B+ | 15 tests | Good camera tests, clean panel architecture; UI panels and settings untested |

### Known Issues

**Medium priority**

- `viz/src/app.rs:264,472` — channel send errors silently dropped (`let _ = tx.send(...)`). Should log on failure.
- `viz/src/panels/left.rs` — O(n) symbol filtering runs every frame; will lag at 10k+ symbols. Cache results, invalidate on search change.
- `viz/src/panels/right.rs` — iterates all edges per edge-kind per frame. Pre-group edges by kind.
- `viz/src/camera.rs` — hit-test is O(n) over all nodes. Fine for <1k nodes, bottleneck at 10k+.

**Low priority**

- `frontend-clang/src/lib.rs:203` — non-UTF-8 file paths silently fall back to empty string.
- `frontend-clang` cache invalidation uses only `compile_commands.json` mtime, not header mtimes.
- `viz/src/settings.rs` — settings path derived from `current_exe()`, fragile on non-standard platforms. No schema version field for migration.
- Hardcoded constants in viz (zoom factor 0.002, 8ms budget, node dimensions) are undocumented.

### Test Coverage Gaps

- **frontend-clang**: only 1 integration test; no tests for malformed JSON, empty commands, cache corruption, or system-include discovery.
- **viz**: file_tree, settings persistence, canvas rendering, left-panel search, right-panel connections, console, and FPS counter are untested.
- **layout**: no large-graph (1k+) stress tests.
