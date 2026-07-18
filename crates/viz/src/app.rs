//! The main eframe application for spaghetti.

use std::sync::{Arc, Mutex};

use core_ir::Graph;
use layout::LayoutState;
use tracing::Level;

use crate::camera::{Camera2D, NodeSizes};
use crate::file_tree::FileTree;
use crate::fps::FpsCounter;
use crate::log_capture::LogBuffer;
use crate::progress::{ProgressMessage, ProgressState};
use crate::settings::ViewSettings;
use crate::state::filter_state::{EdgeKindFilter, SymbolKindFilter};
use crate::state::{
    ConsoleState, FilterState, GraphState, IndexingState, InteractionState, RenderState,
    SimulationState,
};

/// Energy threshold below which the simulation is considered settled and
/// repaints are no longer requested.
pub(crate) const ENERGY_THRESHOLD: f32 = 0.01;

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

/// Main application state, composed of domain-specific sub-structs.
pub struct SpaghettiApp {
    /// Graph data and file tree.
    pub(crate) graph: GraphState,
    /// User interaction: selection, drag, search.
    pub(crate) interaction: InteractionState,
    /// Visibility filtering: edge/node kind toggles, hidden symbols.
    pub(crate) filters: FilterState,
    /// Force-directed layout simulation.
    pub(crate) simulation: SimulationState,
    /// Rendering: colors, camera, FPS.
    pub(crate) render: RenderState,
    /// Console/log viewer.
    pub(crate) console: ConsoleState,
    /// Background indexing and file dialog.
    pub(crate) indexing: IndexingState,
}

impl SpaghettiApp {
    /// Create a new app with a live [`LayoutState`] that drives positions
    /// incrementally each frame.
    pub fn new(
        graph: Graph,
        layout_state: LayoutState,
        log_buffer: Arc<Mutex<LogBuffer>>,
        render: crate::settings::RenderSettings,
        view: ViewSettings,
    ) -> Self {
        let positions = layout_state.positions();
        let mut file_tree = FileTree::from_graph(&graph);
        file_tree.apply_visibility(&view.dir_visibility);
        let hidden_symbols = file_tree.hidden_symbols();
        let camera = Camera2D {
            offset: egui::Vec2::new(view.camera_offset[0], view.camera_offset[1]),
            zoom: view.camera_zoom,
        };

        let mut app = Self {
            graph: GraphState {
                base: graph.clone(),
                graph,
                split_shared_types: view.split_shared_types,
                file_tree,
            },
            interaction: InteractionState {
                selection: None,
                hovered: None,
                dragging: None,
                search: String::new(),
                auto_fitted: false,
                frame_count: 0,
            },
            filters: FilterState {
                edge_filter: EdgeKindFilter::from_saved(&view.edge_filters),
                node_filter: SymbolKindFilter::from_saved(&view.node_filters),
                hidden_symbols,
                hide_edgeless: view.hide_edgeless,
                pending_dir_visibility: view.dir_visibility.clone(),
            },
            simulation: SimulationState {
                layout_state,
                positions,
                node_sizes: NodeSizes(Default::default()),
                paused: false,
                container_rects: Vec::new(),
            },
            render: RenderState {
                render,
                camera,
                fps: FpsCounter::new(60),
            },
            console: ConsoleState {
                show_console: view.show_console,
                log_buffer,
                console_level_filter: level_from_str(&view.console_level),
            },
            indexing: IndexingState::default(),
        };
        app.filters
            .sync_hidden_to_layout(&app.graph, &mut app.simulation);
        app.simulation.update_node_sizes(&app.graph.graph);
        if app.graph.split_shared_types {
            app.rebuild_display_graph();
        }
        app
    }

