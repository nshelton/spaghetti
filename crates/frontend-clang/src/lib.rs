//! `frontend-clang` — libclang-based C++ indexer that produces a [`core_ir::Graph`].
//!
//! Entry point: [`index_project`]. Reads a `compile_commands.json` and drives
//! libclang to extract classes, structs, methods, fields, namespaces, template
//! instantiations, and their relationships.
//!
//! # Dependencies
//!
//! Requires libclang installed on the system:
//! - Linux: `sudo apt install libclang-dev`
//! - macOS: `brew install llvm && export LIBCLANG_PATH=$(brew --prefix llvm)/lib`
//! - Windows: install LLVM prebuilt, set `LIBCLANG_PATH`

use std::path::{Path, PathBuf};

use core_ir::{Edge, EdgeKind, Graph, Location, Symbol, SymbolId, SymbolKind};
// Note: libclang only allows one `Clang` instance per process, so we index
// translation units sequentially rather than with rayon.
use thiserror::Error;
use tracing::{debug, info, warn};

mod cache;

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
/// Each TU produces a partial graph that is merged at the end.
pub fn index_project(compile_commands: &Path) -> Result<Graph, ClangError> {
    index_project_with_progress(compile_commands, |_, _, _| true)
}

/// Index a C++ project with per-TU progress reporting.
///
/// The `on_progress` callback is called before each translation unit with
/// `(current_index, total_count, file_name)`. Return `true` to continue
/// indexing or `false` to cancel.
pub fn index_project_with_progress(
    compile_commands: &Path,
    mut on_progress: impl FnMut(usize, usize, &str) -> bool,
) -> Result<Graph, ClangError> {
    let contents = std::fs::read_to_string(compile_commands)?;
    let commands: Vec<CompileCommand> = serde_json::from_str(&contents)?;

    // Canonicalize the compile_commands.json path so we can resolve relative
    // `directory` entries. Per the JSON Compilation Database spec, `directory`
    // is the working directory of the compile command. In CMake output it is
    // always absolute; for hand-written files it may be relative to the
    // project root. We resolve relative to compile_commands.json's parent.
    let cc_parent_raw = compile_commands.parent().unwrap_or_else(|| Path::new("."));
    let cc_parent = &cc_parent_raw
        .canonicalize()
        .unwrap_or_else(|_| cc_parent_raw.to_path_buf());

    let project_root = compute_project_root(&commands, cc_parent);

    let total = commands.len();
    info!(
        entries = total,
        project_root = %project_root.display(),
        "indexing project from compile_commands.json"
    );

    // Set up per-TU cache.
    let cache_dir = cache::cache_dir(compile_commands);
    let cc_mtime = cache::file_mtime_secs(compile_commands);

    // Create a single Clang instance for the entire indexing run.
    // libclang only allows one `Clang` per process, and init/teardown is
    // expensive — hoisting it here avoids repeating that cost per TU.
    let clang = clang::Clang::new().map_err(|e| ClangError::Parse(e.to_string()))?;
    let index = clang::Index::new(&clang, false, true);

    let mut cache_hits = 0u32;
    let mut partial_graphs = Vec::with_capacity(total);

    for (i, cmd) in commands.iter().enumerate() {
        if !on_progress(i, total, &cmd.file) {
            info!("indexing cancelled at TU {}/{}", i, total);
            break;
        }

        // Per the spec, `directory` is the working directory for the
        // compile command. CMake always writes absolute paths. For
        // relative paths, resolve against the compile_commands.json
        // parent directory.
        let dir_path = Path::new(&cmd.directory);
        let work_dir = if dir_path.is_absolute() {
            dir_path.to_path_buf()
        } else {
            cc_parent.join(dir_path)
        };
        let file_path = work_dir.join(&cmd.file);

        // Extract args early so we can compute the cache key.
        let args = extract_args(cmd);

        let key = cache::cache_key(&file_path, &args, cc_mtime);

        // Try the cache first.
        if let Some(cached) = cache::load(&cache_dir, key) {
            debug!(file = %file_path.display(), symbols = cached.symbol_count(), "cached TU");
            cache_hits += 1;
            partial_graphs.push(cached);
            continue;
        }

        match index_translation_unit(cmd, &work_dir, &project_root, &index, &args) {
            Ok(g) => {
                debug!(file = %file_path.display(), symbols = g.symbol_count(), "indexed TU");
                // Only cache non-empty results — an empty graph likely means
                // indexing failed silently and we don't want to persist that.
                if g.symbol_count() > 0 {
                    cache::store(&cache_dir, key, &g);
                }
                partial_graphs.push(g);
            }
            Err(e) => {
                warn!(file = %file_path.display(), error = %e, "failed to index TU");
            }
        }
    }

    info!(cache_hits, total, "TU cache summary");

    let mut graph = Graph::new();
    for g in partial_graphs {
        graph.merge(g);
    }

    // Deduplicate edges (same headers are processed by multiple TUs)
    dedup_edges(&mut graph);

    info!(
        symbols = graph.symbol_count(),
        edges = graph.edge_count(),
        "indexing complete"
    );
    Ok(graph)
}

