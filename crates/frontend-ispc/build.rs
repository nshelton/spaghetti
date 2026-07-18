fn main() {
    println!("cargo:rerun-if-changed=grammar/parser.c");
    cc::Build::new()
        .include("grammar")
        .file("grammar/parser.c")
        .compile("tree_sitter_ispc");
}
