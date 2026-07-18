//! `frontend-ispc` — tree-sitter-based ISPC indexer that produces a [`core_ir::Graph`].
//!
//! Entry point: [`index_project`]. Reads a `compile_commands.json`, takes the
//! `.ispc` entries (which `frontend-clang` skips), and extracts functions,
//! structs, fields, call edges, and include edges. `.isph` headers reached
//! through quoted includes are parsed transitively, since that is where most
//! ISPC structs and inline kernels live.
//!
//! This is a syntax-level indexer: there is no preprocessor and no type
//! resolution. Call edges are matched by callee name, and `#include` edges
//! are only emitted for quoted includes that resolve relative to the
//! including file.

use std::path::{Path, PathBuf};

use core_ir::{Edge, EdgeKind, Graph, Location, Symbol, SymbolId, SymbolKind};
use thiserror::Error;
use tracing::{info, warn};
use tree_sitter::{Node, Parser};

extern "C" {
    fn tree_sitter_ispc() -> *const tree_sitter::ffi::TSLanguage;
}

/// The tree-sitter language for ISPC (vendored grammar, see `grammar/`).
fn language() -> tree_sitter::Language {
    unsafe { tree_sitter::Language::from_raw(tree_sitter_ispc()) }
}

/// Errors from the ISPC frontend.
#[derive(Debug, Error)]
pub enum IspcError {
    /// Failed to read compile_commands.json.
    #[error("failed to read compile_commands.json: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to parse compile_commands.json.
    #[error("failed to parse compile_commands.json: {0}")]
    Json(#[from] serde_json::Error),

    /// tree-sitter failed to load the ISPC grammar.
    #[error("tree-sitter error: {0}")]
    Parse(String),
}

/// A single entry from `compile_commands.json`. Only the fields the ISPC
/// frontend needs — flags are irrelevant since there is no preprocessor.
#[derive(serde::Deserialize)]
struct CompileCommand {
    directory: String,
    file: String,
}

/// Index the `.ispc` entries of a `compile_commands.json`, returning a [`Graph`].
///
/// Returns an empty graph if the database contains no ISPC entries.
pub fn index_project(compile_commands: &Path) -> Result<Graph, IspcError> {
    index_project_with_progress(compile_commands, |_, _, _| true)
}

/// Index with per-file progress reporting.
///
/// The `on_progress` callback is called before each file with
/// `(current_index, total_count, file_name)`. Return `false` to cancel.
pub fn index_project_with_progress(
    compile_commands: &Path,
    mut on_progress: impl FnMut(usize, usize, &str) -> bool,
) -> Result<Graph, IspcError> {
    let contents = std::fs::read_to_string(compile_commands)?;
    let commands: Vec<CompileCommand> = serde_json::from_str(&contents)?;

    let cc_parent_raw = compile_commands.parent().unwrap_or_else(|| Path::new("."));
    let cc_parent = cc_parent_raw
        .canonicalize()
        .unwrap_or_else(|_| cc_parent_raw.to_path_buf());

    // Computed over ALL entries, exactly like frontend-clang, so relative
    // file paths and TranslationUnit symbol ids line up when the two
    // frontends' graphs are merged.
    let project_root = compute_project_root(&commands, &cc_parent);

    let mut missing = 0u32;
    let ispc_files: Vec<PathBuf> = commands
        .iter()
        .filter_map(|cmd| {
            let dir = Path::new(&cmd.directory);
            let work_dir = if dir.is_absolute() {
                dir.to_path_buf()
            } else {
                cc_parent.join(dir)
            };
            let path = work_dir.join(&cmd.file);
            let is_ispc = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("ispc"));
            if !is_ispc {
                return None;
            }
            if !path.is_file() {
                missing += 1;
                return None;
            }
            Some(path)
        })
        .collect();

    let total = ispc_files.len();
    if total == 0 {
        return Ok(Graph::new());
    }
    info!(
        entries = total,
        missing,
        project_root = %project_root.display(),
        "indexing ISPC translation units"
    );

