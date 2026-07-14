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
use crate::zip::{ZipArchive, ZipEntry, MAX_CONTAINER_ENTRY_BYTES, ZIP_METHOD_STORE};
use crate::{Index, SegmentGraph, Snapshot};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::sync::Mutex;

/// Highest container version this reader understands. Bumped 1 -> 2 for stitched
/// (reload/relaunch) archives, which carry the segment/timeline graph
/// (`STITCHING_DESIGN.md` §9). A reader still rejects any version ABOVE this, and
/// a pre-bump (v1-max) reader rejects a v2 stitched archive rather than silently
/// hiding its rollback graph.
pub const MAX_CONTAINER_VERSION: u64 = 2;
/// The container version at/above which the stitched segment feature is mandatory.
const STITCHED_CONTAINER_VERSION: u64 = 2;
/// The required feature a stitched (v2) container must declare (§9).
const FEATURE_TIMELINE_SEGMENTS_V1: &str = "timeline_segments_v1";
const STITCH_INDEX_SCHEMA: &str = "stitched-main-v1";
const STITCH_ENTRY_ORDER: &str = "lexicographic-path-manifest-last-v1";
/// Zip-bomb structural caps (spec §4.1). Generous for a real campaign yet bounded.
const MAX_ENTRIES: usize = 2_000_000;
const MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024 * 1024; // 64 GiB total

/// Canonical digest domain shared by the stitch planner and the v2 reader. The
/// serialized graph is authoritative; only the digest field itself is removed
/// from the preimage. This makes open-time recomputation independent of any
/// planner object while binding every serialized required input and campaign key.
pub fn canonical_stitch_input_digest(
    game_id: &str,
    graph: &Value,
    segment_indexes: &[Value],
) -> String {
    let mut serialized_graph = graph.clone();
    if let Some(object) = serialized_graph.as_object_mut() {
        object.remove("stitch_input_digest");
    }
    let preimage = json!({
        "game_id": game_id,
        "container_version": STITCHED_CONTAINER_VERSION,
        "required_feature": FEATURE_TIMELINE_SEGMENTS_V1,
        "index_schema": STITCH_INDEX_SCHEMA,
        "serialized_stitch_inputs": serialized_graph,
        "serialized_segment_indexes": segment_indexes,
    });
    let mut canonical = String::new();
    crate::canon::write_canonical(&preimage, &mut canonical);
    Sha256::hex(canonical.as_bytes())
}

