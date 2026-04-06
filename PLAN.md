# spaghetti — Scaffold Plan

## Vision

A cross-platform desktop visualizer for code structure and dataflow. Point it at a C++ project's `compile_commands.json` and get an interactive, zoomable graph of classes, methods, and their relationships (calls, inheritance, field access, includes). Designed to scale to large codebases via level-of-detail rendering and to be queryable from outside the UI (future MCP server).

C++ is the first target. Rust, C#, and JS/TS come later via pluggable frontends that all populate a shared language-agnostic IR.

This is a personal project for GitHub. It is not published to crates.io. Internal crate names are purely for workspace organization.

## Architecture

```
           ┌──────────────────┐
           │ compile_commands │
           └────────┬─────────┘
                    │
           ┌────────▼─────────┐
           │  frontend-clang  │   libclang, rayon per TU
           └────────┬─────────┘
                    │  core_ir::Graph
           ┌────────▼─────────┐      ┌──────────┐
           │     core-ir      │◄─────│  query   │
           └────────┬─────────┘      └──────────┘
                    │
           ┌────────▼─────────┐
           │      layout      │   force-directed (Barnes-Hut)
           └────────┬─────────┘
                    │  Positions
           ┌────────▼─────────┐
           │       viz        │   eframe (egui + wgpu + winit)
           └──────────────────┘
```

`core-ir` is the stable contract. Frontends, layout, query, and viz all depend on it. Nothing above depends on anything below it.

## Why a workspace with multiple crates

This is a personal project and none of these crates will ever be published. The workspace split exists for four practical reasons:

1. **Incremental compile times.** Editing `viz` doesn't recompile `core-ir`.
1. **Enforced layering.** `core-ir` cannot accidentally import egui or clang types because they're not in its `Cargo.toml`.
1. **Clean feature flags.** `frontend-clang` being a separate crate lets us exclude libclang from the build entirely when it's unavailable.
1. **Agent scope guardrails.** When an agent works on layout, the crate boundary makes the task scope unambiguous and discourages sprawl.

## Workspace Layout

```
spaghetti/
  Cargo.toml                 # workspace + workspace.dependencies
  rust-toolchain.toml        # pin stable
  .gitignore
  README.md
  CLAUDE.md
  PLAN.md                    # this file
  crates/
    core-ir/                 # lib
    frontend-clang/          # lib
    layout/                  # lib
    query/                   # lib
    viz/                     # bin (produces the `spaghetti` binary)
  examples/
    tiny-cpp/
      include/shape.h
      include/circle.h
      include/square.h
      src/shape.cpp
      src/circle.cpp
      src/square.cpp
      src/main.cpp
      compile_commands.json  # hand-written, 4 entries
      graph.json             # pre-serialized fallback (see "libclang risk")
```

The `viz` crate's binary target is named `spaghetti` so `cargo run` produces a binary called `spaghetti`, not `viz`.

## Crate Specs

### `core-ir`

Pure data. No I/O, no rendering, no clang types.

Types:

- `SymbolId(u64)` — newtype, derived from a deterministic hash of `(qualified_name, kind, signature)`. Stable across runs for the same input. Long-term replacement: Clang USRs. Leave a `TODO`.
- `FileId(u32)` + `FileTable` — string interning for file paths.
- `Location { file: FileId, line: u32, col: u32 }`
- `SymbolKind` enum (`#[non_exhaustive]`): `Class`, `Struct`, `Function`, `Method`, `Field`, `Namespace`, `TemplateInstantiation`, `TranslationUnit`
- `Symbol { id, kind, name, qualified_name, location, module: Option<String>, attrs: SmallVec<[Attr; 2]> }`
- `EdgeKind` enum (`#[non_exhaustive]`): `Calls`, `Inherits`, `Contains`, `ReadsField`, `WritesField`, `Includes`, `Instantiates`, `HasType`, `Overrides`
- `Edge { from: SymbolId, to: SymbolId, kind: EdgeKind, location: Option<Location> }`
- `Graph { files: FileTable, symbols: IndexMap<SymbolId, Symbol>, edges: Vec<Edge> }`

Methods on `Graph`:

