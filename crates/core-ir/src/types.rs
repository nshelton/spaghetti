//! Core IR types: symbols, edges, and the graph container.

use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during graph operations.
#[derive(Debug, Error)]
pub enum GraphError {
    /// A referenced symbol was not found in the graph.
    #[error("symbol not found: {0:?}")]
    SymbolNotFound(SymbolId),

    /// JSON serialization/deserialization failed.
    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

// ---------------------------------------------------------------------------
// SymbolId
// ---------------------------------------------------------------------------

/// Opaque identifier for a symbol, derived from a deterministic hash of
/// `(qualified_name, kind)`.
///
// TODO: Long-term replacement — use Clang USRs for C++ symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SymbolId(pub u64);

impl SymbolId {
    /// Create a deterministic [`SymbolId`] from a qualified name and kind.
    pub fn from_parts(qualified_name: &str, kind: SymbolKind) -> Self {
        use seahash::hash;
        let mut buf = qualified_name.as_bytes().to_vec();
        buf.push(0xFF);
        buf.extend_from_slice(&(kind as u32).to_le_bytes());
        Self(hash(&buf))
    }
}

// ---------------------------------------------------------------------------
// FileId / FileTable
// ---------------------------------------------------------------------------

/// Interned file path identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FileId(pub u32);

/// String-interning table for file paths.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct FileTable {
    paths: Vec<String>,
    index: HashMap<String, FileId>,
}

impl FileTable {
    /// Intern a file path, returning its [`FileId`].
    pub fn intern(&mut self, path: &str) -> FileId {
        if let Some(&id) = self.index.get(path) {
            return id;
        }
        let id = FileId(self.paths.len() as u32);
        self.paths.push(path.to_owned());
        self.index.insert(path.to_owned(), id);
        id
    }

    /// Resolve a [`FileId`] back to a path string.
    pub fn resolve(&self, id: FileId) -> Option<&str> {
        self.paths.get(id.0 as usize).map(String::as_str)
    }

    /// Number of interned paths.
    pub fn len(&self) -> usize {
        self.paths.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Location
// ---------------------------------------------------------------------------

/// Source location within a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// The file this location refers to.
    pub file: FileId,
    /// 1-based line number.
    pub line: u32,
    /// 1-based column number.
    pub col: u32,
}

// ---------------------------------------------------------------------------
// SymbolKind
// ---------------------------------------------------------------------------

/// The kind of a symbol in the code graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[repr(u32)]
pub enum SymbolKind {
    /// A class declaration.
    Class = 0,
    /// A struct declaration.
    Struct = 1,
    /// A free function.
    Function = 2,
    /// A method (member function).
    Method = 3,
    /// A field (member variable).
    Field = 4,
    /// A namespace.
    Namespace = 5,
    /// A template instantiation.
    TemplateInstantiation = 6,
    /// A translation unit.
    TranslationUnit = 7,
}

// ---------------------------------------------------------------------------
// Attr
// ---------------------------------------------------------------------------

/// Arbitrary attribute attached to a symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attr {
    /// Symbol is virtual.
    Virtual,
    /// Symbol is abstract (pure virtual).
    Abstract,
    /// Symbol is static.
    Static,
    /// Symbol is const.
    Const,
    /// Custom attribute string.
    Custom(String),
}

// ---------------------------------------------------------------------------
// Symbol
// ---------------------------------------------------------------------------

/// A node in the code graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    /// Unique identifier.
    pub id: SymbolId,
    /// What kind of symbol this is.
    pub kind: SymbolKind,
    /// Short name (e.g. `area`).
    pub name: String,
    /// Fully qualified name (e.g. `Circle::area`).
    pub qualified_name: String,
    /// Where the symbol is defined.
    pub location: Option<Location>,
    /// Logical module or namespace grouping.
    pub module: Option<String>,
    /// Additional attributes.
    pub attrs: SmallVec<[Attr; 2]>,
}

// ---------------------------------------------------------------------------
// EdgeKind
// ---------------------------------------------------------------------------

