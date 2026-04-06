# spaghetti

A cross-platform desktop visualizer for code structure and dataflow. Points at a C++ project's `compile_commands.json` and produces an interactive graph of classes, methods, and their relationships. Built in Rust with eframe (egui + wgpu). Designed to grow into a multi-language tool (Rust, C#, JS/TS) via pluggable frontends over a shared IR, and to be queryable from outside the UI for future MCP server integration.

This is a personal project hosted on GitHub. It is not published to crates.io. Internal crate names exist for workspace organization only.

See [docs/Architecture.md](docs/Architecture.md) for detailed architecture documentation.

## Architecture

```
compile_commands.json
        │
        ▼
  frontend-clang ──► core-ir ◄── query
                        │
                        ▼
                     layout
                        │
                        ▼
                       viz  (eframe app, binary = `spaghetti`)
```

`core-ir` is the stable contract. Everything depends on it; it depends on nothing in the workspace. Frontends must not leak language-specific types (e.g. `clang::*`) into their public API.

## Workspace

```
crates/
  core-ir/         # language-agnostic graph types, serde
  frontend-clang/  # libclang → core-ir
  layout/          # force-directed positions
  query/           # subgraph / search / callers-of
  viz/             # the eframe binary (target name: spaghetti)
examples/
  tiny-cpp/        # end-to-end smoke test fixture
```

## Running

```bash
# Fallback path (no libclang required) — uses pre-serialized graph
cargo run -p viz -- examples/tiny-cpp/graph.json

# Real indexing path (requires libclang)
cargo run -p viz -- examples/tiny-cpp/compile_commands.json
```

Expected result: a window showing 3 classes (Shape, Circle, Square) with inheritance edges and call edges from `main`.

## libclang Setup

Required only for the `frontend-clang` crate. The viz app works without it via the JSON fallback.

- **Linux**: `sudo apt install libclang-dev`
- **macOS**: `brew install llvm && export LIBCLANG_PATH=$(brew --prefix llvm)/lib`
- **Windows**: install LLVM prebuilt from llvm.org, set `LIBCLANG_PATH` to `<llvm>/bin`

If libclang is not available in the current environment, the `clang` workspace feature is off by default and the frontend-clang crate is excluded from the build. Do not attempt to stub libclang calls with `todo!()` — use the JSON fallback path instead.

## Testing

```bash
cargo test --workspace                      # default, skips clang integration tests
cargo test --workspace --features clang     # includes libclang-dependent tests
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

Every crate has at least one test. `frontend-clang` integration tests are gated on the `clang` feature.

## Coding Conventions

- No `unwrap()` or `expect()` in library crates. Use `thiserror`, return `Result`.
- `anyhow` is permitted only in the `viz` binary.
- No `println!` — use `tracing`.
- Public items get doc comments.
- `#[non_exhaustive]` on public enums that will grow (`SymbolKind`, `EdgeKind`).
- All shared deps go in root `[workspace.dependencies]`; member crates use `workspace = true`.
- Run `cargo fmt` and `cargo clippy -- -D warnings` before committing.

## Extension Rules

When extending functionality:

1. Update `core-ir` first (add the type / variant).
1. Then the relevant frontend (emit the new data).
1. Then `query` and `layout` if they need to reason about it.
1. Then `viz` (render / interact with it).

Never reverse this order. Never let a downstream crate define the shape of upstream data.

Before adding a dependency, check `[workspace.dependencies]` — it's probably already there. If not, add it at the workspace root and inherit with `workspace = true` in the member crate.

## Non-Goals (do not implement without explicit approval)

This list exists because scope creep kills this kind of project. If you find yourself wanting to build any of these, stop and ask.

- Additional language frontends (Rust, C#, JS/TS)
- Hierarchical (Sugiyama) layout
- Custom wgpu render passes — stay in egui `Painter`
- Quadtree / spatial indexing
- Level-of-detail rendering
- Instanced edge/node rendering
- Persistence, project config, settings UI
- LSP integration
- MCP server
- Incremental re-indexing
- Uninstantiated template modeling
- SFINAE / concept overload visualization
- Edge kinds beyond `Calls` and `Inherits` in the clang frontend
- Symbol kinds beyond `Class` and `Method` in the clang frontend

The core-ir enums may contain more variants than the frontend currently emits — that is intentional, to avoid churning the IR later.

## Current Status

The v0 scaffold is complete and functional. To verify the project is healthy:

```bash
cargo check --workspace && cargo test --workspace && cargo run -p viz -- examples/tiny-cpp/graph.json
```

All three should succeed and a window should appear showing the example graph.

## Handoff Rule

**Stop and ask before adding functionality not listed above or in the Non-Goals section.** Helpful expansion is actively harmful. The goal is a tight, working foundation.