    /// Re-derive the displayed graph after the split-shared-types mode
    /// changes: rebuild the file tree and layout engine, carry over positions
    /// of surviving nodes, and seed new clones near their container.
    pub(crate) fn rebuild_display_graph(&mut self) {
        let old_positions = self.simulation.layout_state.positions();
        self.graph.rebuild_display();

        let vis = self.graph.file_tree.visibility_map();
        self.graph.file_tree = FileTree::from_graph(&self.graph.graph);
        self.graph.file_tree.apply_visibility(&vis);

        let params = self.simulation.layout_state.params().clone();
        self.simulation.layout_state = LayoutState::new(&self.graph.graph, 42, params);

        // Deterministic jitter keeps seeded clones from stacking exactly on
        // their container (coincident nodes stall repulsion).
        for &id in self.graph.graph.symbols.keys() {
            if let Some(&pos) = old_positions.0.get(&id) {
                self.simulation.layout_state.set_position(id, pos);
            } else if let Some(parent) = self.simulation.layout_state.parent_of(id) {
                if let Some(&ppos) = old_positions.0.get(&parent) {
                    let jitter = glam::Vec2::new(
                        (id.0 & 0xFF) as f32 / 255.0 - 0.5,
                        ((id.0 >> 8) & 0xFF) as f32 / 255.0 - 0.5,
                    ) * 40.0;
                    self.simulation.layout_state.set_position(id, ppos + jitter);
                }
            }
        }
        self.simulation.layout_state.reheat();

        self.filters
            .sync_hidden_symbols(&self.graph, &mut self.simulation);
        self.simulation.positions = self.simulation.layout_state.positions();
        self.simulation.container_rects.clear();
        self.interaction.dragging = None;
        if let Some(sel) = self.interaction.selection {
            if !self.graph.graph.symbols.contains_key(&sel) {
                self.interaction.selection = None;
            }
        }
    }