    let mut parser = Parser::new();
    parser
        .set_language(&language())
        .map_err(|e| IspcError::Parse(e.to_string()))?;

    let mut graph = Graph::new();
    let mut pending_headers: Vec<PathBuf> = Vec::new();
    for (i, file) in ispc_files.iter().enumerate() {
        let name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if !on_progress(i, total, &name) {
            info!("ISPC indexing cancelled at {}/{}", i, total);
            break;
        }
        parse_into(
            &mut parser,
            file,
            &project_root,
            &mut graph,
            &mut pending_headers,
        );
    }

    // Transitively parse .isph/.ispc files reached via quoted includes —
    // headers hold most ISPC structs and inline functions. Other included
    // file types (shared C headers) get Includes edges but are not parsed.
    let mut visited: std::collections::HashSet<PathBuf> = ispc_files.into_iter().collect();
    let mut headers = 0u32;
    while let Some(header) = pending_headers.pop() {
        let is_ispc_source = header
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("isph") || e.eq_ignore_ascii_case("ispc"));
        if !is_ispc_source || !visited.insert(header.clone()) {
            continue;
        }
        headers += 1;
        parse_into(
            &mut parser,
            &header,
            &project_root,
            &mut graph,
            &mut pending_headers,
        );
    }

    info!(
        headers,
        symbols = graph.symbol_count(),
        edges = graph.edge_count(),
        "ISPC indexing complete"
    );
    Ok(graph)
}

/// Read and parse one ISPC file into `graph`, collecting resolved quoted
/// includes into `pending`.
fn parse_into(
    parser: &mut Parser,
    file: &Path,
    project_root: &Path,
    graph: &mut Graph,
    pending: &mut Vec<PathBuf>,
) {
    let src = match std::fs::read(file) {
        Ok(s) => s,
        Err(e) => {
            warn!(file = %file.display(), error = %e, "failed to read ISPC file");
            return;
        }
    };
    match parser.parse(&src, None) {
        Some(tree) => index_file(file, &src, tree.root_node(), project_root, graph, pending),
        None => warn!(file = %file.display(), "tree-sitter failed to parse ISPC file"),
    }
}

/// Extract symbols and edges from one parsed ISPC file into `graph`,
/// pushing resolved quoted includes onto `pending`.
fn index_file(
    path: &Path,
    src: &[u8],
    root: Node,
    project_root: &Path,
    graph: &mut Graph,
    pending: &mut Vec<PathBuf>,
) {
    let rel = path
        .strip_prefix(project_root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned();
    let tu_id = SymbolId::from_parts(&rel, SymbolKind::TranslationUnit);
    ensure_tu_symbol(graph, tu_id, &rel);

    let ctx = FileCtx {
        src,
        rel: &rel,
        abs: path,
        tu_id,
        project_root,
    };
    visit(root, &ctx, graph, pending);
}

/// Per-file context threaded through the visitors.
struct FileCtx<'a> {
    src: &'a [u8],
    rel: &'a str,
    abs: &'a Path,
    tu_id: SymbolId,
    project_root: &'a Path,
}

/// Walk top-level (and preprocessor-nested) declarations.
fn visit(node: Node, ctx: &FileCtx, graph: &mut Graph, pending: &mut Vec<PathBuf>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_definition" => emit_function(child, ctx, graph),
            "struct_specifier" => emit_struct(child, ctx, graph),
            "preproc_include" => emit_include(child, ctx, graph, pending),
            _ => visit(child, ctx, graph, pending),
        }
    }
}

