//! A reference `.battleplan` PACKER (writer side).
//!
//! NOTE ON OWNERSHIP: the *production* writer lives in the Python capture loop
//! (see `replay_ui/WRITER_INTEGRATION_PLAN.md`); this Rust packer is the
//! REFERENCE implementation + the test fixture builder. It is loss-LESS by
//! construction: payload files are stored as RAW BYTES, never re-serialized, so
//! the Absent/null/0 trichotomy, the rollback filenames, and compact-JSON
//! formatting all survive unchanged (spec §5).
//!
//! It also backs a tiny CLI (`src/bin/pack_replay.rs`) so a directory store can be
//! packed/round-tripped from the command line for verification.

use crate::canon::manifest_canonical_bytes;
use crate::sha256::{to_hex, Sha256};
use crate::zip::validate_path;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde_json::{json, Value};
use std::io::Write;

/// One file to pack: a container path + its RAW bytes + a role label.
pub struct PackEntry {
    pub path: String,
    pub bytes: Vec<u8>,
    pub role: &'static str, // "index" | "snapshot" | "timeline"
    pub hour_index: Option<i64>,
}

/// Identity recorded into the manifest (the values the capture read at session
/// start — all already redacted of personal paths upstream).
#[derive(Clone, Default)]
pub struct PackIdentity {
    pub schema_version: Option<String>,
    pub build_exe_sha256: Option<String>,
    pub build_pinned: bool,
    /// the whole provenance.mod object (or None = mod-set not read)
    pub mod_identity: Option<Value>,
    pub created_utc: Option<String>,
}

#[derive(Debug)]
pub enum PackError {
    BadPath(String),
    Io(std::io::Error),
}
impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PackError::BadPath(p) => write!(f, "pack: unsafe path {p}"),
            PackError::Io(e) => write!(f, "pack io: {e}"),
        }
    }
}
impl std::error::Error for PackError {}
impl From<std::io::Error> for PackError {
    fn from(e: std::io::Error) -> Self {
        PackError::Io(e)
    }
}

/// Knobs for building deliberately-malformed containers in tests (never used by
/// the production/CLI path). All default to "well-formed".
#[derive(Clone, Default)]
pub struct PackOptions {
    /// omit `manifest.json` entirely (-> the recovery-only / NoManifest case)
    pub omit_manifest: bool,
    /// inject a `required_features` the reader will not know (-> forward-compat reject)
    pub inject_required_feature: Option<String>,
    /// REPLACE the whole `required_features` field with an arbitrary JSON value
    /// (e.g. a STRING instead of an array) to exercise the reader's malformed-type
    /// fail-closed path. Takes precedence over `inject_required_feature`.
    pub required_features_raw: Option<Value>,
    /// append a zip entry that is NOT listed in the manifest (-> stowaway reject).
    /// `(path, bytes)`; the manifest digest stays valid because the entry never
    /// enters the manifest — exactly the tamper case the reader must catch.
    pub inject_stowaway: Option<(String, Vec<u8>)>,
}

/// Build a complete `.battleplan` (manifest written last) from the given entries.
/// `game_id` must be the game the entries belong to (their paths are
/// `payload/<game_id>/...`). `deflate` toggles per-entry compression (the reader
/// handles both; STORE keeps the test fully deterministic).
pub fn pack(
    game_id: &str,
    entries: &[PackEntry],
    identity: &PackIdentity,
    deflate: bool,
) -> Result<Vec<u8>, PackError> {
    pack_with(game_id, entries, identity, deflate, &PackOptions::default())
}