    /// Snapshot the current view state for serialization.
    pub(crate) fn view_settings(&self) -> ViewSettings {
        ViewSettings {
            edge_filters: self.filters.edge_filter.to_saved(),
            node_filters: self.filters.node_filter.to_saved(),
            camera_offset: [self.render.camera.offset.x, self.render.camera.offset.y],
            camera_zoom: self.render.camera.zoom,
            show_console: self.console.show_console,
            console_level: format!("{}", self.console.console_level_filter),
            hide_edgeless: self.filters.hide_edgeless,
            split_shared_types: self.graph.split_shared_types,
            dir_visibility: self.graph.file_tree.visibility_map(),
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
                let mut open_recent: Option<std::path::PathBuf> = None;
                ui.menu_button("File", |ui| {
                    let enabled = !self.indexing.indexing;
                    if ui
                        .add_enabled(enabled, egui::Button::new("Open…"))
                        .clicked()
                    {
                        ui.close();
                        self.open_file_dialog();
                    }
                    if !self.indexing.recent_projects.is_empty() {
                        ui.separator();
                        ui.label("Recent Projects");
                        for path in &self.indexing.recent_projects {
                            let label = path
                                .file_name()
                                .and_then(|f| f.to_str())
                                .map(|name| {
                                    path.parent()
                                        .and_then(|p| {
                                            let dir = p.file_name()?.to_str()?;
                                            let grandparent = p
                                                .parent()
                                                .and_then(|gp| gp.file_name())
                                                .and_then(|g| g.to_str());
                                            match grandparent {
                                                Some(gp) => Some(format!("{gp}/{dir}/{name}")),
                                                None => Some(format!("{dir}/{name}")),
                                            }
                                        })
                                        .unwrap_or_else(|| name.to_string())
                                })
                                .unwrap_or_else(|| path.display().to_string());
                            if ui.add_enabled(enabled, egui::Button::new(&label)).clicked() {
                                open_recent = Some(path.clone());
                                ui.close();
                            }
                        }
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                if let Some(path) = open_recent {
                    self.start_indexing(path);
                }

                ui.menu_button("View", |ui| {
                    if crate::widgets::toggle_button(ui, &mut self.console.show_console, "Console")
                        .changed()
                    {
                        ui.close();
                    }
                });

                ui.menu_button("Settings", |ui| {
                    let can_reload =
                        self.indexing.compile_commands_path.is_some() && !self.indexing.indexing;
                    if ui
                        .add_enabled(can_reload, egui::Button::new("Clear Cache & Reload"))
                        .clicked()
                    {
                        if let Some(cc_path) = self.indexing.compile_commands_path.clone() {
                            let cache_dir = frontend_clang::cache_dir(&cc_path);
                            match std::fs::remove_dir_all(&cache_dir) {
                                Ok(()) => {
                                    tracing::info!(
                                        path = %cache_dir.display(),
                                        "cleared TU cache"
                                    );
                                }
                                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                                    tracing::info!("no cache to clear");
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        error = %e,
                                        "failed to clear cache"
                                    );
                                }
                            }
                            self.start_indexing(cc_path);
                        }
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
        self.indexing.pending_file_dialog = Some(rx);
    }

    /// Apply persisted settings (render, view, force params) to the app.
    pub fn apply_saved_settings(&mut self, settings: &crate::settings::AppSettings) {
        self.render.render = settings.render.clone();
        self.filters.edge_filter = EdgeKindFilter::from_saved(&settings.view.edge_filters);
        self.filters.node_filter = SymbolKindFilter::from_saved(&settings.view.node_filters);
        self.console.show_console = settings.view.show_console;
        self.console.console_level_filter = level_from_str(&settings.view.console_level);
        self.filters.hide_edgeless = settings.view.hide_edgeless;
        self.render.camera = Camera2D {
            offset: egui::Vec2::new(
                settings.view.camera_offset[0],
                settings.view.camera_offset[1],
            ),
            zoom: settings.view.camera_zoom,
        };
        *self.simulation.layout_state.params_mut() = settings.force_params.clone();
        self.filters.pending_dir_visibility = settings.view.dir_visibility.clone();
        if self.graph.split_shared_types != settings.view.split_shared_types {
            self.graph.split_shared_types = settings.view.split_shared_types;
            self.rebuild_display_graph();
        }
    }

    /// Start indexing a file in a background thread.
    pub fn start_indexing(&mut self, path: std::path::PathBuf) {
        self.indexing.indexing = true;
        self.indexing.compile_commands_path = Some(path.clone());
        self.indexing.recent_projects.retain(|p| p != &path);
        self.indexing.recent_projects.insert(0, path.clone());
        self.indexing.recent_projects.truncate(5);
        let params = self.simulation.layout_state.params().clone();

        let (progress_tx, progress_rx) = std::sync::mpsc::channel();
        let (cancel_tx, cancel_rx) = std::sync::mpsc::channel::<()>();

        self.indexing.progress_state = Some(ProgressState::new("Indexing…"));
        self.indexing.progress_rx = Some(progress_rx);
        self.indexing.cancel_tx = Some(cancel_tx);

        std::thread::spawn(move || {
            if let Err(e) = progress_tx.send(ProgressMessage::Status(format!(
                "Loading {}…",
                path.display()
            ))) {
                tracing::warn!("progress channel closed: {e}");
                return;
            }

            let tx = progress_tx.clone();
            let result = frontend_clang::index_project_with_progress(&path, move |progress| {
                if cancel_rx.try_recv().is_ok() {
                    return false;
                }
                match progress {
                    frontend_clang::IndexProgress::Parse {
                        current,
                        total,
                        file,
                    } => {
                        let _ = tx.send(ProgressMessage::Progress { current, total });
                        let _ = tx.send(ProgressMessage::Status(format!(
                            "Indexing TU {}/{}: {}",
                            current + 1,
                            total,
                            file,
                        )));
                    }
                    frontend_clang::IndexProgress::Merge { current, total } => {
                        if current == 0 {
                            let _ = tx.send(ProgressMessage::Status("Merging graphs…".into()));
                        }
                        let _ = tx.send(ProgressMessage::SubProgress { current, total });
                    }
                }
                true
            });

            match result {
                Ok(mut graph) => {
                    if graph.symbol_count() == 0 && graph.edge_count() == 0 {
                        let _ = progress_tx.send(ProgressMessage::Cancelled);
                        return;
                    }

                    let _ = progress_tx.send(ProgressMessage::Log(format!(
                        "Indexed {} symbols, {} edges",
                        graph.symbol_count(),
                        graph.edge_count()
                    )));

                    // ISPC pass: picks up the .ispc entries the clang
                    // frontend skips. Fast (pure tree-sitter), so no
                    // cancellation.
                    let _ = progress_tx.send(ProgressMessage::Status("ISPC pass…".into()));
                    let ispc_result =
                        frontend_ispc::index_project_with_progress(&path, |current, total, _| {
                            let _ =
                                progress_tx.send(ProgressMessage::SubProgress { current, total });
                            true
                        });
                    match ispc_result {
                        Ok(ispc_graph) if ispc_graph.symbol_count() > 0 => {
                            let _ = progress_tx.send(ProgressMessage::Log(format!(
                                "ISPC: {} symbols, {} edges",
                                ispc_graph.symbol_count(),
                                ispc_graph.edge_count()
                            )));
                            graph.merge(ispc_graph);
                        }
                        Ok(_) => {}
                        Err(e) => {
                            let _ = progress_tx
                                .send(ProgressMessage::Log(format!("ISPC indexing failed: {e}")));
                        }
                    }

                    let _ = progress_tx.send(ProgressMessage::Status("Building layout…".into()));
                    let layout_state = layout::LayoutState::new(&graph, 42, params);

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
        let mut opened_path = None;
        if let Some(rx) = &self.indexing.pending_file_dialog {
            if let Ok(result) = rx.try_recv() {
                opened_path = result;
                self.indexing.pending_file_dialog = None;
            }
        }
        if let Some(path) = opened_path {
            self.start_indexing(path);
        }

        let messages: Vec<_> = self
            .indexing
            .progress_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();

        for msg in messages {
            if let Some(state) = &mut self.indexing.progress_state {
                state.apply(&msg);
            }

            match msg {
                ProgressMessage::Done {
                    graph,
                    layout_state,
                } => {
                    let mut vis = self.filters.pending_dir_visibility.clone();
                    vis.extend(self.graph.file_tree.visibility_map());
                    self.graph.base = *graph;
                    self.graph.rebuild_display();
                    self.graph.file_tree = FileTree::from_graph(&self.graph.graph);
                    self.graph.file_tree.apply_visibility(&vis);
                    // The background thread laid out the base graph; when
                    // split mode is on the displayed graph differs, so
                    // rebuild the layout for it here.
                    self.simulation.layout_state = if self.graph.split_shared_types {
                        let params = layout_state.params().clone();
                        LayoutState::new(&self.graph.graph, 42, params)
                    } else {
                        *layout_state
                    };
                    self.filters
                        .sync_hidden_symbols(&self.graph, &mut self.simulation);
                    self.simulation.positions = self.simulation.layout_state.positions();
                    self.interaction.selection = None;
                    self.render.camera = Camera2D::default();
                    self.interaction.dragging = None;
                    self.interaction.auto_fitted = false;
                    self.interaction.frame_count = 0;
                    self.simulation.container_rects.clear();
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
        self.indexing.indexing = false;
        self.indexing.progress_state = None;
        self.indexing.progress_rx = None;
        self.indexing.cancel_tx = None;
    }

    /// Draw the progress overlay (modal).
    fn progress_overlay(&mut self, ui: &mut egui::Ui) {
        let Some(state) = &self.indexing.progress_state else {
            return;
        };

        let screen_rect = ui.ctx().content_rect();
        ui.painter()
            .rect_filled(screen_rect, 0.0, egui::Color32::from_black_alpha(160));

        egui::Area::new(egui::Id::new("progress_overlay"))
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_min_width(800.0);
                    ui.vertical(|ui| {
                        ui.heading(&state.status);
                        ui.separator();

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

                        // Secondary bar (green): merge / ISPC phases.
                        if let Some(frac) = state.sub_fraction() {
                            let bar = egui::ProgressBar::new(frac)
                                .fill(egui::Color32::from_rgb(0x3a, 0x9e, 0x4c))
                                .text(format!(
                                    "{}/{}",
                                    state.sub_current,
                                    state.sub_total.unwrap_or(0)
                                ));
                            ui.add(bar);
                        }

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

                        if ui.button("Cancel").clicked() {
                            if let Some(tx) = &self.indexing.cancel_tx {
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
            force_params: self.simulation.layout_state.params().clone(),
            render: self.render.render.clone(),
            view: self.view_settings(),
            recent_projects: self.indexing.recent_projects.clone(),
        };
        settings.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_creates_with_empty_graph() {
        let log_buffer = Arc::new(Mutex::new(LogBuffer::new()));
        let app = SpaghettiApp::empty(log_buffer);
        assert_eq!(app.graph.graph.symbol_count(), 0);
        assert_eq!(app.graph.graph.edge_count(), 0);
        assert!(!app.indexing.indexing);
        assert!(!app.console.show_console);
        assert!(app.interaction.selection.is_none());
    }

    #[test]
    fn split_mode_toggle_rebuilds_display_graph() {
        use core_ir::{Edge, EdgeKind, Symbol, SymbolId, SymbolKind};

        let mut g = Graph::new();
        let mut add = |name: &str, kind: SymbolKind| {
            let id = SymbolId::from_parts(name, kind);
            g.add_symbol(Symbol {
                id,
                kind,
                name: name.to_owned(),
                qualified_name: name.to_owned(),
                location: None,
                module: None,
                attrs: Default::default(),
            });
            id
        };
        let foo = add("Foo", SymbolKind::Struct);
        let bar = add("Bar", SymbolKind::Struct);
        let vec3 = add("vec3", SymbolKind::Struct);
        let fa = add("Foo::a", SymbolKind::Field);
        let bb = add("Bar::b", SymbolKind::Field);
        for (from, to, kind) in [
            (foo, fa, EdgeKind::Contains),
            (bar, bb, EdgeKind::Contains),
            (fa, vec3, EdgeKind::HasType),
            (bb, vec3, EdgeKind::HasType),
        ] {
            g.add_edge(Edge {
                from,
                to,
                kind,
                location: None,
            });
        }

        let layout_state = LayoutState::new(&g, 42, layout::ForceParams::default());
        let log_buffer = Arc::new(Mutex::new(LogBuffer::new()));
        let mut app = SpaghettiApp::new(
            g,
            layout_state,
            log_buffer,
            crate::settings::RenderSettings::default(),
            ViewSettings::default(),
        );
        assert_eq!(app.graph.graph.symbol_count(), 5);

        app.graph.split_shared_types = true;
        app.rebuild_display_graph();
        // Two clones added; base untouched.
        assert_eq!(app.graph.graph.symbol_count(), 7);
        assert_eq!(app.graph.base.symbol_count(), 5);
        let clone_id = SymbolId::from_parts("vec3@Foo", SymbolKind::Struct);
        assert!(app.graph.graph.symbols.contains_key(&clone_id));
        assert!(app.simulation.positions.0.contains_key(&clone_id));

        app.graph.split_shared_types = false;
        app.rebuild_display_graph();
        assert_eq!(app.graph.graph.symbol_count(), 5);
    }

    #[test]
    fn menu_disabled_during_indexing() {
        let log_buffer = Arc::new(Mutex::new(LogBuffer::new()));
        let mut app = SpaghettiApp::empty(log_buffer);

        assert!(!app.indexing.indexing, "not indexing initially");
        app.indexing.indexing = true;
        assert!(app.indexing.indexing, "indexing flag is set");
        app.indexing.indexing = false;
        assert!(!app.indexing.indexing, "indexing flag cleared");
    }
}