/// The kind of relationship between two symbols.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum EdgeKind {
    /// Function/method calls another function/method.
    Calls,
    /// Class/struct inherits from another.
    Inherits,
    /// Symbol contains another (e.g. class contains method).
    Contains,
    /// Function reads a field.
    ReadsField,
    /// Function writes a field.
    WritesField,
    /// File includes another file.
    Includes,
    /// Template instantiation links to primary template.
    Instantiates,
    /// Symbol has a type relationship.
    HasType,
    /// Method overrides a virtual method.
    Overrides,
}

// ---------------------------------------------------------------------------
// Edge
// ---------------------------------------------------------------------------

/// A directed edge in the code graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    /// Source symbol.
    pub from: SymbolId,
    /// Target symbol.
    pub to: SymbolId,
    /// What kind of relationship this edge represents.
    pub kind: EdgeKind,
    /// Where the relationship occurs in source (e.g. the call site).
    pub location: Option<Location>,
}

// ---------------------------------------------------------------------------
// Graph
// ---------------------------------------------------------------------------

/// The central code graph: symbols (nodes) and edges (relationships).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Graph {
    /// Interned file paths.
    pub files: FileTable,
    /// All symbols, keyed by their ID.
    pub symbols: IndexMap<SymbolId, Symbol>,
    /// All edges.
    pub edges: Vec<Edge>,
}

impl Graph {
    /// Create an empty graph.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a symbol to the graph. Returns its [`SymbolId`].
    pub fn add_symbol(&mut self, symbol: Symbol) -> SymbolId {
        let id = symbol.id;
        self.symbols.insert(id, symbol);
        id
    }

    /// Add an edge to the graph.
    pub fn add_edge(&mut self, edge: Edge) {
        self.edges.push(edge);
    }

    /// Iterate over neighbor symbol IDs reachable from `id` via edges whose
    /// kind is in `kinds`. If `kinds` is empty, all edge kinds match.
    pub fn neighbors<'a>(
        &'a self,
        id: SymbolId,
        kinds: &'a [EdgeKind],
    ) -> impl Iterator<Item = SymbolId> + 'a {
        self.edges.iter().filter_map(move |e| {
            if e.from == id && (kinds.is_empty() || kinds.contains(&e.kind)) {
                Some(e.to)
            } else if e.to == id && (kinds.is_empty() || kinds.contains(&e.kind)) {
                Some(e.from)
            } else {
                None
            }
        })
    }

    /// Merge another graph into this one. Symbols with duplicate IDs are
    /// overwritten; edges are appended. File tables are merged.
    pub fn merge(&mut self, other: Graph) {
        // Remap file IDs from `other` into `self.files`.
        let mut file_remap: HashMap<FileId, FileId> = HashMap::new();
        for (old_id_idx, path) in other.files.paths.iter().enumerate() {
            let old_id = FileId(old_id_idx as u32);
            let new_id = self.files.intern(path);
            file_remap.insert(old_id, new_id);
        }

        for (_, mut sym) in other.symbols {
            if let Some(loc) = &mut sym.location {
                if let Some(&new_fid) = file_remap.get(&loc.file) {
                    loc.file = new_fid;
                }
            }
            self.symbols.insert(sym.id, sym);
        }

        for mut edge in other.edges {
            if let Some(loc) = &mut edge.location {
                if let Some(&new_fid) = file_remap.get(&loc.file) {
                    loc.file = new_fid;
                }
            }
            self.edges.push(edge);
        }
    }

    /// Serialize the graph to a JSON string.
    pub fn to_json(&self) -> Result<String, GraphError> {
        serde_json::to_string_pretty(self).map_err(GraphError::Serde)
    }

    /// Deserialize a graph from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, GraphError> {
        serde_json::from_str(json).map_err(GraphError::Serde)
    }

    /// Number of symbols in the graph.
    pub fn symbol_count(&self) -> usize {
        self.symbols.len()
    }

    /// Number of edges in the graph.
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }
}
