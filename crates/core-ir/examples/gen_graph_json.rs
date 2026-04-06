//! Helper to generate examples/tiny-cpp/graph.json with correct SymbolId hashes.

use core_ir::*;

fn make_symbol(
    name: &str,
    qualified: &str,
    kind: SymbolKind,
    loc: Option<Location>,
    attrs: smallvec::SmallVec<[Attr; 2]>,
) -> Symbol {
    Symbol {
        id: SymbolId::from_parts(qualified, kind),
        kind,
        name: name.to_owned(),
        qualified_name: qualified.to_owned(),
        location: loc,
        module: None,
        attrs,
    }
}

fn main() {
    let mut g = Graph::new();

    // Intern files
    let shape_h = g.files.intern("include/shape.h");
    let circle_h = g.files.intern("include/circle.h");
    let square_h = g.files.intern("include/square.h");
    let main_cpp = g.files.intern("src/main.cpp");

    // Classes
    let shape = make_symbol(
        "Shape",
        "Shape",
        SymbolKind::Class,
        Some(Location {
            file: shape_h,
            line: 3,
            col: 1,
        }),
        smallvec::smallvec![Attr::Abstract],
    );
    let circle = make_symbol(
        "Circle",
        "Circle",
        SymbolKind::Class,
        Some(Location {
            file: circle_h,
            line: 5,
            col: 1,
        }),
        Default::default(),
    );
    let square = make_symbol(
        "Square",
        "Square",
        SymbolKind::Class,
        Some(Location {
            file: square_h,
            line: 5,
            col: 1,
        }),
        Default::default(),
    );

    // Methods
    let shape_area = make_symbol(
        "area",
        "Shape::area",
        SymbolKind::Method,
        Some(Location {
            file: shape_h,
            line: 6,
            col: 5,
        }),
        smallvec::smallvec![Attr::Virtual, Attr::Abstract],
    );
    let circle_area = make_symbol(
        "area",
        "Circle::area",
        SymbolKind::Method,
        Some(Location {
            file: circle_h,
            line: 8,
            col: 5,
        }),
        smallvec::smallvec![Attr::Virtual],
    );
    let square_area = make_symbol(
        "area",
        "Square::area",
        SymbolKind::Method,
        Some(Location {
            file: square_h,
            line: 8,
            col: 5,
        }),
        smallvec::smallvec![Attr::Virtual],
    );
    let main_fn = make_symbol(
        "main",
        "main",
        SymbolKind::Function,
        Some(Location {
            file: main_cpp,
            line: 7,
            col: 1,
        }),
        Default::default(),
    );

    let shape_id = shape.id;
    let circle_id = circle.id;
    let square_id = square.id;
    let shape_area_id = shape_area.id;
    let circle_area_id = circle_area.id;
    let square_area_id = square_area.id;
    let main_id = main_fn.id;

    g.add_symbol(shape);
    g.add_symbol(circle);
    g.add_symbol(square);
    g.add_symbol(shape_area);
    g.add_symbol(circle_area);
    g.add_symbol(square_area);
    g.add_symbol(main_fn);

    // Inheritance edges
    g.add_edge(Edge {
        from: circle_id,
        to: shape_id,
        kind: EdgeKind::Inherits,
        location: None,
    });
    g.add_edge(Edge {
        from: square_id,
        to: shape_id,
        kind: EdgeKind::Inherits,
        location: None,
    });

    // Contains edges (class contains method)
    g.add_edge(Edge {
        from: shape_id,
        to: shape_area_id,
        kind: EdgeKind::Contains,
        location: None,
    });
    g.add_edge(Edge {
        from: circle_id,
        to: circle_area_id,
        kind: EdgeKind::Contains,
        location: None,
    });
    g.add_edge(Edge {
        from: square_id,
        to: square_area_id,
        kind: EdgeKind::Contains,
        location: None,
    });

    // Override edges
    g.add_edge(Edge {
        from: circle_area_id,
        to: shape_area_id,
        kind: EdgeKind::Overrides,
        location: None,
    });
    g.add_edge(Edge {
        from: square_area_id,
        to: shape_area_id,
        kind: EdgeKind::Overrides,
        location: None,
    });

    // Call edges from main
    g.add_edge(Edge {
        from: main_id,
        to: circle_area_id,
        kind: EdgeKind::Calls,
        location: Some(Location {
            file: main_cpp,
            line: 13,
            col: 9,
        }),
    });
    g.add_edge(Edge {
        from: main_id,
        to: square_area_id,
        kind: EdgeKind::Calls,
        location: Some(Location {
            file: main_cpp,
            line: 13,
            col: 9,
        }),
    });

    println!("{}", g.to_json().expect("serialize"));
}
