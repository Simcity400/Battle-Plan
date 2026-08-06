//! Lane CZ measurement instrument: attribute post-capture finalize wall time to
//! the phases of [`crate::build_certified_replay_artifacts`].
//!
//! It re-executes exactly the call sequence that function runs -- same decoder,
//! same render-blob preservation, same cache encoder, same whole-artifact parity
//! verification -- with a wall-clock accumulator around each phase, and it
//! re-derives the artifact digests so a run doubles as a byte-identity oracle
//! against the digests a real capture recorded in its session manifest.
//!
//! It runs those phases STRICTLY SEQUENTIALLY, and deliberately so, even though
//! production now overlaps the produce and consume halves on two threads. A
//! phase's cost is only attributable while nothing else is running: overlapped,
//! the same measurement would report wall time that two threads shared and
//! attribute it to whichever phase happened to be sampled. So this instrument
//! answers "where does the work live", not "how long does finalize take" --
//! for the latter, time `build_certified_replay_artifacts` itself (the
//! `cc-cache-probe` instrument does exactly that, and reports the artifact
//! digests alongside, so a timing run is also an identity check).
//!
//! Read-only with respect to its input stream; every output lands in the
//! caller's own empty spill directory. Never on by default, never packaged.

use std::io::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use snapshot::canon_value::CanonValue;
use snapshot::sha256::Sha256;

use crate::{
    HourCursor, IndexSourceDescriptor, OpenError, StreamIdentityMode, WorldHour,
};
use recorder_host::world_fold::schema_version;

#[derive(Default)]
struct Phases {
    decode_fold_snapshot: Duration,
    render_blobs: Duration,
    cache_push: Duration,
    take_index: Duration,
    encoder_finish: Duration,
    index_write: Duration,
    cache_read_back: Duration,
    verify_every_hour: Duration,
    /// Estimated sub-terms of `cache_push`, priced on the same bytes in a
    /// separate pass rather than measured in place.
    sub_canon_parse: Duration,
    sub_canon_serialize: Duration,
    sub_world_json_sha256: Duration,
}

fn secs(d: Duration) -> f64 {
    d.as_secs_f64()
}