- `add_symbol(Symbol) -> SymbolId`
- `add_edge(Edge)`
- `neighbors(SymbolId, &[EdgeKind]) -> impl Iterator<Item = SymbolId>`
- `merge(other: Graph)` — for combining per-TU results
- Serde roundtrip to JSON.

Tests: construction, neighbors with filter, merge determinism, serde roundtrip.

Scaffold minimum: implement everything above. This crate should be genuinely complete.

### `frontend-clang`

Reads `compile_commands.json`, drives libclang, emits `core_ir::Graph`.

Entry point:

```rust
pub fn index_project(compile_commands: &Path) -> Result<Graph>;
```

Implementation notes for the agent:

- Use the `clang` crate (wraps `clang-sys`). Do not write raw FFI.
- Parallelize per translation unit with `rayon`. Each TU returns a partial `Graph`; merge at the end.
- For the scaffold, only emit: `Class`, `Method`, and edges `Calls`, `Inherits`. Everything else is a `TODO`.
- Template handling: visit instantiated specializations, treat each as a distinct `Symbol` with kind `TemplateInstantiation`, link to the primary via `Instantiates`. Do not attempt to model uninstantiated templates.
- All libclang interaction stays inside this crate. No `clang::*` types in public API.

Dependencies: libclang must be present on the system. README must document:

- Linux: `apt install libclang-dev`
- macOS: `brew install llvm` and `export LIBCLANG_PATH=$(brew --prefix llvm)/lib`
- Windows: install LLVM prebuilt, set `LIBCLANG_PATH`

Tests: one integration test parsing `examples/tiny-cpp/` and asserting symbol/edge counts. Gate behind `#[cfg(feature = "clang-tests")]` so default `cargo test` works without libclang.

### `layout`

Pure function from `Graph` → positions. No egui/wgpu deps.

```rust
pub trait Layout {
    fn compute(&self, graph: &Graph) -> Positions;
}

pub struct Positions(pub HashMap<SymbolId, Vec2>);

pub struct ForceDirected { pub seed: u64, pub iterations: u32 }
impl Layout for ForceDirected { /* ... */ }
```

Implementation: Barnes-Hut force-directed, either via the `fdg` crate (check freshness first) or ~200 lines inline. Deterministic given a seed.

Scaffold: `ForceDirected` only. Hierarchical layout is a non-goal for v0.

Tests: determinism (same graph + seed → same positions), non-overlapping for trivial 3-node case.

### `query`

Graph queries callable from viz and (future) an MCP server.

```rust
pub fn subgraph_around(g: &Graph, root: SymbolId, depth: u32, kinds: &[EdgeKind]) -> Graph;
pub fn find_by_name(g: &Graph, pattern: &str) -> Vec<SymbolId>;
pub fn callers_of(g: &Graph, id: SymbolId) -> Vec<SymbolId>;
```

Scaffold: implement all three, keep them simple (linear scans are fine).

Tests: unit tests against hand-built small graphs.

### `viz`

The binary. `eframe`-based, so winit/wgpu are handled by the framework. Binary target name is `spaghetti`.

App state:

```rust
struct SpaghettiApp {
    graph: Graph,
    positions: Positions,
    camera: Camera2D,          // pan + zoom
    selection: Option<SymbolId>,
    edge_filter: EdgeKindFilter,
    search: String,
}
```

UI:

- Left panel: search box, edge-kind filter toggles, symbol list.
- Central panel: graph canvas. Draws with `egui::Painter` — rectangles for nodes, lines for edges, text labels. No custom wgpu render pass in v0.
- Right panel: details for selected symbol (qualified name, location, neighbors).

Interaction:

- Drag background to pan.
- Scroll to zoom.
- Click node to select.
- Linear scan for hit-testing is fine. Add `// TODO: quadtree`.

CLI:

```
spaghetti <path-to-compile_commands.json-or-graph.json>
```

If the argument is a `.json` file containing a pre-serialized `Graph` (detected by trying `serde_json` first), load it directly. This is the libclang fallback path.

## Example Fixture

`examples/tiny-cpp/` contains:

- `Shape` (abstract, virtual `area()`)
- `Circle : Shape`
- `Square : Shape`
- `main.cpp` that constructs both and calls `area()`
- Hand-written `compile_commands.json` with 4 entries
- `graph.json` — pre-serialized `core_ir::Graph` representing the expected output, hand-built or generated once and committed. Used as fallback when libclang is unavailable.

