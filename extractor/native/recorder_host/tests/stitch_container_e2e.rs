//! End-to-end: `plan_stitch` -> canonical container files + snapshot payloads ->
//! pack a real v2 `.battleplan` -> open with the snapshot reader and verify the
//! main timeline, rollback access, the segment graph, integrity, and determinism
//! (`STITCHING_DESIGN.md` §7, §8, §11 gates 1/7/8).

use std::collections::BTreeMap;

use recorder_host::stitching::{
    plan_stitch, AppendTicket, CampaignIdentityResolution, CompatibilityResolution,
    CoverageDisposition, HourObservation, ObservationOwner, PlannedSegment, RawSourceDescriptor,
    RawSourceStorage, SegmentInput, SegmentProvenance, SourceParticipation, StitchOutcome,
    StitchRequiredInputs, StitchedPlan, STITCH_ENTRY_ORDER, STITCH_REQUIRED_FEATURE,
};
use serde_json::{json, Value};
use snapshot::canon::manifest_canonical_bytes;
use snapshot::container::{Container, ExpectedIdentity};
use snapshot::sha256::Sha256;
use snapshot::zip::ZipArchive;
use snapshot::{pack_with, PackEntry, PackIdentity, PackOptions};

const GAME: &str = "g";
const MODSET: &str = "MS";
const K: &[u8] = b"campaign-A";

/// Snapshot body bytes as a pure function of (play_order, hour) so two segments'
/// same-hour snapshots differ and each has a distinct hash.
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
        fold_version: "world-fold-e2e-v1".into(),
        canonical_writer_version: "snapshot-canon-e2e-v1".into(),
        packer_codec_version: "zip-e2e-v1".into(),
        created_utc: "2026-07-12T00:00:00Z".into(),
        compression_method: "store".into(),
        compression_parameters: BTreeMap::from([("level".into(), "none".into())]),
        entry_order: STITCH_ENTRY_ORDER.into(),
    }
}

