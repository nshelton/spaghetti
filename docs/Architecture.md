# Architecture

## Overview

spaghetti is a cross-platform desktop visualizer for code structure and dataflow. It parses C++ projects via `compile_commands.json` (or pre-serialized `graph.json`) and renders an interactive, zoomable graph of symbols and their relationships.

Built in Rust with [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) (egui + wgpu + winit).

## Data Flow

```
compile_commands.json / graph.json
        |
        v
  frontend-clang ──> core-ir <── query
                        |
                        v
                     layout
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

- Accepts either a `compile_commands.json` (live indexing) or a `graph.json` (pre-serialized fallback)
- Currently emits `Class` and `Method` symbol kinds, `Calls` and `Inherits` edge kinds
- Gated behind the `clang` workspace feature; excluded from build when libclang is unavailable

### layout

Force-directed graph layout engine.

- `ForceDirected` algorithm with configurable parameters
- Produces `Positions` — a map from `SymbolId` to 2D coordinates (via `glam::Vec2`)

### query

Graph query and subgraph extraction.

- Subgraph extraction by symbol set
- Search by name (substring match)
- Callers-of / callees-of traversal
- Neighbor traversal filtered by edge kind

### viz

The eframe desktop application (binary name: `spaghetti`).

- 2D camera with pan and zoom
- Node rendering colored by symbol kind
- Edge rendering colored by edge kind
- Left panel: search bar, edge kind filters, scrollable symbol list
- Right panel: selected symbol details (name, qualified name, kind, location, attributes, neighbors)
- Click-to-select nodes with hit testing

## Design Principles

- **core-ir is upstream**: always extend `core-ir` first, then frontends, then query/layout, then viz. Never reverse this order.
- **No frontend leakage**: frontends must not expose language-specific types in their public API.
- **Workspace deps**: all shared dependencies live in root `[workspace.dependencies]`; member crates inherit with `workspace = true`.
- **Error handling**: no `unwrap()`/`expect()` in library crates — use `thiserror` and `Result`. `anyhow` is allowed only in the `viz` binary.
