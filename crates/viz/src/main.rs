//! spaghetti — interactive code structure visualizer.
//!
//! Usage: `spaghetti <path-to-compile_commands.json>`

mod app;
mod camera;

use anyhow::{bail, Result};
use std::path::PathBuf;
use std::time::Instant;
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

    let total_start = Instant::now();

    let indexing_start = Instant::now();
    let graph = frontend_clang::index_project(&path)
        .map_err(|e| anyhow::anyhow!("clang indexing failed: {e}"))?;
    let indexing_elapsed = indexing_start.elapsed();

    info!(
        symbols = graph.symbol_count(),
        edges = graph.edge_count(),
        indexing_ms = format!("{:.1}", indexing_elapsed.as_secs_f64() * 1000.0),
        "indexed project via libclang"
    );

    // Create incremental layout state (the viz drives it frame-by-frame)
    let layout_init_start = Instant::now();
    let layout_state = layout::LayoutState::new(&graph, 42, layout::ForceParams::default());
    let layout_init_elapsed = layout_init_start.elapsed();

    info!(
        layout_init_ms = format!("{:.1}", layout_init_elapsed.as_secs_f64() * 1000.0),
        total_load_ms = format!("{:.1}", total_start.elapsed().as_secs_f64() * 1000.0),
        "load complete, launching UI"
    );

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "spaghetti",
        native_options,
        Box::new(move |_cc| Ok(Box::new(app::SpaghettiApp::new(graph, layout_state)))),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    Ok(())
}