/// Emit a Function symbol, a Contains edge from the TU, and Calls edges
/// from its body.
fn emit_function(def: Node, ctx: &FileCtx, graph: &mut Graph) {
    let Some(declarator) = def.child_by_field_name("declarator") else {
        return;
    };
    let Some(fn_decl) = find_kind(declarator, "function_declarator") else {
        return;
    };
    let Some(name_node) = fn_decl
        .child_by_field_name("declarator")
        .and_then(|d| find_kind(d, "identifier"))
    else {
        return;
    };
    let name = node_text(name_node, ctx.src);
    let id = SymbolId::from_parts(&name, SymbolKind::Function);
    let location = node_location(def, ctx.rel, graph);

    graph.add_symbol(Symbol {
        id,
        kind: SymbolKind::Function,
        name: name.clone(),
        qualified_name: name,
        location: Some(location),
        module: None,
        attrs: Default::default(),
    });
    graph.add_edge(Edge {
        from: ctx.tu_id,
        to: id,
        kind: EdgeKind::Contains,
        location: None,
    });

    if let Some(body) = def.child_by_field_name("body") {
        visit_calls(body, id, ctx, graph);
    }
}

/// Recursively emit Calls edges for every `call_expression` in a function body.
fn visit_calls(node: Node, caller: SymbolId, ctx: &FileCtx, graph: &mut Graph) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(f) = child.child_by_field_name("function") {
                if f.kind() == "identifier" {
                    let name = node_text(f, ctx.src);
                    let callee = SymbolId::from_parts(&name, SymbolKind::Function);
                    let location = node_location(child, ctx.rel, graph);
                    graph.add_edge(Edge {
                        from: caller,
                        to: callee,
                        kind: EdgeKind::Calls,
                        location: Some(location),
                    });
                    if !graph.symbols.contains_key(&callee) {
                        graph.add_symbol(Symbol {
                            id: callee,
                            kind: SymbolKind::Function,
                            name: name.clone(),
                            qualified_name: name,
                            location: None,
                            module: None,
                            attrs: Default::default(),
                        });
                    }
                }
            }
        }
        visit_calls(child, caller, ctx, graph);
    }
}

/// Emit a Struct symbol with its Field children. Only definitions (with a
/// body) are emitted; bare `struct Foo x;` references are ignored.
fn emit_struct(spec: Node, ctx: &FileCtx, graph: &mut Graph) {
    let Some(name_node) = spec.child_by_field_name("name") else {
        return;
    };
    let Some(body) = spec.child_by_field_name("body") else {
        return;
    };
    let sname = node_text(name_node, ctx.src);
    let sid = SymbolId::from_parts(&sname, SymbolKind::Struct);
    let location = node_location(spec, ctx.rel, graph);

    graph.add_symbol(Symbol {
        id: sid,
        kind: SymbolKind::Struct,
        name: sname.clone(),
        qualified_name: sname.clone(),
        location: Some(location),
        module: None,
        attrs: Default::default(),
    });
    graph.add_edge(Edge {
        from: ctx.tu_id,
        to: sid,
        kind: EdgeKind::Contains,
        location: None,
    });

    let mut cursor = body.walk();
    for field_decl in body.named_children(&mut cursor) {
        if field_decl.kind() != "field_declaration" {
            continue;
        }
        // A declaration can name several fields: `float a, b;`
        collect_field_identifiers(field_decl, ctx, graph, &sname, sid);
    }
}

/// Emit a Field symbol + Contains edge for each `field_identifier` in a
/// field declaration.
fn collect_field_identifiers(
    node: Node,
    ctx: &FileCtx,
    graph: &mut Graph,
    struct_name: &str,
    struct_id: SymbolId,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "field_identifier" {
            let fname = node_text(child, ctx.src);
            let qualified = format!("{struct_name}::{fname}");
            let fid = SymbolId::from_parts(&qualified, SymbolKind::Field);
            let location = node_location(child, ctx.rel, graph);
            graph.add_symbol(Symbol {
                id: fid,
                kind: SymbolKind::Field,
                name: fname,
                qualified_name: qualified,
                location: Some(location),
                module: None,
                attrs: Default::default(),
            });
            graph.add_edge(Edge {
                from: struct_id,
                to: fid,
                kind: EdgeKind::Contains,
                location: None,
            });
        } else {
            collect_field_identifiers(child, ctx, graph, struct_name, struct_id);
        }
    }
}