/// Run the certified-artifact build with per-phase wall accounting and return a
/// single-line JSON report.
#[allow(clippy::too_many_arguments)]
pub fn profile_certified_replay_artifacts(
    raw_h4cr_path: &Path,
    output_dir: &Path,
    game_id: &str,
    identity_mode: StreamIdentityMode,
    expected_source_len: u64,
    expected_source_sha256: &str,
    checkpoint_interval_hours: u32,
    sub_terms: bool,
) -> Result<String, String> {
    let raw_h4cr_path = std::fs::canonicalize(raw_h4cr_path).map_err(|e| e.to_string())?;
    let output_dir = std::fs::canonicalize(output_dir).map_err(|e| e.to_string())?;
    if std::fs::read_dir(&output_dir)
        .map_err(|e| e.to_string())?
        .next()
        .is_some()
    {
        return Err(format!(
            "profile spill must be an empty directory: {}",
            output_dir.display()
        ));
    }

    let mut phases = Phases::default();
    let wall_started = Instant::now();

    let cache_path = output_dir.join(snapshot::CHECKPOINT_CACHE_FILE);
    let index_path = output_dir.join(snapshot::HOUR_INDEX_FILE);
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir(parent).map_err(|e| e.to_string())?;
    }
    let cache_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&cache_path)
        .map_err(|e| e.to_string())?;
    let mut cache_encoder = snapshot::ReplayCacheEncoder::new(
        cache_file,
        expected_source_sha256,
        checkpoint_interval_hours,
    )
    .map_err(|e| e.to_string())?;

    let source_file = std::fs::File::open(&raw_h4cr_path).map_err(|e| e.to_string())?;
    let mut cursor = HourCursor::from_reader_with_source(
        Box::new(source_file),
        expected_source_len,
        game_id.to_string(),
        identity_mode,
        Some(IndexSourceDescriptor {
            kind: "stream_file".to_string(),
            canonical_path: raw_h4cr_path.to_string_lossy().into_owned(),
        }),
    );

    let mut pending: Option<(WorldHour, u64)> = None;
    let mut legacy_json_bytes_avoided = 0u64;
    let mut hours_decoded = 0u64;

    loop {
        let started = Instant::now();
        let next = cursor.next_hour();
        phases.decode_fold_snapshot += started.elapsed();
        let Some(hour) = next else { break };
        let hour = hour.map_err(|e| format!("decode: {e:?}"))?;
        hours_decoded += 1;
        let start_offset = cursor
            .index_hours
            .last()
            .filter(|offset| offset.hour_index == hour.hour_index)
            .map(|offset| offset.start_offset)
            .ok_or_else(|| {
                format!(
                    "decoder emitted hour {} without its offset index row",
                    hour.hour_index
                )
            })?;
        if let Some((previous, previous_start)) = pending.take() {
            emit(
                &mut phases,
                &mut cache_encoder,
                &output_dir,
                previous,
                previous_start,
                start_offset,
                &mut legacy_json_bytes_avoided,
                sub_terms,
            )?;
        }
        pending = Some((hour, start_offset));
    }

    let started = Instant::now();
    let stream_index = cursor.take_index().map_err(|e| format!("take_index: {e}"))?;
    phases.take_index += started.elapsed();
    if stream_index.source.decoded_len != expected_source_len
        || stream_index.source.decoded_sha256 != expected_source_sha256
    {
        return Err(format!(
            "certified H4CR identity {}:{} disagrees with expected {expected_source_len}:{expected_source_sha256}",
            stream_index.source.decoded_len, stream_index.source.decoded_sha256,
        ));
    }

    let (last, last_start) = pending.ok_or("stream emitted no hours")?;
    emit(
        &mut phases,
        &mut cache_encoder,
        &output_dir,
        last,
        last_start,
        expected_source_len,
        &mut legacy_json_bytes_avoided,
        sub_terms,
    )?;

    let cache_identity_mode = match identity_mode {
        StreamIdentityMode::Legacy => snapshot::CacheIdentityMode::Legacy,
        StreamIdentityMode::IdentityV1 => snapshot::CacheIdentityMode::IdentityV1,
    };
    let started = Instant::now();
    let (mut cache_file, replay_index) = cache_encoder
        .finish(
            game_id.to_string(),
            schema_version().to_string(),
            cache_identity_mode,
            expected_source_len,
        )
        .map_err(|e| format!("finish: {e}"))?;
    cache_file.flush().map_err(|e| e.to_string())?;
    cache_file.sync_all().map_err(|e| e.to_string())?;
    drop(cache_file);
    phases.encoder_finish += started.elapsed();

    let started = Instant::now();
    let index_bytes = replay_index.to_bytes().map_err(|e| e.to_string())?;
    let mut index_file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&index_path)
        .map_err(|e| e.to_string())?;
    index_file
        .write_all(&index_bytes)
        .map_err(|e| e.to_string())?;
    index_file.flush().map_err(|e| e.to_string())?;
    index_file.sync_all().map_err(|e| e.to_string())?;
    drop(index_file);
    phases.index_write += started.elapsed();

    let started = Instant::now();
    let cache_bytes = std::fs::read(&cache_path).map_err(|e| e.to_string())?;
    phases.cache_read_back += started.elapsed();

    let started = Instant::now();
    let parity_reader = snapshot::ReplayCacheReader::open(
        &index_bytes,
        &cache_bytes,
        expected_source_len,
        expected_source_sha256,
    )
    .map_err(|e| e.to_string())?;
    let max_seek_deltas = parity_reader
        .verify_every_hour()
        .map_err(|e| format!("parity: {e}"))?;
    phases.verify_every_hour += started.elapsed();

    let wall = wall_started.elapsed();
    let hour_count = replay_index.hours().len() as u64;
    let index_sha = Sha256::hex(&index_bytes);
    // `FOLD_PROFILE=1` splits `decode_fold_snapshot` into the fold's own
    // apply/build/serialize terms. Reported verbatim; empty when unset.
    let foldprof = recorder_host::world_fold::foldprof::report().unwrap_or_default();

    Ok(format!(
        "{{\"schema\":\"cz-finalize-profile-v1\",\
         \"hours\":{hour_count},\"hours_decoded\":{hours_decoded},\
         \"stream_bytes\":{expected_source_len},\
         \"legacy_json_bytes_avoided\":{legacy_json_bytes_avoided},\
         \"checkpoint_interval_hours\":{checkpoint_interval_hours},\
         \"checkpoint_cache_len\":{},\"checkpoint_cache_sha256\":\"{}\",\
         \"hour_index_len\":{},\"hour_index_sha256\":\"{index_sha}\",\
         \"max_seek_deltas\":{max_seek_deltas},\
         \"wall_s\":{:.3},\
         \"phase_s\":{{\
         \"decode_fold_snapshot\":{:.3},\"render_blobs\":{:.3},\
         \"cache_push\":{:.3},\"take_index\":{:.3},\"encoder_finish\":{:.3},\
         \"index_write\":{:.3},\"cache_read_back\":{:.3},\
         \"verify_every_hour\":{:.3}}},\
         \"cache_push_sub_s_estimated\":{{\
         \"canon_parse\":{:.3},\"canon_serialize\":{:.3},\
         \"world_json_sha256\":{:.3}}},\
         \"foldprof\":\"{foldprof}\"}}",
        replay_index.cache_len(),
        replay_index.cache_sha256(),
        index_bytes.len(),
        secs(wall),
        secs(phases.decode_fold_snapshot),
        secs(phases.render_blobs),
        secs(phases.cache_push),
        secs(phases.take_index),
        secs(phases.encoder_finish),
        secs(phases.index_write),
        secs(phases.cache_read_back),
        secs(phases.verify_every_hour),
        secs(phases.sub_canon_parse),
        secs(phases.sub_canon_serialize),
        secs(phases.sub_world_json_sha256),
    ))
}

