//! Measurement harness: sweep a real the game install with the SHIPPING
//! `diagnose_text_decode` and report every byte-level decode loss by type.
//!
//! Deliberately calls `paradox_text::diagnose_text_decode` rather than
//! re-deriving the decode rules, so the measurement cannot drift away from the
//! function whose behaviour is being characterized.
//!
//! Usage: cargo run -p portable_content_manifest --example decode_loss_sweep -- <install_root>

use portable_content_manifest::paradox_text::{diagnose_text_decode, TextDecodeLoss};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Text-ish content extensions the replay layer actually parses or reads.
///
/// `.fnt` is included because the HUD emitter decodes BMFont descriptors as
/// text (`bmfont_page_files`) and then BINDS them, so they are as much part of
/// the decode surface as `.gui`/`.gfx`/`.yml`. Binary atlas pages
/// (`.dds`/`.tga`) are deliberately excluded: they are never text-decoded, and
/// running them through the text decoder would fabricate diagnostics for any
/// payload that happens to begin with a BOM-like byte pair.
const EXTS: &[&str] = &[
    "gui", "gfx", "txt", "yml", "yaml", "asset", "mod", "info", "settings", "lua", "csv", "json",
    "fnt", "shader", "fxh",
];

fn main() {
    let root = std::env::args()
        .nth(1)
        .expect("usage: decode_loss_sweep <install_root>");
    let root = PathBuf::from(root);

    let mut files = Vec::new();
    walk(&root, &mut files);
    files.sort();

    let mut by_ext: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_lang: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut n_invalid_utf8 = 0usize;
    let mut n_unpaired = 0usize;
    let mut n_truncated = 0usize;
    let mut n_latin1 = 0usize;
    let mut n_suppressed = 0usize;
    let mut lossy_files: Vec<(PathBuf, Vec<TextDecodeLoss>)> = Vec::new();
    let mut unreadable = 0usize;

    for path in &files {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        *by_ext.entry(ext).or_default() += 1;

        let lang = language_of(&root, path);
        let entry = by_lang.entry(lang).or_default();
        entry.0 += 1;

        let Ok(bytes) = std::fs::read(path) else {
            unreadable += 1;
            continue;
        };
        let losses = diagnose_text_decode(&bytes);
        if losses.is_empty() {
            continue;
        }
        entry.1 += 1;
        for loss in &losses {
            match loss {
                TextDecodeLoss::InvalidUtf8 { .. } => n_invalid_utf8 += 1,
                TextDecodeLoss::UnpairedUtf16Surrogate { .. } => n_unpaired += 1,
                TextDecodeLoss::TruncatedUtf16Unit { .. } => n_truncated += 1,
                TextDecodeLoss::NonUtf8DecodedAsLatin1 { .. } => n_latin1 += 1,
                TextDecodeLoss::Suppressed { count } => n_suppressed += count,
            }
        }
        lossy_files.push((path.clone(), losses));
    }

    println!("install_root: {}", root.display());
    println!("files_scanned: {}", files.len());
    println!("files_unreadable: {unreadable}");
    println!("files_with_decode_loss: {}", lossy_files.len());
    println!();
    println!("loss counts by type:");
    println!("  InvalidUtf8:            {n_invalid_utf8}");
    println!("  UnpairedUtf16Surrogate: {n_unpaired}");
    println!("  TruncatedUtf16Unit:     {n_truncated}");
    println!("  NonUtf8DecodedAsLatin1: {n_latin1}");
    println!("  Suppressed (counted):   {n_suppressed}");
    println!();
    println!("files by extension:");
    for (ext, count) in &by_ext {
        println!("  .{ext}: {count}");
    }
    println!();
    println!("localisation coverage (language: files_scanned, files_with_loss):");
    for (lang, (total, lossy)) in &by_lang {
        println!("  {lang}: {total}, {lossy}");
    }

    if !lossy_files.is_empty() {
        println!();
        println!("lossy files (first 5 losses each):");
        for (path, losses) in &lossy_files {
            let rel = path.strip_prefix(&root).unwrap_or(path);
            println!("  {} [{} losses]", rel.display(), losses.len());
            for loss in losses.iter().take(5) {
                println!("      {loss}");
            }
        }
    }
}

/// Bucket a path by localisation language directory, else by top-level dir.
fn language_of(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let parts: Vec<_> = rel
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect();
    if let Some(i) = parts.iter().position(|p| p == "localisation") {
        return match parts.get(i + 1) {
            Some(next) if next.contains('.') => "localisation/<root>".to_string(),
            Some(next) => format!("localisation/{next}"),
            None => "localisation/<root>".to_string(),
        };
    }
    format!("<non-localisation>/{}", parts.first().cloned().unwrap_or_default())
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.file_type() else { continue };
        if meta.is_dir() {
            walk(&path, out);
        } else if meta.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if EXTS.contains(&ext.as_str()) {
                out.push(path);
            }
        }
    }
}