/// Extract raw compiler arguments from a compile command, skipping the compiler path.
fn extract_args(cmd: &CompileCommand) -> Vec<String> {
    if let Some(arguments) = &cmd.arguments {
        arguments.get(1..).unwrap_or_default().to_vec()
    } else if let Some(command) = &cmd.command {
        let parts: Vec<&str> = command.split_whitespace().collect();
        parts
            .get(1..)
            .unwrap_or_default()
            .iter()
            .map(|s| s.to_string())
            .collect()
    } else {
        vec![]
    }
}

/// Index a single translation unit.
fn index_translation_unit(
    cmd: &CompileCommand,
    work_dir: &Path,
    project_root: &Path,
    index: &clang::Index,
    args: &[String],
) -> Result<Graph, ClangError> {
    let file_path = work_dir.join(&cmd.file);

    // Filter out flags that conflict with libclang's parser (-o, -c, and the
    // source file itself). Keep everything else (-I, -D, -std, -W, etc.).
    let mut filtered_args = Vec::new();
    let mut skip_next = false;
    for arg in args {
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

    // Add system C++ include paths so libclang can resolve standard headers.
    for path in system_include_paths() {
        filtered_args.push(format!("-isystem{}", path));
    }

    let tu = index
        .parser(file_path.to_str().unwrap_or(""))
        .arguments(&filtered_args.iter().map(|s| s.as_str()).collect::<Vec<_>>())
        .parse()
        .map_err(|e| ClangError::Parse(format!("{:?}", e)))?;

    let mut graph = Graph::new();
    visit_cursor(&tu.get_entity(), &mut graph, project_root);
    Ok(graph)
}

/// Returns true if the cursor is located in a file under the project directory.
fn is_in_project(cursor: &clang::Entity, project_dir: &Path) -> bool {
    let loc = match cursor.get_location() {
        Some(l) => l,
        None => return true, // No location = probably a built-in, allow to proceed
    };
    let file_loc = loc.get_file_location();
    match file_loc.file {
        Some(f) => f.get_path().starts_with(project_dir),
        None => true,
    }
}

/// Recursively visit AST nodes and populate the graph.
fn visit_cursor(cursor: &clang::Entity, graph: &mut Graph, base_dir: &Path) {
    use clang::EntityKind;

    match cursor.get_kind() {
        EntityKind::ClassDecl | EntityKind::StructDecl => {
            if cursor.is_definition() && is_in_project(cursor, base_dir) {
                if let Some(name) = cursor.get_name() {
                    let qualified = qualified_name(cursor);
                    let kind = class_or_struct_kind(cursor);
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
                                        let base_kind = class_or_struct_kind(&base_decl);
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

                    // Emit Contains edge: namespace → this class/struct
                    if let Some(parent) = cursor.get_semantic_parent() {
                        if parent.get_kind() == EntityKind::Namespace && parent.get_name().is_some()
                        {
                            let parent_qualified = qualified_name(&parent);
                            let parent_id =
                                SymbolId::from_parts(&parent_qualified, SymbolKind::Namespace);
                            graph.add_edge(Edge {
                                from: parent_id,
                                to: id,
                                kind: EdgeKind::Contains,
                                location: None,
                            });
                        }
                    }
                }
            }
        }

        EntityKind::Namespace => {
            if is_in_project(cursor, base_dir) {
                if let Some(name) = cursor.get_name() {
                    // Skip anonymous namespaces
                    if !name.is_empty() {
                        let qualified = qualified_name(cursor);
                        let kind = SymbolKind::Namespace;
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
                    }
                }
            }
        }

        EntityKind::ClassTemplate => {
            // Class templates are handled similarly to ClassDecl. We emit
            // the template itself as a Class; explicit specializations
            // are handled via ClassTemplatePartialSpecialization below.
            if cursor.is_definition() && is_in_project(cursor, base_dir) {
                if let Some(name) = cursor.get_name() {
                    let qualified = qualified_name(cursor);
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
                }
            }
        }

        EntityKind::Method | EntityKind::FunctionDecl | EntityKind::FunctionTemplate => {
            if !is_in_project(cursor, base_dir) {
                // Don't recurse into system headers
                return;
            }
            if let Some(name) = cursor.get_name() {
                let qualified = qualified_name(cursor);
                let kind = if cursor.get_kind() == EntityKind::Method {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                };
                let location = cursor_location(cursor, graph, base_dir);
                let id = SymbolId::from_parts(&qualified, kind);

                let is_def = cursor.is_definition();
                let has_body = !cursor.get_children().is_empty();

                graph.add_symbol(Symbol {
                    id,
                    kind,
                    name,
                    qualified_name: qualified,
                    location,
                    module: None,
                    attrs: Default::default(),
                });

                // Emit Contains edge: owning class/struct → this method
                if cursor.get_kind() == EntityKind::Method {
                    if let Some(parent) = cursor.get_semantic_parent() {
                        if matches!(
                            parent.get_kind(),
                            EntityKind::ClassDecl
                                | EntityKind::StructDecl
                                | EntityKind::ClassTemplate
                        ) {
                            let parent_qualified = qualified_name(&parent);
                            let parent_kind = class_or_struct_kind(&parent);
                            let parent_id = SymbolId::from_parts(&parent_qualified, parent_kind);
                            graph.add_edge(Edge {
                                from: parent_id,
                                to: id,
                                kind: EdgeKind::Contains,
                                location: None,
                            });
                        }
                    }

                    // Emit Overrides edges
                    if let Some(overridden) = cursor.get_overridden_methods() {
                        for base_method in &overridden {
                            if let Some(base_name) = base_method.get_name() {
                                let base_qualified = qualified_name(base_method);
                                let base_kind = SymbolKind::Method;
                                let base_id = SymbolId::from_parts(&base_qualified, base_kind);

                                // Ensure overridden method symbol exists
                                if !graph.symbols.contains_key(&base_id) {
                                    let base_loc = cursor_location(base_method, graph, base_dir);
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
                                    kind: EdgeKind::Overrides,
                                    location: None,
                                });
                            }
                        }
                    }
                }

                // Emit Contains edge: namespace → this function
                if cursor.get_kind() == EntityKind::FunctionDecl
                    || cursor.get_kind() == EntityKind::FunctionTemplate
                {
                    if let Some(parent) = cursor.get_semantic_parent() {
                        if parent.get_kind() == EntityKind::Namespace && parent.get_name().is_some()
                        {
                            let parent_qualified = qualified_name(&parent);
                            let parent_id =
                                SymbolId::from_parts(&parent_qualified, SymbolKind::Namespace);
                            graph.add_edge(Edge {
                                from: parent_id,
                                to: id,
                                kind: EdgeKind::Contains,
                                location: None,
                            });
                        }
                    }
                }

                // Check for call expressions within this function/method.
                // Use has_body as fallback — is_definition() can return false
                // when there are parse errors (e.g. missing system headers).
                if is_def || has_body {
                    visit_calls(cursor, id, graph, base_dir);
                }
            }
        }

        EntityKind::FieldDecl => {
            if is_in_project(cursor, base_dir) {
                if let Some(name) = cursor.get_name() {
                    let qualified = qualified_name(cursor);
                    let kind = SymbolKind::Field;
                    let location = cursor_location(cursor, graph, base_dir);
                    let id = SymbolId::from_parts(&qualified, kind);

                    graph.add_symbol(Symbol {
                        id,
                        kind,
                        name: name.clone(),
                        qualified_name: qualified,
                        location,
                        module: None,
                        attrs: Default::default(),
                    });

                    // Emit Contains edge: owning class/struct → this field
                    if let Some(parent) = cursor.get_semantic_parent() {
                        if matches!(
                            parent.get_kind(),
                            EntityKind::ClassDecl
                                | EntityKind::StructDecl
                                | EntityKind::ClassTemplate
                        ) {
                            let parent_qualified = qualified_name(&parent);
                            let parent_kind = class_or_struct_kind(&parent);
                            let parent_id = SymbolId::from_parts(&parent_qualified, parent_kind);
                            graph.add_edge(Edge {
                                from: parent_id,
                                to: id,
                                kind: EdgeKind::Contains,
                                location: None,
                            });
                        }
                    }

                    // Emit HasType edge: field → type declaration (if it
                    // refers to a class/struct in the project).
                    if let Some(field_type) = cursor.get_type() {
                        // Peel through pointers, references, and typedefs to
                        // get the underlying named declaration.
                        let canonical = field_type.get_canonical_type();
                        if let Some(type_decl) = canonical.get_declaration() {
                            if matches!(
                                type_decl.get_kind(),
                                EntityKind::ClassDecl
                                    | EntityKind::StructDecl
                                    | EntityKind::ClassTemplate
                            ) {
                                if let Some(type_name) = type_decl.get_name() {
                                    let type_qualified = qualified_name(&type_decl);
                                    let type_kind = class_or_struct_kind(&type_decl);
                                    let type_id = SymbolId::from_parts(&type_qualified, type_kind);

                                    // Ensure the type symbol exists.
                                    if !graph.symbols.contains_key(&type_id) {
                                        let type_loc = cursor_location(&type_decl, graph, base_dir);
                                        graph.add_symbol(Symbol {
                                            id: type_id,
                                            kind: type_kind,
                                            name: type_name,
                                            qualified_name: type_qualified,
                                            location: type_loc,
                                            module: None,
                                            attrs: Default::default(),
                                        });
                                    }

                                    graph.add_edge(Edge {
                                        from: id,
                                        to: type_id,
                                        kind: EdgeKind::HasType,
                                        location: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }

        EntityKind::VarDecl => {
            // For variable declarations whose type is a template
            // specialization, emit a TemplateInstantiation symbol and an
            // Instantiates edge back to the primary template.
            if is_in_project(cursor, base_dir) {
                if let Some(var_type) = cursor.get_type() {
                    emit_template_instantiation(cursor, &var_type, graph, base_dir);
                }
            }
        }

        EntityKind::InclusionDirective => {
            // Emit Includes edges between TranslationUnit symbols.
            // The inclusion directive's file location tells us which TU
            // contains the #include, and get_file() gives us the included file.
            if is_in_project(cursor, base_dir) {
                if let Some(included_file) = cursor.get_file() {
                    let included_path = included_file.get_path();
                    let included_str = included_path
                        .strip_prefix(base_dir)
                        .unwrap_or(&included_path)
                        .to_string_lossy();

                    // Determine the including file from the cursor's location.
                    if let Some(loc) = cursor.get_location() {
                        let file_loc = loc.get_file_location();
                        if let Some(src_file) = file_loc.file {
                            let src_path = src_file.get_path();
                            let src_str = src_path
                                .strip_prefix(base_dir)
                                .unwrap_or(&src_path)
                                .to_string_lossy();

                            let src_id =
                                SymbolId::from_parts(&src_str, SymbolKind::TranslationUnit);
                            let inc_id =
                                SymbolId::from_parts(&included_str, SymbolKind::TranslationUnit);

                            // Ensure both TU symbols exist.
                            ensure_tu_symbol(graph, src_id, &src_str);
                            ensure_tu_symbol(graph, inc_id, &included_str);

                            graph.add_edge(Edge {
                                from: src_id,
                                to: inc_id,
                                kind: EdgeKind::Includes,
                                location: None,
                            });
                        }
                    }
                }
            }
        }

        _ => {}
    }

    // Recurse into children
    for child in cursor.get_children() {
        visit_cursor(&child, graph, base_dir);
    }
}

/// Visit expressions within a function body: calls and field accesses.
fn visit_calls(cursor: &clang::Entity, caller_id: SymbolId, graph: &mut Graph, base_dir: &Path) {
    visit_body(cursor, caller_id, graph, base_dir, false);
}

/// Recursive body visitor. `is_write_context` is true when the current subtree
/// is on the left-hand side of an assignment.
fn visit_body(
    cursor: &clang::Entity,
    caller_id: SymbolId,
    graph: &mut Graph,
    base_dir: &Path,
    is_write_context: bool,
) {
    use clang::EntityKind;

    for child in cursor.get_children() {
        match child.get_kind() {
            EntityKind::CallExpr => {
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
                // Recurse into call arguments (may contain field reads)
                visit_body(&child, caller_id, graph, base_dir, false);
            }

            // Direct field access: obj.field or ptr->field
            EntityKind::MemberRefExpr => {
                if let Some(referenced) = child.get_reference() {
                    if referenced.get_kind() == EntityKind::FieldDecl {
                        emit_field_access(
                            &child,
                            &referenced,
                            caller_id,
                            graph,
                            base_dir,
                            is_write_context,
                        );
                    }
                }
                // Recurse (e.g. chained member access a.b.c)
                visit_body(&child, caller_id, graph, base_dir, false);
            }

            // Constructor member initializer: radius_(radius)
            EntityKind::MemberRef => {
                if let Some(referenced) = child.get_reference() {
                    if referenced.get_kind() == EntityKind::FieldDecl {
                        // Member initializers are always writes.
                        emit_field_access(&child, &referenced, caller_id, graph, base_dir, true);
                    }
                }
            }

            // Binary operators (=, +=, -=, etc.): LHS is a write context.
            EntityKind::BinaryOperator | EntityKind::CompoundAssignOperator => {
                let children = child.get_children();
                if children.len() == 2 {
                    // LHS is write context
                    visit_body(&children[0], caller_id, graph, base_dir, true);
                    // RHS is read context
                    visit_body(&children[1], caller_id, graph, base_dir, false);
                } else {
                    visit_body(&child, caller_id, graph, base_dir, false);
                }
            }

            _ => {
                visit_body(&child, caller_id, graph, base_dir, is_write_context);
            }
        }
    }
}

/// Check a type for template specialization and emit a
/// [`SymbolKind::TemplateInstantiation`] symbol + [`EdgeKind::Instantiates`] edge
/// if it refers to a class template.
fn emit_template_instantiation(
    context_cursor: &clang::Entity,
    ty: &clang::Type,
    graph: &mut Graph,
    base_dir: &Path,
) {
    // Walk through the template argument list. If the type has template
    // arguments, it's a specialization (e.g. std::vector<int>).
    let n_args = ty.get_template_argument_types().map(|a| a.len());
    if n_args.unwrap_or(0) == 0 {
        return;
    }

    // Get the display name of the specialization (e.g. "vector<int>").
    let display = ty.get_display_name();
    if display.is_empty() {
        return;
    }

    // Try to find the primary template declaration.
    let canonical = ty.get_canonical_type();
    let decl = match canonical.get_declaration() {
        Some(d) => d,
        None => return,
    };

    // The primary template is the specialized cursor's template, if available.
    let template_cursor = decl.get_template().unwrap_or(decl);
    let template_name = match template_cursor.get_name() {
        Some(n) => n,
        None => return,
    };

    let template_qualified = qualified_name(&template_cursor);
    let template_kind = class_or_struct_kind(&template_cursor);
    let template_id = SymbolId::from_parts(&template_qualified, template_kind);

    let inst_id = SymbolId::from_parts(&display, SymbolKind::TemplateInstantiation);

    // Only emit if we haven't already created this instantiation.
    if !graph.symbols.contains_key(&inst_id) {
        let location = cursor_location(context_cursor, graph, base_dir);
        graph.add_symbol(Symbol {
            id: inst_id,
            kind: SymbolKind::TemplateInstantiation,
            name: display.clone(),
            qualified_name: display,
            location,
            module: None,
            attrs: Default::default(),
        });
    }

    // Ensure the primary template exists.
    if !graph.symbols.contains_key(&template_id) {
        let tmpl_loc = cursor_location(&template_cursor, graph, base_dir);
        graph.add_symbol(Symbol {
            id: template_id,
            kind: template_kind,
            name: template_name,
            qualified_name: template_qualified,
            location: tmpl_loc,
            module: None,
            attrs: Default::default(),
        });
    }

    graph.add_edge(Edge {
        from: inst_id,
        to: template_id,
        kind: EdgeKind::Instantiates,
        location: None,
    });
}

/// Emit a ReadsField or WritesField edge for a field access.
fn emit_field_access(
    access_cursor: &clang::Entity,
    field_decl: &clang::Entity,
    accessor_id: SymbolId,
    graph: &mut Graph,
    base_dir: &Path,
    is_write: bool,
) {
    if let Some(field_name) = field_decl.get_name() {
        let field_qualified = qualified_name(field_decl);
        let field_id = SymbolId::from_parts(&field_qualified, SymbolKind::Field);

        let edge_kind = if is_write {
            EdgeKind::WritesField
        } else {
            EdgeKind::ReadsField
        };

        let loc = cursor_location(access_cursor, graph, base_dir);
        graph.add_edge(Edge {
            from: accessor_id,
            to: field_id,
            kind: edge_kind,
            location: loc,
        });

        // Ensure field symbol exists
        if !graph.symbols.contains_key(&field_id) {
            let field_loc = cursor_location(field_decl, graph, base_dir);
            graph.add_symbol(Symbol {
                id: field_id,
                kind: SymbolKind::Field,
                name: field_name,
                qualified_name: field_qualified,
                location: field_loc,
                module: None,
                attrs: Default::default(),
            });
        }
    }
}

/// Remove duplicate edges from the graph.
///
/// Multiple translation units may produce identical edges when they include
/// the same headers. We deduplicate by (from, to, kind).
fn dedup_edges(graph: &mut Graph) {
    let mut seen = std::collections::HashSet::new();
    graph
        .edges
        .retain(|e| seen.insert((e.from, e.to, std::mem::discriminant(&e.kind))));
}

/// Map a class/struct/template cursor to the appropriate `SymbolKind`.
///
/// `ClassTemplate` cursors are treated as `Class` since the IR doesn't
/// distinguish templated from non-templated class definitions.
fn class_or_struct_kind(cursor: &clang::Entity) -> SymbolKind {
    use clang::EntityKind;
    match cursor.get_kind() {
        EntityKind::StructDecl => SymbolKind::Struct,
        _ => SymbolKind::Class,
    }
}

/// Ensure a [`SymbolKind::TranslationUnit`] symbol exists in the graph.
fn ensure_tu_symbol(graph: &mut Graph, id: SymbolId, path: &str) {
    if !graph.symbols.contains_key(&id) {
        // Use the file name as the short name.
        let name = std::path::Path::new(path)
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
fn cursor_location(cursor: &clang::Entity, graph: &mut Graph, base_dir: &Path) -> Option<Location> {
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

/// Compute the project root as the longest common ancestor directory of all
/// source files listed in `compile_commands.json`.
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
    // Ensure prefix is a directory, not a file
    if prefix.is_file() {
        prefix = prefix.parent().unwrap_or(cc_parent).to_path_buf();
    }
    // The project root should never be deeper than the compile_commands.json
    // directory.  Source files may all live under a `src/` subdirectory while
    // headers live under `include/`, so the common ancestor of sources alone
    // can be too narrow.  compile_commands.json is conventionally placed at the
    // project root, so clamp to that.
    let cc_abs = cc_parent
        .canonicalize()
        .unwrap_or_else(|_| cc_parent.to_path_buf());
    common_ancestor(&prefix, &cc_abs)
}

/// Find the longest common ancestor path of two paths.
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

/// Discover system C++ include paths by querying the compiler.
///
/// Runs `clang++ -E -x c++ -v /dev/null` and parses the `#include <...>`
/// search list from stderr. Falls back to well-known paths if the command
/// fails.
fn system_include_paths() -> Vec<String> {
    use std::sync::OnceLock;
    static PATHS: OnceLock<Vec<String>> = OnceLock::new();
    PATHS
        .get_or_init(|| {
            // Try LIBCLANG_PATH-relative paths first, then fall back to
            // querying the system compiler.
            if let Ok(libclang_path) = std::env::var("LIBCLANG_PATH") {
                let llvm_root = std::path::Path::new(&libclang_path)
                    .parent()
                    .unwrap_or(std::path::Path::new("."));
                let candidates = [
                    llvm_root.join("include/c++/v1"),
                    llvm_root
                        .join("lib/clang")
                        .join(
                            // Find the clang version directory
                            std::fs::read_dir(llvm_root.join("lib/clang"))
                                .ok()
                                .and_then(|mut d| d.next())
                                .and_then(|e| e.ok())
                                .map(|e| e.file_name().to_string_lossy().into_owned())
                                .unwrap_or_default(),
                        )
                        .join("include"),
                ];
                let mut paths: Vec<String> = candidates
                    .iter()
                    .filter(|p| p.is_dir())
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect();
                if !paths.is_empty() {
                    info!(
                        count = paths.len(),
                        "discovered system includes from LIBCLANG_PATH"
                    );
                    // Also add SDK includes on macOS
                    if let Ok(output) = std::process::Command::new("xcrun")
                        .args(["--show-sdk-path"])
                        .output()
                    {
                        let sdk = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        let usr_include = format!("{sdk}/usr/include");
                        if std::path::Path::new(&usr_include).is_dir() {
                            paths.push(usr_include);
                        }
                    }
                    return paths;
                }
            }

            // Fall back: ask the system clang for its search paths
            let output = std::process::Command::new("clang++")
                .args(["-E", "-x", "c++", "-v", "/dev/null"])
                .output();

            let stderr = match output {
                Ok(o) => String::from_utf8_lossy(&o.stderr).into_owned(),
                Err(_) => return vec![],
            };

            let mut paths = Vec::new();
            let mut in_search_list = false;
            for line in stderr.lines() {
                if line.contains("#include <...> search starts here:") {
                    in_search_list = true;
                    continue;
                }
                if line.contains("End of search list") {
                    break;
                }
                if in_search_list {
                    let path = line.trim();
                    // Skip framework directories
                    if !path.contains("(framework directory)") && !path.is_empty() {
                        paths.push(path.to_string());
                    }
                }
            }
            info!(
                count = paths.len(),
                "discovered system includes from clang++"
            );
            paths
        })
        .clone()
}
