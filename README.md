# spaghetti

A cross-platform desktop visualizer for code structure and dataflow. Point it at a C++ project's `compile_commands.json` and get an interactive, zoomable graph of classes, methods, and their relationships.

Built in Rust with [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) (egui + wgpu + winit).

## Quick Start

```bash
# Clone and build
git clone https://github.com/nshelton/spaghetti.git
cd spaghetti

# Run with the pre-serialized example graph (no libclang required)
cargo run -p viz -- examples/tiny-cpp/graph.json

# Or, if libclang is installed, index from source
cargo run -p viz -- examples/tiny-cpp/compile_commands.json
```

Expected result: a window showing 3 classes (Shape, Circle, Square) with inheritance and call edges.

## libclang Setup

Required only for the `frontend-clang` crate. The app works without it via the JSON fallback path.

- **Linux**: `sudo apt install libclang-dev`
- **macOS**: `brew install llvm && export LIBCLANG_PATH=$(brew --prefix llvm)/lib`
- **Windows**: Install LLVM prebuilt from [llvm.org](https://llvm.org), set `LIBCLANG_PATH` to `<llvm>/bin`

## Building & Testing

```bash
cargo check --workspace
cargo test --workspace                      # skips clang integration tests
cargo test --workspace --features clang     # includes clang tests (requires libclang)
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

## Project Structure

| Crate | Purpose |
|-------|---------|
| `core-ir` | Language-agnostic graph types and serde |
| `frontend-clang` | libclang indexer → core-ir Graph |
| `layout` | Force-directed graph layout |
| `query` | Subgraph extraction, search, callers-of |
| `viz` | eframe desktop app (binary: `spaghetti`) |

See `PLAN.md` for the full v0 scaffold specification.

## License

Personal project. All rights reserved.
