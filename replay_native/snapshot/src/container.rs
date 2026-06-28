//! The `.battleplan` container READER (HOI4REPLAY_CONTAINER_SPEC.md).
//!
//! Opens a portable recording, validates its manifest + integrity + identity, and
//! serves the snapshot series through the SAME shapes the directory store uses
//! ([`crate::Index`], [`crate::Snapshot`]), so the app opens a `.battleplan` with
//! zero changes elsewhere ([`crate::Store`] dispatches to this transparently).
//!
//! Verification is STAGED (spec §4): the manifest, its digest/signature, the path
//! rules, the zip-bomb caps, and `index.json`/`timeline.json` are verified up
//! front; each snapshot is hashed LAZILY on first read, so a multi-GB recording
//! opens fast (Codex d). No snapshot bytes are ever exposed before their entry
//! hash matches.

use crate::canon::manifest_canonical_bytes;
use crate::sha256::{to_hex, Sha256};
use crate::zip::{ZipArchive, ZipEntry};
use crate::{Index, Snapshot};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

pub const MAX_CONTAINER_VERSION: u64 = 1;
/// Zip-bomb structural caps (spec §4.1). Generous for a real campaign yet bounded.
const MAX_ENTRIES: usize = 2_000_000;
const MAX_ENTRY_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GiB per entry
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024 * 1024; // 64 GiB total

#[derive(Debug)]
pub enum ContainerError {
    Io(std::io::Error),
    Zip(crate::zip::ZipError),
    Json(serde_json::Error),
    /// A finalized manifest is missing — unfinalized / recovery-only file.
    NoManifest,
    /// A semantic/integrity/identity rejection, with a human-readable reason.
    Reject(String),
}

impl std::fmt::Display for ContainerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContainerError::Io(e) => write!(f, "io: {e}"),
            ContainerError::Zip(e) => write!(f, "container zip: {e}"),
            ContainerError::Json(e) => write!(f, "container json: {e}"),
            ContainerError::NoManifest => write!(
                f,
                "no manifest.json: unfinalized/recovery-only .battleplan (refused by the normal reader)"
            ),
            ContainerError::Reject(s) => write!(f, "container rejected: {s}"),
        }
    }
}
impl std::error::Error for ContainerError {}
impl From<std::io::Error> for ContainerError {
    fn from(e: std::io::Error) -> Self {
        ContainerError::Io(e)
    }
}
impl From<crate::zip::ZipError> for ContainerError {
    fn from(e: crate::zip::ZipError) -> Self {
        ContainerError::Zip(e)
    }
}
impl From<serde_json::Error> for ContainerError {
    fn from(e: serde_json::Error) -> Self {
        ContainerError::Json(e)
    }
}

/// What identity the caller expects (e.g. the reader-pinned build, the app's known
/// container version). All optional: `None` skips that check. The container always
/// self-checks its own digest + internal consistency regardless.
#[derive(Debug, Clone, Default)]
pub struct ExpectedIdentity {
    /// Reject if the recorded `build.exe_sha256` != this (the reader-pinned build).
    pub build_exe_sha256: Option<String>,
    /// Reject if the recorded `schema_version` != this.
    pub schema_version: Option<String>,
    /// Reject if the recorded `mod_identity.modset_id` != this.
    pub modset_id: Option<String>,
}

/// One verified entry, indexed by its container path.
struct EntryRec {
    zip: ZipEntry,
    expected_sha256: String,
    /// lazily flips true once the bytes have hashed-matched
    verified: bool,
}

/// Public metadata for one manifest-listed payload entry. The bytes are not
/// included here; callers must use [`Container::read_entry_bytes`] so integrity
/// verification stays centralized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerEntry {
    pub path: String,
    pub role: Option<String>,
    pub format: Option<String>,
    pub bytes: u64,
    pub sha256: String,
}

impl ContainerEntry {
    fn from_manifest(ent: &Value) -> Option<ContainerEntry> {
        Some(ContainerEntry {
            path: ent.get("path")?.as_str()?.to_string(),
            role: ent.get("role").and_then(Value::as_str).map(str::to_string),
            format: ent
                .get("format")
                .and_then(Value::as_str)
                .map(str::to_string),
            bytes: ent.get("bytes")?.as_u64()?,
            sha256: ent.get("sha256")?.as_str()?.to_string(),
        })
    }
}

/// An opened, manifest-verified `.battleplan`. Holds the archive in memory and the
/// per-entry expected hashes; snapshot bytes are verified lazily on read.
pub struct Container {
    archive: ZipArchive,
    manifest: Value,
    game_id: String,
    /// container-path -> entry record (post-verification of manifest↔zip agreement)
    by_path: Mutex<HashMap<String, EntryRec>>,
    /// the verified, parsed index (its `file` names are the only filename authority)
    index: Index,
    /// signed | unsigned | sig-invalid(rejected before construction)
    pub signed: bool,
    pub unsigned: bool,
}

