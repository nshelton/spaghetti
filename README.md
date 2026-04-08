# spaghetti

A cross-platform desktop visualizer for code structure and dataflow. Point it at a C++ project's `compile_commands.json` and get an interactive, zoomable graph of classes, methods, and their relationships.

Built in Rust with [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) (egui + wgpu + winit).

![CI](https://github.com/nshelton/spaghetti/actions/workflows/ci.yml/badge.svg)

## Quick Start

```bash
# Clone and build (requires libclang — see setup below)
git clone https://github.com/nshelton/spaghetti.git
cd spaghetti

# macOS
LIBCLANG_PATH=$(brew --prefix llvm)/lib cargo run -p viz -- examples/tiny-cpp/compile_commands.json
```

Expected result: a window showing classes (Shape, Circle, Square), methods, and `main` with inheritance, call, contains, and overrides edges. Nodes animate into place via force-directed layout.

## Features

- **Live indexing** via libclang from `compile_commands.json`
- **Animated force-directed layout** — nodes settle in real time
- **Node dragging** — grab and reposition nodes, simulation continues around them
- Interactive pan and zoom
- Symbol search and filtering
- Edge kind filtering (Calls, Inherits, Contains, Overrides)
- Click-to-select with detail panel showing symbol info and neighbors

## libclang Setup

Required to build. The app indexes C++ source directly via libclang.

- **Linux**: `sudo apt install libclang-dev`
- **macOS**: `brew install llvm && export LIBCLANG_PATH=$(brew --prefix llvm)/lib`
- **Windows**: Install LLVM prebuilt from [llvm.org](https://llvm.org), set `LIBCLANG_PATH` to `<llvm>/bin`

## Building & Testing

```bash
cargo check --workspace
cargo test --workspace                                # skips clang integration tests
cargo test -p frontend-clang --features clang-tests   # includes clang integration tests
cargo clippy --workspace -- -D warnings
cargo fmt --check
```

## Project Structure

| Crate | Purpose |
|-------|---------|
| `core-ir` | Language-agnostic graph types and serde |
| `frontend-clang` | libclang indexer -> core-ir Graph |
| `layout` | Force-directed graph layout |
| `query` | Subgraph extraction, search, callers-of |
| `viz` | eframe desktop app (binary: `spaghetti`) |

See [docs/Architecture.md](docs/Architecture.md) for detailed architecture documentation.

## License

Personal project. All rights reserved.
