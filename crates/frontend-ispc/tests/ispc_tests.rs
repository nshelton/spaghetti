//! Integration tests for frontend-ispc (no external dependencies — the
//! grammar is vendored and compiled into the crate).

use std::path::Path;

use core_ir::{EdgeKind, SymbolKind};
use frontend_ispc::index_project;

#[test]
fn test_index_simple_ispc() {
    let compile_commands =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/compile_commands.json");

    let graph = index_project(&compile_commands).expect("indexing failed");

    let find = |name: &str, kind: SymbolKind| {
        graph
            .symbols
            .values()
            .find(|s| s.name == name && s.kind == kind)
            .unwrap_or_else(|| panic!("missing symbol {name} ({kind:?})"))
    };

    let square = find("square", SymbolKind::Function);
    let compute = find("computeAreas", SymbolKind::Function);
    let sphere = find("Sphere", SymbolKind::Struct);
    find("radius", SymbolKind::Field);
    find("weight", SymbolKind::Field);
    find("bias", SymbolKind::Field);
    let tu = find("simple.ispc", SymbolKind::TranslationUnit);
    let header = find("helper.isph", SymbolKind::TranslationUnit);

    let has_edge = |from, to, kind: EdgeKind| {
        graph
            .edges
            .iter()
            .any(|e| e.from == from && e.to == to && e.kind == kind)
    };

    assert!(
        has_edge(compute.id, square.id, EdgeKind::Calls),
        "computeAreas should call square"
    );
    assert!(
        has_edge(tu.id, compute.id, EdgeKind::Contains),
        "TU should contain computeAreas"
    );
    assert!(
        has_edge(tu.id, sphere.id, EdgeKind::Contains),
        "TU should contain Sphere"
    );
    assert!(
        has_edge(tu.id, header.id, EdgeKind::Includes),
        "TU should include helper.isph"
    );
    assert!(
        graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Contains && e.from == sphere.id)
            .count()
            == 3,
        "Sphere should contain 3 fields"
    );

    // Fields are qualified by their struct.
    let radius = find("radius", SymbolKind::Field);
    assert_eq!(radius.qualified_name, "Sphere::radius");

    // Transitively parsed header contributes its own symbols.
    let material = find("Material", SymbolKind::Struct);
    find("clampf", SymbolKind::Function);
    assert!(
        has_edge(header.id, material.id, EdgeKind::Contains),
        "helper.isph should contain Material"
    );
}