/// Lexically canonicalize a Windows source path without touching the filesystem.
/// The stitcher records already-observed files, so filesystem canonicalization
/// would introduce time-of-check races and would fail for portable containers.
pub fn normalize_stitch_source_path(path: &str) -> Result<String, String> {
    if path.is_empty() || path.contains('\0') {
        return Err("source path is empty or contains NUL".into());
    }
    let mut normalized = path.replace('\\', "/").to_ascii_lowercase();
    if normalized.starts_with("//?/unc/") {
        normalized = format!("//{}", &normalized[8..]);
    } else if normalized.starts_with("//?/") {
        normalized = normalized[4..].to_string();
    }

    enum Prefix {
        Relative,
        Root,
        Drive(String),
        Unc,
    }
    let (prefix, rest, protected_components) = if normalized.starts_with("//") {
        (Prefix::Unc, normalized.trim_start_matches('/'), 2usize)
    } else if normalized.as_bytes().get(1) == Some(&b':') {
        if normalized.len() < 3 || normalized.as_bytes()[2] != b'/' {
            return Err("drive-relative source path is ambiguous".into());
        }
        (
            Prefix::Drive(normalized[..3].to_string()),
            &normalized[3..],
            0,
        )
    } else if let Some(rest) = normalized.strip_prefix('/') {
        (Prefix::Root, rest, 0)
    } else {
        (Prefix::Relative, normalized.as_str(), 0)
    };

    let mut components: Vec<&str> = Vec::new();
    for component in rest.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.len() <= protected_components {
                    return Err("source path escapes its lexical root".into());
                }
                components.pop();
            }
            value => components.push(value),
        }
    }
    if matches!(prefix, Prefix::Unc) && components.len() < 2 {
        return Err("UNC source path lacks server/share".into());
    }
    if components.is_empty() && matches!(prefix, Prefix::Relative) {
        return Err("source path has no canonical components".into());
    }
    let joined = components.join("/");
    Ok(match prefix {
        Prefix::Relative => joined,
        Prefix::Root => format!("/{joined}"),
        Prefix::Drive(drive) => format!("{drive}{joined}"),
        Prefix::Unc => format!("//{joined}"),
    })
}

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
        Self::open_strict(path, expected)
    }

    /// Read the container once and strictly validate the exact manifest buffer
    /// that constructs the in-memory archive.
    pub fn open_strict<P: AsRef<Path>>(
        path: P,
        expected: &ExpectedIdentity,
    ) -> Result<Container, ContainerError> {
        let data = std::fs::read(path.as_ref())?;
        Self::from_bytes_strict(data, expected)
    }

    /// As [`open`](Self::open) but from in-memory bytes (used by tests).
    pub fn from_bytes(
        data: Vec<u8>,
        expected: &ExpectedIdentity,
    ) -> Result<Container, ContainerError> {
        Self::from_bytes_strict(data, expected)
    }

    /// Strict in-memory open. The strict pass returns the semantic value, so
    /// man_bytes is never decoded ordinarily before or after validation.
    pub fn from_bytes_strict(
        data: Vec<u8>,
        expected: &ExpectedIdentity,
    ) -> Result<Container, ContainerError> {
        let archive = ZipArchive::parse(data)?;
        // Validate the physical half of every ZIP entry before reading even the
        // manifest payload. This covers v2 entries that semantic validation does
        // not otherwise materialize, including embedded raw sources.
        archive.validate_local_headers()?;

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
            if e.uncomp_size > MAX_CONTAINER_ENTRY_BYTES {
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

        // Re-derive every payload CRC before decoding the manifest or trusting
        // any field inside it. The manifest is attacker-controlled too, so its
        // version/features must never be able to select or skip this pass.
        archive.validate_payloads()?;

        // (4) manifest present? absent => recovery-only, refused.
        let man_entry = archive
            .by_name("manifest.json")
            .ok_or(ContainerError::NoManifest)?
            .clone();
        let man_bytes = archive.read_entry(&man_entry)?;
        let manifest = crate::strict_json::from_slice(&man_bytes)?;
        validate_strict_manifest(&manifest)?;

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
        // (collect the declared feature names first so the version<->feature
        // coupling below can be checked.)
        let mut declared_features: Vec<String> = Vec::new();
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
                    declared_features.push(name.to_string());
                }
            }
            Some(_) => {
                return Err(ContainerError::Reject(
                    "manifest.required_features must be an array (malformed; fail-closed)".into(),
                ));
            }
        }

        // (6b) version<->feature coupling (§9): a stitched container declares
        // container_version 2 AND required feature `timeline_segments_v1`. A v2+
        // container WITHOUT that feature is malformed — reject fail-closed so a
        // tampered manifest cannot advertise the segment format while omitting the
        // flag old readers key on.
        if cv >= STITCHED_CONTAINER_VERSION
            && !declared_features
                .iter()
                .any(|f| f == FEATURE_TIMELINE_SEGMENTS_V1)
        {
            return Err(ContainerError::Reject(format!(
                "container_version {cv} must declare required feature \
                 {FEATURE_TIMELINE_SEGMENTS_V1:?} (stitched-format coupling; §9)"
            )));
        }
        if cv < STITCHED_CONTAINER_VERSION
            && declared_features
                .iter()
                .any(|f| f == FEATURE_TIMELINE_SEGMENTS_V1)
        {
            return Err(ContainerError::Reject(format!(
                "required feature {FEATURE_TIMELINE_SEGMENTS_V1:?} requires container_version {STITCHED_CONTAINER_VERSION}"
            )));
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
                ));
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
        // Strict (duplicate-key-rejecting) decode per replay-loader hardening,
        // while retaining the parsed Value that stitched-v2 validation consumes.
        let index_value: Value = crate::parse_json_strict(&index_bytes)?;
        let index: Index = serde_json::from_value(index_value.clone())?;
        // index.game_id must agree with the manifest game_id
        if index.game_id != game_id {
            return Err(ContainerError::Reject(format!(
                "index.game_id {:?} != manifest.game_id {game_id:?}",
                index.game_id
            )));
        }

        if cv == STITCHED_CONTAINER_VERSION {
            validate_stitched_v2(&archive, &mut by_path, &manifest, &game_id, &index_value)?;
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
            let snap: Value = crate::strict_json::from_slice(&snap_bytes)?;
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
        Ok(crate::decode_json_strict(&bytes)?)
    }

    /// Read any manifest-listed payload entry as verified raw bytes. This is the
    /// generic path used for optional non-snapshot streams such as
    /// `collector_raw_stream`.
    pub fn read_entry_bytes(&self, path: &str) -> Result<Vec<u8>, ContainerError> {
        let mut guard = self.by_path.lock().expect("container lock");
        verify_and_read(&self.archive, &mut guard, path)
    }

    /// The stitched branch graph (`segments.json`), verified and parsed, or `None`
    /// when this is a plain (non-stitched) container. The graph is the
    /// authoritative rollback/branch map (`STITCHING_DESIGN.md` §7.2); the main
    /// `index()` remains the default-playback view.
    pub fn segments(&self) -> Result<Option<SegmentGraph>, ContainerError> {
        let path = format!("payload/{}/segments.json", self.game_id);
        {
            let guard = self.by_path.lock().expect("container lock");
            if !guard.contains_key(&path) {
                return Ok(None);
            }
        }
        let bytes = self.read_entry_bytes(&path)?;
        Ok(Some(serde_json::from_slice(&bytes)?))
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

/// Strict states that ContainerEntry's permissive projection cannot express.
/// This runs before format, digest, identity, or role-table semantics.
fn validate_strict_manifest(manifest: &Value) -> Result<(), ContainerError> {
    let game_id = manifest.get("game_id").and_then(Value::as_str);
    let timeline_path = game_id.map(|game| format!("payload/{game}/timeline.json"));
    let Some(entries) = manifest.get("entries").and_then(Value::as_array) else {
        return Ok(());
    };

    for (index, entry) in entries.iter().enumerate() {
        let path = entry.get("path").and_then(Value::as_str);
        if let Some(role) = entry.get("role") {
            if !role.is_string() {
                return Err(ContainerError::Reject(format!(
                    "manifest entry {} role is present but is not a string",
                    path.map(|value| format!("{value:?}"))
                        .unwrap_or_else(|| format!("at index {index}"))
                )));
            }
        }
        if let (Some(path), Some(canonical)) = (path, timeline_path.as_deref()) {
            if path != canonical && path.eq_ignore_ascii_case(canonical) {
                return Err(ContainerError::Reject(format!(
                    "manifest entry path {path:?} is an ASCII-case lookalike of canonical timeline {canonical:?}"
                )));
            }
        }
    }
    Ok(())
}

/// Features this reader build understands. A `required_features` entry not here is
/// a hard reject (forward-compat fail-closed; Codex b).
const KNOWN_FEATURES: &[&str] = &[FEATURE_TIMELINE_SEGMENTS_V1];

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedHour {
    segment_id: u32,
    kind: String,
    file: Option<String>,
    sha256: Option<String>,
    reason: Option<String>,
    evidence_id: Option<String>,
    ownership: Value,
}

#[derive(Debug, Clone)]
struct ValidatedParticipation {
    campaign_key_digest: String,
    scheme_major: u32,
    identity_evidence_digest: String,
    play_order: u64,
    session_nonce: String,
    process_start_identity: String,
    record_seq_bounds: (u64, u64),
    byte_bounds: (u64, u64),
}

#[derive(Debug, Clone)]
struct ValidatedRawSource {
    session_nonce: String,
    process_start_identity: String,
    session_path: String,
    stored_digest: String,
    decoded_digest: String,
    record_seq_bounds: (u64, u64),
    byte_bounds: (u64, u64),
    participations: HashMap<String, ValidatedParticipation>,
}

fn reject(message: impl Into<String>) -> ContainerError {
    ContainerError::Reject(message.into())
}

fn required_object<'a>(
    value: &'a Value,
    context: &str,
) -> Result<&'a serde_json::Map<String, Value>, ContainerError> {
    value
        .as_object()
        .ok_or_else(|| reject(format!("{context} must be an object")))
}

fn required_array<'a>(
    value: &'a Value,
    key: &str,
    context: &str,
) -> Result<&'a Vec<Value>, ContainerError> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| reject(format!("{context}.{key} must be an array")))
}

fn required_i64(value: &Value, key: &str, context: &str) -> Result<i64, ContainerError> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| reject(format!("{context}.{key} must be an integer")))
}

fn required_u32(value: &Value, key: &str, context: &str) -> Result<u32, ContainerError> {
    let raw = value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| reject(format!("{context}.{key} must be an unsigned integer")))?;
    u32::try_from(raw).map_err(|_| reject(format!("{context}.{key} exceeds u32")))
}

fn required_u64(value: &Value, key: &str, context: &str) -> Result<u64, ContainerError> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| reject(format!("{context}.{key} must be an unsigned integer")))
}

fn required_string<'a>(
    value: &'a Value,
    key: &str,
    context: &str,
) -> Result<&'a str, ContainerError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| reject(format!("{context}.{key} must be a non-empty string")))
}

fn optional_i64(value: &Value, key: &str, context: &str) -> Result<Option<i64>, ContainerError> {
    match value.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| reject(format!("{context}.{key} must be integer or null"))),
    }
}

fn optional_u32(value: &Value, key: &str, context: &str) -> Result<Option<u32>, ContainerError> {
    match value.get(key) {
        Some(Value::Null) | None => Ok(None),
        Some(v) => {
            let raw = v
                .as_u64()
                .ok_or_else(|| reject(format!("{context}.{key} must be unsigned or null")))?;
            Ok(Some(u32::try_from(raw).map_err(|_| {
                reject(format!("{context}.{key} exceeds u32"))
            })?))
        }
    }
}

fn required_u64_pair(
    value: &Value,
    key: &str,
    context: &str,
) -> Result<(u64, u64), ContainerError> {
    let pair = required_array(value, key, context)?;
    if pair.len() != 2 {
        return Err(reject(format!(
            "{context}.{key} must contain exactly two bounds"
        )));
    }
    let start = pair[0]
        .as_u64()
        .ok_or_else(|| reject(format!("{context}.{key}[0] must be unsigned")))?;
    let end = pair[1]
        .as_u64()
        .ok_or_else(|| reject(format!("{context}.{key}[1] must be unsigned")))?;
    Ok((start, end))
}

