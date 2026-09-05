use std::collections::BTreeMap;
use std::path::Path;

fn raw_read_line(line: &str) -> bool {
    ["fs::", "File::open", "include_str!", "include_bytes!", "read_to_string(", "read_dir(", "use std::fs", "use std::{fs", "::read(", ".read(", ".read_exact(", ".read_to_end(", ".open(", "File::options", "OpenOptions::"]
        .iter().any(|pattern| line.contains(pattern))
}

fn collect(root: &Path, directory: &Path, rows: &mut BTreeMap<String, Vec<String>>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() { collect(root, &path, rows); }
        else if path.extension().is_some_and(|extension| extension == "rs") {
            let relative = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
            if relative.ends_with("/modvfs_read_guard.rs") { continue; }
            let source = std::fs::read_to_string(&path).unwrap();
            let sites: Vec<_> = source.lines().filter(|line| raw_read_line(line))
                .map(|line| line.trim().to_string()).collect();
            if !sites.is_empty() { rows.insert(relative, sites); }
        }
    }
}

#[test]
fn modvfs_raw_read_sites_match_the_sweep() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(3).unwrap();
    let mut actual = BTreeMap::new();
    for directory in ["replay_interface/native", "portable_replay/native"] {
        collect(root, &root.join(directory), &mut actual);
    }
    let expected: BTreeMap<String, Vec<String>> = serde_json::from_str(include_str!("modvfs_read_sites.json")).unwrap();
    assert!(actual.len() > 100, "incomplete native tree sweep");
    let changed: Vec<_> = actual.keys().chain(expected.keys())
        .filter(|file| actual.get(*file) != expected.get(*file)).collect();
    assert!(changed.is_empty(), "native raw-read sites changed: {changed:?}");
}

#[test]
fn modvfs_raw_read_guard_detects_new_files_and_alias_imports() {
    assert!(raw_read_line("let bytes = std::fs::read(root.join(name))?;"));
    assert!(raw_read_line("use std::fs as local;"));
    assert!(raw_read_line("let bytes = local::read(path)?;"));
    assert!(raw_read_line("let handle = options.read(true).open(path)?;"));
    assert!(raw_read_line("let file = File::open(path)?;"));
    assert!(raw_read_line("const GAME: &str = include_str!(\"game.txt\");"));
    assert!(!raw_read_line("let bytes = resolver.read_required_bytes(relative, label)?;"));
}
