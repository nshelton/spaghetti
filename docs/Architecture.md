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
