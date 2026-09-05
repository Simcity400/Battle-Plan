//! Zero-loss proof for the hour decode: every hour of a real recording decodes
//! to the same world document and typed views through the product path
//! (cursor-resumed reader sharing untouched root members, per-country build
//! memo, parallel view builders) as
//! through the reference path (cold cursor walk, serial builders on one
//! warnings list). Whole-capture: run explicitly.
//!
//! `VIEW_PROOF_RECORDING` names the `.battleplan`; `VIEW_PROOF_CACHE_DIR` names
//! a directory holding its `hour_index.h4ix` and `checkpoints.h4cp`.

use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Instant;

use replay_core::{BuildSchedule, RawFrame, TimelineConfig, WorldFrame, WorldTimeline};

const DEFAULT_RECORDING: &str = r"captures\e2e_06_recovered\native_session.battleplan";
const DEFAULT_CACHE_DIR: &str =
    r"captures\e2e_05\.replay_decoder_artifacts-972c_18d2009878690c70-0\replay_decoder";

struct NeverFolds;

impl snapshot::CertifiedPrefoldSpoolDecoder for NeverFolds {
    fn decoder_id(&self) -> &'static str {
        "proof.packaged-cache.never-decodes"
    }

    fn decode_certified_spool(
        &self,
        _request: snapshot::CertifiedPrefoldSpoolRequest,
    ) -> Result<snapshot::PrefoldedSpool, snapshot::PrefoldDecodeError> {
        panic!("a packaged certified cache is never folded from the raw stream");
    }
}

fn scratch_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "view_hour_decode_proof_{label}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// Every hour: the product frame against a reference frame built serially from
/// a second decode of the same hour. The reference builds run on worker threads
/// so the proof costs the product decode plus the reference walk, not their
/// sum times the hour count.
#[test]
#[ignore = "whole-capture proof: run explicitly against a real recording"]
fn every_hour_decodes_identically_through_the_product_and_reference_paths() {
    let recording = PathBuf::from(
        std::env::var("VIEW_PROOF_RECORDING").unwrap_or_else(|_| DEFAULT_RECORDING.to_string()),
    );
    assert!(recording.is_file(), "recording missing: {}", recording.display());
    let spool = scratch_dir("spool");
    let source = replay_core::open_recording_with_spooled_decoder(&recording, &spool, &NeverFolds)
        .expect("open recording");
    let timeline = WorldTimeline::open_with_config(source, TimelineConfig::default())
        .expect("open timeline");
    assert!(
        matches!(timeline.frame_cache_entries(), n if n > 0),
        "the product timeline caches frames"
    );
    let hours = timeline.hours().to_vec();
    println!(
        "{{\"proof\":\"frames\",\"recording\":{:?},\"hours\":{}}}",
        recording.display().to_string().replace('\\', "/"),
        hours.len()
    );

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .saturating_sub(2)
        .clamp(1, 6);
    let started = Instant::now();
    let (job_tx, job_rx) = mpsc::sync_channel::<(i64, std::sync::Arc<WorldFrame>, RawFrame)>(2);
    let job_rx = std::sync::Mutex::new(job_rx);
    let (verdict_tx, verdict_rx) = mpsc::channel::<(i64, bool)>();
    let (mismatches, compared) = std::thread::scope(|scope| {
        let job_rx = &job_rx;
        let timeline = &timeline;
        for _ in 0..workers {
            let verdict_tx = verdict_tx.clone();
            scope.spawn(move || loop {
                let job = job_rx.lock().expect("job lock").recv();
                let Ok((hour, product, raw)) = job else { break };
                let reference = timeline
                    .proof_frame_from_raw(raw, BuildSchedule::Serial)
                    .expect("reference frame");
                let same = *product == reference;
                verdict_tx.send((hour, same)).expect("verdict channel");
            });
        }
        drop(verdict_tx);
        for &hour in &hours {
            let product = timeline.frame(hour).expect("product frame");
            let raw = timeline.proof_raw_frame(hour).expect("raw frame");
            job_tx.send((hour, product, raw)).expect("job channel");
        }
        drop(job_tx);
        let mut mismatches = Vec::new();
        let mut compared = 0usize;
        for (hour, same) in verdict_rx {
            compared += 1;
            if !same {
                mismatches.push(hour);
            }
        }
        (mismatches, compared)
    });
    println!(
        "{{\"proof\":\"frames\",\"compared\":{compared},\"mismatches\":{},\"wall_s\":{:.1}}}",
        mismatches.len(),
        started.elapsed().as_secs_f64()
    );
    let _ = std::fs::remove_dir_all(&spool);
    assert_eq!(compared, hours.len(), "every hour was compared");
    assert!(
        mismatches.is_empty(),
        "hours whose product frame differs from the reference frame: {mismatches:?}"
    );
}

