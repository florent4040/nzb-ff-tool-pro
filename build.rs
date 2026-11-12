//! Intègre l'icône et les informations de version dans l'exe Windows (via windres de MinGW).
//! En cas d'échec, la compilation continue sans icône d'exe (la fenêtre garde son icône).

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=assets/app.rc");
    println!("cargo:rerun-if-changed=assets/app.ico");
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") || target.contains("msvc") {
        return;
    }
    let manifest = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    let out = PathBuf::from(env::var("OUT_DIR").unwrap_or_default()).join("nzbff_res.o");
    for windres in ["windres", "C:/mingw64/bin/windres.exe", "x86_64-w64-mingw32-windres"] {
        let ok = Command::new(windres)
            .current_dir(manifest.join("assets"))
            .args(["--input", "app.rc", "--output-format=coff", "--output"])
            .arg(&out)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            println!("cargo:rustc-link-arg-bins={}", out.display());
            return;
        }
    }
    println!("cargo:warning=windres introuvable : l'exe n'aura pas d'icône (la fenêtre, si).");
}
