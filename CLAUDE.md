# spaghetti

A cross-platform desktop visualizer for code structure and dataflow. Points at a C++ project's `compile_commands.json` and produces an interactive graph of classes, methods, and their relationships. Built in Rust with eframe (egui + wgpu). Designed to grow into a multi-language tool (Rust, C#, JS/TS) via pluggable frontends over a shared IR, and to be queryable from outside the UI for future MCP server integration.

This is a personal project hosted on GitHub. It is not published to crates.io. Internal crate names exist for workspace organization only.

See [docs/Architecture.md](docs/Architecture.md) for detailed architecture documentation.

## Environment Bootstrap (IMPORTANT — run first)

**Before any `cargo build`, `cargo test`, or `cargo check`**, ensure libclang is installed. The `frontend-clang` crate links against it at compile time — the entire workspace fails to build without it.

```bash
bash scripts/setup-libclang.sh
```

On macOS you also need: `export LIBCLANG_PATH=$(brew --prefix llvm)/lib`

This is idempotent and fast if libclang is already present. **Cloud / CI agents must run this before any cargo command.**

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
# Requires libclang
LIBCLANG_PATH=$(brew --prefix llvm)/lib cargo run -p viz -- examples/tiny-cpp/compile_commands.json
```

Expected result: a window showing classes (Shape, Circle, Square), methods, and `main` with inheritance, call, contains, and overrides edges. The force-directed layout animates into place over ~1-2 seconds.

## libclang Setup

Required to build. The viz binary always uses `frontend-clang` for indexing.

- **Linux**: `sudo apt install libclang-dev`
- **macOS**: `brew install llvm && export LIBCLANG_PATH=$(brew --prefix llvm)/lib`
- **Windows**: install LLVM prebuilt from llvm.org, set `LIBCLANG_PATH` to `<llvm>/bin`

## Testing

```bash
cargo test --workspace                      # default, skips clang integration tests
cargo test -p frontend-clang --features clang-tests  # includes libclang-dependent tests
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

Every crate has at least one test. `frontend-clang` integration tests are gated on the `clang-tests` feature. The `viz` crate's `camera` module has unit tests for coordinate transforms and hit-testing.

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
- Symbol kinds beyond `Class`, `Method`, and `Function` in the clang frontend

The core-ir enums may contain more variants than the frontend currently emits — that is intentional, to avoid churning the IR later.

## Current Status

The clang frontend is functional, emitting `Class`, `Method`, and `Function` symbols with `Calls`, `Inherits`, `Contains`, and `Overrides` edges. The layout uses an incremental force-directed simulation (`LayoutState`) driven frame-by-frame with node dragging/pinning support. The camera module (`viz/src/camera.rs`) handles pan, zoom, and hit-testing.

To verify the project is healthy:

```bash
LIBCLANG_PATH=$(brew --prefix llvm)/lib cargo check --workspace && cargo test --workspace && cargo run -p viz -- examples/tiny-cpp/compile_commands.json
```

All three should succeed and a window should appear showing the indexed graph with animated layout.

## Handoff Rule

**Stop and ask before adding functionality not listed above or in the Non-Goals section.** Helpful expansion is actively harmful. The goal is a tight, working foundation.
