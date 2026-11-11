//! Builds libmgba from the pinned submodule and the C shim over it, both
//! through `shim/CMakeLists.txt`, then links them statically.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let repo = manifest
        .parent()
        .and_then(|p| p.parent())
        .expect("crate sits two levels below the repo root");
    let mgba = repo.join("third_party").join("mgba");
    assert!(
        mgba.join("CMakeLists.txt").exists(),
        "libmgba sources missing: run `git submodule update --init` (third_party/mgba, pinned to 662dada)"
    );
    println!("cargo:rerun-if-changed=shim");
    println!("cargo:rerun-if-changed=../../third_party/mgba/src");
    println!("cargo:rerun-if-changed=../../third_party/mgba/include");
    println!("cargo:rerun-if-changed=../../third_party/mgba/CMakeLists.txt");

    let dst = cmake::Config::new(manifest.join("shim"))
        .profile("Release")
        .define("MGBA_DIR", mgba.to_str().expect("UTF-8 path").replace('\\', "/"))
        .build();

    println!("cargo:rustc-link-search=native={}", dst.join("lib").display());
    println!("cargo:rustc-link-lib=static=pact_shim");
    println!("cargo:rustc-link-lib=static=mgba");
    match std::env::var("CARGO_CFG_TARGET_OS").unwrap().as_str() {
        "windows" => {
            for lib in [
                "ws2_32", "shlwapi", "ole32", "uuid", "winmm", "iphlpapi", "user32", "advapi32", "shell32",
            ] {
                println!("cargo:rustc-link-lib={lib}");
            }
        }
        "macos" => {
            println!("cargo:rustc-link-lib=framework=CoreFoundation");
        }
        _ => {
            println!("cargo:rustc-link-lib=pthread");
            println!("cargo:rustc-link-lib=m");
        }
    }
}
