//! Atomic publication of a stitched container (design `STITCHING_DESIGN.md` §7.4).
//!
//! This is the finalize-routing seam: given a sealed [`StitchedPlan`](super::StitchedPlan)
//! and a source of the retained snapshot / raw-stream payloads, it assembles the v2
//! container payload set, packs a STORE-only `.battleplan` (§8 rule 7 —
//! `{level:none}`), and publishes it **build-beside-then-atomic-rename**:
//!
//! 1. derive every container entry from ONE immutable plan (never file enumeration);
//! 2. reference-validate each snapshot payload against the hash the plan recorded
//!    (a payload that does not hash to its coverage disposition aborts before pack);
//! 3. pack the container (v2 + `timeline_segments_v1`, STORE);
//! 4. re-open the packed bytes and run full manifest / hash / reference validation
//!    (`Container::from_bytes` + `verify_all` + stitched-index + graph checks)
//!    BEFORE anything touches the destination;
//! 5. write the bytes to a sibling temp file, fsync, then atomically rename over the
//!    destination.
//!
//! Any failure at any phase leaves the prior published replay and all standalone /
//! session inputs untouched, and removes the temp artifact — matching §7.4 and the
//! §11 gate 10 crash-injection requirement. The plan/payload model is a pure input,
//! so this is fully exercisable offline with synthetic segment fixtures.
//!
//! What this does NOT do (explicit later-lane interfaces): it does not itself run
//! the per-segment folds, form segments from the live byte stream
//! ([`super::formation`]), or emit the new compact-stream wire major (§9). The live
//! `main.rs` finalize path calls this once the sealed [`SegmentInput`](super::SegmentInput)
//! set and its spilled payloads exist.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::Sha256;
use snapshot::{pack_with, PackEntry, PackIdentity, PackOptions};

use super::{CoverageDisposition, PlannedSegment, RawSourceStorage, StitchedPlan};

/// Supplies the exact retained payload bytes a stitched plan references. During live
/// finalize the snapshots are the per-segment fold's spilled files and the raw
/// sources are the retained compact streams; tests supply in-memory maps.
pub trait StitchedPayloadSource {
    /// Snapshot body bytes for one segment's hour. Must hash to the `sha256` the
    /// plan recorded for that hour (enforced by [`build_stitched_entries`]).
    fn snapshot_bytes(&self, segment_id: u32, hour_index: i64) -> Result<Vec<u8>, String>;
    /// The exact byte-for-byte retained raw-source bytes for one source path.
    fn raw_source_bytes(&self, source_path: &str) -> Result<Vec<u8>, String>;
}

/// Outcome of a successful publication.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishReport {
    pub published_path: PathBuf,
    pub container_bytes: usize,
    /// Count of visible main-timeline snapshot hours in the published index.
    pub visible_snapshot_hours: usize,
    /// Count of segments (visible + rollback + empty recovery) in the graph.
    pub segments: usize,
}

/// The v2 STORE publication options a stitched container declares (§8 rule 7 /
/// §9). STORE-only because v2 open can only independently re-verify the `{level:none}`
/// profile; the plan's `compression_method` must agree.
fn stitched_pack_options() -> PackOptions {
    PackOptions {
        container_version: Some(super::STITCH_CONTAINER_VERSION),
        required_features: Some(vec![super::STITCH_REQUIRED_FEATURE.to_string()]),
        ..Default::default()
    }
}

/// The pack identity a stitched publication records. `created_utc` and the schema /
/// mod bindings come from the plan's immutable required-inputs, never sampled at
/// pack time (§8: `created_utc` is an input).
fn pack_identity(plan: &StitchedPlan) -> PackIdentity {
    let mod_identity = plan
        .segments
        .iter()
        .find_map(|segment| segment.input.provenance.modset_id.clone())
        .map(|modset_id| serde_json::json!({ "modset_id": modset_id }));
    PackIdentity {
        schema_version: Some(plan.required_inputs.schema_version.clone()),
        build_exe_sha256: plan
            .segments
            .iter()
            .find_map(|segment| segment.input.provenance.build_exe_sha256.clone()),
        build_pinned: false,
        mod_identity,
        created_utc: Some(plan.required_inputs.created_utc.clone()),
        content_manifest: None,
    }
}

