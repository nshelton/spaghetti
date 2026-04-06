//! Integration tests for frontend-clang (requires libclang).

#![cfg(feature = "clang-tests")]

use std::path::Path;

use core_ir::EdgeKind;
use frontend_clang::index_project;

#[test]
fn test_index_tiny_cpp() {
    let compile_commands = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/tiny-cpp/compile_commands.json");

    let graph = index_project(&compile_commands).expect("indexing failed");

    // Should have at least Shape, Circle, Square classes
    assert!(
        graph.symbol_count() >= 3,
        "expected at least 3 symbols, got {}",
        graph.symbol_count()
    );

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
}
