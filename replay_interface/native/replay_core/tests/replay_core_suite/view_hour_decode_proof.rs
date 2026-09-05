//! Zero-loss proofs for the hour decode, by digest: every hour of a real
//! recording materialises to the same world (sha256 over the canonical JSON of
//! its identity and root members) through the product routes (cursor-resumed
//! reader sharing untouched root members; the single-member read the FRD fold
//! walks) as through the reference paths (a cold checkpoint walk, a
//! whole-world materialisation). Whole-capture: run explicitly.
//!
//! `VIEW_PROOF_RECORDING` names the `.battleplan`; `VIEW_PROOF_CACHE_DIR` names
//! a directory holding its `hour_index.h4ix` and `checkpoints.h4cp`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use replay_core::{TimelineConfig, WorldTimeline};

const DEFAULT_RECORDING: &str = r"captures\e2e_07\native_session.battleplan";
const DEFAULT_CACHE_DIR: &str =
    r"captures\e2e_07\.replay_decoder_artifacts-a920_18d246c157510654-0\replay_decoder";

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

fn cache_dir() -> PathBuf {
    PathBuf::from(
        std::env::var("VIEW_PROOF_CACHE_DIR").unwrap_or_else(|_| DEFAULT_CACHE_DIR.to_string()),
    )
}

fn open_cache(dir: &PathBuf) -> (snapshot::CertifiedReplayCache, Vec<i64>) {
    let index_bytes = std::fs::read(dir.join("hour_index.h4ix")).expect("read index");
    let index = snapshot::ReplayHourIndex::from_bytes(&index_bytes).expect("parse index");
    let hours: Vec<i64> = index.hours().iter().map(|h| h.hour_index()).collect();
    let cache = snapshot::CertifiedReplayCache::open_file(
        &index_bytes,
        std::fs::File::open(dir.join("checkpoints.h4cp")).expect("open cache"),
        index.source_len(),
        index.source_sha256(),
    )
    .expect("admit cache");
    assert!(!hours.is_empty(), "proof requires indexed hours");
    (cache, hours)
}

fn proof_progress(phase: &str, completed: usize, total: usize, hour: i64, started: Instant) {
    use std::io::Write;
    let mut out = std::io::stdout().lock();
    writeln!(out, "{}", serde_json::json!({
        "proof_progress": true,
        "phase": phase,
        "completed": completed,
        "total": total,
        "hour": hour,
        "elapsed_ms": started.elapsed().as_millis(),
    })).expect("write proof progress");
    out.flush().expect("flush proof progress");
}

/// sha256 over the canonical JSON of the root members, in member order.
fn members_digest<'a>(members: impl Iterator<Item = (&'a str, &'a serde_json::Value)>) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    for (key, member) in members {
        hasher.update((key.len() as u64).to_le_bytes());
        hasher.update(key.as_bytes());
        hasher.update(serde_json::to_vec(member).expect("serialisable member"));
        hasher.update([0u8]);
    }
    hasher.finalize().into()
}

fn root_digest(members: &snapshot::MaterializedRoot) -> [u8; 32] {
    members_digest(members.iter().map(|(key, member)| (key.as_str(), member.as_ref())))
}

fn world_digest(world: &serde_json::Value) -> [u8; 32] {
    let serde_json::Value::Object(members) = world else {
        panic!("a world is a JSON object");
    };
    members_digest(members.iter().map(|(key, member)| (key.as_str(), member)))
}

/// Bit-reversed visiting order over `count` positions: every hour after the
/// first lands far from the previous one.
fn scrub_order(count: usize) -> Vec<usize> {
    let bits = usize::BITS - (count.max(2) - 1).leading_zeros();
    (0..(1usize << bits))
        .map(|index| index.reverse_bits() >> (usize::BITS - bits))
        .filter(|&reversed| reversed < count)
        .collect()
}

