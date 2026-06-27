//! dds_audit — walk the resolved the game `gfx/` tree and print a format histogram
//! using the SAME `assets::dds` parser the renderer uses, so the audit reflects
//! exactly how the loader will classify each file.
//!
//! Usage:
//!   cargo run -p assets --bin dds_audit -- [--vanilla <path>] [--limit N] [<subdir>]
//! Game path: `--vanilla` / `BATTLE_PLAN_VANILLA` / documented Steam default.
//! READ-ONLY on the game install.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use assets::dds;
use assets::resolver::GameResolver;

fn arg_value(flag: &str) -> Option<String> {
    let mut it = std::env::args();
    while let Some(a) = it.next() {
        if a == flag {
            return it.next();
        }
    }
    None
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, limit: usize) {
    if out.len() >= limit {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        if out.len() >= limit {
            return;
        }
        let p = e.path();
        if p.is_dir() {
            walk(&p, out, limit);
        } else if p
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("dds"))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

fn main() {
    let vanilla = arg_value("--vanilla");
    let limit: usize = arg_value("--limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(8000);
    // Positional subdir = the first non-flag arg that isn't a flag's VALUE.
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let mut subdir = "gfx".to_string();
    let mut i = 0;
    while i < raw.len() {
        let a = &raw[i];
        if a == "--vanilla" || a == "--limit" {
            i += 2; // skip the flag and its value
            continue;
        }
        if !a.starts_with("--") {
            subdir = a.clone();
            break;
        }
        i += 1;
    }

    let resolver = GameResolver::new(vanilla.as_deref(), None);
    if !resolver.vanilla_exists() {
        eprintln!(
            "BLOCKER: game install not found at {} (pass --vanilla or set BATTLE_PLAN_VANILLA)",
            resolver.vanilla.display()
        );
        std::process::exit(2);
    }
    let root = resolver.vanilla.join(&subdir);
    let mut files = Vec::new();
    walk(&root, &mut files, limit);
    eprintln!("[audit] scanning {} .dds under {}", files.len(), root.display());

    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors = 0usize;
    let mut examples: BTreeMap<String, String> = BTreeMap::new();
    for p in &files {
        let Ok(bytes) = std::fs::read(p) else {
            errors += 1;
            continue;
        };
        match dds::parse_header(&bytes) {
            Ok(h) => {
                let key = format!("{}", h.format);
                *hist.entry(key.clone()).or_default() += 1;
                examples
                    .entry(key)
                    .or_insert_with(|| format!("{} ({}x{})", short(p), h.width, h.height));
            }
            Err(_) => errors += 1,
        }
    }

    let total: usize = hist.values().sum();
    println!("=== the game DDS format audit (n={total}, errors={errors}) ===");
    let mut rows: Vec<_> = hist.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    for (fmt, count) in rows {
        let pct = 100.0 * count as f64 / total.max(1) as f64;
        let ex = examples.get(&fmt).cloned().unwrap_or_default();
        println!("{count:6}  {pct:5.1}%  {fmt:<18}  e.g. {ex}");
    }
}

fn short(p: &Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string()
}
