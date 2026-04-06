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
            // TODO: If this is a compile_commands.json, use frontend-clang to index.
            // For now, only the pre-serialized graph.json path is supported.
            bail!(
                "Could not parse {} as a serialized Graph. \
                 compile_commands.json indexing requires the `clang` feature. \
                 Use a pre-serialized graph.json instead.",
                path.display()
            );
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
