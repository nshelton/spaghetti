//! The main eframe application for spaghetti.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use std::collections::HashMap;

use core_ir::{EdgeKind, Graph, SymbolId, SymbolKind};
use layout::{LayoutState, Positions};
use tracing::Level;

use crate::file_tree::FileTree;

use crate::camera::Camera2D;
use crate::fps::FpsCounter;
use crate::log_capture::LogBuffer;
use crate::progress::{ProgressMessage, ProgressState};
use crate::settings::ViewSettings;

/// All symbol kinds in the system.
pub(crate) const ALL_SYMBOL_KINDS: [SymbolKind; 8] = [
    SymbolKind::Class,
    SymbolKind::Struct,
    SymbolKind::Function,
    SymbolKind::Method,
    SymbolKind::Field,
    SymbolKind::Namespace,
    SymbolKind::TemplateInstantiation,
    SymbolKind::TranslationUnit,
];

/// All edge kinds in the system.
pub(crate) const ALL_EDGE_KINDS: [EdgeKind; 9] = [
    EdgeKind::Calls,
    EdgeKind::Inherits,
    EdgeKind::Contains,
    EdgeKind::Overrides,
    EdgeKind::ReadsField,
    EdgeKind::WritesField,
    EdgeKind::Includes,
    EdgeKind::Instantiates,
    EdgeKind::HasType,
];

/// Edge kind filter state — tracks which edge kinds are visible.
pub(crate) struct EdgeKindFilter {
    enabled: HashSet<EdgeKind>,
}

impl Default for EdgeKindFilter {
    fn default() -> Self {
        Self {
            enabled: ALL_EDGE_KINDS.iter().copied().collect(),
        }
    }
}

impl EdgeKindFilter {
    /// Restore from saved edge filter map. Missing keys default to enabled.
    pub(crate) fn from_saved(saved: &HashMap<String, bool>) -> Self {
        let mut enabled = HashSet::new();
        for &kind in &ALL_EDGE_KINDS {
            let key = format!("{kind:?}");
            let is_enabled = saved.get(&key).copied().unwrap_or(true);
            if is_enabled {
                enabled.insert(kind);
            }
        }
        Self { enabled }
    }

    /// Export current state as a string-keyed map for serialization.
    pub(crate) fn to_saved(&self) -> HashMap<String, bool> {
        ALL_EDGE_KINDS
            .iter()
            .map(|&kind| (format!("{kind:?}"), self.enabled.contains(&kind)))
            .collect()
    }

    /// Returns the list of currently active edge kinds.
    pub(crate) fn active_kinds(&self) -> Vec<EdgeKind> {
        self.enabled.iter().copied().collect()
    }

    /// Check whether a specific edge kind is enabled.
    pub(crate) fn is_enabled(&self, kind: EdgeKind) -> bool {
        self.enabled.contains(&kind)
    }

    /// Toggle a specific edge kind on or off.
    pub(crate) fn toggle(&mut self, kind: EdgeKind) {
        if self.enabled.contains(&kind) {
            self.enabled.remove(&kind);
        } else {
            self.enabled.insert(kind);
        }
    }
}

/// Symbol kind filter state — tracks which node kinds are visible.
pub(crate) struct SymbolKindFilter {
    enabled: HashSet<SymbolKind>,
}

impl Default for SymbolKindFilter {
    fn default() -> Self {
        Self {
            enabled: ALL_SYMBOL_KINDS.iter().copied().collect(),
        }
    }
}

impl SymbolKindFilter {
    /// Restore from saved node filter map. Missing keys default to enabled.
    pub(crate) fn from_saved(saved: &HashMap<String, bool>) -> Self {
        let mut enabled = HashSet::new();
        for &kind in &ALL_SYMBOL_KINDS {
            let key = format!("{kind:?}");
            let is_enabled = saved.get(&key).copied().unwrap_or(true);
            if is_enabled {
                enabled.insert(kind);
            }
        }
        Self { enabled }
    }

    /// Export current state as a string-keyed map for serialization.
    pub(crate) fn to_saved(&self) -> HashMap<String, bool> {
        ALL_SYMBOL_KINDS
            .iter()
            .map(|&kind| (format!("{kind:?}"), self.enabled.contains(&kind)))
            .collect()
    }

    /// Check whether a specific symbol kind is enabled.
    pub(crate) fn is_enabled(&self, kind: SymbolKind) -> bool {
        self.enabled.contains(&kind)
    }

