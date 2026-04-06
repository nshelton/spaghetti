//! `frontend-clang` — libclang-based C++ indexer that produces a [`core_ir::Graph`].
//!
//! Entry point: [`index_project`]. Reads a `compile_commands.json` and drives
//! libclang to extract classes, methods, and their relationships.
//!
//! # Dependencies
//!
//! Requires libclang installed on the system:
//! - Linux: `sudo apt install libclang-dev`
//! - macOS: `brew install llvm && export LIBCLANG_PATH=$(brew --prefix llvm)/lib`
//! - Windows: install LLVM prebuilt, set `LIBCLANG_PATH`

use std::path::Path;

use core_ir::{
    Edge, EdgeKind, Graph, Location, Symbol, SymbolId, SymbolKind,
};
// Note: libclang only allows one `Clang` instance per process, so we index
// translation units sequentially rather than with rayon.
use thiserror::Error;
use tracing::{debug, info, warn};

/// Errors from the clang frontend.
#[derive(Debug, Error)]
pub enum ClangError {
    /// Failed to read compile_commands.json.
    #[error("failed to read compile_commands.json: {0}")]
    Io(#[from] std::io::Error),

    /// Failed to parse compile_commands.json.
    #[error("failed to parse compile_commands.json: {0}")]
    Json(#[from] serde_json::Error),

    /// libclang failed to parse a translation unit.
    #[error("libclang failed to parse translation unit: {0}")]
    Parse(String),
}

/// A single entry from `compile_commands.json`.
#[derive(serde::Deserialize)]
struct CompileCommand {
    directory: String,
    command: Option<String>,
    arguments: Option<Vec<String>>,
    file: String,
}

/// Index a C++ project from its `compile_commands.json`, returning a unified [`Graph`].
///
/// Parallelizes per translation unit using rayon. Each TU produces a partial
/// graph that is merged at the end.
pub fn index_project(compile_commands: &Path) -> Result<Graph, ClangError> {
    let contents = std::fs::read_to_string(compile_commands)?;
    let commands: Vec<CompileCommand> = serde_json::from_str(&contents)?;

    // Canonicalize the compile_commands.json parent so we can resolve relative
    // `directory` entries against it as a fallback.
    let cc_parent = compile_commands
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .canonicalize()
        .unwrap_or_else(|_| compile_commands.parent().unwrap_or(Path::new(".")).to_path_buf());

    info!(
        entries = commands.len(),
        "indexing project from compile_commands.json"
    );

    let partial_graphs: Vec<Graph> = commands
        .iter()
        .filter_map(|cmd| {
            // `directory` in compile_commands.json is the build working
            // directory. Try as-is first (absolute or relative to CWD), then
            // fall back to resolving relative to the compile_commands.json location.
            let dir_path = std::path::PathBuf::from(&cmd.directory);
            let work_dir = if dir_path.is_absolute() {
                dir_path
            } else {
                dir_path.canonicalize()
                    .or_else(|_| cc_parent.join(&cmd.directory).canonicalize())
                    .unwrap_or_else(|_| cc_parent.join(&cmd.directory))
            };
            let file_path = work_dir.join(&cmd.file);
            match index_translation_unit(cmd, &work_dir) {
                Ok(g) => {
                    debug!(file = %file_path.display(), symbols = g.symbol_count(), "indexed TU");
                    Some(g)
                }
                Err(e) => {
                    warn!(file = %file_path.display(), error = %e, "failed to index TU");
                    None
                }
            }
        })
        .collect();

    let mut graph = Graph::new();
    for g in partial_graphs {
        graph.merge(g);
    }

    info!(
        symbols = graph.symbol_count(),
        edges = graph.edge_count(),
        "indexing complete"
    );
    Ok(graph)
}

/// Index a single translation unit.
fn index_translation_unit(cmd: &CompileCommand, work_dir: &Path) -> Result<Graph, ClangError> {
    let clang = clang::Clang::new().map_err(|e| ClangError::Parse(e.to_string()))?;
    let index = clang::Index::new(&clang, false, true);

    let file_path = work_dir.join(&cmd.file);

    // Extract compiler arguments
    let args = if let Some(arguments) = &cmd.arguments {
        arguments[1..].to_vec() // skip the compiler path
    } else if let Some(command) = &cmd.command {
        let parts: Vec<&str> = command.split_whitespace().collect();
        parts[1..].iter().map(|s| s.to_string()).collect()
    } else {
        vec![]
    };

    // Filter out flags that conflict with libclang's parser (-o, -c, and the
    // source file itself). Keep everything else (-I, -D, -std, -W, etc.).
    let mut filtered_args = Vec::new();
    let mut skip_next = false;
    for arg in &args {
        if skip_next {
            skip_next = false;
            continue;
        }
        if arg == "-o" || arg == "-c" {
            skip_next = true;
            continue;
        }
        // Skip the source file path (clang parser gets it separately)
        if arg == &cmd.file || arg.ends_with(".cpp") || arg.ends_with(".c") {
            continue;
        }
        // Resolve relative -I paths to absolute
        if arg.starts_with("-I") && !arg.starts_with("-I/") {
            let include_path = work_dir.join(&arg[2..]);
            filtered_args.push(format!("-I{}", include_path.display()));
        } else {
            filtered_args.push(arg.clone());
        }
    }

    let tu = index
        .parser(file_path.to_str().unwrap_or(""))
        .arguments(&filtered_args.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        .parse()
        .map_err(|e| ClangError::Parse(format!("{:?}", e)))?;

    let mut graph = Graph::new();
    visit_cursor(&tu.get_entity(), &mut graph, work_dir);
    Ok(graph)
}

/// Recursively visit AST nodes and populate the graph.
fn visit_cursor(cursor: &clang::Entity, graph: &mut Graph, base_dir: &Path) {
    use clang::EntityKind;

    match cursor.get_kind() {
        EntityKind::ClassDecl | EntityKind::StructDecl => {
            if cursor.is_definition() {
                if let Some(name) = cursor.get_name() {
                    let qualified = qualified_name(cursor);
                    // Treat structs as classes for now (v0 scope)
                    let kind = SymbolKind::Class;
                    let location = cursor_location(cursor, graph, base_dir);
                    let id = SymbolId::from_parts(&qualified, kind);

                    graph.add_symbol(Symbol {
                        id,
                        kind,
                        name,
                        qualified_name: qualified,
                        location,
                        module: None,
                        attrs: Default::default(),
                    });

                    // Check for base classes (inheritance)
                    for child in cursor.get_children() {
                        if child.get_kind() == EntityKind::BaseSpecifier {
                            if let Some(base_type) = child.get_type() {
                                if let Some(base_decl) = base_type.get_declaration() {
                                    if let Some(base_name) = base_decl.get_name() {
                                        let base_qualified = qualified_name(&base_decl);
                                        let base_kind = SymbolKind::Class;
                                        let base_id =
                                            SymbolId::from_parts(&base_qualified, base_kind);

                                        // Ensure base symbol exists
                                        if !graph.symbols.contains_key(&base_id) {
                                            let base_loc =
                                                cursor_location(&base_decl, graph, base_dir);
                                            graph.add_symbol(Symbol {
                                                id: base_id,
                                                kind: base_kind,
                                                name: base_name,
                                                qualified_name: base_qualified,
                                                location: base_loc,
                                                module: None,
                                                attrs: Default::default(),
                                            });
                                        }

                                        graph.add_edge(Edge {
                                            from: id,
                                            to: base_id,
                                            kind: EdgeKind::Inherits,
                                            location: None,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        EntityKind::Method | EntityKind::FunctionDecl => {
            if let Some(name) = cursor.get_name() {
                let qualified = qualified_name(cursor);
                let kind = if cursor.get_kind() == EntityKind::Method {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                let location = cursor_location(cursor, graph, base_dir);
                let id = SymbolId::from_parts(&qualified, kind);

                graph.add_symbol(Symbol {
                    id,
                    kind,
                    name,
                    qualified_name: qualified,
                    location,
                    module: None,
                    attrs: Default::default(),
                });

                // Check for call expressions within this function/method
                if cursor.is_definition() {
                    visit_calls(cursor, id, graph, base_dir);
                }
            }
        }

        // TODO: Handle TemplateInstantiation, Field, Namespace, etc.
        _ => {}
    }

    // Recurse into children
    for child in cursor.get_children() {
        visit_cursor(&child, graph, base_dir);
    }
}

/// Visit call expressions within a function body.
fn visit_calls(
    cursor: &clang::Entity,
    caller_id: SymbolId,
    graph: &mut Graph,
    base_dir: &Path,
) {
    use clang::EntityKind;

    for child in cursor.get_children() {
        if child.get_kind() == EntityKind::CallExpr {
            if let Some(referenced) = child.get_reference() {
                if let Some(ref_name) = referenced.get_name() {
                    let ref_qualified = qualified_name(&referenced);
                    let ref_kind = if referenced.get_kind() == EntityKind::Method {
                        SymbolKind::Method
                    } else {
                        SymbolKind::Function
                    };
                    let callee_id = SymbolId::from_parts(&ref_qualified, ref_kind);

                    let call_loc = cursor_location(&child, graph, base_dir);
                    graph.add_edge(Edge {
                        from: caller_id,
                        to: callee_id,
                        kind: EdgeKind::Calls,
                        location: call_loc,
                    });

                    // Ensure callee symbol exists
                    if !graph.symbols.contains_key(&callee_id) {
                        let ref_loc = cursor_location(&referenced, graph, base_dir);
                        graph.add_symbol(Symbol {
                            id: callee_id,
                            kind: ref_kind,
                            name: ref_name,
                            qualified_name: ref_qualified,
                            location: ref_loc,
                            module: None,
                            attrs: Default::default(),
                        });
                    }
                }
            }
        }
        // Recurse to find nested calls
        visit_calls(&child, caller_id, graph, base_dir);
    }
}

/// Build a qualified name from a cursor's semantic parent chain.
fn qualified_name(cursor: &clang::Entity) -> String {
    let mut parts = Vec::new();
    if let Some(name) = cursor.get_name() {
        parts.push(name);
    }
    let mut parent = cursor.get_semantic_parent();
    while let Some(p) = parent {
        use clang::EntityKind;
        match p.get_kind() {
            EntityKind::ClassDecl
            | EntityKind::StructDecl
            | EntityKind::Namespace
            | EntityKind::ClassTemplate => {
                if let Some(name) = p.get_name() {
                    parts.push(name);
                }
            }
            _ => break,
        }
        parent = p.get_semantic_parent();
    }
    parts.reverse();
    parts.join("::")
}

/// Extract a source location from a cursor.
fn cursor_location(
    cursor: &clang::Entity,
    graph: &mut Graph,
    base_dir: &Path,
) -> Option<Location> {
    let loc = cursor.get_location()?;
    let file_loc = loc.get_file_location();

    let file_path = file_loc.file?.get_path();
    let path_str = file_path
        .strip_prefix(base_dir)
        .unwrap_or(&file_path)
        .to_string_lossy();

    let file_id = graph.files.intern(&path_str);
    Some(Location {
        file: file_id,
        line: file_loc.line,
        col: file_loc.column,
    })
}