#[allow(clippy::too_many_arguments)]
fn emit(
    phases: &mut Phases,
    cache_encoder: &mut snapshot::ReplayCacheEncoder<std::fs::File>,
    output_dir: &Path,
    hour: WorldHour,
    start_offset: u64,
    end_offset: u64,
    legacy_json_bytes_avoided: &mut u64,
    sub_terms: bool,
) -> Result<(), String> {
    let started = Instant::now();
    recorder_host::world_fold::preserve_production_render_blobs_from_json(
        output_dir,
        &hour.snapshot,
    )
    .map_err(|e: std::io::Error| format!("render blobs: {e}"))?;
    phases.render_blobs += started.elapsed();

    *legacy_json_bytes_avoided += hour.snapshot.len() as u64;

    if sub_terms {
        let started = Instant::now();
        let value = CanonValue::parse(&hour.snapshot).map_err(|e| e.to_string())?;
        phases.sub_canon_parse += started.elapsed();
        let started = Instant::now();
        let round_trip = value.to_json_bytes();
        phases.sub_canon_serialize += started.elapsed();
        std::hint::black_box(&round_trip);
        let started = Instant::now();
        let digest = Sha256::hex(&hour.snapshot);
        phases.sub_world_json_sha256 += started.elapsed();
        std::hint::black_box(&digest);
    }

    let hour_index = hour.hour_index;
    let started = Instant::now();
    cache_encoder
        .push(snapshot::CacheBuildHour {
            hour_index,
            date: hour.date,
            h4cr_start_offset: start_offset,
            h4cr_end_offset: end_offset,
            world_json: hour.snapshot,
        })
        .map_err(|e| format!("cache push hour {hour_index}: {e}"))?;
    phases.cache_push += started.elapsed();
    Ok(())
}

/// Silence the unused-import warning when `OpenError` is not referenced.
#[allow(dead_code)]
type _UnusedOpenError = OpenError;
