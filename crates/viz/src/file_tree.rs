//! File-system tree built from symbol locations in the graph.
//!
//! Used by the left panel to display a collapsible directory/file hierarchy
//! with visibility toggles that control which nodes appear in the canvas
//! and participate in the layout simulation.

use std::collections::{HashMap, HashSet};

use core_ir::{Graph, SymbolId, SymbolKind};

/// A directory node in the file tree.
pub struct DirNode {
    /// Directory name (just the final component, e.g. `shapes`).
    pub name: String,
    /// Whether this directory (and all descendants) is visible.
    pub visible: bool,
    /// Subdirectories, sorted by name.
    pub children_dirs: Vec<DirNode>,
    /// Files directly in this directory, sorted by name.
    pub files: Vec<FileNode>,
}

/// A file node in the file tree.
pub struct FileNode {
    /// File name (e.g. `circle.cpp`).
    pub name: String,
    /// Symbols defined in this file.
    pub symbols: Vec<(SymbolId, SymbolKind, String)>,
}

/// The complete file tree for a project.
pub struct FileTree {
    /// Root-level directories.
    pub roots: Vec<DirNode>,
    /// Root-level files (files with no directory component).
    pub root_files: Vec<FileNode>,
    /// Symbols with no location (external/stdlib).
    pub external_symbols: Vec<(SymbolId, SymbolKind, String)>,
    /// Whether external/stdlib symbols are visible.
    pub externals_visible: bool,
}

impl FileTree {
    /// Build a file tree from a graph's symbols and file table.
    pub fn from_graph(graph: &Graph) -> Self {
        // Group symbols by their file path.
        let mut file_symbols: HashMap<String, Vec<(SymbolId, SymbolKind, String)>> = HashMap::new();
        let mut external_symbols: Vec<(SymbolId, SymbolKind, String)> = Vec::new();

        for (_, sym) in &graph.symbols {
            let Some(loc) = &sym.location else {
                external_symbols.push((sym.id, sym.kind, sym.name.clone()));
                continue;
            };
            let Some(path_str) = graph.files.resolve(loc.file) else {
                external_symbols.push((sym.id, sym.kind, sym.name.clone()));
                continue;
            };
            // External (absolute path) symbols.
            if path_str.starts_with('/') {
                external_symbols.push((sym.id, sym.kind, sym.name.clone()));
                continue;
            }
            file_symbols.entry(path_str.to_owned()).or_default().push((
                sym.id,
                sym.kind,
                sym.name.clone(),
            ));
        }

        // Build a temporary nested map: path components → symbols.
        let mut builder = DirBuilder::default();
        for (path, symbols) in file_symbols {
            builder.insert(&path, symbols);
        }

        let (roots, root_files) = builder.build();

        Self {
            roots,
            root_files,
            external_symbols,
            externals_visible: false,
        }
    }

    /// Collect all hidden `SymbolId`s based on the current visibility toggles.
    pub fn hidden_symbols(&self) -> HashSet<SymbolId> {
        let mut hidden = HashSet::new();
        if !self.externals_visible {
            for &(id, _, _) in &self.external_symbols {
                hidden.insert(id);
            }
        }
        for dir in &self.roots {
            collect_hidden_dir(dir, &mut hidden);
        }
        hidden
    }

    /// Apply saved directory visibility. Keys are slash-joined directory paths
    /// (e.g. `"shapes"`, `"shapes/internals"`). Directories not present in the
    /// map keep their default (visible). Stale paths are silently ignored.
    pub fn apply_visibility(&mut self, saved: &HashMap<String, bool>) {
        for dir in &mut self.roots {
            apply_visibility_recursive(dir, "", saved);
        }
    }

    /// Export current directory visibility as a path-keyed map.
    /// Only records directories that are hidden (to keep the file small).
    pub fn visibility_map(&self) -> HashMap<String, bool> {
        let mut map = HashMap::new();
        for dir in &self.roots {
            collect_visibility_recursive(dir, "", &mut map);
        }
        map
    }
}

fn collect_hidden_dir(dir: &DirNode, hidden: &mut HashSet<SymbolId>) {
    if !dir.visible {
        // Everything under this dir is hidden.
        collect_all_dir(dir, hidden);
        return;
    }
    for child in &dir.children_dirs {
        collect_hidden_dir(child, hidden);
    }
}

fn collect_all_dir(dir: &DirNode, hidden: &mut HashSet<SymbolId>) {
    for file in &dir.files {
        for &(id, _, _) in &file.symbols {
            hidden.insert(id);
        }
    }
    for child in &dir.children_dirs {
        collect_all_dir(child, hidden);
    }
}