fn snap_seg(play_order: u64, range: std::ops::RangeInclusive<i64>) -> SegmentInput {
    let hours: Vec<i64> = range.collect();
    let append_ticket = AppendTicket {
        campaign_key: K.to_vec(),
        play_order,
        session_nonce: format!("nonce-{play_order}"),
        process_start_identity: format!("pid-{play_order}"),
    };
    let epoch_token = format!("epoch-{play_order}");
    let owner = ObservationOwner {
        ticket: append_ticket.clone(),
        in_session_ordinal: 0,
        epoch_token: epoch_token.clone(),
    };
    let mut coverage = BTreeMap::new();
    let mut observations = BTreeMap::new();
    for (index, h) in hours.iter().copied().enumerate() {
        let disposition = CoverageDisposition::Snapshot {
            sha256: Sha256::hex(&snap_payload(play_order, h)),
            date: Some(format!("1936.1.1.{h}")),
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
    let source_bytes = raw_payload(play_order, byte_end as usize);
    let source_digest = Sha256::hex(&source_bytes);
    SegmentInput {
        play_order,
        in_session_ordinal: 0,
        identity: CampaignIdentityResolution::Verified {
            scheme_major: 1,
            campaign_key: K.to_vec(),
            evidence_digest: [0u8; 32],
        },
        ticket: Some(append_ticket.clone()),
        compatibility: CompatibilityResolution::Proven {
            evidence_id: format!("compat-{play_order}"),
            compatibility_class: "test-decoder-v1".into(),
            build_exe_sha256: "ab".repeat(32),
            modset_id: MODSET.into(),
            schema_version: "1".into(),
            static_catalog_digest: "cd".repeat(32),
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
                ticket: append_ticket,
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
            build_exe_sha256: Some("ab".repeat(32)),
            modset_id: Some(MODSET.into()),
            schema_version: Some("1".into()),
            static_catalog_digest: Some("cd".repeat(32)),
            source_path: Some(source_path),
            source_stored_digest: Some(source_digest.clone()),
            source_decoded_digest: Some(source_digest),
            record_seq_bounds: Some((0, record_end)),
            byte_bounds: Some((0, byte_end)),
            ..Default::default()
        },
    }
}

/// Build the full container payload set (stitched files + snapshot payloads) for a
/// plan, deterministically.
fn container_entries(plan: &StitchedPlan) -> Vec<PackEntry> {
    let mut entries: Vec<PackEntry> = Vec::new();
    // stitched index/segments/segment-index files.
    for f in plan.to_container_files() {
        let role: &'static str = match f.role {
            "index" => "index",
            "segments" => "segments",
            "segment_index" => "segment_index",
            _ => "aux",
        };
        entries.push(PackEntry {
            path: format!("payload/{GAME}/{}", f.rel_path),
            bytes: f.bytes,
            role,
            hour_index: None,
        });
    }
    // Each exact raw source is stored once, independently of how many segments
    // reference its participation bounds.
    let mut raw_sources = BTreeMap::new();
    for segment in &plan.segments {
        let source = segment.input.raw_source.as_ref().unwrap();
        raw_sources.insert(
            source.source_path.clone(),
            raw_payload(
                segment.input.play_order,
                (source.byte_bounds.1 - source.byte_bounds.0) as usize,
            ),
        );
    }
    for (path, bytes) in raw_sources {
        entries.push(PackEntry {
            path: format!("payload/{GAME}/{path}"),
            bytes,
            role: "raw_source",
            hour_index: None,
        });
    }
    // snapshot payloads: EVERY snapshot hour of EVERY segment (visible + rollback),
    // named by the segment's assigned global ordinal.
    for p in &plan.segments {
        let po = p.input.play_order;
        for (h, disp) in p.input.coverage.iter() {
            if disp.is_snapshot() {
                let file = PlannedSegment::snapshot_file(p.segment_id, *h);
                entries.push(PackEntry {
                    path: format!("payload/{GAME}/{file}"),
                    bytes: snap_payload(po, *h),
                    role: "snapshot",
                    hour_index: Some(*h),
                });
            }
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    entries
}

fn canon(value: &Value) -> Vec<u8> {
    let mut canonical = String::new();
    snapshot::canon::write_canonical(value, &mut canonical);
    canonical.into_bytes()
}

fn push_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn push_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_le_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn stored_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, bytes) in entries {
        let name = name.as_bytes();
        let size = u32::try_from(bytes.len()).unwrap();
        let name_len = u16::try_from(name.len()).unwrap();
        let crc = crc32(bytes);
        let local_offset = u32::try_from(out.len()).unwrap();

        push_u32(&mut out, 0x0403_4b50);
        push_u16(&mut out, 20);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u16(&mut out, 0);
        push_u32(&mut out, crc);
        push_u32(&mut out, size);
        push_u32(&mut out, size);
        push_u16(&mut out, name_len);
        push_u16(&mut out, 0);
        out.extend_from_slice(name);
        out.extend_from_slice(bytes);

        push_u32(&mut central, 0x0201_4b50);
        push_u16(&mut central, 20);
        push_u16(&mut central, 20);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, crc);
        push_u32(&mut central, size);
        push_u32(&mut central, size);
        push_u16(&mut central, name_len);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u16(&mut central, 0);
        push_u32(&mut central, 0);
        push_u32(&mut central, local_offset);
        central.extend_from_slice(name);
    }
    let central_offset = u32::try_from(out.len()).unwrap();
    let central_size = u32::try_from(central.len()).unwrap();
    out.extend_from_slice(&central);
    let count = u16::try_from(entries.len()).unwrap();
    push_u32(&mut out, 0x0605_4b50);
    push_u16(&mut out, 0);
    push_u16(&mut out, 0);
    push_u16(&mut out, count);
    push_u16(&mut out, count);
    push_u32(&mut out, central_size);
    push_u32(&mut out, central_offset);
    push_u16(&mut out, 0);
    out
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes(bytes[offset..offset + 2].try_into().unwrap())
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn reorder_central_directory(mut bytes: Vec<u8>, desired_order: &[String]) -> Vec<u8> {
    let eocd = bytes.len() - 22;
    assert_eq!(read_u32(&bytes, eocd), 0x0605_4b50);
    let central_size = read_u32(&bytes, eocd + 12) as usize;
    let central_offset = read_u32(&bytes, eocd + 16) as usize;
    let central_end = central_offset + central_size;
    let mut records = BTreeMap::new();
    let mut cursor = central_offset;
    while cursor < central_end {
        assert_eq!(read_u32(&bytes, cursor), 0x0201_4b50);
        let name_len = read_u16(&bytes, cursor + 28) as usize;
        let extra_len = read_u16(&bytes, cursor + 30) as usize;
        let comment_len = read_u16(&bytes, cursor + 32) as usize;
        let next = cursor + 46 + name_len + extra_len + comment_len;
        let name = std::str::from_utf8(&bytes[cursor + 46..cursor + 46 + name_len])
            .unwrap()
            .to_string();
        assert!(records.insert(name, bytes[cursor..next].to_vec()).is_none());
        cursor = next;
    }
    assert_eq!(cursor, central_end);

    let mut reordered = Vec::with_capacity(central_size);
    for name in desired_order {
        reordered.extend_from_slice(&records.remove(name).expect("central record"));
    }
    assert!(records.is_empty());
    assert_eq!(reordered.len(), central_size);
    bytes[central_offset..central_end].copy_from_slice(&reordered);
    bytes
}

fn lexicographic_central_with_nonlexicographic_local_zip(bytes: Vec<u8>) -> Vec<u8> {
    let archive = ZipArchive::parse(bytes).expect("parse packer output");
    let mut payloads = BTreeMap::new();
    let mut manifest_bytes = None;
    let mut manifest = None;
    for entry in archive.entries() {
        let body = archive.read_entry(entry).expect("read packed entry");
        if entry.name == "manifest.json" {
            manifest = Some(serde_json::from_slice::<Value>(&body).expect("manifest json"));
            manifest_bytes = Some(body);
        } else {
            payloads.insert(entry.name.clone(), body);
        }
    }
    let manifest = manifest.expect("manifest entry");
    let declared_order: Vec<String> = manifest["entries"]
        .as_array()
        .expect("manifest entries")
        .iter()
        .map(|row| row["path"].as_str().unwrap().to_string())
        .collect();
    let mut physical_entries = Vec::new();
    for path in declared_order.iter().rev() {
        physical_entries.push((
            path.clone(),
            payloads.remove(path).expect("manifest payload"),
        ));
    }
    assert!(payloads.is_empty());
    physical_entries.push(("manifest.json".into(), manifest_bytes.unwrap()));
    let mut central_order = declared_order;
    central_order.push("manifest.json".into());
    reorder_central_directory(stored_zip(&physical_entries), &central_order)
}

fn resealed_nonlexicographic_manifest_and_zip(bytes: Vec<u8>) -> Vec<u8> {
    let archive = ZipArchive::parse(bytes).expect("parse packer output");
    let mut payloads = BTreeMap::new();
    let mut manifest = None;
    for entry in archive.entries() {
        let body = archive.read_entry(entry).expect("read packed entry");
        if entry.name == "manifest.json" {
            manifest = Some(serde_json::from_slice::<Value>(&body).expect("manifest json"));
        } else {
            payloads.insert(entry.name.clone(), body);
        }
    }
    let mut manifest = manifest.expect("manifest entry");
    manifest["entries"]
        .as_array_mut()
        .expect("manifest entries")
        .reverse();
    manifest["manifest_digest"] = json!(Sha256::hex(&manifest_canonical_bytes(&manifest)));

    let mut rebuilt = Vec::new();
    for row in manifest["entries"].as_array().unwrap() {
        let path = row["path"].as_str().unwrap().to_string();
        rebuilt.push((
            path.clone(),
            payloads.remove(&path).expect("manifest payload"),
        ));
    }
    assert!(payloads.is_empty());
    rebuilt.push((
        "manifest.json".into(),
        serde_json::to_vec(&manifest).unwrap(),
    ));
    stored_zip(&rebuilt)
}

fn replace_json_entry(entries: &mut [PackEntry], suffix: &str, mutate: impl FnOnce(&mut Value)) {
    let entry = entries
        .iter_mut()
        .find(|entry| entry.path.ends_with(suffix))
        .unwrap_or_else(|| panic!("missing entry {suffix}"));
    let mut value: Value = serde_json::from_slice(&entry.bytes).unwrap();
    mutate(&mut value);
    entry.bytes = canon(&value);
}

fn rewrite_play_order(entries: &mut [PackEntry], segment_id: u32, new_order: u64) {
    let graph_entry = entries
        .iter_mut()
        .find(|entry| entry.path.ends_with("segments.json"))
        .unwrap();
    let mut graph: Value = serde_json::from_slice(&graph_entry.bytes).unwrap();
    let source_key = {
        let segment = &mut graph["segments"][segment_id as usize];
        let source_key = segment["source_ref"]["source_key"]
            .as_str()
            .unwrap()
            .to_string();
        segment["play_order"] = json!(new_order);
        segment["ticket"]["play_order"] = json!(new_order);
        source_key
    };
    let source = graph["raw_sources"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|source| source["source_key"].as_str() == Some(&source_key))
        .unwrap();
    let participation = &mut source["participations"][0];
    participation["ticket"]["play_order"] = json!(new_order);
    let mut binding = participation.clone();
    binding.as_object_mut().unwrap().remove("participation_id");
    let participation_id = Sha256::hex(&canon(&binding));
    participation["participation_id"] = json!(participation_id);
    graph["segments"][segment_id as usize]["source_ref"]["participation_id"] =
        json!(participation_id);
    graph_entry.bytes = canon(&graph);

    replace_json_entry(
        entries,
        &format!("segments/s{segment_id:08}.index.json"),
        |index| {
            index["source_ref"]["participation_id"] = json!(participation_id);
            for collection in ["hours", "transition_coverage"] {
                for row in index[collection].as_array_mut().unwrap() {
                    row["owner"]["play_order"] = json!(new_order);
                    if let Some(observations) =
                        row.get_mut("observations").and_then(Value::as_array_mut)
                    {
                        for observation in observations {
                            observation["owner"]["play_order"] = json!(new_order);
                        }
                    }
                }
            }
        },
    );
    replace_json_entry(entries, "index.json", |index| {
        for collection in ["snapshots", "coverage"] {
            for row in index[collection].as_array_mut().unwrap() {
                if row["owner"]["session_nonce"] == json!(format!("nonce-{segment_id}")) {
                    row["owner"]["play_order"] = json!(new_order);
                }
            }
        }
    });
}

fn plan_with_multiple_observations() -> StitchedPlan {
    let mut segment = snap_seg(0, 0..=0);
    let owner = segment.observations[&0][0].owner.clone();
    let disposition = segment.coverage[&0].clone();
    segment
        .observations
        .get_mut(&0)
        .unwrap()
        .push(HourObservation {
            observation_id: "obs-0-0-later".into(),
            owner,
            record_seq_bounds: (2, 3),
            byte_bounds: (100, 200),
            disposition,
        });
    let source_digest = Sha256::hex(&raw_payload(0, 200));
    segment.provenance.record_seq_bounds = Some((0, 3));
    segment.provenance.byte_bounds = Some((0, 200));
    segment.provenance.source_stored_digest = Some(source_digest.clone());
    segment.provenance.source_decoded_digest = Some(source_digest.clone());
    let source = segment.raw_source.as_mut().unwrap();
    source.stored_digest = source_digest.clone();
    source.decoded_digest = source_digest;
    source.record_seq_bounds = (0, 3);
    source.byte_bounds = (0, 200);
    source.participations[0].record_seq_bounds = (0, 3);
    source.participations[0].byte_bounds = (0, 200);
    match plan_stitch(GAME, stitch_inputs(), vec![segment]) {
        StitchOutcome::Stitched(plan) => plan,
        StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
    }
}

fn identity() -> PackIdentity {
    PackIdentity {
        schema_version: Some("1".into()),
        build_exe_sha256: Some("ab".repeat(32)),
        build_pinned: false,
        mod_identity: Some(serde_json::json!({ "modset_id": MODSET })),
        created_utc: Some("2026-07-12T00:00:00Z".into()),
        content_manifest: None,
    }
}

fn stitched_opts() -> PackOptions {
    PackOptions {
        container_version: Some(2),
        required_features: Some(vec![STITCH_REQUIRED_FEATURE.into()]),
        ..Default::default()
    }
}

fn plan_rollback() -> StitchedPlan {
    // §11 gate 1: rollback inside one campaign — S0=0..3, S1=2..2 (supersedes hour 2).
    match plan_stitch(
        "g",
        stitch_inputs(),
        vec![snap_seg(0, 0..=3), snap_seg(1, 2..=2)],
    ) {
        StitchOutcome::Stitched(p) => p,
        StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
    }
}

fn plan_same_cutover() -> StitchedPlan {
    match plan_stitch(
        GAME,
        stitch_inputs(),
        vec![snap_seg(0, 0..=3), snap_seg(1, 2..=2), snap_seg(2, 2..=2)],
    ) {
        StitchOutcome::Stitched(plan) => plan,
        StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
    }
}

#[test]
fn stitched_plan_packs_and_reader_serves_main_and_rollback() {
    let plan = plan_rollback();
    let bytes = pack_with(
        GAME,
        &container_entries(&plan),
        &identity(),
        false,
        &stitched_opts(),
    )
    .expect("pack stitched container");
    let c = Container::from_bytes(bytes, &ExpectedIdentity::default())
        .expect("open stitched container");

    // main timeline: hours 0,1 from S0; hour 2 from S1. Hour 3 (S0's abandoned
    // future beyond S1's head=2) is REMOVED from main by S1's cutover and becomes
    // rollback-only — playback ends at hour 2 (design §6 abandoned-future rule).
    assert!(c.index().is_stitched());
    let hours: Vec<i64> = c
        .index()
        .snapshots
        .iter()
        .filter_map(|s| s.hour_index)
        .collect();
    assert_eq!(hours, vec![0, 1, 2]);
    // hour 2 resolves to segment 1's file (the later branch), not segment 0's.
    let h2 = c
        .index()
        .snapshots
        .iter()
        .find(|s| s.hour_index == Some(2))
        .unwrap();
    assert_eq!(h2.file, "s00000001.000000000002.snapshot.json");
    // and its bytes carry po=1 (segment 1), proving the later branch won.
    let s = c.load(2).unwrap();
    assert_eq!(s.rest.get("po").and_then(|v| v.as_u64()), Some(1));

    // the SUPERSEDED hour-2 snapshot (segment 0, po=0) is retained and loadable.
    let rollback = c
        .load_file("s00000000.000000000002.snapshot.json")
        .expect("segment 0 rollback hour 2 retained");
    assert_eq!(rollback.rest.get("po").and_then(|v| v.as_u64()), Some(0));
    // the abandoned-future hour-3 snapshot (segment 0) is also retained as
    // rollback-only: loadable by file, absent from the main index.
    assert!(c.load_file("s00000000.000000000003.snapshot.json").is_ok());
    assert!(!c.index().snapshots.iter().any(|s| s.hour_index == Some(3)));

    // the branch graph is present and records the supersession.
    let g = c.segments().unwrap().expect("segments graph");
    let seg0 = g.segments.iter().find(|s| s.segment_id == 0).unwrap();
    assert!(seg0
        .superseded_spans
        .iter()
        .any(|s| s["superseded_by"].as_i64() == Some(1)));

    // integrity: every entry (incl. the rollback snapshot) hash-verifies.
    c.verify_all().expect("verify_all");
}

#[test]
fn valid_multi_observation_plan_packs_and_opens() {
    let plan = plan_with_multiple_observations();
    let bytes = pack_with(
        GAME,
        &container_entries(&plan),
        &identity(),
        false,
        &stitched_opts(),
    )
    .expect("pack multi-observation stitched container");
    let container = Container::from_bytes(bytes, &ExpectedIdentity::default())
        .expect("open multi-observation stitched container");
    container.verify_all().expect("verify every retained byte");
    assert_eq!(container.load(0).unwrap().hour_index, Some(0));
}

#[test]
fn same_cutover_ticket_order_is_proven_at_open() {
    let plan = plan_same_cutover();
    let entries = container_entries(&plan);
    let valid = pack_with(GAME, &entries, &identity(), false, &stitched_opts()).unwrap();
    Container::from_bytes(valid, &ExpectedIdentity::default())
        .expect("correctly ticket-ordered same-cutover container opens");

    let mut duplicate = container_entries(&plan);
    rewrite_play_order(&mut duplicate, 2, 1);
    let duplicate = pack_with(GAME, &duplicate, &identity(), false, &stitched_opts()).unwrap();
    let duplicate_error = Container::from_bytes(duplicate, &ExpectedIdentity::default())
        .err()
        .expect("duplicate play_order must reject")
        .to_string();
    assert!(duplicate_error.contains("play_order"), "{duplicate_error}");

    let mut reversed = container_entries(&plan);
    rewrite_play_order(&mut reversed, 1, 2);
    rewrite_play_order(&mut reversed, 2, 1);
    let reversed = pack_with(GAME, &reversed, &identity(), false, &stitched_opts()).unwrap();
    let reversed_error = Container::from_bytes(reversed, &ExpectedIdentity::default())
        .err()
        .expect("reversed play_order must reject")
        .to_string();
    assert!(reversed_error.contains("play_order"), "{reversed_error}");
}

#[test]
fn open_rejects_created_utc_different_from_planned_publication() {
    let plan = plan_rollback();
    let mut actual_identity = identity();
    actual_identity.created_utc = Some("2026-07-13T00:00:00Z".into());
    let bytes = pack_with(
        GAME,
        &container_entries(&plan),
        &actual_identity,
        false,
        &stitched_opts(),
    )
    .unwrap();
    assert!(Container::from_bytes(bytes, &ExpectedIdentity::default()).is_err());
}

#[test]
fn open_rejects_compression_different_from_planned_publication() {
    let plan = plan_rollback();
    let bytes = pack_with(
        GAME,
        &container_entries(&plan),
        &identity(),
        true,
        &stitched_opts(),
    )
    .unwrap();
    assert!(Container::from_bytes(bytes, &ExpectedIdentity::default()).is_err());
}

#[test]
fn open_rejects_compression_profile_different_from_planned_publication() {
    let mut inputs = stitch_inputs();
    inputs
        .compression_parameters
        .insert("level".into(), "different".into());
    let plan = match plan_stitch(GAME, inputs, vec![snap_seg(0, 0..=3), snap_seg(1, 2..=2)]) {
        StitchOutcome::Stitched(plan) => plan,
        StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
    };
    let bytes = pack_with(
        GAME,
        &container_entries(&plan),
        &identity(),
        false,
        &stitched_opts(),
    )
    .unwrap();
    assert!(Container::from_bytes(bytes, &ExpectedIdentity::default()).is_err());
}

#[test]
fn open_fails_closed_when_declared_deflate_profile_cannot_be_reverified() {
    let mut inputs = stitch_inputs();
    inputs.compression_method = "deflate".into();
    inputs
        .compression_parameters
        .insert("level".into(), "6".into());
    let plan = match plan_stitch(GAME, inputs, vec![snap_seg(0, 0..=3), snap_seg(1, 2..=2)]) {
        StitchOutcome::Stitched(plan) => plan,
        StitchOutcome::Separate { reason } => panic!("expected Stitched: {reason}"),
    };
    let bytes = pack_with(
        GAME,
        &container_entries(&plan),
        &identity(),
        true,
        &stitched_opts(),
    )
    .unwrap();
    assert!(Container::from_bytes(bytes, &ExpectedIdentity::default()).is_err());
}

#[test]
fn open_rejects_entry_order_different_from_planned_publication() {
    let plan = plan_rollback();
    let mut entries = container_entries(&plan);
    entries.reverse();
    let bytes = pack_with(GAME, &entries, &identity(), false, &stitched_opts()).unwrap();
    assert!(Container::from_bytes(bytes, &ExpectedIdentity::default()).is_err());

    let canonical = pack_with(
        GAME,
        &container_entries(&plan),
        &identity(),
        false,
        &stitched_opts(),
    )
    .unwrap();
    let handcrafted = resealed_nonlexicographic_manifest_and_zip(canonical);
    assert!(
        Container::from_bytes(handcrafted, &ExpectedIdentity::default()).is_err(),
        "matching non-lexicographic manifest and ZIP order must not satisfy the declared policy"
    );

    let canonical = pack_with(
        GAME,
        &container_entries(&plan),
        &identity(),
        false,
        &stitched_opts(),
    )
    .unwrap();
    let split_order = lexicographic_central_with_nonlexicographic_local_zip(canonical);
    assert!(
        Container::from_bytes(split_order, &ExpectedIdentity::default()).is_err(),
        "lexicographic central-directory claims must not hide non-lexicographic physical order"
    );
}
