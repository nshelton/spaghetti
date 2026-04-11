//! spaghetti — interactive code structure visualizer.
//!
//! Starts with an empty canvas. If a previous project was opened, it is
//! automatically reloaded on launch. Use File > Open to pick a new project.

mod app;
mod camera;
mod file_tree;
mod fps;
mod log_capture;
mod panels;
mod progress;
mod settings;
mod state;
mod widgets;

use std::sync::{Arc, Mutex};

use anyhow::Result;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

use log_capture::{LogBuffer, LogCaptureLayer};

fn main() -> Result<()> {
    // Set up shared log buffer and tracing subscriber with capture layer.
    let log_buffer = Arc::new(Mutex::new(LogBuffer::new()));
    let capture_layer = LogCaptureLayer::new(Arc::clone(&log_buffer));

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .with(capture_layer)
        .init();

    // Load persisted settings (or defaults if no file yet).
    let saved_settings = settings::AppSettings::load();
    let recent_projects = saved_settings.recent_projects.clone();

    // Start with an empty canvas; auto-load last project if available.
    let mut app = app::SpaghettiApp::empty(log_buffer);
    app.indexing.recent_projects = recent_projects;
    app.apply_saved_settings(&saved_settings);

    // If a recent project exists, queue it for background loading.
    if let Some(last_project) = app.indexing.recent_projects.first().cloned() {
        if last_project.exists() {
            app.start_indexing(last_project);
        }
    }

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1280.0, 800.0]),
        ..Default::default()
    };

    eframe::run_native(
        "spaghetti",
        native_options,
        Box::new(move |cc| {
            // Load IBM Plex Mono as the app-wide font.
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "IBMPlexMono".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../../../assets/fonts/IBMPlexMono-Regular.ttf"
                ))),
            );
            // Use as primary for both proportional and monospace families.
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .insert(0, "IBMPlexMono".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .insert(0, "IBMPlexMono".to_owned());
            cc.egui_ctx.set_fonts(fonts);

            Ok(Box::new(app))
        }),
    )
    .map_err(|e| anyhow::anyhow!("eframe error: {e}"))?;

    Ok(())
}
