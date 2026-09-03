// `FOLD_SOURCE_FINGERPRINT`: one digest over every source file that can
// change a folded byte -- this crate, every path dependency reachable from
// it (the fold, the domain grammars, the lane decoders, the cache encoder)
// and the workspace lock. A fold cache entry is keyed by it, so an edit to any
// of those crates makes every cached fold a miss instead of a stale hit.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest dir"));
    // The recorder compiles this crate by source path, so its build script
    // includes this one; both root the closure at replay_decoder so the two
    // binaries carry one fingerprint.
    let root = manifest_dir.join("../replay_decoder");
    let root = if root.join("Cargo.toml").is_file() {
        root
    } else {
        manifest_dir.clone()
    };
    let mut crates = BTreeSet::new();
    collect_path_dependencies(&root, &mut crates);
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    for crate_dir in &crates {
        collect_source_files(crate_dir, crate_dir, &mut files);
        println!("cargo:rerun-if-changed={}", crate_dir.display());
    }
    let lock = root.join("../Cargo.lock");
    if lock.is_file() {
        files.insert(lock.canonicalize().unwrap_or(lock));
    }
    // Paths are hashed relative to the repository so identical sources in two
    // worktrees share one fingerprint (and so one cache entry).
    let repo_root = root
        .canonicalize()
        .ok()
        .and_then(|dir| dir.ancestors().nth(3).map(Path::to_path_buf));
    let mut hasher = Sha256::new();
    for file in &files {
        let shown = repo_root
            .as_deref()
            .and_then(|repo| file.strip_prefix(repo).ok())
            .unwrap_or(file);
        let relative = shown.to_string_lossy().replace('\\', "/");
        hasher.update(relative.as_bytes());
        hasher.update(b"\0");
        hasher.update(&fs::read(file).unwrap_or_default());
        hasher.update(b"\n");
        println!("cargo:rerun-if-changed={}", file.display());
    }
    println!(
        "cargo:rustc-env=FOLD_SOURCE_FINGERPRINT={}",
        hex(&hasher.finalize())
    );
    println!("cargo:rustc-env=FOLD_SOURCE_FINGERPRINT_FILES={}", files.len());
}

/// Every crate reachable through `path = "..."` entries of `[dependencies]`
/// and `[target.'...'.dependencies]` tables, this crate included.
fn collect_path_dependencies(crate_dir: &Path, out: &mut BTreeSet<PathBuf>) {
    let canonical = match crate_dir.canonicalize() {
        Ok(path) => path,
        Err(_) => return,
    };
    if !out.insert(canonical.clone()) {
        return;
    }
    let manifest = match fs::read_to_string(canonical.join("Cargo.toml")) {
        Ok(text) => text,
        Err(_) => return,
    };
    let mut in_dependency_table = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dependency_table = trimmed == "[dependencies]"
                || (trimmed.starts_with("[target.") && trimmed.ends_with(".dependencies]"));
            continue;
        }
        if !in_dependency_table {
            continue;
        }
        if let Some(path) = path_value(trimmed) {
            collect_path_dependencies(&canonical.join(path), out);
        }
    }
}

/// The `path = "..."` value of one dependency line, if it carries one.
fn path_value(line: &str) -> Option<&str> {
    let start = line.find("path")?;
    let rest = &line[start + 4..];
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn collect_source_files(root: &Path, dir: &Path, out: &mut BTreeSet<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if matches!(
                name.as_str(),
                "target" | ".cargo-target" | "tests" | "benches" | "runs" | ".git"
            ) {
                continue;
            }
            collect_source_files(root, &path, out);
        } else if name.ends_with(".rs") || name == "Cargo.toml" {
            out.insert(path);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

// A self-contained SHA-256 so the build script needs no dependency; the value
// only has to be stable and collision-resistant, never interoperable.
struct Sha256 {
    state: [u32; 8],
    buffer: Vec<u8>,
    length: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c,
                0x1f83d9ab, 0x5be0cd19,
            ],
            buffer: Vec::new(),
            length: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.length += data.len() as u64;
        self.buffer.extend_from_slice(data);
        while self.buffer.len() >= 64 {
            let block: [u8; 64] = self.buffer[..64].try_into().expect("64 bytes");
            self.compress(&block);
            self.buffer.drain(..64);
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_length = self.length.wrapping_mul(8);
        self.buffer.push(0x80);
        while self.buffer.len() % 64 != 56 {
            self.buffer.push(0);
        }
        self.buffer.extend_from_slice(&bit_length.to_be_bytes());
        let blocks = std::mem::take(&mut self.buffer);
        for block in blocks.chunks_exact(64) {
            let block: [u8; 64] = block.try_into().expect("64 bytes");
            self.compress(&block);
        }
        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
            0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
            0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
            0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
            0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
            0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
            0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
            0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
            0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for (i, chunk) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes(chunk.try_into().expect("4 bytes"));
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in self.state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
}
