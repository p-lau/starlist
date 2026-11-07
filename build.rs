//! Embed the dashboard (dashboard/build, made by `npm run build` there) when it
//! has been built. Without it the binary serves a page saying how to build it,
//! so `cargo build` never needs Node.

use std::path::{Path, PathBuf};

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn main() {
    if std::env::var_os("CARGO_FEATURE_DASHBOARD").is_none() {
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("dashboard")
        .join("build");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut files = vec![];
    walk(&root, &mut files);
    files.sort();
    let mut code = String::from(
        "/// Published path, contents, gzipped contents.\npub type File = (&'static str, &'static [u8], Option<&'static [u8]>);\npub static FILES: &[File] = &[\n",
    );
    for f in &files {
        let ext = f.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext == "gz" || ext == "br" {
            continue;
        }
        println!("cargo:rerun-if-changed={}", f.display());
        let rel = f
            .strip_prefix(&root)
            .unwrap()
            .to_string_lossy()
            .replace('\\', "/");
        let mut gz = f.clone().into_os_string();
        gz.push(".gz");
        let gz = PathBuf::from(gz);
        let gz = if gz.exists() {
            format!("Some(include_bytes!({:?}))", gz.to_string_lossy())
        } else {
            "None".to_string()
        };
        code.push_str(&format!(
            "    ({rel:?}, include_bytes!({:?}), {gz}),\n",
            f.to_string_lossy()
        ));
    }
    code.push_str("];\n");
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("dashboard.rs");
    std::fs::write(out, code).unwrap();
}