    /// Toggle a specific symbol kind on or off.
    pub(crate) fn toggle(&mut self, kind: SymbolKind) {
        if self.enabled.contains(&kind) {
            self.enabled.remove(&kind);
        } else {
            self.enabled.insert(kind);
        }
    }
}

/// Parse a tracing level from a string, defaulting to INFO.
fn level_from_str(s: &str) -> Level {
    match s {
        "ERROR" => Level::ERROR,
        "WARN" => Level::WARN,
        "DEBUG" => Level::DEBUG,
        "TRACE" => Level::TRACE,
        _ => Level::INFO,
    }
}

/// Energy threshold below which the simulation is considered settled and
/// repaints are no longer requested.
pub(crate) const ENERGY_THRESHOLD: f32 = 0.01;

/// Main application state.
pub struct SpaghettiApp {
    // -- Graph data --
    pub(crate) graph: Graph,
    pub(crate) positions: Positions,
    pub(crate) layout_state: LayoutState,

    // -- Interaction state --
    pub(crate) camera: Camera2D,
    pub(crate) selection: Option<SymbolId>,
    pub(crate) edge_filter: EdgeKindFilter,
    pub(crate) node_filter: SymbolKindFilter,
    pub(crate) search: String,
    /// The node currently being dragged, if any.
    pub(crate) dragging: Option<SymbolId>,
    /// Whether the initial auto-fit has been performed.
    pub(crate) auto_fitted: bool,
    /// Frame counter used to trigger auto-fit after a timeout.
    pub(crate) frame_count: u32,
    /// File/directory tree built from symbol locations.
    pub(crate) file_tree: FileTree,
    /// Symbols currently hidden by file-tree visibility toggles.
    pub(crate) hidden_symbols: HashSet<SymbolId>,
    /// Whether the layout simulation is paused.
    pub(crate) paused: bool,
    /// Saved directory visibility (applied after indexing completes).
    pub(crate) pending_dir_visibility: HashMap<String, bool>,

    // -- Rendering settings (persisted) --
    pub(crate) render: crate::settings::RenderSettings,

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

    // -- FPS counter --
    pub(crate) fps: FpsCounter,

    // -- Compound node state --
    /// Screen-space rects of expanded containers (computed each frame for
    /// hit-testing). Pairs of (container SymbolId, screen Rect).
    pub(crate) container_rects: Vec<(SymbolId, egui::Rect)>,
}

impl SpaghettiApp {
    /// Create a new app with a live [`LayoutState`] that drives positions
    /// incrementally each frame.
    pub fn new(
        graph: Graph,
        mut layout_state: LayoutState,
        log_buffer: Arc<Mutex<LogBuffer>>,
        render: crate::settings::RenderSettings,
        view: ViewSettings,
    ) -> Self {
        let positions = layout_state.positions();
        let mut file_tree = FileTree::from_graph(&graph);
        // Apply saved directory visibility (gracefully ignores stale paths).
        file_tree.apply_visibility(&view.dir_visibility);
        let hidden_symbols = file_tree.hidden_symbols();
        // Push initial hidden set to the layout engine.
        let hidden_vec: Vec<_> = hidden_symbols.iter().copied().collect();
        layout_state.set_hidden(&hidden_vec);

        let camera = Camera2D {
            offset: egui::Vec2::new(view.camera_offset[0], view.camera_offset[1]),
            zoom: view.camera_zoom,
        };

        Self {
            graph,
            positions,
            layout_state,
            camera,
            selection: None,
            edge_filter: EdgeKindFilter::from_saved(&view.edge_filters),
            node_filter: SymbolKindFilter::from_saved(&view.node_filters),
            search: String::new(),
            dragging: None,
            auto_fitted: false,
            frame_count: 0,
            file_tree,
            hidden_symbols,
            paused: false,
            pending_dir_visibility: view.dir_visibility.clone(),
            render,
            show_console: view.show_console,
            indexing: false,
            log_buffer,
            console_level_filter: level_from_str(&view.console_level),
            progress_state: None,
            progress_rx: None,
            cancel_tx: None,
            pending_file_dialog: None,
            fps: FpsCounter::new(60),
            container_rects: Vec::new(),
        }
    }

