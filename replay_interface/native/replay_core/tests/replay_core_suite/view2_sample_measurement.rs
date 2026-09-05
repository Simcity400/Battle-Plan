use std::collections::BTreeSet;
use std::io::Write;
use std::path::PathBuf;
use std::time::Instant;

use replay_core::{RawFrame, TimelineConfig, WorldTimeline};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn emit(row: Value) {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{row}").expect("sample progress");
    out.flush().expect("flush sample progress");
}

fn timed<T>(phase: &str, sample: usize, hour: i64, f: impl FnOnce() -> T) -> T {
    emit(json!({"view2_sample":true,"event":"begin","phase":phase,"sample":sample,"hour":hour}));
    let started = Instant::now();
    let result = f();
    emit(json!({"view2_sample":true,"event":"complete","phase":phase,"sample":sample,
        "hour":hour,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0}));
    result
}

struct HashWriter<'a>(&'a mut Sha256);
impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

fn members_digest<'a>(members: impl Iterator<Item = (&'a str, &'a Value)>) -> [u8; 32] {
    let mut hash = Sha256::new();
    for (key, value) in members {
        hash.update((key.len() as u64).to_le_bytes());
        hash.update(key.as_bytes());
        serde_json::to_writer(HashWriter(&mut hash), value).expect("stream full member");
        hash.update([0]);
    }
    hash.finalize().into()
}

fn raw_digest(raw: &RawFrame) -> [u8; 32] {
    let mut hash = Sha256::new();
    serde_json::to_writer(HashWriter(&mut hash), &(
        raw.hour_index, raw.date.as_slice(), raw.save_version.as_slice(),
        raw.schema_version.as_slice(),
    )).expect("stream identity");
    hash.update([0]);
    hash.update(members_digest(raw.rest.iter().map(|(k, v)| (k.as_str(), v))));
    hash.finalize().into()
}