End-to-end smoke test:

```
cargo run -p viz -- examples/tiny-cpp/compile_commands.json
```

Must open a window showing 3 class nodes with 2 inheritance edges and call edges from `main`.

## Dependency Pinning

Declared in root `Cargo.toml` under `[workspace.dependencies]`:

- `eframe` — latest stable at scaffold time, pin exact minor
- `egui` — same minor as eframe
- `egui-wgpu` — same minor as eframe
- `wgpu` — inherit from eframe, do not pin independently
- `clang` crate — latest stable, document libclang version compatibility
- `serde = { version = "1", features = ["derive"] }`
- `serde_json = "1"`
- `rayon = "1"`
- `indexmap = { version = "2", features = ["serde"] }`
- `smallvec = "1"`
- `thiserror = "1"` — for library crates
- `anyhow = "1"` — for the viz binary only
- `tracing = "0.1"`
- `tracing-subscriber = "0.3"`
- `glam` — latest, for `Vec2`

Every crate uses `workspace = true` inheritance. Do not re-declare versions in member crates.

## Coding Conventions

- No `unwrap()` or `expect()` in library crates. Use `thiserror` and return `Result`.
- `anyhow` is permitted only in `viz`.
- No `println!`; use `tracing`.
- Public items get doc comments.
- `#[non_exhaustive]` on public enums that will grow (`SymbolKind`, `EdgeKind`).
- `cargo fmt` and `cargo clippy -- -D warnings` must pass before committing.

## Definition of Done (v0 scaffold)

All of the following must be true:

1. `cargo check --workspace` passes.
1. `cargo test --workspace` passes (clang-tests feature off).
1. `cargo clippy --workspace -- -D warnings` passes.
1. `cargo run -p viz -- examples/tiny-cpp/graph.json` opens a window showing the 3-class graph. JSON fallback path — guaranteed to work without libclang.
1. If libclang is available: `cargo run -p viz -- examples/tiny-cpp/compile_commands.json` produces the same result via real indexing.
1. `README.md` documents setup, libclang install, and the run command.
1. `CLAUDE.md` is present and complete.
1. Each crate has at least one passing test.

## Non-Goals for v0

Do **not** implement any of the following. If tempted, leave a `// TODO:` with a one-line note and stop.

- Rust, C#, or JS/TS frontends
- Hierarchical (Sugiyama) layout
- Custom `wgpu` render passes (stay in egui Painter)
- Quadtree / spatial indexing
- Level-of-detail rendering
- Instanced edge/node rendering
- Persistence, project config files, settings UI
- LSP integration
- MCP server
- Incremental re-indexing on file change
- Uninstantiated template modeling
- SFINAE / concept overload resolution visualization
- Any `EdgeKind` beyond `Calls` and `Inherits` in the clang frontend (the enum variants exist in core-ir, but the frontend only emits these two)
- Any `SymbolKind` beyond `Class` and `Method` in the clang frontend

## Build Order

Build and verify crates in this order. Run `cargo check -p <crate>` after each and fix before moving on. Commit after each crate reaches a compilable + tested state.

1. `core-ir` — types, graph ops, serde, tests
1. `layout` — ForceDirected, determinism test
1. `query` — three functions, unit tests
1. `examples/tiny-cpp/` — C++ sources, compile_commands.json, hand-built graph.json
1. `viz` — eframe app loading graph.json, drawing nodes/edges, pan/zoom/select
1. `frontend-clang` — libclang indexing (may be skipped if libclang unavailable; see risk below)

## libclang Risk

If the build environment does not have libclang available, **do not** flail trying to install it or stub out the crate with `todo!()`. Instead:

- Put `frontend-clang` behind a default-off workspace feature `clang`.
- Ensure the `graph.json` fallback path in `viz` works end-to-end without the `clang` feature.
- Document the limitation in `README.md` and leave a clear note in `CLAUDE.md` that clang indexing should be completed in a follow-up session on a machine with libclang installed.

The scaffold must reach "window shows three classes" via the JSON fallback regardless of libclang availability.

## Handoff Rule

Stop and ask before adding any functionality not in this plan. Helpful expansion is harmful here — the goal is a tight, working foundation, not a feature-complete tool.