/// Every hour: the projected world reached by resuming from a rung or the
/// cursor at varied distances equals the one reached by the plain sequential
/// walk from the checkpoint. Reader A walks in order and never resumes from a
/// rung; reader B visits the hours in four interleaved passes so it resumes
/// from rungs, from its cursor, and from checkpoints.
#[test]
#[ignore = "whole-capture proof: run explicitly against a real cache"]
fn every_hour_reconstructs_identically_from_rungs_and_from_the_sequential_walk() {
    let dir = PathBuf::from(
        std::env::var("VIEW_PROOF_CACHE_DIR").unwrap_or_else(|_| DEFAULT_CACHE_DIR.to_string()),
    );
    let index_bytes = std::fs::read(dir.join("hour_index.h4ix")).expect("read index");
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes).expect("parse index");
    let hours: Vec<i64> = index.hours().iter().map(|h| h.hour_index()).collect();
    let open = || {
        snapshot::CertifiedReplayCache::open_file(
            &index_bytes,
            std::fs::File::open(dir.join("checkpoints.h4cp")).expect("open cache"),
            index.source_len(),
            index.source_sha256(),
        )
        .expect("admit cache")
    };
    let sequential = open();
    sequential.set_rung_policy(snapshot::RungPolicy {
        stride: 0,
        max_rungs: 0,
    });
    let resumed = open();
    resumed.set_rung_policy(snapshot::RungPolicy {
        stride: 4,
        max_rungs: 400,
    });
    println!(
        "{{\"proof\":\"reader\",\"cache_dir\":{:?},\"hours\":{}}}",
        dir.display().to_string().replace('\\', "/"),
        hours.len()
    );
    let started = Instant::now();
    let mut digests: std::collections::BTreeMap<i64, [u8; 32]> = std::collections::BTreeMap::new();
    for &hour in &hours {
        let (world, receipt) = sequential.reconstruct_projected(hour).expect("sequential");
        assert_ne!(receipt.resumed_from, snapshot::ResumeSource::Rung);
        digests.insert(hour, projected_digest(&world));
    }
    let mut resumed_from = std::collections::BTreeMap::new();
    let mut mismatches = Vec::new();
    for offset in 0..4usize {
        for &hour in hours.iter().skip(offset).step_by(4) {
            let (world, receipt) = resumed.reconstruct_projected(hour).expect("resumed");
            *resumed_from
                .entry(format!("{:?}", receipt.resumed_from))
                .or_insert(0usize) += 1;
            if projected_digest(&world) != digests[&hour] {
                mismatches.push(hour);
            }
        }
    }
    println!(
        "{{\"proof\":\"reader\",\"compared\":{},\"mismatches\":{},\"resumed_from\":{:?},\"rungs_held\":{},\"wall_s\":{:.1}}}",
        hours.len(),
        mismatches.len(),
        resumed_from,
        resumed.rung_count(),
        started.elapsed().as_secs_f64()
    );
    assert!(
        resumed_from.get("Rung").copied().unwrap_or(0) > 0,
        "the interleaved passes resumed from rungs"
    );
    assert!(
        mismatches.is_empty(),
        "hours whose rung-resumed world differs from the sequential walk: {mismatches:?}"
    );
}

