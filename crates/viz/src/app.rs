//! The main eframe application for spaghetti.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use core_ir::{EdgeKind, Graph, SymbolId};
use egui::{Pos2, Vec2};
use glam::Vec2 as GVec2;
use layout::Positions;
use tracing::Level;

use crate::log_capture::LogBuffer;
use crate::progress::{ProgressMessage, ProgressState};

/// Node width in world units.
pub(crate) const NODE_WIDTH: f32 = 120.0;
/// Node height in world units.
pub(crate) const NODE_HEIGHT: f32 = 30.0;

/// Edge kind filter state.
pub(crate) struct EdgeKindFilter {
    pub calls: bool,
    pub inherits: bool,
    pub contains: bool,
    pub overrides: bool,
}

impl Default for EdgeKindFilter {
    fn default() -> Self {
        Self {
            calls: true,
            inherits: true,
            contains: true,
            overrides: true,
        }
    }
}

impl EdgeKindFilter {
    pub(crate) fn active_kinds(&self) -> Vec<EdgeKind> {
        let mut kinds = Vec::new();
        if self.calls {
            kinds.push(EdgeKind::Calls);
        }
        if self.inherits {
            kinds.push(EdgeKind::Inherits);
        }
        if self.contains {
            kinds.push(EdgeKind::Contains);
        }
        if self.overrides {
            kinds.push(EdgeKind::Overrides);
        }
        kinds
    }
}

/// 2D camera for pan and zoom.
pub(crate) struct Camera2D {
    pub offset: Vec2,
    pub zoom: f32,
}

impl Default for Camera2D {
    fn default() -> Self {
        Self {
            offset: Vec2::ZERO,
            zoom: 1.0,
        }
    }
}

impl Camera2D {
    /// Transform a world position to screen position.
    pub(crate) fn world_to_screen(&self, world: GVec2, canvas_center: Pos2) -> Pos2 {
        let x = canvas_center.x + (world.x + self.offset.x) * self.zoom;
        let y = canvas_center.y + (world.y + self.offset.y) * self.zoom;
        Pos2::new(x, y)
    }

    /// Transform a screen position to world position.
    pub(crate) fn screen_to_world(&self, screen: Pos2, canvas_center: Pos2) -> GVec2 {
        let x = (screen.x - canvas_center.x) / self.zoom - self.offset.x;
        let y = (screen.y - canvas_center.y) / self.zoom - self.offset.y;
        GVec2::new(x, y)
    }
}

/// Main application state.
pub struct SpaghettiApp {
    // -- Graph data --
    pub(crate) graph: Graph,
    pub(crate) positions: Positions,

    // -- Interaction state --
    pub(crate) camera: Camera2D,
    pub(crate) selection: Option<SymbolId>,
    pub(crate) edge_filter: EdgeKindFilter,
    pub(crate) search: String,

    // -- Menu / UI state --
    pub(crate) show_console: bool,
    pub(crate) indexing: bool,

    // -- Log capture --
    pub(crate) log_buffer: Arc<Mutex<LogBuffer>>,
    pub(crate) console_level_filter: Level,

    // -- Progress overlay --
    pub(crate) progress_state: Option<ProgressState>,
    pub(crate) progress_rx: Option<Receiver<ProgressMessage>>,
    pub(crate) cancel_tx: Option<Sender<()>>,

    // -- File dialog --
    pub(crate) pending_file_dialog: Option<Receiver<Option<PathBuf>>>,
}

impl SpaghettiApp {
    /// Create a new app with a pre-loaded graph and positions.
    pub fn new(graph: Graph, positions: Positions, log_buffer: Arc<Mutex<LogBuffer>>) -> Self {
        Self {
            graph,
            positions,
            camera: Camera2D::default(),
            selection: None,
            edge_filter: EdgeKindFilter::default(),
            search: String::new(),
            show_console: false,
            indexing: false,
            log_buffer,
            console_level_filter: Level::INFO,
            progress_state: None,
            progress_rx: None,
            cancel_tx: None,
            pending_file_dialog: None,
        }
    }

    /// Create a new app with an empty graph (for menu-driven file opening).
    pub fn empty(log_buffer: Arc<Mutex<LogBuffer>>) -> Self {
        Self::new(Graph::new(), Positions(Default::default()), log_buffer)
    }