/// The raw body as the build receives it: identity, then the root members.
fn raw_frame_digest(raw: &replay_core::RawFrame) -> [u8; 32] {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    hasher.update(serde_json::to_vec(&(
        raw.hour_index,
        raw.date.as_slice(),
        raw.save_version.as_slice(),
        raw.schema_version.as_slice(),
    )).expect("serialisable identity with explicit presence"));
    hasher.update([0u8]);
    hasher.update(members_digest(raw.rest.iter().map(|(key, member)| (key.as_str(), member))));
    hasher.finalize().into()
}

#[test]
fn digest_distinguishes_presence_values_and_gap_witnesses() {
    use serde_json::json;
    let base = replay_core::RawFrame {
        hour_index: 7,
        date: None,
        save_version: None,
        schema_version: None,
        rest: replay_core::RootMembers::new(),
    };
    let mut variants = vec![base.clone()];
    for text in ["", "x", "x\0y"] {
        let mut date = base.clone();
        date.date = Some(text.into());
        variants.push(date);
        let mut version = base.clone();
        version.save_version = Some(text.into());
        variants.push(version);
    }
    for value in [json!(null), json!(0), json!(""), json!([]), json!({})] {
        let mut frame = base.clone();
        frame.schema_version = Some(value);
        variants.push(frame);
    }
    let mut hour = base.clone();
    hour.hour_index += 1;
    variants.push(hour);
    for value in [json!(null), json!(0), json!(false), json!(""), json!([]),
        json!({}), json!([0, 1]), json!([1, 0]),
        json!({"cause": "partial", "witness": 1}),
        json!({"cause": "partial", "witness": 2}),
        json!({"cause": "missed", "witness": 1}),
    ] {
        let mut frame = base.clone();
        frame.rest.insert("gap".into(), value);
        variants.push(frame);
    }
    for key in ["gap\0tail", "other"] {
        let mut frame = base.clone();
        frame.rest.insert(key.into(), json!(null));
        variants.push(frame);
    }
    let digests: std::collections::BTreeSet<_> = variants.iter().map(raw_frame_digest).collect();
    assert_eq!(digests.len(), variants.len(), "no truth state, identity or gap witness collapsed");
    assert_eq!(raw_frame_digest(&base), raw_frame_digest(&base.clone()));
}

/// Every hour, in playback order: the raw body the product route hands the
/// build (the cursor-resumed, member-sharing route) and the FRD member by the
/// single-member route the history fold walks, each against a whole-world
/// materialisation of the same hour, by digest.
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
    let hours = timeline.hours().to_vec();
    assert!(!hours.is_empty(), "proof requires recorded hours");
    println!(
        "{{\"proof\":\"frames\",\"recording\":{:?},\"hours\":{}}}",
        recording.display().to_string().replace('\\', "/"),
        hours.len()
    );
    let started = Instant::now();
    let mut mismatches = Vec::new();
    let mut member_mismatches = Vec::new();
    let mut compared = 0usize;
    let mut members_present = 0usize;
    let mut product_digests = BTreeMap::new();
    let phase_started = Instant::now();
    for (completed, &hour) in hours.iter().enumerate() {
        let product = timeline.proof_product_raw_frame(hour).expect("product raw frame");
        product_digests.insert(hour, raw_frame_digest(&product));
        proof_progress("frames.product", completed + 1, hours.len(), hour, phase_started);
    }
    let mut reference_digests = BTreeMap::new();
    let mut reference_members = BTreeMap::new();
    let phase_started = Instant::now();
    for (completed, &hour) in hours.iter().enumerate() {
        let reference = timeline.proof_raw_frame(hour).expect("reference frame");
        reference_digests.insert(hour, raw_frame_digest(&reference));
        let key = "_r2_direct_frd_transitions";
        reference_members.insert(hour, reference.rest.get(key).map(|value| {
            members_digest([(key, value)].into_iter())
        }));
        proof_progress("frames.reference", completed + 1, hours.len(), hour, phase_started);
    }
    let phase_started = Instant::now();
    for (completed, &hour) in hours.iter().enumerate() {
        compared += 1;
        if product_digests[&hour] != reference_digests[&hour] {
            mismatches.push(hour);
        }
        let (key, member) = timeline.proof_frd_member(hour).expect("frd member");
        assert_eq!(key, "_r2_direct_frd_transitions", "proof uses the production FRD member");
        members_present += usize::from(member.is_some());
        let digest = member.as_ref().map(|value| members_digest([(key, value)].into_iter()));
        if digest != reference_members[&hour] {
            member_mismatches.push(hour);
        }
        proof_progress("frames.frd", completed + 1, hours.len(), hour, phase_started);
    }
    println!(
        "{{\"proof\":\"frames\",\"compared\":{compared},\"mismatches\":{},\"frd_members\":{members_present},\"frd_member_mismatches\":{},\"wall_s\":{:.1}}}",
        mismatches.len(),
        member_mismatches.len(),
        started.elapsed().as_secs_f64()
    );
    let _ = std::fs::remove_dir_all(&spool);
    assert_eq!(compared, hours.len(), "every hour was compared");
    assert!(
        mismatches.is_empty(),
        "hours whose product raw frame differs from the reference frame: {mismatches:?}"
    );
    assert!(
        member_mismatches.is_empty(),
        "hours whose single-member FRD read differs from the reference frame: {member_mismatches:?}"
    );
}

