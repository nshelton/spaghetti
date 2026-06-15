//! Build script for the `spaghetti` binary.
//!
//! On macOS, bake an rpath into the linked binary pointing at the
//! directory that holds `libclang.dylib` so dyld can resolve
//! `@rpath/libclang.dylib` at launch without the user having to set
//! `DYLD_FALLBACK_LIBRARY_PATH` on every invocation. Homebrew's LLVM
//! installs the dylib with an install name of `@rpath/libclang.dylib`,
//! so the executable must carry at least one matching rpath.
//!
//! No-op on other platforms: Linux typically has libclang on an
//! `ldconfig`-indexed path and Windows has its own DLL resolution
//! rules.

fn main() {
    println!("cargo:rerun-if-env-changed=LIBCLANG_PATH");

    if !cfg!(target_os = "macos") {
        return;
    }

    if let Some(path) = find_libclang_dir() {
        println!("cargo:rustc-link-arg=-Wl,-rpath,{path}");
    }
}

/// Locate the directory that contains `libclang.dylib`. Prefers
/// `$LIBCLANG_PATH` (matching how `clang-sys` resolves libclang at
/// link time); falls back to `brew --prefix llvm` so a Homebrew-
/// installed LLVM works without the user setting any env var.
fn find_libclang_dir() -> Option<String> {
    if let Ok(path) = std::env::var("LIBCLANG_PATH") {
        if !path.is_empty() {
            return Some(path);
        }
    }

    let output = std::process::Command::new("brew")
        .args(["--prefix", "llvm"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let prefix = String::from_utf8(output.stdout).ok()?;
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return None;
    }
    Some(format!("{prefix}/lib"))
}