/// As [`pack`] but with [`PackOptions`] for negative-test fixtures.
pub fn pack_with(
    game_id: &str,
    entries: &[PackEntry],
    identity: &PackIdentity,
    deflate: bool,
    opts: &PackOptions,
) -> Result<Vec<u8>, PackError> {
    // --- manifest entries (sha + bytes), in canonical path order ---
    let mut manifest_entries: Vec<Value> = Vec::with_capacity(entries.len());
    let mut zip_files: Vec<ZipFile> = Vec::with_capacity(entries.len() + 1);

    let mut counts_snapshots = 0u64;
    for e in entries {
        validate_path(&e.path).map_err(|_| PackError::BadPath(e.path.clone()))?;
        let sha = to_hex(&Sha256::digest(&e.bytes));
        let mut ent = json!({
            "path": e.path,
            "sha256": sha,
            "bytes": e.bytes.len() as u64,
            "role": e.role,
        });
        if let Some(h) = e.hour_index {
            ent.as_object_mut()
                .unwrap()
                .insert("hour_index".into(), json!(h));
        }
        if e.role == "snapshot" {
            counts_snapshots += 1;
        }
        manifest_entries.push(ent);
        zip_files.push(ZipFile {
            name: e.path.clone(),
            data: e.bytes.clone(),
            deflate,
        });
    }
    // canonical path order so the manifest entry list is stable
    manifest_entries.sort_by(|a, b| {
        a["path"]
            .as_str()
            .unwrap_or("")
            .cmp(b["path"].as_str().unwrap_or(""))
    });

    // --- the manifest object (digest filled in below) ---
    let required_features = match (&opts.required_features_raw, &opts.inject_required_feature) {
        (Some(raw), _) => raw.clone(),
        (None, Some(f)) => json!([f]),
        (None, None) => json!([]),
    };
    let mut manifest = json!({
        "format": "hoi4replay",
        "container_version": 1,
        "required_features": required_features,
        "optional_features": [],
        "schema_version": identity.schema_version,
        "game_id": game_id,
        "build": {
            "exe_sha256": identity.build_exe_sha256,
            "pinned": identity.build_pinned,
        },
        "mod_identity": identity.mod_identity.clone().unwrap_or(Value::Null),
        "created_utc": identity.created_utc,
        "recorder": { "tool": "hoi4replay-recorder", "tool_version": "1.0.0" },
        "counts": { "snapshots": counts_snapshots },
        "entries": manifest_entries,
        "manifest_digest": Value::Null,
        "signature": Value::Null,
    });
    let digest = to_hex(&Sha256::digest(&manifest_canonical_bytes(&manifest)));
    manifest
        .as_object_mut()
        .unwrap()
        .insert("manifest_digest".into(), json!(digest));

    // inject a stowaway (a zip entry the manifest does not list) BEFORE the
    // manifest, so the reader's coverage check sees an uncovered entry.
    if let Some((path, data)) = &opts.inject_stowaway {
        zip_files.push(ZipFile {
            name: path.clone(),
            data: data.clone(),
            deflate: false,
        });
    }

    // manifest.json is written LAST (spec §6). The omit-manifest test fixture
    // produces a recovery-only file (no manifest) to exercise the NoManifest path.
    if !opts.omit_manifest {
        let manifest_bytes = serde_json::to_vec(&manifest).expect("manifest serializes");
        zip_files.push(ZipFile {
            name: "manifest.json".into(),
            data: manifest_bytes,
            deflate: false,
        });
    }

    Ok(write_zip(&zip_files)?)
}

// --------------------------------------------------------------------------
// minimal ZIP writer (STORE or DEFLATE; CRC32; no Zip64; deterministic)
// --------------------------------------------------------------------------
struct ZipFile {
    name: String,
    data: Vec<u8>,
    deflate: bool,
}

fn write_zip(files: &[ZipFile]) -> std::io::Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let mut offsets: Vec<u32> = Vec::with_capacity(files.len());

    for f in files {
        let crc = crc32fast::hash(&f.data);
        let (method, comp): (u16, Vec<u8>) = if f.deflate {
            let mut enc = DeflateEncoder::new(Vec::new(), Compression::default());
            enc.write_all(&f.data)?;
            (8, enc.finish()?)
        } else {
            (0, f.data.clone())
        };
        let lho = out.len() as u32;
        offsets.push(lho);

        // local file header
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes()); // sig
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time (deterministic 0)
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        out.extend_from_slice(&(f.data.len() as u32).to_le_bytes());
        out.extend_from_slice(&(f.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(f.name.as_bytes());
        out.extend_from_slice(&comp);

        // central directory header
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // time
        central.extend_from_slice(&0u16.to_le_bytes()); // date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(comp.len() as u32).to_le_bytes());
        central.extend_from_slice(&(f.data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(f.name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk no
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&lho.to_le_bytes());
        central.extend_from_slice(f.name.as_bytes());
    }

    let cd_off = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);

    // end of central directory
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // disk
    out.extend_from_slice(&0u16.to_le_bytes()); // cd disk
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&(files.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_off.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment len
    let _ = offsets;
    Ok(out)
}