/// Every hour: the world reached along the sequential walk (the cursor resumed
/// at every delta hour, each checkpoint hour decoded in place) equals the one
/// reached cold from its checkpoint (the cursor forgotten before every hour),
/// by digest; then a scrub over the cold reader, whose every hour is a
/// checkpoint decode plus its deltas.
#[test]
#[ignore = "whole-capture proof: run explicitly against a real cache"]
fn every_hour_reconstructs_identically_from_the_checkpoint_and_from_the_sequential_walk() {
    let dir = cache_dir();
    let (sequential, hours) = open_cache(&dir);
    let (cold, _) = open_cache(&dir);
    let workers = sequential.workers();
    println!(
        "{{\"proof\":\"reader\",\"cache_dir\":{:?},\"hours\":{},\"workers\":{workers}}}",
        dir.display().to_string().replace('\\', "/"),
        hours.len()
    );
    let started = Instant::now();
    let mut digests: BTreeMap<i64, [u8; 32]> = BTreeMap::new();
    let mut resumed = 0usize;
    let mut checkpoints = 0usize;
    let phase_started = Instant::now();
    for (completed, &hour) in hours.iter().enumerate() {
        let (members, receipt) = sequential.reconstruct_root_members(hour, workers).expect("sequential");
        match (receipt.resumed_from, receipt.applied_deltas) {
            (snapshot::ResumeSource::Cursor, _) => resumed += 1,
            (snapshot::ResumeSource::Checkpoint, 0) => checkpoints += 1,
            (source, deltas) => panic!("hour {hour} lost the sequential cursor: {source:?} + {deltas} delta(s)"),
        }
        digests.insert(hour, root_digest(&members));
        proof_progress("reader.sequential", completed + 1, hours.len(), hour, phase_started);
    }
    let mut mismatches = Vec::new();
    let mut compared = 0usize;
    let mut max_deltas = 0u32;
    let mut walk_ms: Vec<f64> = Vec::new();
    let phase_started = Instant::now();
    for (completed, &hour) in hours.iter().enumerate() {
        cold.forget_scrub_cursor();
        let (members, receipt) = cold.reconstruct_root_members(hour, workers).expect("cold");
        assert_eq!(receipt.resumed_from, snapshot::ResumeSource::Checkpoint);
        assert_eq!(receipt.walked_deltas, receipt.applied_deltas);
        max_deltas = max_deltas.max(receipt.applied_deltas);
        walk_ms.push(receipt.walk_ms + receipt.finish_ms);
        compared += 1;
        if root_digest(&members) != digests[&hour] {
            mismatches.push(hour);
        }
        proof_progress("reader.cold", completed + 1, hours.len(), hour, phase_started);
    }
    let phase_started = Instant::now();
    for (completed, position) in scrub_order(hours.len()).into_iter().enumerate() {
        let hour = hours[position];
        let (members, _) = cold.reconstruct_root_members(hour, workers).expect("scrub");
        compared += 1;
        if root_digest(&members) != digests[&hour] {
            mismatches.push(hour);
        }
        proof_progress("reader.scrub", completed + 1, hours.len(), hour, phase_started);
    }
    walk_ms.sort_by(|a, b| a.partial_cmp(b).expect("no NaN"));
    let p = |q: f64| walk_ms[((walk_ms.len() - 1) as f64 * q).round() as usize];
    println!(
        "{{\"proof\":\"reader\",\"compared\":{compared},\"mismatches\":{},\"sequential_resumed\":{resumed},\"sequential_checkpoints\":{checkpoints},\"max_deltas\":{max_deltas},\"cold_hour_ms_p50\":{:.1},\"cold_hour_ms_p99\":{:.1},\"wall_s\":{:.1}}}",
        mismatches.len(),
        p(0.5),
        p(0.99),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(
        resumed + checkpoints,
        hours.len(),
        "every sequential hour resumed the cursor or decoded its own checkpoint"
    );
    assert!(resumed > 0 && checkpoints > 0, "exercise both cursor and checkpoint routes");
    assert_eq!(compared, hours.len() * 2, "every hour was compared cold and scrubbed");
    assert!(
        mismatches.is_empty(),
        "hours whose cold reconstruction differs from the sequential walk: {mismatches:?}"
    );
}

/// Every hour: the root members the shared route hands out (a forward step
/// re-materialising only the members its delta touched, then a scrub so cursor
/// resets and far walks are exercised) equal the whole-world materialisation of
/// the same hour, by digest.
#[test]
#[ignore = "whole-capture proof: run explicitly against a real cache"]
fn every_hour_materialises_identically_through_the_shared_and_the_whole_world_routes() {
    let dir = cache_dir();
    let (shared, hours) = open_cache(&dir);
    let (whole, _) = open_cache(&dir);
    let workers = shared.workers();
    println!(
        "{{\"proof\":\"materialise\",\"cache_dir\":{:?},\"hours\":{}}}",
        dir.display().to_string().replace('\\', "/"),
        hours.len()
    );
    let started = Instant::now();
    let mut digests: BTreeMap<i64, [u8; 32]> = BTreeMap::new();
    for &hour in &hours {
        let (world, _) = whole.reconstruct_value(hour).expect("whole world");
        digests.insert(hour, world_digest(&world));
    }
    let mut mismatches = Vec::new();
    let mut compared = 0usize;
    let mut previous: Option<Arc<snapshot::MaterializedRoot>> = None;
    let mut shared_members = 0usize;
    for &hour in &hours {
        let (members, _) = shared.reconstruct_root_members(hour, workers).expect("shared, forward");
        if let Some(previous) = previous.as_ref() {
            shared_members += members
                .iter()
                .filter(|(key, member)| {
                    previous
                        .iter()
                        .any(|(seen, held)| seen == key && Arc::ptr_eq(held, member))
                })
                .count();
        }
        compared += 1;
        if root_digest(&members) != digests[&hour] {
            mismatches.push(hour);
        }
        previous = Some(members);
    }
    for position in scrub_order(hours.len()) {
        let hour = hours[position];
        let (members, _) = shared.reconstruct_root_members(hour, workers).expect("shared, scrub");
        compared += 1;
        if root_digest(&members) != digests[&hour] {
            mismatches.push(hour);
        }
    }
    println!(
        "{{\"proof\":\"materialise\",\"compared\":{compared},\"mismatches\":{},\"members_shared_forward\":{shared_members},\"wall_s\":{:.1}}}",
        mismatches.len(),
        started.elapsed().as_secs_f64()
    );
    assert_eq!(compared, hours.len() * 2, "every hour was compared forward and scrubbed");
    assert!(shared_members > 0, "the forward walk shared untouched members");
    assert!(
        mismatches.is_empty(),
        "hours whose shared root members differ from the whole-world materialisation: {mismatches:?}"
    );
}

#[path = "view2_sample_measurement.rs"]
mod sampled;
