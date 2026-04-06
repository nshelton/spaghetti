//! spaghetti — interactive code structure visualizer.
//!
//! Usage: `spaghetti <path-to-graph.json-or-compile_commands.json>`

mod app;

use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use tracing::info;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let path = match std::env::args().nth(1) {
        Some(p) => PathBuf::from(p),
        None => bail!("Usage: spaghetti <path-to-graph.json-or-compile_commands.json>"),
    };

    let json = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    // Try loading as a pre-serialized Graph first (the libclang-free fallback).
    let graph = match core_ir::Graph::from_json(&json) {
        Ok(g) => {
            info!(
                symbols = g.symbol_count(),
                edges = g.edge_count(),
                "loaded graph from JSON"
            );
            g
        }
        Err(_) => {
            // Not a pre-serialized graph — try as compile_commands.json via libclang.
            load_via_clang(&path)?
        }
    };

    // Compute layout
    let layout_algo = layout::ForceDirected::default();
    let positions = layout::Layout::compute(&layout_algo, &graph);

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "spaghetti",
        native_options,
        Box::new(move |_cc| Ok(Box::new(app::SpaghettiApp::new(graph, positions)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    Ok(())
}

/// Attempt to index a compile_commands.json using the clang frontend.
#[cfg(feature = "clang")]
fn load_via_clang(path: &std::path::Path) -> Result<core_ir::Graph> {
    let graph = frontend_clang::index_project(path)
        .map_err(|e| anyhow::anyhow!("clang indexing failed: {e}"))?;
    info!(
        symbols = graph.symbol_count(),
        edges = graph.edge_count(),
        "indexed project via libclang"
    );
    Ok(graph)
}

#[cfg(not(feature = "clang"))]
fn load_via_clang(path: &std::path::Path) -> Result<core_ir::Graph> {
    anyhow::bail!(
        "Could not parse {} as a serialized Graph. \
         compile_commands.json indexing requires the `clang` feature. \
         Rebuild with: cargo run -p viz --features clang",
        path.display()
    );
}
