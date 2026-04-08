//! spaghetti — interactive code structure visualizer.
//!
//! Usage:
//! - `spaghetti <path>` — open a `compile_commands.json` or `graph.json`
//! - `spaghetti` — start with an empty canvas (use File > Open)

mod app;
mod camera;
mod fps;
mod log_capture;
mod panels;
mod progress;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use log_capture::{LogBuffer, LogCaptureLayer};

fn main() -> Result<()> {
    // Set up shared log buffer and tracing subscriber with capture layer.
    let log_buffer = Arc::new(Mutex::new(LogBuffer::new()));
    let capture_layer = LogCaptureLayer::new(Arc::clone(&log_buffer));

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(capture_layer)
        .init();

    // Optional CLI argument: path to compile_commands.json or graph.json.
    let path = std::env::args().nth(1).map(PathBuf::from);

    let app = if let Some(ref path) = path {
        if !path.exists() {
            anyhow::bail!("File not found: {}", path.display());
        }

        let graph = frontend_clang::index_project(path)
            .map_err(|e| anyhow::anyhow!("clang indexing failed: {e}"))?;

        info!(
            symbols = graph.symbol_count(),
            edges = graph.edge_count(),
            "indexed project"
        );

        // Create incremental layout state (the viz drives it frame-by-frame)
        let layout_state = layout::LayoutState::new(&graph, 42, layout::ForceParams::default());

        app::SpaghettiApp::new(graph, layout_state, log_buffer)
    } else {
        info!("no file argument — starting with empty canvas");
        app::SpaghettiApp::empty(log_buffer)
    };

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "spaghetti",
        native_options,
        Box::new(move |_cc| Ok(Box::new(app))),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    Ok(())
}