/// Emit an Includes edge for a quoted `#include "..."` that resolves next to
/// the including file, and queue the target for parsing. Angle includes are
/// skipped — resolving them would require the compiler's `-I` search path.
fn emit_include(inc: Node, ctx: &FileCtx, graph: &mut Graph, pending: &mut Vec<PathBuf>) {
    let Some(path_node) = inc.child_by_field_name("path") else {
        return;
    };
    if path_node.kind() != "string_literal" {
        return;
    }
    let Some(content) = find_kind(path_node, "string_content") else {
        return;
    };
    let target = node_text(content, ctx.src);
    let Some(dir) = ctx.abs.parent() else {
        return;
    };
    let candidate = dir.join(&target);
    if !candidate.is_file() {
        return;
    }
    let resolved = candidate.canonicalize().unwrap_or(candidate);
    let inc_rel = resolved
        .strip_prefix(ctx.project_root)
        .unwrap_or(&resolved)
        .to_string_lossy();
    let inc_id = SymbolId::from_parts(&inc_rel, SymbolKind::TranslationUnit);
    ensure_tu_symbol(graph, inc_id, &inc_rel);
    graph.add_edge(Edge {
        from: ctx.tu_id,
        to: inc_id,
        kind: EdgeKind::Includes,
        location: None,
    });
    pending.push(resolved);
}

/// Depth-first search for the first descendant of the given kind
/// (including `node` itself).
fn find_kind<'t>(node: Node<'t>, kind: &str) -> Option<Node<'t>> {
    if node.kind() == kind {
        return Some(node);
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(found) = find_kind(child, kind) {
            return Some(found);
        }
    }
    None
}

/// Node source text (lossy on invalid UTF-8).
fn node_text(node: Node, src: &[u8]) -> String {
    String::from_utf8_lossy(&src[node.byte_range()]).into_owned()
}

/// 1-based location of a node within `rel` (interned into the graph).
fn node_location(node: Node, rel: &str, graph: &mut Graph) -> Location {
    let pos = node.start_position();
    Location {
        file: graph.files.intern(rel),
        line: pos.row as u32 + 1,
        col: pos.column as u32 + 1,
    }
}

/// Ensure a [`SymbolKind::TranslationUnit`] symbol exists in the graph.
fn ensure_tu_symbol(graph: &mut Graph, id: SymbolId, path: &str) {
    if !graph.symbols.contains_key(&id) {
        let name = Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string());
        graph.add_symbol(Symbol {
            id,
            kind: SymbolKind::TranslationUnit,
            name,
            qualified_name: path.to_string(),
            location: None,
            module: None,
            attrs: Default::default(),
        });
    }
}

/// Longest common ancestor of all entries, clamped to the
/// compile_commands.json directory. Mirrors frontend-clang so both
/// frontends agree on relative paths.
fn compute_project_root(commands: &[CompileCommand], cc_parent: &Path) -> PathBuf {
    let abs_paths: Vec<PathBuf> = commands
        .iter()
        .map(|cmd| {
            let dir = Path::new(&cmd.directory);
            let work_dir = if dir.is_absolute() {
                dir.to_path_buf()
            } else {
                cc_parent.join(dir)
            };
            let file_path = work_dir.join(&cmd.file);
            file_path.canonicalize().unwrap_or(file_path)
        })
        .collect();

    if abs_paths.is_empty() {
        return cc_parent.to_path_buf();
    }

    let mut prefix = abs_paths[0].clone();
    for path in &abs_paths[1..] {
        prefix = common_ancestor(&prefix, path);
    }
    if prefix.is_file() {
        prefix = prefix.parent().unwrap_or(cc_parent).to_path_buf();
    }
    let cc_abs = cc_parent
        .canonicalize()
        .unwrap_or_else(|_| cc_parent.to_path_buf());
    common_ancestor(&prefix, &cc_abs)
}

/// Longest common ancestor path of two paths.
fn common_ancestor(a: &Path, b: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for (ca, cb) in a.components().zip(b.components()) {
        if ca == cb {
            result.push(ca);
        } else {
            break;
        }
    }
    result
}