/// Assemble the full, deterministic container payload set for a plan, pulling bytes
/// from `source` and reference-validating each snapshot against its recorded hash.
///
/// Fails closed on: a segment with no raw-source ledger; a snapshot payload whose
/// bytes do not hash to the plan's recorded `sha256`; or an embedded raw source whose
/// stored bytes do not match the recorded stored digest. Only embeddable
/// (`EmbeddedCampaign`) raw sources are packed; standalone sources are referenced by
/// the segments graph but never embedded. Entries are sorted by path so the packed
/// container is byte-deterministic (§8).
pub fn build_stitched_entries<S: StitchedPayloadSource>(
    plan: &StitchedPlan,
    source: &S,
) -> Result<Vec<PackEntry>, String> {
    let game = plan.game_id.as_str();
    let mut entries: Vec<PackEntry> = Vec::new();

    // Stitched index / segments / per-segment index files (canonical bytes).
    for f in plan.to_container_files() {
        entries.push(PackEntry {
            path: format!("payload/{game}/{}", f.rel_path),
            bytes: f.bytes,
            role: match f.role {
                "index" => "index",
                "segments" => "segments",
                "segment_index" => "segment_index",
                _ => "aux",
            },
            hour_index: None,
        });
    }

    // Each exact raw source is stored ONCE, byte-for-byte, independent of how many
    // segments reference its participation bounds (§7). Deduplicate by source path.
    let mut raw_paths: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for segment in &plan.segments {
        let raw = segment.input.raw_source.as_ref().ok_or_else(|| {
            format!(
                "segment {} has no raw-source ledger; cannot publish",
                segment.segment_id
            )
        })?;
        raw_paths.insert(raw.source_path.clone());
    }
    for path in raw_paths {
        // The descriptor for this source path (identical across every segment that
        // references it — the plan already proved that agreement).
        let descriptor = plan
            .segments
            .iter()
            .find_map(|segment| {
                segment
                    .input
                    .raw_source
                    .as_ref()
                    .filter(|raw| raw.source_path == path)
            })
            .ok_or_else(|| format!("no raw-source descriptor for {path}; cannot publish"))?;
        match &descriptor.storage {
            // A standalone source (e.g. a multi-campaign A->B->A stream) is REFERENCED
            // by the segments graph, never embedded: `Container::from_bytes`
            // (validate_raw_sources) rejects a standalone raw source that appears in the
            // container manifest. Do not pack its bytes.
            RawSourceStorage::Standalone => continue,
            // An embeddable source is stored ONCE, byte-for-byte. Reference-validate the
            // STORED (as-embedded) bytes against the recorded STORED digest — that is
            // exactly what the packed manifest entry hashes to and what container
            // re-open checks (`entry.expected_sha256 == stored_digest`), so build-time
            // and re-open validation agree even when the source is compressed
            // (.h4cr.zst, stored digest != decoded digest).
            RawSourceStorage::EmbeddedCampaign { .. } => {
                let bytes = source.raw_source_bytes(&path)?;
                let actual = Sha256::hex(&bytes);
                if actual != descriptor.stored_digest {
                    return Err(format!(
                        "raw source {path} stored bytes hash {actual} != recorded stored digest {}",
                        descriptor.stored_digest
                    ));
                }
                entries.push(PackEntry {
                    path: format!("payload/{game}/{path}"),
                    bytes,
                    role: "raw_source",
                    hour_index: None,
                });
            }
        }
    }

    // Every snapshot hour of every segment (visible + rollback), named by the
    // segment's assigned global ordinal. Each payload must hash to the recorded
    // coverage sha256 — the build-time half of §7.4 reference validation.
    for segment in &plan.segments {
        for (hour, disposition) in segment.input.coverage.iter() {
            if let CoverageDisposition::Snapshot { sha256 } = disposition {
                let bytes = source.snapshot_bytes(segment.segment_id, *hour)?;
                let actual = Sha256::hex(&bytes);
                if &actual != sha256 {
                    return Err(format!(
                        "segment {} hour {hour} snapshot hashes {actual} != recorded {sha256}",
                        segment.segment_id
                    ));
                }
                entries.push(PackEntry {
                    path: format!(
                        "payload/{game}/{}",
                        PlannedSegment::snapshot_file(segment.segment_id, *hour)
                    ),
                    bytes,
                    role: "snapshot",
                    hour_index: Some(*hour),
                });
            }
        }
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// Pack a plan into stitched container bytes and independently re-open + validate
/// them (no destination is touched). This is the packing+validation core shared by
/// the atomic publisher and its tests.
pub fn pack_and_validate<S: StitchedPayloadSource>(
    plan: &StitchedPlan,
    source: &S,
) -> Result<(Vec<u8>, PublishReport), String> {
    let entries = build_stitched_entries(plan, source)?;
    let bytes = pack_with(
        &plan.game_id,
        &entries,
        &pack_identity(plan),
        false, // STORE, not deflate — §8 rule 7
        &stitched_pack_options(),
    )
    .map_err(|error| format!("pack stitched container failed: {error}"))?;

    // Full manifest / hash / reference validation from the CONTAINER BYTES before
    // publish (§7.4): open (which cross-checks the stitched index, segments graph,
    // ticket order, digests, ZIP order, created_utc, and STORE profile), verify
    // every retained payload hash, and confirm the stitched shape.
    let container = Container::from_bytes(bytes.clone(), &ExpectedIdentity::default())
        .map_err(|error| format!("stitched container failed to re-open for validation: {error}"))?;
    container
        .verify_all()
        .map_err(|error| format!("stitched container payload verification failed: {error}"))?;
    if !container.index().is_stitched() {
        return Err("packed container is not a stitched (stitched-main-v1) index".to_string());
    }
    let graph = container
        .segments()
        .map_err(|error| format!("stitched container graph failed to parse: {error}"))?
        .ok_or_else(|| "packed v2 container has no segments graph".to_string())?;

    // Every visible snapshot the plan expects must resolve in the published index.
    let visible = plan.visible_snapshot_files();
    for (hour, file) in &visible {
        let resolved = container
            .index()
            .snapshots
            .iter()
            .find(|snapshot| snapshot.hour_index == Some(*hour))
            .ok_or_else(|| format!("visible hour {hour} missing from published index"))?;
        if &resolved.file != file {
            return Err(format!(
                "visible hour {hour} resolves to {} but the plan names {file}",
                resolved.file
            ));
        }
    }

    let report = PublishReport {
        published_path: PathBuf::new(),
        container_bytes: bytes.len(),
        visible_snapshot_hours: visible.len(),
        segments: graph.segments.len(),
    };
    Ok((bytes, report))
}

/// Publish a stitched plan to `dest` build-beside-then-atomic-rename (§7.4).
///
/// The container is packed and fully re-validated in memory first; only then are the
/// bytes written to a sibling temp file, fsynced, and atomically renamed over `dest`.
/// On ANY failure the prior file at `dest` is left untouched and the temp artifact is
/// removed, so a crash at seal / pack / publish never destroys the previous replay or
/// the immutable inputs.
pub fn publish_stitched_container<S: StitchedPayloadSource>(
    plan: &StitchedPlan,
    source: &S,
    dest: &Path,
) -> Result<PublishReport, String> {
    let (bytes, mut report) = pack_and_validate(plan, source)?;

    let parent = dest.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create output directory failed: {error}"))?;
    }
    let temp = sibling_temp_path(dest);

    // Build beside: write + fsync the temp file, then atomically rename over dest.
    // A failure before the rename leaves dest untouched; clean the temp on any error.
    let write_result = (|| -> std::io::Result<()> {
        let mut file = fs::File::create(&temp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let cleanup = fs::remove_file(&temp);
        return Err(with_cleanup_note(
            format!("write stitched temp container failed: {error}"),
            &temp,
            cleanup,
        ));
    }

    // Same-volume atomic replacement. Rust's Windows `fs::rename` does NOT
    // replace an existing destination, so use MoveFileExW with the explicit
    // REPLACE_EXISTING + WRITE_THROUGH contract there. A failure still leaves
    // the prior destination intact.
    if let Err(error) = atomic_replace(&temp, dest) {
        let cleanup = fs::remove_file(&temp);
        return Err(with_cleanup_note(
            format!("atomic publish (rename) failed: {error}"),
            &temp,
            cleanup,
        ));
    }

    report.published_path = dest.to_path_buf();
    Ok(report)
}

#[cfg(not(windows))]
fn atomic_replace(temp: &Path, dest: &Path) -> std::io::Result<()> {
    fs::rename(temp, dest)
}

#[cfg(windows)]
fn atomic_replace(temp: &Path, dest: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x0000_0001;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x0000_0008;

    #[link(name = "kernel32")]
    extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    fn wide(path: &Path) -> std::io::Result<Vec<u16>> {
        let mut out: Vec<u16> = path.as_os_str().encode_wide().collect();
        if out.contains(&0) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "atomic publish path contains an embedded NUL",
            ));
        }
        out.push(0);
        Ok(out)
    }

    let existing = wide(temp)?;
    let replacement = wide(dest)?;
    let ok = unsafe {
        MoveFileExW(
            existing.as_ptr(),
            replacement.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// A sibling temp path in the destination's own directory (so the rename is a
/// same-volume atomic replace). Uniqued by pid + nanos to avoid concurrent collision.
fn sibling_temp_path(dest: &Path) -> PathBuf {
    let nonce = format!(
        "{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let file_name = dest
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "stitched".to_string());
    let temp_name = format!(".{file_name}.stitch-tmp-{nonce}");
    match dest.parent().filter(|p| !p.as_os_str().is_empty()) {
        Some(parent) => parent.join(temp_name),
        None => PathBuf::from(temp_name),
    }
}

/// Compose a publish-failure message, surfacing a temp-file cleanup failure when one
/// occurred. The atomic-publish contract (§7.4 / §11 gate 10) is that a failed publish
/// leaves NO partial `.stitch-tmp-*` artifact behind; if the OS denies the delete
/// (Windows file lock / ACL) that guarantee is broken, so the caller must be told a
/// partial temp remains rather than have the error silently swallowed. `primary` (the
/// original failure) is kept as the reported cause. A `NotFound` cleanup result means
/// the temp is already gone — the contract holds, so nothing is appended.
fn with_cleanup_note(primary: String, temp: &Path, cleanup: std::io::Result<()>) -> String {
    match cleanup {
        Ok(()) => primary,
        Err(remove_error) if remove_error.kind() == std::io::ErrorKind::NotFound => primary,
        Err(remove_error) => format!(
            "{primary}; ALSO failed to remove temp artifact {}: {remove_error} \
             (a partial temp file remains and must be cleaned up manually)",
            temp.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    use crate::stitching::{
        plan_stitch, AppendTicket, CampaignIdentityResolution, CompatibilityResolution,
        CoverageDisposition, HourObservation, ObservationOwner, RawSourceDescriptor,
        RawSourceStorage, SegmentInput, SegmentProvenance, SourceParticipation, StitchOutcome,
        StitchRequiredInputs, STITCH_ENTRY_ORDER,
    };

    const GAME: &str = "g";
    const MODSET: &str = "MS";
    const K: &[u8] = b"campaign-A";

    fn snap_payload(play_order: u64, hour: i64) -> Vec<u8> {
        format!(
            r#"{{"schema_version":"1","game_id":"{GAME}","hour_index":{hour},"provenance":{{"mod":{{"modset_id":"{MODSET}"}}}},"po":{play_order},"countries":{{}},"states":{{}}}}"#
        )
        .into_bytes()
    }

    fn raw_payload(play_order: u64, len: usize) -> Vec<u8> {
        vec![(play_order as u8).wrapping_add(1); len]
    }

    fn stitch_inputs() -> StitchRequiredInputs {
        StitchRequiredInputs {
            schema_version: "1".into(),
            fold_version: "wf-v1".into(),
            canonical_writer_version: "canon-v1".into(),
            packer_codec_version: "zip-v1".into(),
            created_utc: "2026-07-12T00:00:00Z".into(),
            compression_method: "store".into(),
            compression_parameters: BTreeMap::from([("level".into(), "none".into())]),
            entry_order: STITCH_ENTRY_ORDER.into(),
        }
    }

    fn snap_seg(play_order: u64, range: std::ops::RangeInclusive<i64>) -> SegmentInput {
        let hours: Vec<i64> = range.collect();
        let ticket = AppendTicket {
            campaign_key: K.to_vec(),
            play_order,
            session_nonce: format!("nonce-{play_order}"),
            process_start_identity: format!("pid-{play_order}"),
        };
        let epoch_token = format!("epoch-{play_order}");
        let owner = ObservationOwner {
            ticket: ticket.clone(),
            in_session_ordinal: 0,
            epoch_token: epoch_token.clone(),
        };
        let mut coverage = BTreeMap::new();
        let mut observations = BTreeMap::new();
        for (index, h) in hours.iter().copied().enumerate() {
            let disposition = CoverageDisposition::Snapshot {
                sha256: Sha256::hex(&snap_payload(play_order, h)),
            };
            coverage.insert(h, disposition.clone());
            observations.insert(
                h,
                vec![HourObservation {
                    observation_id: format!("obs-{play_order}-{h}"),
                    owner: owner.clone(),
                    record_seq_bounds: ((index as u64) * 2, (index as u64) * 2 + 1),
                    byte_bounds: ((index as u64) * 100, (index as u64 + 1) * 100),
                    disposition,
                }],
            );
        }
        let record_end = (hours.len() as u64) * 2 - 1;
        let byte_end = (hours.len() as u64) * 100;
        let source_path = format!("sessions/p{play_order:08}/collector_stream.h4cr");
        let source_digest = Sha256::hex(&raw_payload(play_order, byte_end as usize));
        SegmentInput {
            play_order,
            in_session_ordinal: 0,
            identity: CampaignIdentityResolution::Verified {
                scheme_major: 1,
                campaign_key: K.to_vec(),
                evidence_digest: [0u8; 32],
            },
            ticket: Some(ticket.clone()),
            compatibility: CompatibilityResolution::Proven {
                evidence_id: format!("compat-{play_order}"),
                compatibility_class: "decoder-v1".into(),
                build_exe_sha256: "build-a".into(),
                modset_id: MODSET.into(),
                schema_version: "1".into(),
                static_catalog_digest: "catalog-a".into(),
            },
            transition_coverage: BTreeMap::new(),
            coverage,
            observations,
            raw_source: Some(RawSourceDescriptor {
                session_nonce: format!("nonce-{play_order}"),
                process_start_identity: format!("pid-{play_order}"),
                source_path: source_path.clone(),
                stored_digest: source_digest.clone(),
                decoded_digest: source_digest.clone(),
                record_seq_bounds: (0, record_end),
                byte_bounds: (0, byte_end),
                storage: RawSourceStorage::EmbeddedCampaign {
                    campaign_key: K.to_vec(),
                },
                participations: vec![SourceParticipation {
                    ticket,
                    scheme_major: 1,
                    identity_evidence_digest: [0u8; 32],
                    record_seq_bounds: (0, record_end),
                    byte_bounds: (0, byte_end),
                }],
            }),
            provenance: SegmentProvenance {
                session_nonce: format!("nonce-{play_order}"),
                process_start_identity: format!("pid-{play_order}"),
                epoch_token,
                stream_major: 2,
                build_exe_sha256: Some("build-a".into()),
                modset_id: Some(MODSET.into()),
                schema_version: Some("1".into()),
                static_catalog_digest: Some("catalog-a".into()),
                source_path: Some(source_path),
                source_stored_digest: Some(source_digest.clone()),
                source_decoded_digest: Some(source_digest),
                record_seq_bounds: Some((0, record_end)),
                byte_bounds: Some((0, byte_end)),
                ..Default::default()
            },
        }
    }

    /// Multi-segment synthetic session with an in-stream rollback (§11 gate 1):
    /// S0=0..3 then S1=2..2 supersedes hour 2; hour 3 is abandoned-future rollback.
    fn rollback_plan() -> StitchedPlan {
        match plan_stitch(
            GAME,
            stitch_inputs(),
            vec![snap_seg(0, 0..=3), snap_seg(1, 2..=2)],
        ) {
            StitchOutcome::Stitched(plan) => plan,
            StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
        }
    }

    /// In-memory payload source keyed exactly as the plan references payloads.
    struct MemorySource {
        snapshots: BTreeMap<(u32, i64), Vec<u8>>,
        raw: BTreeMap<String, Vec<u8>>,
    }

    impl MemorySource {
        fn for_plan(plan: &StitchedPlan) -> MemorySource {
            let mut snapshots = BTreeMap::new();
            let mut raw = BTreeMap::new();
            for segment in &plan.segments {
                let po = segment.input.play_order;
                for (hour, disposition) in segment.input.coverage.iter() {
                    if disposition.is_snapshot() {
                        snapshots.insert((segment.segment_id, *hour), snap_payload(po, *hour));
                    }
                }
                let source = segment.input.raw_source.as_ref().unwrap();
                let len = (source.byte_bounds.1 - source.byte_bounds.0) as usize;
                raw.insert(source.source_path.clone(), raw_payload(po, len));
            }
            MemorySource { snapshots, raw }
        }
    }

    impl StitchedPayloadSource for MemorySource {
        fn snapshot_bytes(&self, segment_id: u32, hour_index: i64) -> Result<Vec<u8>, String> {
            self.snapshots
                .get(&(segment_id, hour_index))
                .cloned()
                .ok_or_else(|| format!("no snapshot for segment {segment_id} hour {hour_index}"))
        }
        fn raw_source_bytes(&self, source_path: &str) -> Result<Vec<u8>, String> {
            self.raw
                .get(source_path)
                .cloned()
                .ok_or_else(|| format!("no raw source {source_path}"))
        }
    }

    struct TempDir(PathBuf);
    impl TempDir {
        fn new(tag: &str) -> TempDir {
            let mut p = std::env::temp_dir();
            p.push(format!(
                "hoi4_stitch_publish_{tag}_{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&p).unwrap();
            TempDir(p)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// The plan -> pack -> read round-trip on a multi-segment synthetic session:
    /// publish, then re-open the published file and confirm the main timeline,
    /// rollback retention, and integrity.
    #[test]
    fn publishes_multi_segment_session_and_reopens() {
        let plan = rollback_plan();
        let source = MemorySource::for_plan(&plan);
        let dir = TempDir::new("roundtrip");
        let dest = dir.0.join("replay.battleplan");

        let report = publish_stitched_container(&plan, &source, &dest).unwrap();
        assert_eq!(report.published_path, dest);
        assert!(dest.exists());
        // main hours 0,1 from S0; hour 2 from the later branch S1; hour 3 abandoned.
        assert_eq!(report.visible_snapshot_hours, 3);
        assert_eq!(report.segments, 2);
        // No temp artifact survives a success.
        let leftovers: Vec<_> = fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("stitch-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp artifact leaked on success");

        let bytes = fs::read(&dest).unwrap();
        let container = Container::from_bytes(bytes, &ExpectedIdentity::default()).unwrap();
        assert!(container.index().is_stitched());
        let h2 = container
            .index()
            .snapshots
            .iter()
            .find(|s| s.hour_index == Some(2))
            .unwrap();
        assert_eq!(h2.file, "s00000001.000000000002.snapshot.json");
        // hour 2 resolves to the later branch (po=1).
        assert_eq!(
            container
                .load(2)
                .unwrap()
                .rest
                .get("po")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
        // superseded + abandoned-future snapshots retained rollback-only.
        assert!(container
            .load_file("s00000000.000000000002.snapshot.json")
            .is_ok());
        assert!(container
            .load_file("s00000000.000000000003.snapshot.json")
            .is_ok());
        container.verify_all().unwrap();
    }

    #[test]
    fn atomic_publish_replaces_an_existing_verified_destination() {
        let plan = rollback_plan();
        let source = MemorySource::for_plan(&plan);
        let dir = TempDir::new("replace_existing");
        let dest = dir.0.join("replay.battleplan");

        publish_stitched_container(&plan, &source, &dest).unwrap();
        let first = fs::read(&dest).unwrap();
        publish_stitched_container(&plan, &source, &dest).unwrap();
        let second = fs::read(&dest).unwrap();
        assert_eq!(second, first);
        assert!(Container::from_bytes(second, &ExpectedIdentity::default()).is_ok());
    }

    #[test]
    fn packing_is_byte_deterministic() {
        let plan = rollback_plan();
        let source = MemorySource::for_plan(&plan);
        let (a, _) = pack_and_validate(&plan, &source).unwrap();
        let (b, _) = pack_and_validate(&plan, &source).unwrap();
        assert_eq!(a, b, "same plan + payloads -> byte-identical container");
    }

    #[test]
    fn tampered_snapshot_payload_aborts_before_publish() {
        // §7.4 reference validation: a payload that does not hash to its recorded
        // coverage sha256 must abort at build, before any pack/publish.
        let plan = rollback_plan();
        let mut source = MemorySource::for_plan(&plan);
        // Corrupt the byte content of one snapshot without updating the plan hash.
        let key = *source.snapshots.keys().next().unwrap();
        source.snapshots.insert(key, b"tampered".to_vec());
        let error = build_stitched_entries(&plan, &source)
            .err()
            .expect("tampered snapshot must abort");
        assert!(error.contains("hashes"), "{error}");
    }

    #[test]
    fn tampered_raw_source_aborts_before_publish() {
        let plan = rollback_plan();
        let mut source = MemorySource::for_plan(&plan);
        let path = source.raw.keys().next().unwrap().clone();
        source
            .raw
            .insert(path, b"different-length-and-content".to_vec());
        let error = build_stitched_entries(&plan, &source)
            .err()
            .expect("tampered raw source must abort");
        assert!(error.contains("stored digest"), "{error}");
    }

    /// §11 gate 10 crash-injection: a failure during the publish (here a payload the
    /// source cannot supply, standing in for a mid-finalize crash) must leave the
    /// PRIOR published replay at the destination untouched and leave no temp behind.
    #[test]
    fn failed_publish_preserves_prior_replay_and_leaves_no_temp() {
        let dir = TempDir::new("crash");
        let dest = dir.0.join("replay.battleplan");
        // A valid prior replay is already published.
        let good = rollback_plan();
        publish_stitched_container(&good, &MemorySource::for_plan(&good), &dest).unwrap();
        let prior_bytes = fs::read(&dest).unwrap();

        // A new publish whose payload source is missing a snapshot fails.
        let plan = rollback_plan();
        let mut broken = MemorySource::for_plan(&plan);
        broken.snapshots.clear(); // cannot supply any snapshot payload
        let error = publish_stitched_container(&plan, &broken, &dest).unwrap_err();
        assert!(error.contains("no snapshot"), "{error}");

        // The prior replay is byte-for-byte intact and no temp artifact leaked.
        assert_eq!(
            fs::read(&dest).unwrap(),
            prior_bytes,
            "prior replay was disturbed"
        );
        let leftovers: Vec<_> = fs::read_dir(&dir.0)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains("stitch-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp artifact leaked on failure");
    }

    /// Model a compressed `.h4cr.zst` source: the STORED (compressed) bytes hash to
    /// `stored_digest`, while `decoded_digest` is the hash of the different decoded
    /// bytes. The `MemorySource` supplies the stored bytes.
    fn compressed_source_plan() -> StitchedPlan {
        let mut seg = snap_seg(0, 0..=2);
        let decoded = Sha256::hex(b"decoded-plaintext-distinct-from-the-stored-bytes");
        seg.raw_source.as_mut().unwrap().decoded_digest = decoded.clone();
        seg.provenance.source_decoded_digest = Some(decoded);
        match plan_stitch(GAME, stitch_inputs(), vec![seg]) {
            StitchOutcome::Stitched(plan) => plan,
            StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
        }
    }

    #[test]
    fn compressed_source_validates_stored_digest_and_reopens() {
        // Finding 1: a compressed source has stored_digest != decoded_digest. The
        // embedded (stored) bytes must be reference-validated against the STORED digest
        // — exactly what container re-open checks (entry.expected_sha256 ==
        // stored_digest) — so build-time and re-open agree. Before the fix the stored
        // bytes were checked against decoded_digest, so this aborted at build.
        let plan = compressed_source_plan();
        let source = MemorySource::for_plan(&plan);
        // build + pack + independent re-open all pass.
        let (_bytes, report) = pack_and_validate(&plan, &source).unwrap();
        assert_eq!(report.visible_snapshot_hours, 3);
    }

    /// A single-segment plan whose lone raw source is marked standalone (the shape a
    /// multi-campaign A->B->A stream takes — it cannot be embedded under one campaign).
    fn standalone_source_plan() -> StitchedPlan {
        let mut seg = snap_seg(0, 0..=2);
        seg.raw_source.as_mut().unwrap().storage = RawSourceStorage::Standalone;
        match plan_stitch(GAME, stitch_inputs(), vec![seg]) {
            StitchOutcome::Stitched(plan) => plan,
            StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
        }
    }

    #[test]
    fn standalone_source_is_referenced_not_embedded() {
        // Finding 2: a standalone source must NOT be embedded — Container::from_bytes
        // (validate_raw_sources) rejects a standalone raw source present in the
        // manifest. Before the fix every raw source was embedded, so re-open failed.
        let plan = standalone_source_plan();
        let source = MemorySource::for_plan(&plan);
        let dir = TempDir::new("standalone");
        let dest = dir.0.join("replay.battleplan");

        let report = publish_stitched_container(&plan, &source, &dest).unwrap();
        assert_eq!(report.segments, 1);
        assert_eq!(report.visible_snapshot_hours, 3);

        // Re-open and confirm the standalone source payload is not embedded.
        let bytes = fs::read(&dest).unwrap();
        let container = Container::from_bytes(bytes, &ExpectedIdentity::default()).unwrap();
        assert!(
            container.entries_by_role("raw_source").is_empty(),
            "standalone raw source must not be embedded in the container"
        );
        let raw_path = &plan.segments[0]
            .input
            .raw_source
            .as_ref()
            .unwrap()
            .source_path;
        assert!(
            container
                .load_file(&format!("payload/{GAME}/{raw_path}"))
                .is_err(),
            "standalone source payload must be absent from the container"
        );
    }

    #[test]
    fn temp_cleanup_failure_is_surfaced_not_swallowed() {
        // Finding 5: when the post-write cleanup of the partial temp fails (Windows
        // file lock / ACL), the caller must be told a temp remains rather than have the
        // error silently discarded. A NotFound (already-gone) or Ok cleanup leaves the
        // primary error untouched.
        let temp = PathBuf::from(r"C:\replay\.replay.battleplan.stitch-tmp-123");
        let primary = "atomic publish (rename) failed: disk full".to_string();

        let ok = with_cleanup_note(primary.clone(), &temp, Ok(()));
        assert_eq!(ok, primary, "successful cleanup adds no note");

        let gone = with_cleanup_note(
            primary.clone(),
            &temp,
            Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
        );
        assert_eq!(gone, primary, "an already-gone temp satisfies the contract");

        let denied = with_cleanup_note(
            primary.clone(),
            &temp,
            Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
        );
        assert!(
            denied.starts_with(&primary),
            "primary error kept as the cause"
        );
        assert!(
            denied.contains("failed to remove temp artifact")
                && denied.contains("partial temp file remains"),
            "cleanup failure must be surfaced: {denied}"
        );
    }
}
