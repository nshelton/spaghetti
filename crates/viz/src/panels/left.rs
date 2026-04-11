//! Left panel: search bar, edge filters, and file tree.

use crate::app::SpaghettiApp;
use crate::file_tree::{self, DirNode, FileNode};

impl SpaghettiApp {
    /// Draw the left panel: search, filters, file tree.
    pub(crate) fn left_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("left_panel")
            .default_size(260.0)
            .show_inside(ui, |ui| {
                ui.heading("spaghetti");
                ui.label(format!(
                    "{} nodes, {} edges",
                    self.graph.symbol_count(),
                    self.graph.edge_count()
                ));
                ui.separator();

                // Search
                ui.label("Search:");
                ui.text_edit_singleline(&mut self.search);
                ui.separator();

                // File tree
                ui.label("Files:");
                let mut visibility_changed = false;

                egui::ScrollArea::vertical().show(ui, |ui| {
                    let search_lower = self.search.to_lowercase();
                    let search = if search_lower.is_empty() {
                        None
                    } else {
                        Some(search_lower.as_str())
                    };

                    // Root-level directories
                    for dir in &mut self.file_tree.roots {
                        visibility_changed |= draw_dir_node(ui, dir, &mut self.selection, search);
                    }

                    // Root-level files
                    for file in &self.file_tree.root_files {
                        draw_file_node(ui, file, &mut self.selection, search);
                    }

                    // External symbols (collapsed, with visibility toggle)
                    if !self.file_tree.external_symbols.is_empty() {
                        let ext_count = self.file_tree.external_symbols.len();
                        let id = ui.make_persistent_id("_externals");
                        egui::collapsing_header::CollapsingState::load_with_default_open(
                            ui.ctx(),
                            id,
                            false,
                        )
                        .show_header(ui, |ui| {
                            if ui
                                .checkbox(&mut self.file_tree.externals_visible, "")
                                .changed()
                            {
                                visibility_changed = true;
                            }
                            ui.label(format!("<external> ({ext_count} symbols)"));
                        })
                        .body(|ui| {
                            for &(sym_id, kind, ref name) in &self.file_tree.external_symbols {
                                if let Some(search) = search {
                                    if !name.to_lowercase().contains(search) {
                                        continue;
                                    }
                                }
                                let label = format!("{kind:?} {name}");
                                let selected = self.selection == Some(sym_id);
                                if ui.selectable_label(selected, &label).clicked() {
                                    self.selection = Some(sym_id);
                                }
                            }
                        });
                    }
                });

                // If visibility changed, recompute hidden set and push to layout.
                if visibility_changed {
                    self.sync_hidden_symbols();
                }

                ui.separator();

                // Collapse / Expand All buttons
                ui.horizontal(|ui| {
                    if ui.button("Collapse All").clicked() {
                        self.layout_state.collapse_all();
                        self.sync_hidden_symbols();
                    }
                    if ui.button("Expand All").clicked() {
                        self.layout_state.expand_all();
                        self.sync_hidden_symbols();
                    }
                });
            });
    }
}

/// Draw a directory node with a visibility toggle and collapsible children.
/// Returns `true` if visibility was toggled.
fn draw_dir_node(
    ui: &mut egui::Ui,
    dir: &mut DirNode,
    selection: &mut Option<core_ir::SymbolId>,
    search: Option<&str>,
) -> bool {
    let mut changed = false;

    let id = ui.make_persistent_id(format!("dir_{}", dir.name));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
        .show_header(ui, |ui| {
            if ui.checkbox(&mut dir.visible, "").changed() {
                // Propagate visibility to all children.
                set_visibility_recursive(dir, dir.visible);
                changed = true;
            }
            let total = count_symbols_in_dir(dir);
            ui.label(format!("{}/  ({total} symbols)", dir.name));
        })
        .body(|ui| {
            for child_dir in &mut dir.children_dirs {
                changed |= draw_dir_node(ui, child_dir, selection, search);
            }
            for file in &dir.files {
                draw_file_node(ui, file, selection, search);
            }
        });

    changed
}

/// Draw a file node as a collapsible header with symbol summary.
fn draw_file_node(
    ui: &mut egui::Ui,
    file: &FileNode,
    selection: &mut Option<core_ir::SymbolId>,
    search: Option<&str>,
) {
    let summary = file_tree::file_summary(file);
    let id = ui.make_persistent_id(format!("file_{}", file.name));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
        .show_header(ui, |ui| {
            ui.label(&summary);
        })
        .body(|ui| {
            for &(sym_id, kind, ref name) in &file.symbols {
                if let Some(search) = search {
                    if !name.to_lowercase().contains(search) {
                        continue;
                    }
                }
                let label = format!("{kind:?} {name}");
                let selected = *selection == Some(sym_id);
                if ui.selectable_label(selected, &label).clicked() {
                    *selection = Some(sym_id);
                }
            }
        });
}

/// Recursively set visibility on all subdirectories.
fn set_visibility_recursive(dir: &mut DirNode, visible: bool) {
    dir.visible = visible;
    for child in &mut dir.children_dirs {
        set_visibility_recursive(child, visible);
    }
}

/// Count total symbols under a directory (recursively).
fn count_symbols_in_dir(dir: &DirNode) -> usize {
    let mut count: usize = dir.files.iter().map(|f| f.symbols.len()).sum();
    for child in &dir.children_dirs {
        count += count_symbols_in_dir(child);
    }
    count
}
