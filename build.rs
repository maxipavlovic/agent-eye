// Embeds face/dist (the built TypeScript board) into the binary. Without a built face the table
// is empty and `/` serves a plain notice, so `cargo build` needs no node.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            let name = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if !name.ends_with(".map") {
                out.push((name, p.clone()));
            }
        }
    }
}

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dist = root.join("face").join("dist");
    println!("cargo:rerun-if-changed={}", dist.display());
    println!("cargo:rerun-if-changed=build.rs");
    let mut files = vec![];
    walk(&dist, &dist, &mut files);
    // A face without index.html is not a face.
    if !files.iter().any(|(n, _)| n == "index.html") {
        files.clear();
    }
    let mut src = String::from("pub static FACE_FILES: &[(&str, &[u8])] = &[\n");
    for (name, path) in &files {
        println!("cargo:rerun-if-changed={}", path.display());
        writeln!(
            src,
            "    ({:?}, include_bytes!({:?})),",
            name,
            path.to_string_lossy()
        )
        .unwrap();
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("face_files.rs");
    std::fs::write(out, src).unwrap();
}
