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

/// How much of the world document is REBUILT per hour that did not move.
///
/// The finalize's dominant cost is per-hour whole-document work: the fold
/// materializes every section of the world, it is serialized, re-parsed and
/// diffed. Any design that stops doing that has to reuse last hour's answer for
/// the parts that did not change, and the first question such a design has to
/// survive is whether ANYTHING is stable at a granularity a fold can track.
///
/// This counts it, per top-level section, at the section's direct-child
/// granularity -- one country, one state, one division -- because that is the
/// grain the fold's own state is keyed by. `unchanged` is a child whose whole
/// subtree is identical to the same-named child of the previous hour.
///
/// It is a structural walk of values already in memory -- no serialization, and
/// the previous hour is kept as 64-bit subtree hashes plus one child-name map
/// per section rather than as a second world document. It is not allocation
/// free: it rebuilds those maps, cloning the section and child names, once per
/// hour. That is a measurable cost, so the census runs only under its own flag
/// and its time is never charged to a phase.
///
/// Hash equality is not value equality; a collision would report a changed
/// child as unchanged, which biases the answer the flattering way. So the
/// encoding is at least made INJECTIVE before the 64-bit fold: every scalar
/// body and every member name is length-prefixed and domain-tagged, which is
/// what stops `{"a":"x","bc":null}` and `{"a":"xb","c":null}` from feeding the
/// mixer the same sequence. What remains is an ordinary 64-bit collision
/// probability over ~2e6 comparisons -- a design input's worth of confidence,
/// not a proof, and never used to decide anything a byte depends on.
#[derive(Default)]
struct EntityCensus {
    /// section -> (children seen, children unchanged from the previous hour)
    sections: std::collections::BTreeMap<String, (u64, u64)>,
    previous: std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>>,
    hours: u64,
    /// Wall time this census spent, so it can be taken back out of `wall_s`.
    elapsed: Duration,
}

impl EntityCensus {
    fn hash_value(value: &CanonValue, state: &mut u64) {
        #[inline]
        fn mix(state: &mut u64, value: u64) {
            *state = (*state ^ value).wrapping_mul(0x100_0000_01b3);
            *state ^= *state >> 29;
        }
        // Length-prefixed, so a byte run cannot be re-divided between two
        // adjacent fields and hash the same. Without this the encoding is not
        // injective and the census over-reports "unchanged" for reasons that
        // have nothing to do with hash width.
        fn bytes(state: &mut u64, data: &[u8]) {
            mix(state, data.len() as u64);
            for byte in data {
                mix(state, *byte as u64);
            }
        }
        match value {
            CanonValue::Null => mix(state, 1),
            CanonValue::Bool(false) => mix(state, 2),
            CanonValue::Bool(true) => mix(state, 3),
            CanonValue::Number(token) => {
                mix(state, 4);
                bytes(state, token.as_bytes());
            }
            CanonValue::String(body) => {
                mix(state, 5);
                bytes(state, body.as_bytes());
            }
            CanonValue::Array(values) => {
                mix(state, 6);
                mix(state, values.len() as u64);
                for entry in values {
                    Self::hash_value(entry, state);
                }
            }
            CanonValue::Object(members) => {
                mix(state, 7);
                mix(state, members.len() as u64);
                for (name, member) in members {
                    bytes(state, name.as_bytes());
                    Self::hash_value(member, state);
                }
            }
        }
    }

    fn observe(&mut self, world: &CanonValue) {
        let started = Instant::now();
        self.observe_inner(world);
        self.elapsed += started.elapsed();
    }

