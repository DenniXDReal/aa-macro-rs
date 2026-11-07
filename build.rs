//! Bakes assets/routes/*.json, assets/lobby/*.json, assets/patterns/* and assets/stages.json
//! into the exe so the single .exe carries every default.

use std::fs;
use std::path::Path;

fn list(dir: &str) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
                .map(|p| {
                    let stem = p.file_stem().unwrap().to_string_lossy().to_string();
                    let file = p.file_name().unwrap().to_string_lossy().to_string();
                    (stem, format!("/{dir}/{file}"))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

fn main() {
    println!("cargo:rerun-if-changed=assets");
    let mut out = String::new();
    for (name, dir) in [
        ("ROUTES", "assets/routes"),
        ("LOBBY", "assets/lobby"),
        ("MAPDATA", "assets/mapdata"),
    ] {
        out.push_str(&format!("pub const {name}: &[(&str, &str)] = &[\n"));
        for (stem, abs) in list(dir) {
            out.push_str(&format!(
                "    ({stem:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {abs:?}))),\n"
            ));
        }
        out.push_str("];\n");
    }
    // your captured patterns, shipped as defaults (user copies win)
    out.push_str("pub const PATTERNS: &[(&str, &str, Option<&[u8]>)] = &[\n");
    for (stem, abs) in list("assets/patterns") {
        let png = format!("assets/patterns/{stem}.png");
        let png = if Path::new(&png).exists() {
            format!("Some(include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{png}\")))")
        } else {
            "None".into()
        };
        out.push_str(&format!(
            "    ({stem:?}, include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), {abs:?})), {png}),\n"
        ));
    }
    out.push_str("];\n");
    // built-in minimap pictures (assets/minimaps/<Map>.png)
    out.push_str("pub const MINIMAPS: &[(&str, &[u8])] = &[\n");
    let mut mm: Vec<String> = fs::read_dir("assets/minimaps")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("png"))
                .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    mm.sort();
    for f in mm {
        let stem = f.trim_end_matches(".png");
        out.push_str(&format!(
            "    ({stem:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/assets/minimaps/{f}\"))),\n"
        ));
    }
    out.push_str("];\n");
    out.push_str("pub const STAGES: &str = include_str!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/assets/stages.json\"));\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("assets_gen.rs");
    fs::write(dest, out).unwrap();
}
