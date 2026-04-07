//! spaghetti — interactive code structure visualizer.
//!
//! Usage: `spaghetti <path-to-compile_commands.json>`

mod app;
mod camera;

use anyhow::{bail, Result};
use std::path::PathBuf;
use tracing::info;

fn main() -> Result<()> {
    tracing_subscriber::fmt::init();

    let path = match std::env::args().nth(1) {
        Some(p) => PathBuf::from(p),
        None => bail!("Usage: spaghetti <path-to-compile_commands.json>"),
    };

    if !path.exists() {
        bail!("File not found: {}", path.display());
    }

    let graph = frontend_clang::index_project(&path)
        .map_err(|e| anyhow::anyhow!("clang indexing failed: {e}"))?;

    info!(
        symbols = graph.symbol_count(),
        edges = graph.edge_count(),
        "indexed project via libclang"
    );

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