/// Intermediate builder for constructing the directory tree.
#[derive(Default)]
struct DirBuilder {
    /// Subdirectories keyed by name.
    children: HashMap<String, DirBuilder>,
    /// Files at this level: filename → symbols.
    files: HashMap<String, Vec<(SymbolId, SymbolKind, String)>>,
}

impl DirBuilder {
    fn insert(&mut self, path: &str, symbols: Vec<(SymbolId, SymbolKind, String)>) {
        let components: Vec<&str> = path.split('/').collect();
        if components.len() == 1 {
            // Just a filename, no directory.
            self.files
                .entry(components[0].to_owned())
                .or_default()
                .extend(symbols);
            return;
        }
        // Navigate to the right directory.
        let dir_parts = &components[..components.len() - 1];
        let filename = components[components.len() - 1];

        let mut current = self;
        for &part in dir_parts {
            current = current.children.entry(part.to_owned()).or_default();
        }
        current
            .files
            .entry(filename.to_owned())
            .or_default()
            .extend(symbols);
    }

    fn build(self) -> (Vec<DirNode>, Vec<FileNode>) {
        let mut dirs: Vec<DirNode> = self
            .children
            .into_iter()
            .map(|(name, builder)| {
                let (children_dirs, files) = builder.build();
                DirNode {
                    name,
                    visible: true,
                    children_dirs,
                    files,
                }
            })
            .collect();
        dirs.sort_by(|a, b| a.name.cmp(&b.name));

        let mut files: Vec<FileNode> = self
            .files
            .into_iter()
            .map(|(name, mut symbols)| {
                symbols.sort_by(|a, b| a.2.cmp(&b.2));
                FileNode { name, symbols }
            })
            .collect();
        files.sort_by(|a, b| a.name.cmp(&b.name));

        (dirs, files)
    }
}

/// Build a summary string for a file node, e.g. "circle.cpp (1 class, 3 methods)".
pub fn file_summary(file: &FileNode) -> String {
    let mut classes = 0u32;
    let mut methods = 0u32;
    let mut functions = 0u32;
    let mut fields = 0u32;
    let mut namespaces = 0u32;
    for &(_, kind, _) in &file.symbols {
        match kind {
            SymbolKind::Class | SymbolKind::Struct => classes += 1,
            SymbolKind::Method => methods += 1,
            SymbolKind::Function => functions += 1,
            SymbolKind::Field => fields += 1,
            SymbolKind::Namespace => namespaces += 1,
            _ => {}
        }
    }

    let mut parts = Vec::new();
    if namespaces > 0 {
        parts.push(format!("{namespaces} ns"));
    }
    if classes > 0 {
        parts.push(format!(
            "{classes} {}",
            if classes == 1 { "class" } else { "classes" }
        ));
    }
    if methods > 0 {
        parts.push(format!(
            "{methods} {}",
            if methods == 1 { "method" } else { "methods" }
        ));
    }
    if functions > 0 {
        parts.push(format!(
            "{functions} {}",
            if functions == 1 { "fn" } else { "fns" }
        ));
    }
    if fields > 0 {
        parts.push(format!(
            "{fields} {}",
            if fields == 1 { "field" } else { "fields" }
        ));
    }

    if parts.is_empty() {
        format!("{} ({} symbols)", file.name, file.symbols.len())
    } else {
        format!("{} ({})", file.name, parts.join(", "))
    }
}

fn apply_visibility_recursive(dir: &mut DirNode, parent_path: &str, saved: &HashMap<String, bool>) {
    let path = if parent_path.is_empty() {
        dir.name.clone()
    } else {
        format!("{parent_path}/{}", dir.name)
    };
    if let Some(&vis) = saved.get(&path) {
        dir.visible = vis;
    }
    for child in &mut dir.children_dirs {
        apply_visibility_recursive(child, &path, saved);
    }
}

fn collect_visibility_recursive(dir: &DirNode, parent_path: &str, map: &mut HashMap<String, bool>) {
    let path = if parent_path.is_empty() {
        dir.name.clone()
    } else {
        format!("{parent_path}/{}", dir.name)
    };
    // Only store non-default (hidden) directories to keep the file compact.
    if !dir.visible {
        map.insert(path.clone(), false);
    }
    for child in &dir.children_dirs {
        collect_visibility_recursive(child, &path, map);
    }
}
