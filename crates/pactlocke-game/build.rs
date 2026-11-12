//! Bundles every `profiles/*.toml` into the binary, so adding a game is adding
//! a file.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("profiles");
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("the profiles directory exists")
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();

    let mut out = String::from("&[\n");
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let full = path.to_string_lossy().replace('\\', "/");
        writeln!(out, "    ({name:?}, include_str!({full:?})),").unwrap();
    }
    out.push(']');

    let dest = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("bundled_profiles.rs");
    std::fs::write(dest, out).unwrap();
}
