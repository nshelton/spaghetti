//! Core IR types: symbols, edges, and the graph container.

use std::collections::{HashMap, HashSet};

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
    ///
    /// # Stability guarantees
    ///
    /// The returned `u64` is **stable across runs, platforms, and Rust
    /// compiler versions** for a given `(qualified_name, kind)` pair.
    ///
    /// * **Hash function:** seahash v4 (`seahash::hash`). This is a
    ///   portable, non-keyed hash whose output depends only on the input
    ///   bytes — it uses no randomness and no platform-specific intrinsics.
    /// * **Input normalization:** leading/trailing whitespace is trimmed and
    ///   interior runs of whitespace are collapsed to a single ASCII space
    ///   (`0x20`). This means `"Foo :: bar"` and `"Foo ::  bar"` produce
    ///   the same ID.
    /// * **Encoding:** the normalized name bytes are concatenated with a
    ///   `0xFF` separator and the `SymbolKind` discriminant as a
    ///   little-endian `u32`, so different kinds always hash differently
    ///   even for the same name.
    ///
    /// Changing any of the above (hash algorithm, normalization rules, or
    /// encoding) is a **breaking change** that invalidates persisted IDs and
    /// golden-file tests.
    pub fn from_parts(qualified_name: &str, kind: SymbolKind) -> Self {
        use seahash::hash;
        let normalized = normalize_name(qualified_name);
        let mut buf = normalized.as_bytes().to_vec();
        buf.push(0xFF);
        buf.extend_from_slice(&(kind as u32).to_le_bytes());
        Self(hash(&buf))
    }
}

/// Trim leading/trailing whitespace and collapse interior runs of whitespace
/// to a single ASCII space.
fn normalize_name(name: &str) -> String {
    let mut result = String::with_capacity(name.len());
    let mut prev_was_space = true; // true to trim leading whitespace
    for ch in name.chars() {
        if ch.is_whitespace() {
            if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            result.push(ch);
            prev_was_space = false;
        }
    }
    // Trim trailing space if present
    if result.ends_with(' ') {
        result.pop();
    }
    result
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
    /// Symbol is external to the project (e.g. from a system header).
    External,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl Edge {
    /// The identity of an edge for deduplication purposes: `(from, to, kind)`.
    ///
    /// Two edges are considered duplicates when they share the same source,
    /// target, and kind — regardless of location. During a merge the first
    /// occurrence is kept and later duplicates are discarded.
    fn dedup_key(&self) -> (SymbolId, SymbolId, EdgeKind) {
        (self.from, self.to, self.kind)
    }
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

    /// Returns `true` if the symbol appears to be external (stdlib / system
    /// header).
    ///
    /// The clang frontend strips the project root from file paths, so
    /// project-local files have relative paths while external files retain
    /// their absolute paths. A symbol with no location at all is also
    /// considered external.
    pub fn is_external(&self, id: SymbolId) -> bool {
        match self.symbols.get(&id) {
            Some(sym) => {
                if sym.attrs.contains(&Attr::External) {
                    return true;
                }
                match &sym.location {
                    Some(loc) => {
                        let path = self.files.resolve(loc.file).unwrap_or("");
                        path.starts_with('/')
                    }
                    None => true,
                }
            }
            None => true,
        }
    }

    /// Iterate over neighbor symbol IDs connected to `id` via edges whose
    /// kind is in `kinds`. If `kinds` is empty, all edge kinds match.
    ///
    /// # Direction semantics
    ///
    /// Traversal is **bidirectional**: a neighbor is any node connected to
    /// `id` by an edge where `id` appears as either the source (`from`) or
    /// the target (`to`). This means `neighbors(A, &[])` returns B if there
    /// is an edge `A → B` *or* `B → A`.
    ///
    /// # Unknown nodes
    ///
    /// If `id` does not exist in the graph, the iterator yields no items
    /// (it does **not** return an error).
    ///
    /// # Duplicates
    ///
    /// If multiple edges connect the same pair of nodes (possibly with
    /// different kinds), the neighbor's ID appears once per matching edge.
    /// Callers that need a unique set should collect into a `HashSet`.
    ///
    /// # Self-loops
    ///
    /// A self-loop (an edge where `from == to == id`) yields the node
    /// exactly once per matching edge, not twice.
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

    /// Merge another graph into this one.
    ///
    /// # Merge semantics
    ///
    /// **Strategy: last wins.** When `other` contains a symbol whose
    /// [`SymbolId`] already exists in `self`, the incoming symbol
    /// *replaces* the existing one. This is the simplest deterministic
    /// policy and matches the natural expectation that a later indexing
    /// pass produces more-up-to-date data.
    ///
    /// **File-table remapping.** Each [`FileId`] in `other` is remapped
    /// to the corresponding entry in `self.files` (interning the path if
    /// it is new). All [`Location`] fields on symbols and edges are
    /// rewritten with the remapped IDs before insertion.
    ///
    /// **Edge deduplication.** An edge from `other` is only inserted if
    /// no edge with the same `(from, to, kind)` triple already exists in
    /// `self`. Location is *not* considered part of edge identity — only
    /// the structural triple matters. The first occurrence wins (i.e.
    /// the edge already present in `self` is kept).
    pub fn merge(&mut self, other: Graph) {
        // 1. Remap file IDs from `other` into `self.files`.
        let mut file_remap: HashMap<FileId, FileId> = HashMap::new();
        for (old_id_idx, path) in other.files.paths.iter().enumerate() {
            let old_id = FileId(old_id_idx as u32);
            let new_id = self.files.intern(path);
            file_remap.insert(old_id, new_id);
        }

        // 2. Merge symbols — last wins on conflict.
        for (_, mut sym) in other.symbols {
            if let Some(loc) = &mut sym.location {
                if let Some(&new_fid) = file_remap.get(&loc.file) {
                    loc.file = new_fid;
                }
            }
            self.symbols.insert(sym.id, sym);
        }

        // 3. Build a set of existing edge identity keys for O(1) lookup.
        let mut existing_edges: HashSet<(SymbolId, SymbolId, EdgeKind)> =
            self.edges.iter().map(|e| e.dedup_key()).collect();

        // 4. Merge edges — skip duplicates.
        for mut edge in other.edges {
            if let Some(loc) = &mut edge.location {
                if let Some(&new_fid) = file_remap.get(&loc.file) {
                    loc.file = new_fid;
                }
            }
            let key = edge.dedup_key();
            if existing_edges.insert(key) {
                self.edges.push(edge);
            }
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