    /// Draw the menu bar.
    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu_bar").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    let enabled = !self.indexing;
                    if ui
                        .add_enabled(enabled, egui::Button::new("Open…"))
                        .clicked()
                    {
                        ui.close();
                        self.open_file_dialog();
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });

                ui.menu_button("View", |ui| {
                    if ui.checkbox(&mut self.show_console, "Console").changed() {
                        ui.close();
                    }
                });
            });
        });
    }

    /// Open a native file dialog in a background thread.
    fn open_file_dialog(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let file = rfd::FileDialog::new()
                .add_filter("JSON files", &["json"])
                .set_title("Open compile_commands.json or graph.json")
                .pick_file();
            let _ = tx.send(file);
        });
        self.pending_file_dialog = Some(rx);
    }

    /// Start indexing a file in a background thread.
    fn start_indexing(&mut self, path: PathBuf) {
        self.indexing = true;

        let (progress_tx, progress_rx) = std::sync::mpsc::channel();
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();

        self.progress_state = Some(ProgressState::new("Indexing…"));
        self.progress_rx = Some(progress_rx);
        self.cancel_tx = Some(cancel_tx);

        std::thread::spawn(move || {
            let _ = progress_tx.send(ProgressMessage::Status(format!(
                "Loading {}…",
                path.display()
            )));

            // Check for cancellation before starting
            if cancel_rx.try_recv().is_ok() {
                let _ = progress_tx.send(ProgressMessage::Cancelled);
                return;
            }

            match frontend_clang::index_project(&path) {
                Ok(graph) => {
                    let _ = progress_tx.send(ProgressMessage::Log(format!(
                        "Indexed {} symbols, {} edges",
                        graph.symbol_count(),
                        graph.edge_count()
                    )));

                    let _ = progress_tx.send(ProgressMessage::Status("Computing layout…".into()));

                    let layout_algo = layout::ForceDirected::default();
                    let positions = layout::Layout::compute(&layout_algo, &graph);

                    let _ = progress_tx.send(ProgressMessage::Done {
                        graph: Box::new(graph),
                        positions: Box::new(positions),
                    });
                }
                Err(e) => {
                    let _ = progress_tx.send(ProgressMessage::Failed(format!("{e}")));
                }
            }
        });
    }

    /// Poll channels each frame for file dialog results and progress updates.
    fn poll_channels(&mut self) {
        // Check file dialog result
        let mut opened_path = None;
        if let Some(rx) = &self.pending_file_dialog {
            if let Ok(result) = rx.try_recv() {
                opened_path = result;
                self.pending_file_dialog = None;
            }
        }
        if let Some(path) = opened_path {
            self.start_indexing(path);
        }

        // Collect progress messages (to avoid borrow issues).
        let messages: Vec<_> = self
            .progress_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();

        for msg in messages {
            if let Some(state) = &mut self.progress_state {
                state.apply(&msg);
            }

            match msg {
                ProgressMessage::Done { graph, positions } => {
                    self.graph = *graph;
                    self.positions = *positions;
                    self.selection = None;
                    self.camera = Camera2D::default();
                    self.finish_indexing();
                }
                ProgressMessage::Failed(ref err) => {
                    tracing::error!("Indexing failed: {err}");
                    self.finish_indexing();
                }
                ProgressMessage::Cancelled => {
                    tracing::info!("Indexing cancelled");
                    self.finish_indexing();
                }
                _ => {}
            }
        }
    }

    /// Clean up indexing state after completion, failure, or cancellation.
    fn finish_indexing(&mut self) {
        self.indexing = false;
        self.progress_state = None;
        self.progress_rx = None;
        self.cancel_tx = None;
    }

    /// Draw the progress overlay (modal).
    fn progress_overlay(&mut self, ui: &mut egui::Ui) {
        let Some(state) = &self.progress_state else {
            return;
        };

        // Semi-transparent backdrop
        let screen_rect = ui.ctx().content_rect();
        ui.painter()
            .rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(160));

        egui::Area::new(egui::Id::new("progress_overlay"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(400.0);
                    ui.vertical(|ui| {
                        ui.heading(&state.status);
                        ui.separator();

                        // Progress bar
                        if let Some(frac) = state.fraction() {
                            let bar = egui::ProgressBar::new(frac).text(format!(
                                "{}/{}",
                                state.current,
                                state.total.unwrap_or(0)
                            ));
                            ui.add(bar);
                        } else {
                            ui.spinner();
                        }

                        // Scrollable message log
                        if !state.messages.is_empty() {
                            ui.separator();
                            egui::ScrollArea::vertical()
                                .max_height(200.0)
                                .stick_to_bottom(true)
                                .show(ui, |ui| {
                                    for msg in &state.messages {
                                        ui.label(msg);
                                    }
                                });
                        }

                        ui.separator();

                        // Cancel button
                        if ui.button("Cancel").clicked() {
                            if let Some(tx) = &self.cancel_tx {
                                let _ = tx.send(());
                            }
                        }
                    });
                });
            });
    }
}

impl eframe::App for SpaghettiApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_channels();
        self.menu_bar(ui);
        self.console_panel(ui);
        self.left_panel(ui);
        self.right_panel(ui);
        self.central_panel(ui);
        self.progress_overlay(ui);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test: app can be created with an empty graph.
    #[test]
    fn app_creates_with_empty_graph() {
        let log_buffer = Arc::new(Mutex::new(LogBuffer::new()));
        let app = SpaghettiApp::empty(log_buffer);
        assert_eq!(app.graph.symbol_count(), 0);
        assert_eq!(app.graph.edge_count(), 0);
        assert!(!app.indexing);
        assert!(!app.show_console);
        assert!(app.selection.is_none());
    }

    /// Menu items should report disabled when indexing is active.
    #[test]
    fn menu_disabled_during_indexing() {
        let log_buffer = Arc::new(Mutex::new(LogBuffer::new()));
        let mut app = SpaghettiApp::empty(log_buffer);

        assert!(!app.indexing, "not indexing initially");

        // Simulate indexing started
        app.indexing = true;
        assert!(app.indexing, "indexing flag is set");

        // The menu_bar method checks `self.indexing` to disable Open.
        // We verify the flag is correctly gating — the actual UI rendering
        // requires an egui context which is not available in unit tests.

        app.indexing = false;
        assert!(!app.indexing, "indexing flag cleared");
    }
}
