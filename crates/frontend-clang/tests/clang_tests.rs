//! Integration tests for frontend-clang (requires libclang).

#![cfg(feature = "clang-tests")]

use std::path::Path;

use core_ir::EdgeKind;
use frontend_clang::index_project;

#[test]
fn test_index_tiny_cpp() {
    let compile_commands =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/tiny-cpp/compile_commands.json");

    let graph = index_project(&compile_commands).expect("indexing failed");

    // Should have at least Shape, Circle, Square classes
    assert!(
        graph.symbol_count() >= 3,
        "expected at least 3 symbols, got {}",
        graph.symbol_count()
    );

    // Dump all symbols and edges for debugging
    eprintln!("=== Symbols ({}) ===", graph.symbol_count());
    for sym in graph.symbols.values() {
        eprintln!("  {:?} {} ({})", sym.kind, sym.qualified_name, sym.name);
    }
    eprintln!("=== Edges ({}) ===", graph.edge_count());
    for edge in &graph.edges {
        let from_name = graph
            .symbols
            .get(&edge.from)
            .map(|s| s.qualified_name.as_str())
            .unwrap_or("?");
        let to_name = graph
            .symbols
            .get(&edge.to)
            .map(|s| s.qualified_name.as_str())
            .unwrap_or("?");
        eprintln!("  {:?}: {} -> {}", edge.kind, from_name, to_name);
    }

    // Should have inheritance edges
    let inherits_count = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Inherits)
        .count();
    assert!(
        inherits_count >= 2,
        "expected at least 2 inheritance edges, got {}",
        inherits_count
    );

    // Should have call edges (main calls area())
    let calls_count = graph
        .edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .count();
    assert!(
        calls_count >= 1,
        "expected at least 1 call edge, got {}",
        calls_count
    );
}
