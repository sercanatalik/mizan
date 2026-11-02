//! Embed bundled adapters (`adapters/*/adapter.toml` and `script.ts`) into the binary.
//! Fixtures are not embedded.

use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../adapters");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut dirs: Vec<_> = std::fs::read_dir(&root)
        .map(|r| {
            r.flatten()
                .map(|e| e.path())
                .filter(|p| p.join("adapter.toml").is_file())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort();
    let mut out = String::from("pub static BUNDLED: &[crate::registry::Bundled] = &[\n");
    for d in dirs {
        let manifest = d.join("adapter.toml");
        let script = d.join("script.ts");
        println!("cargo:rerun-if-changed={}", manifest.display());
        println!("cargo:rerun-if-changed={}", script.display());
        let name = d.file_name().unwrap().to_string_lossy();
        let script_expr = if script.is_file() {
            format!("Some(include_str!({:?}))", script.canonicalize().unwrap())
        } else {
            "None".into()
        };
        writeln!(
            out,
            "    crate::registry::Bundled {{ dir: {name:?}, manifest: include_str!({:?}), script: {script_expr} }},",
            manifest.canonicalize().unwrap()
        )
        .unwrap();
    }
    out.push_str("];\n");
    std::fs::write(
        Path::new(&std::env::var("OUT_DIR").unwrap()).join("bundled.rs"),
        out,
    )
    .unwrap();
}