/// Every hour: the root members the shared route hands out (a forward step
/// re-materialising only the members its delta touched, then a second pass in
/// bit-reversed order so cursor resets and far walks are exercised) equal the
/// whole-world materialisation of the same hour, member by member.
#[test]
#[ignore = "whole-capture proof: run explicitly against a real cache"]
fn every_hour_materialises_identically_through_the_shared_and_the_whole_world_routes() {
    let dir = PathBuf::from(
        std::env::var("VIEW_PROOF_CACHE_DIR").unwrap_or_else(|_| DEFAULT_CACHE_DIR.to_string()),
    );
    let index_bytes = std::fs::read(dir.join("hour_index.h4ix")).expect("read index");
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes).expect("parse index");
    let hours: Vec<i64> = index.hours().iter().map(|h| h.hour_index()).collect();
    let open = || {
        snapshot::CertifiedReplayCache::open_file(
            &index_bytes,
            std::fs::File::open(dir.join("checkpoints.h4cp")).expect("open cache"),
            index.source_len(),
            index.source_sha256(),
        )
        .expect("admit cache")
    };
    let shared = open();
    let whole = open();
    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(2)
        .clamp(1, 8);
    println!(
        "{{\"proof\":\"materialise\",\"cache_dir\":{:?},\"hours\":{}}}",
        dir.display().to_string().replace('\\', "/"),
        hours.len()
    );
    let started = Instant::now();
    let mut digests: std::collections::BTreeMap<i64, [u8; 32]> = std::collections::BTreeMap::new();
    for &hour in &hours {
        let (world, _) = whole.reconstruct_materialized(hour, workers).expect("whole world");
        digests.insert(hour, value_digest(&world));
    }
    let mut mismatches = Vec::new();
    let mut compared = 0usize;
    let mut check = |hour: i64, members: &snapshot::MaterializedRoot| {
        let world = serde_json::Value::Object(
            members
                .iter()
                .map(|(key, member)| (key.clone(), member.as_ref().clone()))
                .collect(),
        );
        compared += 1;
        if value_digest(&world) != digests[&hour] {
            mismatches.push(hour);
        }
    };
    for &hour in &hours {
        let (members, _) = shared.reconstruct_root_members(hour, workers).expect("shared, forward");
        check(hour, &members);
    }
    let bits = usize::BITS - (hours.len().max(2) - 1).leading_zeros();
    for index in 0..(1usize << bits) {
        let reversed = index.reverse_bits() >> (usize::BITS - bits);
        if reversed >= hours.len() || reversed % 211 != 0 {
            continue;
        }
        let hour = hours[reversed];
        let (members, _) = shared.reconstruct_root_members(hour, workers).expect("shared, scrub");
        check(hour, &members);
    }
    println!(
        "{{\"proof\":\"materialise\",\"compared\":{compared},\"mismatches\":{},\"wall_s\":{:.1}}}",
        mismatches.len(),
        started.elapsed().as_secs_f64()
    );
    assert!(compared >= hours.len(), "every hour was compared at least once");
    assert!(
        mismatches.is_empty(),
        "hours whose shared root members differ from the whole-world materialisation: {mismatches:?}"
    );
}

fn value_digest(value: &serde_json::Value) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(serde_json::to_vec(value).expect("serialisable world"));
    hasher.finalize().into()
}

fn projected_digest(value: &snapshot::canon_to_json::ProjectedValue) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hash_projected(value, &mut hasher);
    hasher.finalize().into()
}

fn hash_projected(value: &snapshot::canon_to_json::ProjectedValue, hasher: &mut sha2::Sha256) {
    use sha2::Digest;
    use snapshot::canon_to_json::ProjectedValue;
    match value {
        ProjectedValue::Null => hasher.update([0u8]),
        ProjectedValue::Bool(flag) => hasher.update([1u8, u8::from(*flag)]),
        ProjectedValue::Number(number) => {
            hasher.update([2u8]);
            hasher.update(number.to_string().as_bytes());
            hasher.update([0u8]);
        }
        ProjectedValue::String(text) => {
            hasher.update([3u8]);
            hasher.update(text.as_bytes());
            hasher.update([0u8]);
        }
        ProjectedValue::Array(items) => {
            hasher.update([4u8]);
            hasher.update((items.len() as u64).to_le_bytes());
            for item in items.iter() {
                hash_projected(item, hasher);
            }
        }
        ProjectedValue::Object(members) => {
            hasher.update([5u8]);
            hasher.update((members.len() as u64).to_le_bytes());
            for (name, member) in members.iter() {
                hasher.update(name.as_bytes());
                hasher.update([0u8]);
                hash_projected(member, hasher);
            }
        }
    }
}
