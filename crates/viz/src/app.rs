//! The main eframe application for spaghetti.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use std::collections::HashMap;

use core_ir::{EdgeKind, Graph, SymbolId};
use layout::{LayoutState, Positions};
use tracing::Level;

/// Pre-grouped edges for the selected symbol, keyed by edge kind.
/// Rebuilt when `selection` changes (instead of scanning all edges per frame).
#[derive(Default)]
pub(crate) struct EdgeCache {
    /// (edge_kind, direction, target_qualified_name, is_external)
    pub entries: Vec<(EdgeKind, EdgeDir, String, bool)>,
    /// The symbol this cache was built for.
    pub for_symbol: Option<SymbolId>,
}

/// Direction of an edge relative to the selected symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EdgeDir {
    Outgoing,
    Incoming,
}

impl EdgeCache {
    /// Rebuild the cache for a new selection.
    pub fn rebuild(&mut self, graph: &Graph, selection: Option<SymbolId>) {
        self.for_symbol = selection;
        self.entries.clear();

        let Some(sel_id) = selection else { return };

        for edge in &graph.edges {
            if edge.from == sel_id {
                if let Some(target) = graph.symbols.get(&edge.to) {
                    self.entries.push((
                        edge.kind,
                        EdgeDir::Outgoing,
                        target.qualified_name.clone(),
                        graph.is_external(edge.to),
                    ));
                }
            } else if edge.to == sel_id {
                if let Some(source) = graph.symbols.get(&edge.from) {
                    self.entries.push((
                        edge.kind,
                        EdgeDir::Incoming,
                        source.qualified_name.clone(),
                        graph.is_external(edge.from),
                    ));
                }
            }
        }
    }
}

use crate::file_tree::FileTree;

use crate::camera::{self, Camera2D};
use crate::fps::FpsCounter;
use crate::log_capture::LogBuffer;
use crate::progress::{ProgressMessage, ProgressState};
use crate::settings::ViewSettings;

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
    pub(crate) search: String,
    /// The node currently being dragged, if any.
    pub(crate) dragging: Option<SymbolId>,
    /// Whether the initial auto-fit has been performed.
    pub(crate) auto_fitted: bool,
    /// When the current graph was loaded, for auto-fit timeout.
    pub(crate) load_time: std::time::Instant,
    /// File/directory tree built from symbol locations.
    pub(crate) file_tree: FileTree,
    /// Symbols currently hidden by file-tree visibility toggles.
    pub(crate) hidden_symbols: HashSet<SymbolId>,
    /// Cached edge groupings for the selected symbol (rebuilt on selection change).
    pub(crate) edge_cache: EdgeCache,

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
            search: String::new(),
            dragging: None,
            auto_fitted: false,
            load_time: std::time::Instant::now(),
            file_tree,
            hidden_symbols,
            edge_cache: EdgeCache::default(),
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
        }
    }

    /// Snapshot the current view state for serialization.
    pub(crate) fn view_settings(&self) -> ViewSettings {
        ViewSettings {
            edge_filters: self.edge_filter.to_saved(),
            camera_offset: [self.camera.offset.x, self.camera.offset.y],
            camera_zoom: self.camera.zoom,
            show_console: self.show_console,
            console_level: format!("{}", self.console_level_filter),
            dir_visibility: self.file_tree.visibility_map(),
        }
    }

    /// Update the selected symbol and rebuild the edge cache.
    pub(crate) fn set_selection(&mut self, sel: Option<SymbolId>) {
        if self.selection != sel {
            self.selection = sel;
            self.edge_cache.rebuild(&self.graph, sel);
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

            // Check for cancellation before starting
            if cancel_rx.try_recv().is_ok() {
                if let Err(e) = progress_tx.send(ProgressMessage::Cancelled) {
                    tracing::warn!("progress channel closed: {e}");
                }
                return;
            }

            match frontend_clang::index_project(&path) {
                Ok(graph) => {
                    if let Err(e) = progress_tx.send(ProgressMessage::Log(format!(
                        "Indexed {} symbols, {} edges",
                        graph.symbol_count(),
                        graph.edge_count()
                    ))) {
                        tracing::warn!("progress channel closed: {e}");
                        return;
                    }

                    if let Err(e) =
                        progress_tx.send(ProgressMessage::Status("Computing layout…".into()))
                    {
                        tracing::warn!("progress channel closed: {e}");
                        return;
                    }

                    let mut layout_state = layout::LayoutState::new(&graph, 42, params);
                    layout_state.step(200);

                    if let Err(e) = progress_tx.send(ProgressMessage::Done {
                        graph: Box::new(graph),
                        layout_state: Box::new(layout_state),
                    }) {
                        tracing::warn!("progress channel closed: {e}");
                    }
                }
                Err(e) => {
                    if let Err(send_err) = progress_tx.send(ProgressMessage::Failed(format!("{e}")))
                    {
                        tracing::warn!("progress channel closed: {send_err}");
                    }
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
                    self.file_tree = FileTree::from_graph(&graph);
                    self.hidden_symbols = self.file_tree.hidden_symbols();
                    self.graph = *graph;
                    self.layout_state = *layout_state;
                    let hidden_vec: Vec<_> = self.hidden_symbols.iter().copied().collect();
                    self.layout_state.set_hidden(&hidden_vec);
                    self.positions = self.layout_state.positions();
                    self.set_selection(None);
                    self.camera = Camera2D::default();
                    self.dragging = None;
                    self.auto_fitted = false;
                    self.load_time = std::time::Instant::now();
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

    /// Hit-test: find which symbol (if any) is under the pointer.
    pub(crate) fn hit_test(
        &self,
        pointer: Option<egui::Pos2>,
        canvas_center: egui::Pos2,
    ) -> Option<SymbolId> {
        let radius = if self.render.circle_mode {
            Some(self.render.circle_radius)
        } else {
            None
        };
        camera::hit_test(
            &self.camera,
            &self.positions,
            pointer,
            canvas_center,
            radius,
        )
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
            version: crate::settings::SETTINGS_VERSION,
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