fn hex(bytes: [u8; 32]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn picks(hours: &[i64]) -> [usize; 3] {
    assert!(hours.len() >= 3, "sample requires three distinct indexed hours");
    [0, hours.len() / 2, hours.len() - 1]
}

fn open_timeline(hours: &[i64]) -> WorldTimeline {
    let recording = PathBuf::from(std::env::var("VIEW_PROOF_RECORDING").expect("recording"));
    let spool = PathBuf::from(std::env::var("VIEW_SAMPLE_SPOOL").expect("private sample spool"));
    std::fs::create_dir_all(&spool).expect("sample spool");
    let source = replay_core::open_recording_with_spooled_decoder(
        &recording, &spool, &super::NeverFolds,
    ).expect("admit packaged recording without a second prefold");
    let timeline = WorldTimeline::open_with_config(source, TimelineConfig::default())
        .expect("sample timeline");
    assert_eq!(timeline.hours(), hours, "recording/cache hour identity");
    timeline
}

fn product(hours: &[i64]) {
    for (sample, position) in picks(hours).into_iter().enumerate() {
        let hour = hours[position];
        let timeline = timed("product.open", sample, hour, || open_timeline(hours));
        let cold = timed("product.cold", sample, hour, || timeline.seek(hour).expect("cold seek"));
        let cold_digest = timed("digest.product.cold", sample, hour, ||
            hex(members_digest(cold.raw_for_consumer_tests().iter().map(|(k, v)| (k.as_str(), v)))));
        let pointer = std::sync::Arc::as_ptr(&cold);
        drop(cold);
        let warm = timed("product.warm", sample, hour, || timeline.seek(hour).expect("warm seek"));
        assert_eq!(pointer, std::sync::Arc::as_ptr(&warm), "warm route must hit the frame cache");
        let warm_digest = timed("digest.product.warm", sample, hour, ||
            hex(members_digest(warm.raw_for_consumer_tests().iter().map(|(k, v)| (k.as_str(), v)))));
        assert_eq!(cold_digest, warm_digest, "complete normalized root values differ");
        drop(warm);
        let raw = timed("product.raw", sample, hour, || timeline.proof_product_raw_frame(hour).expect("raw product"));
        let raw_sha = timed("digest.product.raw", sample, hour, || hex(raw_digest(&raw)));
        drop(raw);
        emit(json!({"view2_sample":true,"event":"product","sample":sample,"hour":hour,
            "cold_digest":cold_digest,"warm_digest":warm_digest,"raw_digest":raw_sha,
            "frame_cache_entries":timeline.frame_cache_entries()}));
        drop(timeline);
    }
}

fn reader_row(cache: &snapshot::CertifiedReplayCache, phase: &str, sample: usize, hour: i64) {
    let (root, receipt) = timed(phase, sample, hour, ||
        cache.reconstruct_root_members(hour, replay_core::world::product_worker_count()).expect("reader sample"));
    let digest = timed(&format!("digest.{phase}"), sample, hour, ||
        hex(members_digest(root.iter().map(|(k, v)| (k.as_str(), v.as_ref())))));
    emit(json!({"view2_sample":true,"event":"reader","phase":phase,"sample":sample,"hour":hour,
        "digest":digest,"resumed_from":format!("{:?}",receipt.resumed_from),
        "checkpoint_hour":receipt.checkpoint_hour,"applied_deltas":receipt.applied_deltas,
        "walked_deltas":receipt.walked_deltas,"configured_bound":receipt.configured_bound,
        "walk_ms":receipt.walk_ms,"finish_ms":receipt.finish_ms,
        "walk_read_ms":receipt.walk_read_ms,"walk_decode_ms":receipt.walk_decode_ms,
        "walk_apply_ms":receipt.walk_apply_ms}));
    if phase == "reader.cold" {
        assert_eq!(receipt.resumed_from, snapshot::ResumeSource::Checkpoint);
        assert_eq!(receipt.walked_deltas, receipt.applied_deltas);
    }
}

fn reader(cache: &snapshot::CertifiedReplayCache, hours: &[i64]) {
    for (sample, position) in picks(hours).into_iter().enumerate() {
        let hour = hours[position];
        cache.forget_scrub_cursor();
        reader_row(cache, "reader.cold", sample, hour);
        reader_row(cache, "reader.warm", sample, hour);
        cache.forget_scrub_cursor();
        let prior = position.saturating_sub(1);
        let target = if position == 0 { 1 } else { position };
        reader_row(cache, "reader.prime", sample, hours[prior]);
        reader_row(cache, "reader.cursor", sample, hours[target]);
    }
}

fn reference(cache: snapshot::CertifiedReplayCache, hours: &[i64]) {
    let mut targets: BTreeSet<usize> = picks(hours).into_iter().collect();
    targets.insert(1);
    for position in picks(hours) { targets.insert(position.saturating_sub(1)); }
    for position in targets {
        let hour = hours[position];
        cache.forget_scrub_cursor();
        let (world, _) = timed("reference.root", position, hour, ||
            cache.reconstruct_value(hour).expect("independent whole-world reference"));
        let digest = timed("digest.reference.root", position, hour, ||
            hex(members_digest(world.as_object().expect("world object").iter().map(|(k, v)| (k.as_str(), v)))));
        let body = snapshot::Snapshot::from_world_value(world).expect("reference identity");
        let raw = RawFrame { hour_index:hour, date:body.date, save_version:body.save_version,
            schema_version:body.schema_version, rest:body.rest.into() };
        let raw_sha = hex(raw_digest(&raw));
        drop(raw);
        emit(json!({"view2_sample":true,"event":"reference_root","hour":hour,
            "digest":digest,"raw_digest":raw_sha}));
    }
    drop(cache);
    let timeline = timed("reference.open", 0, hours[0], || open_timeline(hours));
    for (sample, position) in picks(hours).into_iter().enumerate() {
        let hour = hours[position];
        let raw = timed("reference.raw", sample, hour, || timeline.proof_raw_frame(hour).expect("source reference"));
        let digest = timed("digest.reference.raw", sample, hour, || hex(raw_digest(&raw)));
        drop(raw);
        emit(json!({"view2_sample":true,"event":"reference_raw","sample":sample,"hour":hour,"digest":digest}));
    }
}

#[test]
#[ignore = "bounded real-recording sample: run only through run_sample_measurement.py"]
fn sample_open_and_seek() {
    let mode = std::env::var("VIEW_SAMPLE_MODE").expect("sample mode");
    assert!(["product", "reader", "reference"].contains(&mode.as_str()));
    let dir = PathBuf::from(std::env::var("VIEW_PROOF_CACHE_DIR").expect("current cache"));
    let (mut cache, hours) = timed("cache.open", 0, 0, || super::open_cache(&dir));
    cache.set_workers(replay_core::world::product_worker_count());
    let selected: Vec<_> = picks(&hours).into_iter().map(|p| hours[p]).collect();
    emit(json!({"view2_sample":true,"event":"selection","mode":mode,"hours":hours,
        "selected":selected,"workers":cache.workers(),"cold_definition":"fresh application state; OS cache uncontrolled"}));
    match mode.as_str() {
        "product" => { drop(cache); product(&hours); }
        "reader" => reader(&cache, &hours),
        "reference" => reference(cache, &hours),
        _ => unreachable!(),
    }
    emit(json!({"view2_sample":true,"event":"done","mode":mode}));
}

#[test]
fn streaming_digest_preserves_existing_complete_preimage() {
    let worlds = [
        json!({"empty":{},"array":[],"null":null,"text":"a\u{0}b","n":0}),
        json!({"gap":{"reason":"torn","start":1,"end":2},"value":[false,1,2.5]}),
    ];
    for world in worlds {
        assert_eq!(members_digest(world.as_object().unwrap().iter().map(|(k,v)| (k.as_str(),v))),
            super::world_digest(&world));
    }
    let mut raw = RawFrame { hour_index:1,date:None,save_version:None,schema_version:None,
        rest:serde_json::Map::new().into() };
    let mut digests = BTreeSet::new();
    for value in [None, Some(Value::Null), Some(json!({})), Some(json!([])), Some(json!(0))] {
        raw.schema_version = value;
        assert_eq!(raw_digest(&raw), super::raw_frame_digest(&raw));
        digests.insert(raw_digest(&raw));
    }
    assert_eq!(digests.len(), 5);
}