    fn observe_inner(&mut self, world: &CanonValue) {
        let CanonValue::Object(sections) = world else {
            return;
        };
        self.hours += 1;
        let mut current: std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>> =
            std::collections::BTreeMap::new();
        for (section_name, section) in sections {
            let CanonValue::Object(children) = section else {
                continue;
            };
            let mut hashes = std::collections::BTreeMap::new();
            let previous = self.previous.get(section_name);
            let counters = self
                .sections
                .entry(section_name.clone())
                .or_insert((0, 0));
            for (child_name, child) in children {
                let mut hash = 0xcbf2_9ce4_8422_2325u64;
                Self::hash_value(child, &mut hash);
                counters.0 += 1;
                if previous.and_then(|rows| rows.get(child_name)) == Some(&hash) {
                    counters.1 += 1;
                }
                hashes.insert(child_name.clone(), hash);
            }
            current.insert(section_name.clone(), hashes);
        }
        self.previous = current;
    }

    fn report(&self) -> String {
        if self.hours == 0 {
            return "null".to_string();
        }
        let rows: Vec<String> = self
            .sections
            .iter()
            .filter(|(_, (seen, _))| *seen > 0)
            .map(|(name, (seen, unchanged))| {
                format!("[\"{name}\",{seen},{unchanged}]")
            })
            .collect();
        format!("[{}]", rows.join(","))
    }
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
    entity_census: bool,
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
    let mut census = EntityCensus::default();
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
                if entity_census { Some(&mut census) } else { None },
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
    // Where the per-hour world document's BYTES live, measured on the last
    // hour -- the widest one, since the world only grows. The finalize cost is
    // dominated by per-hour whole-document work, so any incremental design has
    // to say which of these sections it would stop rebuilding; this is the list
    // it has to answer, sized.
    let census_started = Instant::now();
    let last_hour_member_bytes = member_byte_census(&last.snapshot);
    let mut instrument_only = census_started.elapsed();
    emit(
        &mut phases,
        &mut cache_encoder,
        &output_dir,
        last,
        last_start,
        expected_source_len,
        &mut legacy_json_bytes_avoided,
        sub_terms,
        if entity_census { Some(&mut census) } else { None },
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

    // Mirrors production exactly (see `build_certified_replay_artifacts`): the
    // cache is opened, not read back. The sequential hash that binds it to H4IX
    // stays inside the parity block, which is where it was timed before too.
    let started = Instant::now();
    let cache_file = std::fs::File::open(&cache_path).map_err(|e| e.to_string())?;
    phases.cache_read_back += started.elapsed();

    let started = Instant::now();
    let parity_reader = snapshot::CertifiedReplayCache::open_file(
        &index_bytes,
        cache_file,
        expected_source_len,
        expected_source_sha256,
    )
    .map_err(|e| e.to_string())?;
    let max_seek_deltas = parity_reader
        .verify_every_hour()
        .map_err(|e| format!("parity: {e}"))?;
    phases.verify_every_hour += started.elapsed();

    // The censuses are instrument work, not finalize work: they parse and
    // re-serialize an hour production never touches twice, and no phase timer
    // covers them. Left inside `wall_s` they would show up as finalize getting
    // slower every time the instrument learned to report one more thing.
    instrument_only += census.elapsed;
    let wall = wall_started
        .elapsed()
        .checked_sub(instrument_only)
        .unwrap_or_default();
    let hour_count = replay_index.hours().len() as u64;
    let index_sha = Sha256::hex(&index_bytes);
    let record_bytes = record_byte_census(replay_index.hours());
    let entity_census_report = census.report();
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
         \"wall_s\":{:.3},\"instrument_only_s\":{:.3},\
         \"phase_s\":{{\
         \"decode_fold_snapshot\":{:.3},\"render_blobs\":{:.3},\
         \"cache_push\":{:.3},\"take_index\":{:.3},\"encoder_finish\":{:.3},\
         \"index_write\":{:.3},\"cache_read_back\":{:.3},\
         \"verify_every_hour\":{:.3}}},\
         \"cache_push_sub_s_estimated\":{{\
         \"canon_parse\":{:.3},\"canon_serialize\":{:.3},\
         \"world_json_sha256\":{:.3}}},\
         \"record_bytes\":{record_bytes},\
         \"last_hour_member_bytes\":{last_hour_member_bytes},         \"entity_census\":{entity_census_report},\
         \"foldprof\":\"{foldprof}\"}}",
        replay_index.cache_len(),
        replay_index.cache_sha256(),
        index_bytes.len(),
        secs(wall),
        secs(instrument_only),
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

/// Per-top-level-member serialized byte sizes of one world hour, biggest first,
/// as a JSON array of `[name, bytes]` pairs.
///
/// Sizes are the member's own compact encoding — what a document that carried
/// only that member would spend on it — so they sum to the document length less
/// its punctuation. Returns `[]` if the body is not a canonical object, which
/// the instrument treats as "nothing to say", never as a failure: this is a
/// design input, not a check.
fn member_byte_census(world_json: &[u8]) -> String {
    let Ok(value) = CanonValue::parse(world_json) else {
        return "[]".to_string();
    };
    let CanonValue::Object(members) = &value else {
        return "[]".to_string();
    };
    let mut sizes: Vec<(usize, &str)> = members
        .iter()
        .map(|(name, member)| (member.to_json_bytes().len(), name.as_str()))
        .collect();
    sizes.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    let rows: Vec<String> = sizes
        .iter()
        .map(|(bytes, name)| format!("[\"{name}\",{bytes}]"))
        .collect();
    format!("[{}]", rows.join(","))
}

/// Cache record sizes, split by kind.
///
/// The delta total against the checkpoint total is the measured answer to "how
/// much of the world document actually moves in an hour" — the ceiling on what
/// an incremental fold could avoid rebuilding.
fn record_byte_census(hours: &[snapshot::ReplayHourEntry]) -> String {
    let mut checkpoint_bytes = 0u64;
    let mut checkpoint_count = 0u64;
    let mut delta_bytes = 0u64;
    let mut delta_count = 0u64;
    let mut delta_max = 0u64;
    for entry in hours {
        if entry.delta_steps() == 0 {
            checkpoint_bytes += entry.cache_record_len();
            checkpoint_count += 1;
        } else {
            delta_bytes += entry.cache_record_len();
            delta_count += 1;
            delta_max = delta_max.max(entry.cache_record_len());
        }
    }
    format!(
        "{{\"checkpoint_records\":{checkpoint_count},\"checkpoint_bytes\":{checkpoint_bytes},\
         \"delta_records\":{delta_count},\"delta_bytes\":{delta_bytes},\
         \"delta_max_bytes\":{delta_max}}}"
    )
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
    census: Option<&mut EntityCensus>,
) -> Result<(), String> {
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
    // The hour's ONE parse, shared by render-blob preservation and the encoder,
    // exactly as `build_certified_replay_artifacts::emit_cache_hour` does it.
    // Charged to `cache_push`: it is the encoder's parse, handed out.
    let started = Instant::now();
    let prepared = snapshot::CanonicalWorldHour::parse(hour.snapshot, hour_index)
        .map_err(|e| format!("canonical parse hour {hour_index}: {e}"))?;
    phases.cache_push += started.elapsed();

    // Outside every phase timer: a design input, not part of the cost being
    // attributed.
    if let Some(census) = census {
        census.observe(prepared.value());
    }

    let started = Instant::now();
    recorder_host::world_fold::preserve_production_render_blobs_from_canon(
        output_dir,
        prepared.value(),
    )
    .map_err(|e: std::io::Error| format!("render blobs: {e}"))?;
    phases.render_blobs += started.elapsed();

    let started = Instant::now();
    cache_encoder
        .push_prepared(
            snapshot::CacheBuildHourMeta {
                hour_index,
                date: hour.date,
                h4cr_start_offset: start_offset,
                h4cr_end_offset: end_offset,
            },
            prepared,
        )
        .map_err(|e| format!("cache push hour {hour_index}: {e}"))?;
    phases.cache_push += started.elapsed();
    Ok(())
}

/// Silence the unused-import warning when `OpenError` is not referenced.
#[allow(dead_code)]
type _UnusedOpenError = OpenError;