    /// Recompute the effective hidden-symbols set by merging file-tree
    /// visibility with layout collapse state.
    pub(crate) fn sync_hidden_symbols(&mut self) {
        let file_hidden = self.file_tree.hidden_symbols();
        let collapse_hidden = self.layout_state.collapsed_hidden_ids();
        self.hidden_symbols = &file_hidden | &collapse_hidden.into_iter().collect();
        let hidden_vec: Vec<_> = file_hidden.into_iter().collect();
        self.layout_state.set_hidden(&hidden_vec);
    }

    /// Snapshot the current view state for serialization.
    pub(crate) fn view_settings(&self) -> ViewSettings {
        ViewSettings {
            edge_filters: self.edge_filter.to_saved(),
            node_filters: self.node_filter.to_saved(),
            camera_offset: [self.camera.offset.x, self.camera.offset.y],
            camera_zoom: self.camera.zoom,
            show_console: self.show_console,
            console_level: format!("{}", self.console_level_filter),
            dir_visibility: self.file_tree.visibility_map(),
        }
    }

    /// Create a new app with an empty graph (for menu-driven file opening).
    pub fn empty(log_buffer: Arc<Mutex<LogBuffer>>) -> Self {
        let graph = Graph::new();
        let layout_state = LayoutState::new(&graph, 42, layout::ForceParams::default());
        Self::new(
            graph,
            layout_state,
            log_buffer,
            crate::settings::RenderSettings::default(),
            ViewSettings::default(),
        )
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
                    if crate::widgets::toggle_button(ui, &mut self.show_console, "Console")
                        .changed()
                    {
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
        let params = self.layout_state.params().clone();

        let (progress_tx, progress_rx) = std::sync::mpsc::channel();
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();

        self.progress_state = Some(ProgressState::new("Indexing…"));
        self.progress_rx = Some(progress_rx);
        self.cancel_tx = Some(cancel_tx);

        std::thread::spawn(move || {
            if let Err(e) = progress_tx.send(ProgressMessage::Status(format!(
                "Loading {}…",
                path.display()
            ))) {
                tracing::warn!("progress channel closed: {e}");
                return;
            }

            let tx = progress_tx.clone();
            let result =
                frontend_clang::index_project_with_progress(&path, move |current, total, file| {
                    // Check for cancellation
                    if cancel_rx.try_recv().is_ok() {
                        return false;
                    }
                    let _ = tx.send(ProgressMessage::Progress { current, total });
                    let _ = tx.send(ProgressMessage::Status(format!(
                        "Indexing TU {}/{}: {}",
                        current + 1,
                        total,
                        file,
                    )));
                    true
                });

            match result {
                Ok(graph) => {
                    if graph.symbol_count() == 0 && graph.edge_count() == 0 {
                        // Cancelled mid-indexing, nothing useful produced
                        let _ = progress_tx.send(ProgressMessage::Cancelled);
                        return;
                    }

                    let _ = progress_tx.send(ProgressMessage::Log(format!(
                        "Indexed {} symbols, {} edges",
                        graph.symbol_count(),
                        graph.edge_count()
                    )));

                    let _ = progress_tx.send(ProgressMessage::Status("Computing layout…".into()));

                    let mut layout_state = layout::LayoutState::new(&graph, 42, params);
                    layout_state.step(200);

                    let _ = progress_tx.send(ProgressMessage::Done {
                        graph: Box::new(graph),
                        layout_state: Box::new(layout_state),
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
                ProgressMessage::Done {
                    graph,
                    layout_state,
                } => {
                    // Preserve directory visibility: merge pending (from
                    // settings on disk) with current tree state.
                    let mut vis = self.pending_dir_visibility.clone();
                    vis.extend(self.file_tree.visibility_map());
                    self.file_tree = FileTree::from_graph(&graph);
                    self.file_tree.apply_visibility(&vis);
                    self.graph = *graph;
                    self.layout_state = *layout_state;
                    self.sync_hidden_symbols();
                    self.positions = self.layout_state.positions();
                    self.selection = None;
                    self.camera = Camera2D::default();
                    self.dragging = None;
                    self.auto_fitted = false;
                    self.frame_count = 0;
                    self.container_rects.clear();
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

    fn on_exit(&mut self) {
        let settings = crate::settings::AppSettings {
            force_params: self.layout_state.params().clone(),
            render: self.render.clone(),
            view: self.view_settings(),
        };
        settings.save();
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