fn required_sha256(value: &Value, key: &str, context: &str) -> Result<String, ContainerError> {
    let digest = required_string(value, key, context)?;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(reject(format!("{context}.{key} must be 64 hex characters")));
    }
    Ok(digest.to_string())
}

fn required_hex_bytes(value: &Value, key: &str, context: &str) -> Result<Vec<u8>, ContainerError> {
    let encoded = required_string(value, key, context)?;
    if encoded.len() % 2 != 0
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(reject(format!(
            "{context}.{key} must be canonical lowercase even-length hex"
        )));
    }
    encoded
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |byte: u8| match byte {
                b'0'..=b'9' => byte - b'0',
                b'a'..=b'f' => byte - b'a' + 10,
                _ => unreachable!("hex shape validated"),
            };
            Ok((nibble(pair[0]) << 4) | nibble(pair[1]))
        })
        .collect()
}

fn raw_source_key(session: &str, process: &str, stored: &str, decoded: &str) -> String {
    let mut bytes = Vec::new();
    for value in [session, process, stored, decoded] {
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
    Sha256::hex(&bytes)
}

fn parse_head(value: &Value, context: &str) -> Result<Option<(u32, i64)>, ContainerError> {
    if value.is_null() {
        return Ok(None);
    }
    required_object(value, context)?;
    Ok(Some((
        required_u32(value, "segment_id", context)?,
        required_i64(value, "hour_index", context)?,
    )))
}

fn parse_disposition(
    row: &Value,
    segment_id: u32,
    game_id: &str,
    by_path: &HashMap<String, EntryRec>,
    context: &str,
    allow_snapshot: bool,
) -> Result<ValidatedHour, ContainerError> {
    required_object(row, context)?;
    let kind = required_string(row, "kind", context)?;
    match kind {
        "snapshot" if allow_snapshot => {
            let file = required_string(row, "file", context)?.to_string();
            let sha = required_sha256(row, "sha256", context)?;
            let path = format!("payload/{game_id}/{file}");
            let entry = by_path
                .get(&path)
                .ok_or_else(|| reject(format!("{context} references missing snapshot {file:?}")))?;
            if entry.expected_sha256 != sha {
                return Err(reject(format!(
                    "{context} graph/index/hash disagreement for {file:?}"
                )));
            }
            Ok(ValidatedHour {
                segment_id,
                kind: kind.into(),
                file: Some(file),
                sha256: Some(sha),
                reason: None,
                evidence_id: None,
                ownership: Value::Null,
            })
        }
        "snapshot" => Err(reject(format!(
            "{context} transition coverage cannot be snapshot"
        ))),
        "capture_failed" | "coalesced" | "gap" => Ok(ValidatedHour {
            segment_id,
            kind: kind.into(),
            file: None,
            sha256: None,
            reason: Some(required_string(row, "reason", context)?.to_string()),
            evidence_id: Some(required_string(row, "evidence_id", context)?.to_string()),
            ownership: Value::Null,
        }),
        other => Err(reject(format!(
            "{context} has unknown coverage discriminant {other:?}"
        ))),
    }
}

fn insert_unique(
    map: &mut BTreeMap<i64, ValidatedHour>,
    hour: i64,
    value: ValidatedHour,
    label: &str,
) -> Result<(), ContainerError> {
    if map.insert(hour, value).is_some() {
        return Err(reject(format!("duplicate visible hour {hour} in {label}")));
    }
    Ok(())
}

fn validate_owner(
    row: &Value,
    campaign_key_digest: &str,
    play_order: u64,
    session_nonce: &str,
    process_start_identity: &str,
    in_session_ordinal: u32,
    epoch_token: &str,
    context: &str,
) -> Result<Value, ContainerError> {
    let owner = row
        .get("owner")
        .ok_or_else(|| reject(format!("{context}.owner is mandatory")))?;
    required_object(owner, &format!("{context}.owner"))?;
    let owner_context = format!("{context}.owner");
    let actual_campaign = required_sha256(owner, "campaign_key_digest", &owner_context)?;
    if actual_campaign != campaign_key_digest
        || required_u64(owner, "play_order", &owner_context)? != play_order
        || required_string(owner, "session_nonce", &owner_context)? != session_nonce
        || required_string(owner, "process_start_identity", &owner_context)?
            != process_start_identity
        || required_u32(owner, "in_session_ordinal", &owner_context)? != in_session_ordinal
        || required_string(owner, "epoch_token", &owner_context)? != epoch_token
    {
        return Err(reject(format!(
            "{context}.owner disagrees with exact segment/ticket identity"
        )));
    }
    Ok(owner.clone())
}

fn selected_ownership_value(row: &Value, context: &str) -> Result<Value, ContainerError> {
    let owner = row
        .get("owner")
        .ok_or_else(|| reject(format!("{context}.owner is mandatory")))?
        .clone();
    required_object(&owner, &format!("{context}.owner"))?;
    let selected = row.get("selected_observation_id");
    if selected.is_none() {
        return Ok(json!({"owner": owner}));
    }
    if row.get("selection_rule").and_then(Value::as_str) != Some("greatest_record_sequence_v1") {
        return Err(reject(format!(
            "{context}.selection_rule is unknown/missing"
        )));
    }
    let observation_id = required_string(row, "selected_observation_id", context)?;
    let record_bounds = required_u64_pair(row, "source_record_seq_bounds", context)?;
    let byte_bounds = required_u64_pair(row, "source_byte_bounds", context)?;
    if record_bounds.0 > record_bounds.1 || byte_bounds.0 >= byte_bounds.1 {
        return Err(reject(format!(
            "{context} selected observation bounds are invalid"
        )));
    }
    Ok(json!({
        "owner": owner,
        "selection_rule": "greatest_record_sequence_v1",
        "selected_observation_id": observation_id,
        "source_record_seq_bounds": [record_bounds.0, record_bounds.1],
        "source_byte_bounds": [byte_bounds.0, byte_bounds.1],
    }))
}

fn validate_observed_hour(
    row: &Value,
    disposition: &mut ValidatedHour,
    campaign_key_digest: &str,
    play_order: u64,
    session_nonce: &str,
    process_start_identity: &str,
    in_session_ordinal: u32,
    epoch_token: &str,
    segment_record_bounds: (u64, u64),
    segment_byte_bounds: (u64, u64),
    all_observation_bounds: &mut Vec<((u64, u64), (u64, u64), i64, String)>,
    context: &str,
) -> Result<(), ContainerError> {
    let count = required_u64(row, "observation_count", context)?;
    let observations = required_array(row, "observations", context)?;
    if count == 0 || count as usize != observations.len() {
        return Err(reject(format!(
            "{context}.observation_count disagrees with retained observations"
        )));
    }
    if row.get("selection_rule").and_then(Value::as_str) != Some("greatest_record_sequence_v1") {
        return Err(reject(format!(
            "{context}.selection_rule is unknown/missing"
        )));
    }
    let selected_id = required_string(row, "selected_observation_id", context)?;
    let top_owner = validate_owner(
        row,
        campaign_key_digest,
        play_order,
        session_nonce,
        process_start_identity,
        in_session_ordinal,
        epoch_token,
        context,
    )?;
    let top_record = required_u64_pair(row, "source_record_seq_bounds", context)?;
    let top_byte = required_u64_pair(row, "source_byte_bounds", context)?;
    let mut previous_record_end = None;
    let mut previous_byte_end = None;
    let mut ids = BTreeSet::new();
    let mut selected_signature = None;
    for (ordinal, observation) in observations.iter().enumerate() {
        let observation_context = format!("{context}.observations[{ordinal}]");
        required_object(observation, &observation_context)?;
        let observation_id = required_string(observation, "observation_id", &observation_context)?;
        if !ids.insert(observation_id.to_string()) {
            return Err(reject(format!(
                "{context} has duplicate observation evidence"
            )));
        }
        let owner = validate_owner(
            observation,
            campaign_key_digest,
            play_order,
            session_nonce,
            process_start_identity,
            in_session_ordinal,
            epoch_token,
            &observation_context,
        )?;
        let record_bounds =
            required_u64_pair(observation, "record_seq_bounds", &observation_context)?;
        let byte_bounds = required_u64_pair(observation, "byte_bounds", &observation_context)?;
        if record_bounds.0 > record_bounds.1
            || byte_bounds.0 >= byte_bounds.1
            || record_bounds.0 < segment_record_bounds.0
            || record_bounds.1 > segment_record_bounds.1
            || byte_bounds.0 < segment_byte_bounds.0
            || byte_bounds.1 > segment_byte_bounds.1
            || previous_record_end.is_some_and(|end| record_bounds.0 <= end)
            || previous_byte_end.is_some_and(|end| byte_bounds.0 < end)
        {
            return Err(reject(format!(
                "{context} observations have ambiguous/overlapping source order"
            )));
        }
        previous_record_end = Some(record_bounds.1);
        previous_byte_end = Some(byte_bounds.1);
        all_observation_bounds.push((
            record_bounds,
            byte_bounds,
            required_i64(row, "hour_index", context)?,
            observation_id.to_string(),
        ));
        let kind = required_string(observation, "kind", &observation_context)?;
        let signature = match kind {
            "snapshot" => json!({
                "kind": kind,
                "sha256": required_sha256(observation, "sha256", &observation_context)?,
            }),
            "capture_failed" | "coalesced" | "gap" => {
                let evidence = required_string(observation, "evidence_id", &observation_context)?;
                if evidence != observation_id {
                    return Err(reject(format!(
                        "{observation_context} disposition evidence disagrees with observation id"
                    )));
                }
                json!({
                    "kind": kind,
                    "reason": required_string(observation, "reason", &observation_context)?,
                    "evidence_id": evidence,
                })
            }
            other => {
                return Err(reject(format!(
                    "{observation_context} has unknown coverage discriminant {other:?}"
                )));
            }
        };
        selected_signature = Some((
            observation_id.to_string(),
            owner,
            record_bounds,
            byte_bounds,
            signature,
        ));
    }
    let (last_id, last_owner, last_record, last_byte, signature) = selected_signature.unwrap();
    let top_signature = match disposition.kind.as_str() {
        "snapshot" => json!({"kind":"snapshot", "sha256": disposition.sha256}),
        _ => json!({
            "kind": disposition.kind,
            "reason": disposition.reason,
            "evidence_id": disposition.evidence_id,
        }),
    };
    if selected_id != last_id
        || top_owner != last_owner
        || top_record != last_record
        || top_byte != last_byte
        || top_signature != signature
    {
        return Err(reject(format!(
            "{context} selected observation is not the last complete source-sequence observation"
        )));
    }
    disposition.ownership = selected_ownership_value(row, context)?;
    Ok(())
}

fn validate_raw_sources(
    graph: &Value,
    campaign_key_digest: &str,
    game_id: &str,
    by_path: &HashMap<String, EntryRec>,
) -> Result<HashMap<String, ValidatedRawSource>, ContainerError> {
    let rows = required_array(graph, "raw_sources", "segments.json")?;
    if rows.is_empty() {
        return Err(reject("segments.json.raw_sources cannot be empty"));
    }
    let mut sources = HashMap::new();
    let mut source_paths = HashMap::new();
    for (ordinal, row) in rows.iter().enumerate() {
        let context = format!("segments.json.raw_sources[{ordinal}]");
        required_object(row, &context)?;
        let source_key = required_sha256(row, "source_key", &context)?;
        let session_nonce = required_string(row, "session_nonce", &context)?.to_string();
        let process_start_identity =
            required_string(row, "process_start_identity", &context)?.to_string();
        let session_path = required_string(row, "session_path", &context)?.to_string();
        let canonical_path = normalize_stitch_source_path(&session_path).map_err(|reason| {
            reject(format!(
                "{context}.session_path is not canonicalizable: {reason}"
            ))
        })?;
        let stored_digest = required_sha256(row, "stored_digest", &context)?;
        let decoded_digest = required_sha256(row, "decoded_digest", &context)?;
        if source_key
            != raw_source_key(
                &session_nonce,
                &process_start_identity,
                &stored_digest,
                &decoded_digest,
            )
        {
            return Err(reject(format!(
                "{context}.source_key disagrees with session/process/digests"
            )));
        }
        if let Some(previous_key) = source_paths.insert(canonical_path.clone(), source_key.clone())
        {
            if previous_key != source_key {
                return Err(reject(format!(
                    "{context}.session_path aliases distinct raw source identities"
                )));
            }
        }
        let source_record = required_u64_pair(row, "record_seq_bounds", &context)?;
        let source_byte = required_u64_pair(row, "byte_bounds", &context)?;
        if source_record.0 > source_record.1 || source_byte.0 >= source_byte.1 {
            return Err(reject(format!("{context} has invalid source bounds")));
        }
        let manifest_path = format!("payload/{game_id}/{session_path}");
        let storage = row
            .get("storage")
            .ok_or_else(|| reject(format!("{context}.storage is mandatory")))?;
        required_object(storage, &format!("{context}.storage"))?;
        let storage_kind = required_string(storage, "kind", &format!("{context}.storage"))?;
        let embedded_owner = match storage_kind {
            "standalone" => None,
            "embedded_campaign" => Some(required_sha256(
                storage,
                "campaign_key_digest",
                &format!("{context}.storage"),
            )?),
            other => {
                return Err(reject(format!(
                    "{context}.storage has unknown kind {other:?}"
                )));
            }
        };
        if embedded_owner
            .as_ref()
            .is_some_and(|owner| owner != campaign_key_digest)
        {
            return Err(reject(format!(
                "{context} embedded owner disagrees with stitched campaign"
            )));
        }
        match storage_kind {
            "standalone" => {
                if by_path.contains_key(&manifest_path) {
                    return Err(reject(format!(
                        "{context} standalone raw source must not be embedded in a campaign container"
                    )));
                }
            }
            "embedded_campaign" => {
                let entry = by_path.get(&manifest_path).ok_or_else(|| {
                    reject(format!("{context} references missing embedded raw source"))
                })?;
                if entry.expected_sha256 != stored_digest {
                    return Err(reject(format!(
                        "{context} raw-source stored digest disagrees with manifest"
                    )));
                }
            }
            _ => unreachable!("storage kind validated above"),
        }

        let participation_rows = required_array(row, "participations", &context)?;
        if participation_rows.is_empty() {
            return Err(reject(format!("{context}.participations cannot be empty")));
        }
        let mut participations = HashMap::new();
        let mut previous_record_end = None;
        let mut previous_byte_end = None;
        for (participation_ordinal, participation) in participation_rows.iter().enumerate() {
            let participation_context =
                format!("{context}.participations[{participation_ordinal}]");
            required_object(participation, &participation_context)?;
            let participation_id =
                required_sha256(participation, "participation_id", &participation_context)?;
            let mut participation_binding = participation.clone();
            participation_binding
                .as_object_mut()
                .expect("object validated above")
                .remove("participation_id");
            let mut canonical_binding = String::new();
            crate::canon::write_canonical(&participation_binding, &mut canonical_binding);
            if Sha256::hex(canonical_binding.as_bytes()) != participation_id {
                return Err(reject(format!(
                    "{participation_context}.participation_id disagrees with exact binding"
                )));
            }
            let ticket = participation
                .get("ticket")
                .ok_or_else(|| reject(format!("{participation_context}.ticket is mandatory")))?;
            required_object(ticket, &format!("{participation_context}.ticket"))?;
            let ticket_context = format!("{participation_context}.ticket");
            let participation_campaign =
                required_sha256(ticket, "campaign_key_digest", &ticket_context)?;
            let play_order = required_u64(ticket, "play_order", &ticket_context)?;
            let ticket_session =
                required_string(ticket, "session_nonce", &ticket_context)?.to_string();
            let ticket_process =
                required_string(ticket, "process_start_identity", &ticket_context)?.to_string();
            if ticket_session != session_nonce || ticket_process != process_start_identity {
                return Err(reject(format!(
                    "{participation_context} has foreign session/process ownership"
                )));
            }
            let scheme_major = required_u32(participation, "scheme_major", &participation_context)?;
            let identity_evidence_digest = required_sha256(
                participation,
                "identity_evidence_digest",
                &participation_context,
            )?;
            let record_bounds =
                required_u64_pair(participation, "record_seq_bounds", &participation_context)?;
            let byte_bounds =
                required_u64_pair(participation, "byte_bounds", &participation_context)?;
            if record_bounds.0 > record_bounds.1
                || byte_bounds.0 >= byte_bounds.1
                || record_bounds.0 < source_record.0
                || record_bounds.1 > source_record.1
                || byte_bounds.0 < source_byte.0
                || byte_bounds.1 > source_byte.1
                || previous_record_end.is_some_and(|end| record_bounds.0 <= end)
                || previous_byte_end.is_some_and(|end| byte_bounds.0 < end)
            {
                return Err(reject(format!(
                    "{participation_context} has overlapping/invalid source bounds"
                )));
            }
            if embedded_owner
                .as_ref()
                .is_some_and(|owner| owner != &participation_campaign)
            {
                return Err(reject(format!(
                    "{context} embeds cross-campaign participation"
                )));
            }
            previous_record_end = Some(record_bounds.1);
            previous_byte_end = Some(byte_bounds.1);
            if participations
                .insert(
                    participation_id.clone(),
                    ValidatedParticipation {
                        campaign_key_digest: participation_campaign,
                        scheme_major,
                        identity_evidence_digest,
                        play_order,
                        session_nonce: ticket_session,
                        process_start_identity: ticket_process,
                        record_seq_bounds: record_bounds,
                        byte_bounds,
                    },
                )
                .is_some()
            {
                return Err(reject(format!("{context} has duplicate participation ids")));
            }
        }
        if sources
            .insert(
                source_key,
                ValidatedRawSource {
                    session_nonce,
                    process_start_identity,
                    session_path: canonical_path,
                    stored_digest,
                    decoded_digest,
                    record_seq_bounds: source_record,
                    byte_bounds: source_byte,
                    participations,
                },
            )
            .is_some()
        {
            return Err(reject("segments.json has duplicate raw source keys"));
        }
    }
    Ok(sources)
}

fn validate_stitched_v2(
    archive: &ZipArchive,
    by_path: &mut HashMap<String, EntryRec>,
    manifest: &Value,
    game_id: &str,
    index: &Value,
) -> Result<(), ContainerError> {
    required_object(index, "index")?;
    if index.get("index_schema").and_then(Value::as_str) != Some("stitched-main-v1") {
        return Err(reject("v2 index.index_schema must be \"stitched-main-v1\""));
    }
    if index.get("segments_file").and_then(Value::as_str) != Some("segments.json") {
        return Err(reject("v2 index.segments_file must be \"segments.json\""));
    }

    let graph_path = format!("payload/{game_id}/segments.json");
    if !by_path.contains_key(&graph_path) {
        return Err(reject("v2 container is missing segments.json"));
    }
    let graph_bytes = verify_and_read(archive, by_path, &graph_path)?;
    let graph: Value = serde_json::from_slice(&graph_bytes)?;
    required_object(&graph, "segments.json")?;
    let declared_stitch_digest = required_sha256(&graph, "stitch_input_digest", "segments.json")?;
    let declared_ledger_digest =
        required_sha256(&graph, "play_order_ledger_digest", "segments.json")?;
    for key in [
        "fold_version",
        "schema_version",
        "canonical_writer_version",
        "packer_codec_version",
    ] {
        required_string(&graph, key, "segments.json")?;
    }
    let publication = graph
        .get("publication")
        .ok_or_else(|| reject("segments.json.publication is mandatory"))?;
    required_object(publication, "segments.json.publication")?;
    let declared_created =
        required_string(publication, "created_utc", "segments.json.publication")?;
    let declared_compression = required_string(
        publication,
        "compression_method",
        "segments.json.publication",
    )?;
    let declared_entry_order =
        required_string(publication, "entry_order", "segments.json.publication")?;
    let compression_parameters = publication
        .get("compression_parameters")
        .and_then(Value::as_object)
        .filter(|parameters| {
            !parameters.is_empty()
                && parameters.iter().all(|(key, value)| {
                    !key.is_empty() && value.as_str().is_some_and(|v| !v.is_empty())
                })
        })
        .ok_or_else(|| {
            reject("segments.json.publication.compression_parameters is missing/invalid")
        })?;
    let manifest_created = required_string(manifest, "created_utc", "manifest")?;
    if declared_created != manifest_created {
        return Err(reject(
            "segments.json.publication.created_utc disagrees with manifest.created_utc",
        ));
    }
    if graph.get("schema_version").and_then(Value::as_str)
        != manifest.get("schema_version").and_then(Value::as_str)
    {
        return Err(reject(
            "segments.json.schema_version disagrees with manifest.schema_version",
        ));
    }
    if declared_compression != "store"
        || compression_parameters != json!({"level":"none"}).as_object().unwrap()
    {
        return Err(reject(
            "v2 publication compression must be independently-verifiable store/level=none",
        ));
    }
    if archive
        .entries()
        .iter()
        .any(|entry| entry.method != ZIP_METHOD_STORE)
    {
        return Err(reject(
            "segments.json publication compression disagrees with actual ZIP methods",
        ));
    }
    if declared_entry_order != STITCH_ENTRY_ORDER {
        return Err(reject(
            "segments.json.publication.entry_order is unsupported/ambiguous",
        ));
    }
    let actual_order: Vec<&str> = archive
        .entries()
        .iter()
        .map(|entry| entry.name.as_str())
        .collect();
    let declared_order: Vec<&str> = required_array(manifest, "entries", "manifest")?
        .iter()
        .map(|entry| required_string(entry, "path", "manifest.entries"))
        .collect::<Result<_, _>>()?;
    let mut canonical_order = declared_order.clone();
    canonical_order.sort_unstable();
    if actual_order.last().copied() != Some("manifest.json")
        || actual_order[..actual_order.len() - 1] != declared_order
        || declared_order != canonical_order
        || archive
            .entries()
            .windows(2)
            .any(|pair| pair[0].local_header_off >= pair[1].local_header_off)
    {
        return Err(reject(
            "segments.json publication entry_order disagrees with actual ZIP order",
        ));
    }
    let campaign = graph
        .get("campaign")
        .ok_or_else(|| reject("segments.json.campaign is mandatory"))?;
    required_object(campaign, "segments.json.campaign")?;
    let campaign_scheme_major = required_u32(campaign, "scheme_major", "segments.json.campaign")?;
    let campaign_key_digest =
        required_sha256(campaign, "campaign_key_digest", "segments.json.campaign")?;
    let campaign_key = required_hex_bytes(campaign, "campaign_key", "segments.json.campaign")?;
    if Sha256::hex(&campaign_key) != campaign_key_digest {
        return Err(reject(
            "segments.json.campaign raw key disagrees with campaign_key_digest",
        ));
    }
    graph
        .get("stream_major")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .ok_or_else(|| reject("segments.json.stream_major must be a positive integer"))?;
    let segment_rows = required_array(&graph, "segments", "segments.json")?;
    if segment_rows.is_empty() {
        return Err(reject("segments.json.segments cannot be empty"));
    }

    #[derive(Clone)]
    struct TicketAuthority {
        array_position: usize,
        segment_id: u32,
        play_order: u64,
        in_session_ordinal: u32,
        session_nonce: String,
        process_start_identity: String,
        epoch_token: String,
    }
    let mut authorities = Vec::with_capacity(segment_rows.len());
    let mut seen_ranks = BTreeSet::new();
    let mut order_to_ticket: BTreeMap<u64, (String, String)> = BTreeMap::new();
    for (array_position, segment) in segment_rows.iter().enumerate() {
        let context = format!("segments.json.segments[{array_position}]");
        required_object(segment, &context)?;
        let segment_id = required_u32(segment, "segment_id", &context)?;
        let play_order = required_u64(segment, "play_order", &context)?;
        let in_session_ordinal = required_u32(segment, "in_session_ordinal", &context)?;
        let session_nonce = required_string(segment, "session_nonce", &context)?.to_string();
        let process_start_identity =
            required_string(segment, "process_start_identity", &context)?.to_string();
        let epoch_token = required_string(segment, "epoch_token", &context)?.to_string();
        let ticket = segment
            .get("ticket")
            .ok_or_else(|| reject(format!("{context}.ticket is mandatory")))?;
        let ticket_context = format!("{context}.ticket");
        required_object(ticket, &ticket_context)?;
        if required_sha256(ticket, "campaign_key_digest", &ticket_context)? != campaign_key_digest
            || required_u64(ticket, "play_order", &ticket_context)? != play_order
            || required_string(ticket, "session_nonce", &ticket_context)? != session_nonce
            || required_string(ticket, "process_start_identity", &ticket_context)?
                != process_start_identity
        {
            return Err(reject(format!(
                "{context}.ticket disagrees with exact campaign/segment owner"
            )));
        }
        if !seen_ranks.insert((play_order, in_session_ordinal)) {
            return Err(reject(format!(
                "duplicate (play_order {play_order}, in_session_ordinal {in_session_ordinal}); ticket authority is ambiguous"
            )));
        }
        let ticket_identity = (session_nonce.clone(), process_start_identity.clone());
        if order_to_ticket
            .insert(play_order, ticket_identity.clone())
            .is_some_and(|previous| previous != ticket_identity)
        {
            return Err(reject(format!(
                "play_order {play_order} maps to conflicting AppendTickets; ticket authority is ambiguous"
            )));
        }
        authorities.push(TicketAuthority {
            array_position,
            segment_id,
            play_order,
            in_session_ordinal,
            session_nonce,
            process_start_identity,
            epoch_token,
        });
    }
    authorities.sort_by_key(|authority| (authority.play_order, authority.in_session_ordinal));
    for (rank, authority) in authorities.iter().enumerate() {
        if authority.array_position != rank || authority.segment_id as usize != rank {
            return Err(reject(format!(
                "segments.json array/segment_id order disagrees with AppendTicket play_order rank {rank}"
            )));
        }
    }
    let ledger = Value::Array(
        authorities
            .iter()
            .map(|authority| {
                json!({
                    "campaign_key_digest": campaign_key_digest,
                    "play_order": authority.play_order,
                    "in_session_ordinal": authority.in_session_ordinal,
                    "session_nonce": authority.session_nonce,
                    "process_start_identity": authority.process_start_identity,
                    "epoch_token": authority.epoch_token,
                })
            })
            .collect(),
    );
    let mut canonical_ledger = String::new();
    crate::canon::write_canonical(&ledger, &mut canonical_ledger);
    let recomputed_ledger_digest = Sha256::hex(canonical_ledger.as_bytes());
    if declared_ledger_digest != recomputed_ledger_digest {
        return Err(reject(format!(
            "segments.json.play_order_ledger_digest mismatch (recomputed {recomputed_ledger_digest}, declared {declared_ledger_digest})"
        )));
    }

    let raw_sources = validate_raw_sources(&graph, &campaign_key_digest, game_id, by_path)?;
    let mut segment_hours: Vec<BTreeMap<i64, ValidatedHour>> = Vec::new();
    let mut serialized_segment_indexes = Vec::with_capacity(segment_rows.len());
    let mut visible = BTreeMap::new();
    let mut insertion_main: BTreeMap<i64, u32> = BTreeMap::new();
    let mut segment_extents_by_source: HashMap<String, Vec<((u64, u64), (u64, u64), u32)>> =
        HashMap::new();
    for (expected_id, authority) in authorities.iter().enumerate() {
        let segment = &segment_rows[authority.array_position];
        let context = format!("segments.json.segments[{expected_id}]");
        required_object(segment, &context)?;
        let segment_id = required_u32(segment, "segment_id", &context)?;
        if segment_id as usize != expected_id {
            return Err(reject(format!(
                "{context}.segment_id must be dense play order {expected_id}"
            )));
        }
        let play_order = required_u64(segment, "play_order", &context)?;
        let in_session_ordinal = required_u32(segment, "in_session_ordinal", &context)?;
        let session_nonce = required_string(segment, "session_nonce", &context)?.to_string();
        let process_start_identity =
            required_string(segment, "process_start_identity", &context)?.to_string();
        let epoch_token = required_string(segment, "epoch_token", &context)?.to_string();
        let ticket = segment
            .get("ticket")
            .ok_or_else(|| reject(format!("{context}.ticket is mandatory")))?;
        required_object(ticket, &format!("{context}.ticket"))?;
        let ticket_context = format!("{context}.ticket");
        if required_sha256(ticket, "campaign_key_digest", &ticket_context)? != campaign_key_digest
            || required_u64(ticket, "play_order", &ticket_context)? != play_order
            || required_string(ticket, "session_nonce", &ticket_context)? != session_nonce
            || required_string(ticket, "process_start_identity", &ticket_context)?
                != process_start_identity
        {
            return Err(reject(format!(
                "{context}.ticket disagrees with exact campaign/segment owner"
            )));
        }
        let identity = segment
            .get("identity")
            .ok_or_else(|| reject(format!("{context}.identity is mandatory")))?;
        required_object(identity, &format!("{context}.identity"))?;
        let identity_context = format!("{context}.identity");
        let identity_evidence_digest =
            required_sha256(identity, "evidence_digest", &identity_context)?;
        if required_u32(identity, "scheme_major", &identity_context)? != campaign_scheme_major
            || required_sha256(identity, "campaign_key_digest", &identity_context)?
                != campaign_key_digest
        {
            return Err(reject(format!(
                "{context}.identity disagrees with stitched campaign"
            )));
        }
        let source_ref = segment
            .get("source_ref")
            .ok_or_else(|| reject(format!("{context}.source_ref is mandatory")))?;
        required_object(source_ref, &format!("{context}.source_ref"))?;
        let source_context = format!("{context}.source_ref");
        let source_key = required_sha256(source_ref, "source_key", &source_context)?;
        let participation_id = required_sha256(source_ref, "participation_id", &source_context)?;
        let raw_source = raw_sources
            .get(&source_key)
            .ok_or_else(|| reject(format!("{source_context} references unknown raw source")))?;
        let participation = raw_source
            .participations
            .get(&participation_id)
            .ok_or_else(|| reject(format!("{source_context} references unknown participation")))?;
        let segment_record_bounds =
            required_u64_pair(source_ref, "record_seq_bounds", &source_context)?;
        let segment_byte_bounds = required_u64_pair(source_ref, "byte_bounds", &source_context)?;
        let source_session_path = required_string(source_ref, "session_path", &source_context)?;
        let canonical_source_session_path = normalize_stitch_source_path(source_session_path)
            .map_err(|reason| {
                reject(format!(
                    "{source_context}.session_path is not canonicalizable: {reason}"
                ))
            })?;
        if canonical_source_session_path != raw_source.session_path
            || required_sha256(source_ref, "stored_digest", &source_context)?
                != raw_source.stored_digest
            || required_sha256(source_ref, "decoded_digest", &source_context)?
                != raw_source.decoded_digest
            || raw_source.session_nonce != session_nonce
            || raw_source.process_start_identity != process_start_identity
            || participation.campaign_key_digest != campaign_key_digest
            || participation.scheme_major != campaign_scheme_major
            || participation.identity_evidence_digest != identity_evidence_digest
            || participation.play_order != play_order
            || participation.session_nonce != session_nonce
            || participation.process_start_identity != process_start_identity
            || segment_record_bounds.0 < participation.record_seq_bounds.0
            || segment_record_bounds.1 > participation.record_seq_bounds.1
            || segment_byte_bounds.0 < participation.byte_bounds.0
            || segment_byte_bounds.1 > participation.byte_bounds.1
            || segment_record_bounds.0 < raw_source.record_seq_bounds.0
            || segment_record_bounds.1 > raw_source.record_seq_bounds.1
            || segment_byte_bounds.0 < raw_source.byte_bounds.0
            || segment_byte_bounds.1 > raw_source.byte_bounds.1
        {
            return Err(reject(format!(
                "{source_context} disagrees with exact source/participation ownership"
            )));
        }
        segment_extents_by_source
            .entry(source_key)
            .or_default()
            .push((segment_record_bounds, segment_byte_bounds, segment_id));
        let expected_index = format!("segments/s{segment_id:08}.index.json");
        if required_string(segment, "index_file", &context)? != expected_index {
            return Err(reject(format!(
                "{context} segment index reference is not canonical"
            )));
        }
        let index_path = format!("payload/{game_id}/{expected_index}");
        if !by_path.contains_key(&index_path) {
            return Err(reject(format!(
                "segment index {expected_index:?} is missing"
            )));
        }
        let bytes = verify_and_read(archive, by_path, &index_path)?;
        let segment_index: Value = serde_json::from_slice(&bytes)?;
        required_object(&segment_index, &expected_index)?;
        serialized_segment_indexes.push(segment_index.clone());
        if required_u32(&segment_index, "segment_id", &expected_index)? != segment_id {
            return Err(reject(format!(
                "segment index {expected_index:?} id disagrees with graph"
            )));
        }
        if segment_index.get("source_ref") != Some(source_ref) {
            return Err(reject(format!(
                "segment index {expected_index:?} source_ref disagrees with graph"
            )));
        }
        let cutover = optional_i64(segment, "cutover_hour", &context)?;
        let head = optional_i64(segment, "head_hour", &context)?;
        if optional_i64(&segment_index, "cutover_hour", &expected_index)? != cutover
            || optional_i64(&segment_index, "head_hour", &expected_index)? != head
        {
            return Err(reject(format!(
                "segment index {expected_index:?} cutover/head disagrees with graph"
            )));
        }
        let mut local = BTreeMap::new();
        let mut classified = BTreeSet::new();
        let mut all_observation_bounds = Vec::new();
        for (ordinal, row) in required_array(&segment_index, "hours", &expected_index)?
            .iter()
            .enumerate()
        {
            let row_context = format!("{expected_index}.hours[{ordinal}]");
            let hour = required_i64(row, "hour_index", &row_context)?;
            let mut disposition =
                parse_disposition(row, segment_id, game_id, by_path, &row_context, true)?;
            validate_observed_hour(
                row,
                &mut disposition,
                &campaign_key_digest,
                play_order,
                &session_nonce,
                &process_start_identity,
                in_session_ordinal,
                &epoch_token,
                segment_record_bounds,
                segment_byte_bounds,
                &mut all_observation_bounds,
                &row_context,
            )?;
            if let Some(file) = &disposition.file {
                let expected_file = format!("s{segment_id:08}.{hour:012}.snapshot.json");
                if file != &expected_file {
                    return Err(reject(format!(
                        "{row_context} filename disagrees with segment/hour"
                    )));
                }
            }
            if local.insert(hour, disposition).is_some() {
                return Err(reject(format!("duplicate hour {hour} in {expected_index}")));
            }
        }
        all_observation_bounds.sort();
        for adjacent in all_observation_bounds.windows(2) {
            if adjacent[1].0 .0 <= adjacent[0].0 .1 || adjacent[1].1 .0 < adjacent[0].1 .1 {
                return Err(reject(format!(
                    "{expected_index} observation evidence overlaps across hours {} and {}",
                    adjacent[0].2, adjacent[1].2
                )));
            }
        }
        match (cutover, head) {
            (None, None) if local.is_empty() => {}
            (Some(start), Some(end)) if start <= end => {
                let keys: Vec<i64> = local.keys().copied().collect();
                if keys != (start..=end).collect::<Vec<_>>() {
                    return Err(reject(format!(
                        "truncated segment index {expected_index:?}"
                    )));
                }
            }
            _ => {
                return Err(reject(format!(
                    "segment index {expected_index:?} has invalid empty/cutover/head shape"
                )));
            }
        }
        let mut transition_hours = Vec::new();
        for (ordinal, row) in
            required_array(&segment_index, "transition_coverage", &expected_index)?
                .iter()
                .enumerate()
        {
            let row_context = format!("{expected_index}.transition_coverage[{ordinal}]");
            let hour = required_i64(row, "hour_index", &row_context)?;
            transition_hours.push(hour);
            let mut disposition =
                parse_disposition(row, segment_id, game_id, by_path, &row_context, false)?;
            validate_owner(
                row,
                &campaign_key_digest,
                play_order,
                &session_nonce,
                &process_start_identity,
                in_session_ordinal,
                &epoch_token,
                &row_context,
            )?;
            disposition.ownership = selected_ownership_value(row, &row_context)?;
            if local.insert(hour, disposition).is_some() {
                return Err(reject(format!(
                    "duplicate transition/observed hour {hour} in {expected_index}"
                )));
            }
        }
        for (span_no, span) in required_array(segment, "visible_spans", &context)?
            .iter()
            .enumerate()
        {
            let span_context = format!("{context}.visible_spans[{span_no}]");
            let start = required_i64(span, "start_hour", &span_context)?;
            let end = required_i64(span, "end_hour", &span_context)?;
            if start > end {
                return Err(reject(format!("{span_context} has reversed bounds")));
            }
            for hour in start..=end {
                let disposition = local.get(&hour).cloned().ok_or_else(|| {
                    reject(format!("{span_context} references uncovered hour {hour}"))
                })?;
                insert_unique(
                    &mut visible,
                    hour,
                    disposition,
                    "segments.json visible spans",
                )?;
                if !classified.insert(hour) {
                    return Err(reject(format!(
                        "{span_context} overlaps another visible/superseded classification"
                    )));
                }
            }
        }
        for (span_no, span) in required_array(segment, "superseded_spans", &context)?
            .iter()
            .enumerate()
        {
            let span_context = format!("{context}.superseded_spans[{span_no}]");
            let start = required_i64(span, "start_hour", &span_context)?;
            let end = required_i64(span, "end_hour", &span_context)?;
            let by = required_u32(span, "superseded_by", &span_context)?;
            if start > end || by <= segment_id || by as usize >= segment_rows.len() {
                return Err(reject(format!("{span_context} has invalid bounds/owner")));
            }
            for hour in start..=end {
                if !local.contains_key(&hour) {
                    return Err(reject(format!(
                        "{span_context} references uncovered hour {hour}"
                    )));
                }
                if !classified.insert(hour) {
                    return Err(reject(format!(
                        "{span_context} overlaps another visible/superseded classification"
                    )));
                }
            }
        }
        if classified.iter().copied().collect::<Vec<_>>()
            != local.keys().copied().collect::<Vec<_>>()
        {
            return Err(reject(format!(
                "{context} visible/superseded spans do not classify every retained hour"
            )));
        }
        let expected_parent = cutover.and_then(|start| {
            insertion_main
                .range(..start)
                .next_back()
                .map(|(_, owner)| *owner)
        });
        if optional_u32(segment, "parent_segment", &context)? != expected_parent {
            return Err(reject(format!(
                "{context}.parent_segment was not captured at insertion time"
            )));
        }
        for hour in transition_hours {
            insertion_main.insert(hour, segment_id);
        }
        if let (Some(start), Some(end)) = (cutover, head) {
            let removed: Vec<i64> = insertion_main
                .range(start..)
                .map(|(hour, _)| *hour)
                .collect();
            for hour in removed {
                insertion_main.remove(&hour);
            }
            for hour in start..=end {
                insertion_main.insert(hour, segment_id);
            }
        }
        segment_hours.push(local);
    }

    for (source_key, extents) in &mut segment_extents_by_source {
        extents.sort();
        for adjacent in extents.windows(2) {
            if adjacent[1].0 .0 <= adjacent[0].0 .1 || adjacent[1].1 .0 < adjacent[0].1 .1 {
                return Err(reject(format!(
                    "raw source {source_key} has overlapping segment extents {} and {}",
                    adjacent[0].2, adjacent[1].2
                )));
            }
        }
    }
    if segment_extents_by_source.len() != raw_sources.len() {
        return Err(reject(
            "segments.json contains an unreferenced raw source descriptor",
        ));
    }

    let mut index_main = BTreeMap::new();
    let mut previous_snapshot: Option<i64> = None;
    for (ordinal, row) in required_array(index, "snapshots", "index")?
        .iter()
        .enumerate()
    {
        let context = format!("index.snapshots[{ordinal}]");
        let hour = required_i64(row, "hour_index", &context)?;
        if previous_snapshot.is_some_and(|previous| hour <= previous) {
            return Err(reject(format!(
                "duplicate visible hour {hour} in index snapshots"
            )));
        }
        previous_snapshot = Some(hour);
        if required_string(row, "kind", &context)? != "snapshot" {
            return Err(reject(format!("{context}.kind must be snapshot")));
        }
        let file = required_string(row, "file", &context)?.to_string();
        let sha256 = required_sha256(row, "sha256", &context)?;
        let ownership = selected_ownership_value(row, &context)?;
        let graph_hour = visible
            .get(&hour)
            .ok_or_else(|| reject(format!("{context} is absent from graph visible spans")))?;
        if graph_hour.kind != "snapshot"
            || graph_hour.file.as_deref() != Some(file.as_str())
            || graph_hour.sha256.as_deref() != Some(sha256.as_str())
            || graph_hour.ownership != ownership
        {
            return Err(reject(format!("{context} graph/index disagreement")));
        }
        insert_unique(&mut index_main, hour, graph_hour.clone(), "index")?;
    }
    let mut previous_coverage_end: Option<i64> = None;
    for (ordinal, row) in required_array(index, "coverage", "index")?
        .iter()
        .enumerate()
    {
        let context = format!("index.coverage[{ordinal}]");
        let start = required_i64(row, "start_hour", &context)?;
        let end = required_i64(row, "end_hour", &context)?;
        if start > end || previous_coverage_end.is_some_and(|previous| start <= previous) {
            return Err(reject(format!(
                "{context} has overlapping/out-of-order bounds"
            )));
        }
        previous_coverage_end = Some(end);
        let segment_id = required_u32(row, "segment_id", &context)?;
        let mut disposition =
            parse_disposition(row, segment_id, game_id, by_path, &context, false)?;
        disposition.ownership = selected_ownership_value(row, &context)?;
        for hour in start..=end {
            insert_unique(&mut index_main, hour, disposition.clone(), "index")?;
        }
    }
    if index_main != visible {
        return Err(reject(
            "main index and segment graph visible coverage disagree",
        ));
    }
    let mut expected_slices: Vec<(u32, i64, i64)> = Vec::new();
    for (hour, disposition) in &visible {
        if let Some((segment_id, _, end)) = expected_slices.last_mut() {
            if *segment_id == disposition.segment_id && *end + 1 == *hour {
                *end = *hour;
                continue;
            }
        }
        expected_slices.push((disposition.segment_id, *hour, *hour));
    }
    let actual_slices: Vec<(u32, i64, i64)> =
        required_array(&graph, "main_slices", "segments.json")?
            .iter()
            .enumerate()
            .map(|(ordinal, slice)| {
                let context = format!("segments.json.main_slices[{ordinal}]");
                Ok((
                    required_u32(slice, "segment_id", &context)?,
                    required_i64(slice, "start_hour", &context)?,
                    required_i64(slice, "end_hour", &context)?,
                ))
            })
            .collect::<Result<_, ContainerError>>()?;
    if actual_slices != expected_slices {
        return Err(reject(
            "segments.json main_slices disagree with visible graph",
        ));
    }
    let expected_head = visible
        .iter()
        .next_back()
        .map(|(hour, value)| (value.segment_id, *hour));
    if parse_head(
        index.get("main_head").unwrap_or(&Value::Null),
        "index.main_head",
    )? != expected_head
        || parse_head(
            graph.get("main_head").unwrap_or(&Value::Null),
            "segments.json.main_head",
        )? != expected_head
    {
        return Err(reject("main_head graph/index disagreement"));
    }
    let recomputed_stitch_digest =
        canonical_stitch_input_digest(game_id, &graph, &serialized_segment_indexes);
    if declared_stitch_digest != recomputed_stitch_digest {
        return Err(reject(format!(
            "segments.json.stitch_input_digest mismatch (recomputed {recomputed_stitch_digest}, declared {declared_stitch_digest})"
        )));
    }
    Ok(())
}

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
