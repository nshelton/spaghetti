//! Build script for the `spaghetti` binary.
//!
//! On macOS, bake an rpath into the linked binary pointing at
//! `$LIBCLANG_PATH` so dyld can find `libclang.dylib` at runtime
//! without the user having to set `DYLD_FALLBACK_LIBRARY_PATH` on
//! every invocation. Homebrew's LLVM installs `libclang.dylib` with
//! an install name of `@rpath/libclang.dylib`, so the executable
//! must carry at least one rpath directory containing the dylib.
//!
//! No-op on other platforms: Linux typically has libclang on an
//! `ldconfig`-indexed path, and Windows has its own DLL resolution
//! rules.

fn main() {
    println!("cargo:rerun-if-env-changed=LIBCLANG_PATH");

    if !cfg!(target_os = "macos") {
        return;
    }

    let Ok(path) = std::env::var("LIBCLANG_PATH") else {
        return;
    };
    if path.is_empty() {
        return;
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,{path}");
}