impl Container {
    pub fn manifest(&self) -> &Value {
        &self.manifest
    }
    pub fn game_id(&self) -> &str {
        &self.game_id
    }
    pub fn index(&self) -> &Index {
        &self.index
    }
    /// Manifest-listed payload entries in manifest order.
    pub fn entries(&self) -> Vec<ContainerEntry> {
        self.manifest
            .get("entries")
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(ContainerEntry::from_manifest)
                    .collect()
            })
            .unwrap_or_default()
    }
    /// Manifest-listed payload entries with a specific role.
    pub fn entries_by_role(&self, role: &str) -> Vec<ContainerEntry> {
        self.entries()
            .into_iter()
            .filter(|entry| entry.role.as_deref() == Some(role))
            .collect()
    }

    /// Open + verify a `.battleplan` from a path. `expected` enforces identity
    /// (build/schema/mod) when its fields are set; pass `Default::default()` for
    /// integrity-only (still fully checksum-verified + consistency-checked).
    pub fn open<P: AsRef<Path>>(
        path: P,
        expected: &ExpectedIdentity,
    ) -> Result<Container, ContainerError> {
        let data = fs::read(path.as_ref())?;
        Self::from_bytes(data, expected)
    }

    /// As [`open`](Self::open) but from in-memory bytes (used by tests).
    pub fn from_bytes(
        data: Vec<u8>,
        expected: &ExpectedIdentity,
    ) -> Result<Container, ContainerError> {
        let archive = ZipArchive::parse(data)?;

        // structural zip-bomb caps (spec §4.1, step 9)
        if archive.entries().len() > MAX_ENTRIES {
            return Err(ContainerError::Reject(format!(
                "too many entries ({} > {})",
                archive.entries().len(),
                MAX_ENTRIES
            )));
        }
        let mut total: u64 = 0;
        for e in archive.entries() {
            if e.uncomp_size > MAX_ENTRY_BYTES {
                return Err(ContainerError::Reject(format!(
                    "entry {} exceeds per-entry cap",
                    e.name
                )));
            }
            total = total.saturating_add(e.uncomp_size);
        }
        if total > MAX_TOTAL_BYTES {
            return Err(ContainerError::Reject(
                "total uncompressed size exceeds cap".into(),
            ));
        }

        // (4) manifest present? absent => recovery-only, refused.
        let man_entry = archive
            .by_name("manifest.json")
            .ok_or(ContainerError::NoManifest)?
            .clone();
        let man_bytes = archive.read_entry(&man_entry)?;
        let manifest: Value = serde_json::from_slice(&man_bytes)?;

        // (5) format + version
        if manifest.get("format").and_then(Value::as_str) != Some("hoi4replay") {
            return Err(ContainerError::Reject(
                "manifest.format != \"hoi4replay\"".into(),
            ));
        }
        let cv = manifest
            .get("container_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| ContainerError::Reject("manifest.container_version missing".into()))?;
        if cv > MAX_CONTAINER_VERSION {
            return Err(ContainerError::Reject(format!(
                "container_version {cv} > reader max {MAX_CONTAINER_VERSION}"
            )));
        }

        // (6) required_features the reader must understand. FAIL-CLOSED on shape:
        // absent or JSON null = "none required" (allowed); a PRESENT value of any
        // other type, or any non-string element, is malformed and MUST be rejected
        // — otherwise a tampered manifest could smuggle an unknown required feature
        // past this gate by encoding it as a non-array (e.g. a bare string), which
        // `as_array` would silently treat as "no required features" (Codex b).
        match manifest.get("required_features") {
            None | Some(Value::Null) => {}
            Some(Value::Array(rf)) => {
                for feat in rf {
                    let name = feat.as_str().ok_or_else(|| {
                        ContainerError::Reject(
                            "manifest.required_features has a non-string element (malformed)"
                                .into(),
                        )
                    })?;
                    if !KNOWN_FEATURES.contains(&name) {
                        return Err(ContainerError::Reject(format!(
                            "unknown required feature {name:?} in required_features"
                        )));
                    }
                }
            }
            Some(_) => {
                return Err(ContainerError::Reject(
                    "manifest.required_features must be an array (malformed; fail-closed)".into(),
                ))
            }
        }

        // (7) manifest_digest over the canonical manifest (strips digest+signature)
        let declared_digest = manifest
            .get("manifest_digest")
            .and_then(Value::as_str)
            .ok_or_else(|| ContainerError::Reject("manifest.manifest_digest missing".into()))?
            .to_string();
        let recomputed = to_hex(&Sha256::digest(&manifest_canonical_bytes(&manifest)));
        if recomputed != declared_digest {
            return Err(ContainerError::Reject(format!(
                "manifest_digest mismatch (recomputed {recomputed}, declared {declared_digest})"
            )));
        }

        // (8) signature: present-and-valid => signed; present-and-invalid => reject;
        // absent => unsigned (allowed). v1 ships the verification SLOT; an actual
        // Ed25519 verify is wired when a key policy is configured (the digest is the
        // signed payload). Until then a NON-NULL signature with no verifier policy
        // is conservatively REJECTED rather than trusted (fail-closed).
        let sig = manifest.get("signature");
        let (signed, unsigned) = match sig {
            None | Some(Value::Null) => (false, true),
            Some(_) => {
                return Err(ContainerError::Reject(
                    "manifest.signature present but no signature-verification policy is \
                     configured in this reader build (fail-closed; spec §3.2)"
                        .into(),
                ))
            }
        };

        // identity expectations (spec §4.1 step optional / §2)
        let schema_version = manifest.get("schema_version").and_then(Value::as_str);
        if let Some(exp) = &expected.schema_version {
            if schema_version != Some(exp.as_str()) {
                return Err(ContainerError::Reject(format!(
                    "schema_version {:?} != expected {exp:?}",
                    schema_version
                )));
            }
        }
        let build_sha = manifest
            .get("build")
            .and_then(|b| b.get("exe_sha256"))
            .and_then(Value::as_str);
        if let Some(exp) = &expected.build_exe_sha256 {
            if build_sha != Some(exp.as_str()) {
                return Err(ContainerError::Reject(format!(
                    "build.exe_sha256 {:?} != expected {exp:?}",
                    build_sha
                )));
            }
        }
        let modset_id = manifest
            .get("mod_identity")
            .and_then(|m| m.get("modset_id"))
            .and_then(Value::as_str);
        if let Some(exp) = &expected.modset_id {
            if modset_id != Some(exp.as_str()) {
                return Err(ContainerError::Reject(format!(
                    "mod_identity.modset_id {:?} != expected {exp:?}",
                    modset_id
                )));
            }
        }

        let game_id = manifest
            .get("game_id")
            .and_then(Value::as_str)
            .ok_or_else(|| ContainerError::Reject("manifest.game_id missing".into()))?
            .to_string();

        // Build the path -> expected-hash map from manifest.entries, and confirm
        // every manifest entry actually exists in the zip with the declared size
        // (and vice-versa: a zip entry not in the manifest is a tamper signal).
        let entries = manifest
            .get("entries")
            .and_then(Value::as_array)
            .ok_or_else(|| ContainerError::Reject("manifest.entries missing".into()))?;
        let mut by_path: HashMap<String, EntryRec> = HashMap::new();
        // (filled below; later verify-marks need it mutable)
        for ent in entries {
            let p = ent
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| ContainerError::Reject("entry.path missing".into()))?;
            let sha = ent
                .get("sha256")
                .and_then(Value::as_str)
                .ok_or_else(|| ContainerError::Reject(format!("entry {p}: sha256 missing")))?;
            let declared_bytes = ent
                .get("bytes")
                .and_then(Value::as_u64)
                .ok_or_else(|| ContainerError::Reject(format!("entry {p}: bytes missing")))?;
            let ze = archive.by_name(p).ok_or_else(|| {
                ContainerError::Reject(format!("manifest entry {p} not present in zip"))
            })?;
            // declared bytes must equal the zip's uncompressed size (zip-bomb guard
            // + manifest-can't-lie-about-size). (Codex b)
            if ze.uncomp_size != declared_bytes {
                return Err(ContainerError::Reject(format!(
                    "entry {p}: manifest bytes {declared_bytes} != zip uncompressed {}",
                    ze.uncomp_size
                )));
            }
            if by_path
                .insert(
                    p.to_string(),
                    EntryRec {
                        zip: ze.clone(),
                        expected_sha256: sha.to_string(),
                        verified: false,
                    },
                )
                .is_some()
            {
                return Err(ContainerError::Reject(format!(
                    "duplicate manifest entry {p}"
                )));
            }
        }
        // every payload/* zip entry must be covered by the manifest (no stowaways)
        for ze in archive.entries() {
            if ze.name == "manifest.json" {
                continue;
            }
            if !by_path.contains_key(&ze.name) {
                return Err(ContainerError::Reject(format!(
                    "zip entry {} is not listed in the manifest (tamper signal)",
                    ze.name
                )));
            }
        }

        // (10) verify + parse index.json NOW (it is the spine the app reads first).
        // `by_path` is already `mut` (its verify-marks need it); verify_and_read
        // hashes the bytes against the manifest before returning them.
        let index_path = format!("payload/{game_id}/index.json");
        let index_bytes = verify_and_read(&archive, &mut by_path, &index_path)?;
        let index: Index = serde_json::from_slice(&index_bytes)?;
        // index.game_id must agree with the manifest game_id
        if index.game_id != game_id {
            return Err(ContainerError::Reject(format!(
                "index.game_id {:?} != manifest.game_id {game_id:?}",
                index.game_id
            )));
        }

        // (11) verify timeline.json now if it is declared
        let timeline_path = format!("payload/{game_id}/timeline.json");
        if by_path.contains_key(&timeline_path) {
            let _ = verify_and_read(&archive, &mut by_path, &timeline_path)?;
        }

        // (12) identity consistency: manifest.mod_identity.modset_id must match a
        // representative snapshot's provenance.mod.modset_id (a manifest must not be
        // able to lie about identity). We check the FIRST snapshot in the index.
        if let Some(first) = index.snapshots.first() {
            let snap_path = format!("payload/{game_id}/{}", first.file);
            let snap_bytes = verify_and_read(&archive, &mut by_path, &snap_path)?;
            let snap: Value = serde_json::from_slice(&snap_bytes)?;
            let snap_modset = snap
                .get("provenance")
                .and_then(|p| p.get("mod"))
                .and_then(|m| m.get("modset_id"))
                .and_then(Value::as_str);
            // Both null is fine (mod-set not read either place). If the manifest
            // claims a modset id, a snapshot that names a DIFFERENT one is a lie.
            if let (Some(m), Some(s)) = (modset_id, snap_modset) {
                if m != s {
                    return Err(ContainerError::Reject(format!(
                        "manifest modset_id {m:?} disagrees with snapshot provenance.mod {s:?}"
                    )));
                }
            }
        }

        Ok(Container {
            archive,
            manifest,
            game_id,
            by_path: Mutex::new(by_path),
            index,
            signed,
            unsigned,
        })
    }

    /// Load a snapshot by its index `file` name (the only filename authority).
    /// Verifies the entry's hash lazily; never exposes unverified bytes (Codex d).
    pub fn load_file(&self, file: &str) -> Result<Snapshot, ContainerError> {
        let path = format!("payload/{}/{}", self.game_id, file);
        let bytes = self.read_entry_bytes(&path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Read any manifest-listed payload entry as verified raw bytes. This is the
    /// generic path used for optional non-snapshot streams such as
    /// `collector_raw_stream`.
    pub fn read_entry_bytes(&self, path: &str) -> Result<Vec<u8>, ContainerError> {
        let mut guard = self.by_path.lock().expect("container lock");
        verify_and_read(&self.archive, &mut guard, path)
    }

    /// Load a snapshot by hour_index — resolved through the INDEX (`file` field),
    /// not by synthesizing a filename, so segment-prefixed rollback files load too.
    pub fn load(&self, hour_index: i64) -> Result<Snapshot, ContainerError> {
        let file = self
            .index
            .snapshots
            .iter()
            .find(|e| e.hour_index == Some(hour_index))
            .map(|e| e.file.clone())
            .ok_or_else(|| {
                ContainerError::Reject(format!("no snapshot for hour_index {hour_index}"))
            })?;
        self.load_file(&file)
    }

    /// Full background verification: hash EVERY declared entry. Returns Ok(()) when
    /// all match; the first mismatch is an error. (Optional; never gates open.)
    pub fn verify_all(&self) -> Result<(), ContainerError> {
        let mut guard = self.by_path.lock().expect("container lock");
        let paths: Vec<String> = guard.keys().cloned().collect();
        for p in paths {
            verify_and_read(&self.archive, &mut guard, &p)?;
        }
        Ok(())
    }
}

/// Features this reader build understands. A `required_features` entry not here is
/// a hard reject (forward-compat fail-closed; Codex b).
const KNOWN_FEATURES: &[&str] = &[];

/// Read an entry's bytes and verify them against the manifest hash, marking it
/// verified so a re-read is hash-free. Never returns bytes whose hash didn't match.
fn verify_and_read(
    archive: &ZipArchive,
    by_path: &mut HashMap<String, EntryRec>,
    path: &str,
) -> Result<Vec<u8>, ContainerError> {
    let rec = by_path
        .get(path)
        .ok_or_else(|| ContainerError::Reject(format!("entry {path} not in manifest")))?;
    let bytes = archive.read_entry(&rec.zip)?;
    let got = to_hex(&Sha256::digest(&bytes));
    if got != rec.expected_sha256 {
        return Err(ContainerError::Reject(format!(
            "entry {path}: sha256 {got} != manifest {}",
            rec.expected_sha256
        )));
    }
    // mark verified (cheap; lets a re-read skip nothing today but documents intent
    // and supports a future "skip if already verified" fast path)
    if let Some(rec) = by_path.get_mut(path) {
        rec.verified = true;
    }
    Ok(bytes)
}
