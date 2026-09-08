//! The publish tail (lane PT, 2026-09-02): the per-hour ONLINE work between the
//! reader fan-out's join and the hour's seal, and what this lane moved off it.
//!
//! Every test here is against the shipped helpers -- the PSD1 preparation the
//! delta-enabled workers run, the tail's staged append, the d81 encode split,
//! the paused scheduler waits -- and against the frame builder's own source
//! where the claim is about WHERE a statement runs.

use super::*;
use compact_contract::payload_span_delta as psd;

const D47: u16 = COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE;
const D43: u16 = COMPACT_DOMAIN_MARKET_TRADE_LIVE;
const FIRST_HOUR: i32 = 60_830_824;

fn payload(elapsed_wall_ms: u64, body: &[u8]) -> Vec<u8> {
    let mut out = elapsed_wall_ms.to_le_bytes().to_vec();
    out.extend_from_slice(body);
    out
}

fn body_of_len(len: usize, seed: u8) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(13).wrapping_add(seed))
        .collect()
}

/// The hours a domain walks through every PSD1 body kind: a first keyframe, a
/// delta, a no-change hour, a second delta, and the cadence keyframe.
fn every_body_kind_hours() -> Vec<(i32, Vec<u8>)> {
    let base = body_of_len(20_000, 3);
    let mut changed = base.clone();
    changed[7_000..7_200].fill(0x5A);
    vec![
        (FIRST_HOUR, payload(10, &base)),
        (FIRST_HOUR + 1, payload(20, &changed)),
        (FIRST_HOUR + 2, payload(30, &changed)),
        (FIRST_HOUR + 3, payload(40, &base)),
        (
            FIRST_HOUR + psd::KEYFRAME_CADENCE_HOURS as i32,
            payload(50, &base),
        ),
    ]
}

fn inline_record(
    trackers: &mut CompactPerfGateTrackers,
    sequence: u64,
    hour: i32,
    domain_id: u16,
    payload: &[u8],
) -> Vec<u8> {
    inline_record_with_status(trackers, sequence, hour, domain_id, 0, payload)
}

fn inline_record_with_status(
    trackers: &mut CompactPerfGateTrackers,
    sequence: u64,
    hour: i32,
    domain_id: u16,
    status_flags: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    append_compact_span_delta_data_record(
        &mut out,
        &mut trackers.payload_span_delta,
        sequence,
        hour,
        domain_id,
        status_flags,
        payload,
    );
    out
}

fn prepared_record(
    trackers: &mut CompactPerfGateTrackers,
    prepared: Option<PreparedSpanDelta>,
    sequence: u64,
    hour: i32,
    domain_id: u16,
    payload: &[u8],
) -> Vec<u8> {
    prepared_record_with_status(trackers, prepared, sequence, hour, domain_id, 0, payload)
}

fn prepared_record_with_status(
    trackers: &mut CompactPerfGateTrackers,
    prepared: Option<PreparedSpanDelta>,
    sequence: u64,
    hour: i32,
    domain_id: u16,
    status_flags: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    append_prepared_span_delta_data_record(
        &mut out,
        &mut trackers.payload_span_delta,
        sequence,
        hour,
        domain_id,
        prepared,
        status_flags,
        payload,
    );
    out
}

/// The worker's preparation, staged by the tail, seals the bytes the inline
/// wrap seals -- through every body kind, with the producer sealed between
/// frames exactly as the frame builder seals it.
///
/// kills: a preparation that stages a different baseline from the inline wrap
/// (the next hour's delta would differ); a stage that skips the keyframe mark
/// (the cadence keyframe would differ); the placeholder sequence left in the
/// envelope; a fallback arm that wraps twice.
#[test]
fn a_worker_prepared_span_delta_record_seals_the_inline_bytes() {
    let mut inline = CompactPerfGateTrackers::default();
    let mut split = CompactPerfGateTrackers::default();
    for (index, (hour, bytes)) in every_body_kind_hours().into_iter().enumerate() {
        let sequence = 40 + index as u64;
        let expected = inline_record(&mut inline, sequence, hour, D47, &bytes);
        let before = format!("{:?}", split.payload_span_delta);
        let prepared = prepare_span_delta_data_record(&split.payload_span_delta, hour, D47, 0, &bytes);
        assert!(prepared.is_some(), "hour {index}: d47 is delta-enabled");
        assert_eq!(
            format!("{:?}", split.payload_span_delta),
            before,
            "hour {index}: preparing must not touch the producer"
        );
        let actual = prepared_record(&mut split, prepared, sequence, hour, D47, &bytes);
        assert_eq!(
            actual, expected,
            "hour {index}: the record must be byte-identical"
        );
        let header = compact_contract::parse_compact_record_header(&actual).unwrap();
        assert_eq!(header.sequence, sequence);
        assert_eq!(
            header.record_flags,
            compact_contract::COMPACT_RECORD_FLAG_PAYLOAD_SPAN_DELTA,
            "hour {index}: the prepared record rides as a PSD1 envelope"
        );
        commit_delta_pin_transactions(&mut inline, &mut Vec::new());
        commit_delta_pin_transactions(&mut split, &mut Vec::new());
        assert_eq!(
            format!("{:?}", split.payload_span_delta),
            format!("{:?}", inline.payload_span_delta),
            "hour {index}: the sealed producers must agree"
        );
    }
}

/// A preparation the producer no longer recognises falls back to the inline
/// wrap of the payload the tail still holds, and an absent preparation does
/// the same: neither arm seals a byte the inline path would not have.
#[test]
fn a_stale_or_absent_preparation_falls_back_to_the_inline_wrap() {
    let hours = every_body_kind_hours();
    let mut inline = CompactPerfGateTrackers::default();
    let mut split = CompactPerfGateTrackers::default();
    // Frame 1: both wrap the first hour inline and seal.
    let first = inline_record(&mut inline, 1, hours[0].0, D43, &hours[0].1);
    assert_eq!(
        prepared_record(&mut split, None, 1, hours[0].0, D43, &hours[0].1),
        first,
        "an absent preparation is the inline wrap"
    );
    commit_delta_pin_transactions(&mut inline, &mut Vec::new());
    commit_delta_pin_transactions(&mut split, &mut Vec::new());
    // Frame 2: a preparation built against the sealed baseline, then a wrap for
    // the same domain that moves the staged baseline before it is staged.
    let stale = prepare_span_delta_data_record(
        &split.payload_span_delta,
        hours[1].0,
        D43,
        0,
        &hours[1].1,
    );
    assert!(stale.is_some());
    let moved_inline = inline_record(&mut inline, 2, hours[1].0, D43, &hours[1].1);
    let moved_split = inline_record(&mut split, 2, hours[1].0, D43, &hours[1].1);
    assert_eq!(moved_inline, moved_split);
    let expected = inline_record(&mut inline, 3, hours[3].0, D43, &hours[3].1);
    let actual = prepared_record(&mut split, stale, 3, hours[3].0, D43, &hours[3].1);
    assert_eq!(
        actual, expected,
        "a stale preparation must be refused and the payload wrapped inline"
    );
    assert_eq!(
        format!("{:?}", split.payload_span_delta),
        format!("{:?}", inline.payload_span_delta),
    );
}

/// A domain the producer refuses is appended unwrapped and unflagged by both
/// arms: `prepare` yields no preparation and the inline wrap refuses the same
/// way.
#[test]
fn a_refused_domain_is_unwrapped_on_both_arms() {
    let mut trackers = CompactPerfGateTrackers::default();
    let too_short = vec![1u8, 2, 3];
    assert!(prepare_span_delta_data_record(
        &trackers.payload_span_delta,
        FIRST_HOUR,
        D47,
        0,
        &too_short
    )
    .is_none());
    let expected = inline_record(&mut trackers, 9, FIRST_HOUR, D47, &too_short);
    let actual = prepared_record(&mut trackers, None, 9, FIRST_HOUR, D47, &too_short);
    assert_eq!(actual, expected);
    let header = compact_contract::parse_compact_record_header(&actual).unwrap();
    assert_eq!(header.record_flags, 0, "a refused payload rides unflagged");
}

/// d81's encode is split into a pure half the worker runs and a refusal the
/// tail applies; the composition is the encode the tail always ran.
#[test]
fn the_d81_encode_split_composes_to_the_inline_encode() {
    // The hub-less shape every panic substitute has: the inline encode refuses
    // and marks the read; the split refuses with the same reason and the tail
    // marks the read identically.
    let mut inline = compact_rail_replay_worker_panic_read(0);
    let mut split = compact_rail_replay_worker_panic_read(0);
    assert!(encode_compact_rail_replay_production(&mut inline, 5).is_none());
    let refusal = encode_compact_rail_replay_production_payload(&split, 5)
        .expect_err("a hub-less read cannot encode");
    assert_eq!(refusal, D81EncodeRefusal::HubSimulationAbsent);
    apply_d81_encode_refusal(&mut split, refusal);
    assert_eq!(inline.status_flags, split.status_flags);
    assert_eq!(
        format!("{:?}", inline.first_failure),
        format!("{:?}", split.first_failure)
    );
    assert_eq!(inline.provinces.len(), split.provinces.len());
    assert_eq!(inline.observed_neighbors, split.observed_neighbors);

    // And an encodable shape: an empty but complete hub snapshot.
    let mut inline = compact_rail_replay_worker_panic_read(0);
    inline.hub_simulation = Some(Vec::new());
    let mut split = compact_rail_replay_worker_panic_read(0);
    split.hub_simulation = Some(Vec::new());
    let expected = encode_compact_rail_replay_production(&mut inline, 5);
    let actual = match encode_compact_rail_replay_production_payload(&split, 5) {
        Ok(payload) => Some(payload),
        Err(refusal) => {
            apply_d81_encode_refusal(&mut split, refusal);
            None
        }
    };
    assert_eq!(actual, expected);
    assert_eq!(inline.status_flags, split.status_flags);
}

/// The capture loop's body: from its label to the post-loop flush.
fn capture_loop_body(source: &str) -> &str {
    let start = source
        .find("        'capture: while Instant::now() <= deadline {")
        .expect("the capture loop");
    let end = source[start..]
        .find("        // A capture that left the loop between staging a finished frame and")
        .map(|at| start + at)
        .expect("the post-loop flush follows the capture loop");
    &source[start..end]
}

/// Every wait the capture loop takes is idle to the hourly service timer.
///
/// The timer's setup phase is claimed by the next admitted hour as its capture
/// prefix, and its owned phases are the hour's own service; a park, a hold or a
/// back-off inside either was charged to an hour as work. P4 charged the
/// rendezvous hold to capture (119 ms of a 244 ms mean) and to the terminal
/// hour (292 ms), and the identity back-offs to whichever hour followed.
///
/// kills: any of the five wait sites unwrapped from `pause()`; a new
/// `thread::sleep` added to the loop without one.
#[test]
fn every_wait_in_the_capture_loop_is_paused_for_the_hourly_service_timer() {
    let source = super::tests::collector_source_lf();
    let body = capture_loop_body(&source);
    let mut waits = 0usize;
    for needle in [
        "thread::sleep(",
        ".rendezvous_with_hour_boundary(",
        "runtime.wait_for_hour(",
    ] {
        let mut from = 0usize;
        while let Some(hit) = body[from..].find(needle) {
            let at = from + hit;
            from = at + needle.len();
            waits += 1;
            let line = body[..at].matches('\n').count();
            let preceding: String = body[..at]
                .lines()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(
                preceding.contains("let _service_idle = hourly_service_capture::pause();"),
                "the wait `{needle}` at capture-loop line {line} must be inside a \
                 `hourly_service_capture::pause()` scope opened within the four lines above it"
            );
        }
    }
    assert_eq!(
        waits, 7,
        "the capture loop holds exactly the five sleeps and two scheduler waits this law \
         covers; a new wait arrives with a pause and a count here"
    );
}

/// The degraded-hour run-log note is staged with the frame and written by the
/// in-frame publisher after the frames, never on the sweep thread.
///
/// kills: `record_attach_worker_error("frame_degraded", ..)` restored to the
/// loop; the note staged before the boundary is admitted (a put-back frame
/// would stage it twice); the publisher writing notes before its frames.
#[test]
fn the_degraded_hour_note_is_written_by_the_publisher_after_its_frames() {
    let source = super::tests::collector_source_lf();
    let body = capture_loop_body(&source);
    assert!(
        !body.contains("record_attach_worker_error("),
        "the capture loop must not append to the run log on the sweep thread"
    );
    let staged_at = body
        .find("staged_sidecar_notes.push(DeferredSidecarNote::frame_degraded(")
        .expect("the degraded note is staged");
    let admitted_at = body
        .find("PreboundaryPublication::Admitted => {}")
        .expect("the boundary is observed before the frame is staged");
    let frame_staged_at = body
        .find("kind: StagedPublicationKind::Data {")
        .expect("the DATA frame is staged");
    assert!(
        admitted_at < staged_at && staged_at < frame_staged_at,
        "the note is staged after the boundary admitted the frame and beside the frame"
    );
    let publisher = item_body(&source, "fn run_staged_publications(");
    let last_write = publisher
        .rfind("transport.write_preboundary_compact_frame(&frame.bytes)")
        .expect("the publisher offers the frames");
    let notes_at = publisher
        .find("for note in sidecar_notes {")
        .expect("the publisher writes the notes");
    assert!(
        last_write < notes_at,
        "notes are written after every frame was offered"
    );
    assert_eq!(
        body.matches("staged_sidecar_notes.clear();").count(),
        1,
        "the notes are cleared once, with the frames they describe"
    );
    let note = DeferredSidecarNote::frame_degraded(60_830_824, "d43:0x00000200[x]".to_string());
    assert_eq!(note.phase, "frame_degraded");
    assert_eq!(
        note.message,
        "hour 60830824 published with degraded domains: d43:0x00000200[x]"
    );
}

/// One function's source text, from its signature to the next item at column
/// zero.
fn item_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} must exist"));
    let end = source[start + 1..]
        .find("\nfn ")
        .map(|at| start + 1 + at)
        .unwrap_or(source.len());
    &source[start..end]
}

/// The text of the `lines` lines following `anchor`'s occurrence at `at`.
fn lines_after(body: &str, at: usize, lines: usize) -> &str {
    let mut end = at;
    for _ in 0..lines {
        match body[end..].find('\n') {
            Some(next) => end += next + 1,
            None => return &body[at..],
        }
    }
    &body[at..end]
}

/// Every delta-enabled domain prepares its record on its worker and stages
/// it on the tail; no domain wraps inline on the serial tail any more.
///
/// kills: a spawn site restored to the plain helper (the closure would still
/// compile and the tail's fallback would wrap inline on every frame); a tail
/// block that drops the `.take()` and always falls back; a sixth domain wrapped
/// inline without a preparation.
#[test]
fn every_delta_enabled_domain_prepares_on_its_worker_and_stages_on_the_tail() {
    let source = super::tests::collector_source_lf();
    let body = item_body(&source, "fn build_compact_extraction_gate_records(");
    const WIRED: &[(&str, &str)] = &[
        (
            "COMPACT_DOMAIN_MARKET_TRADE_LIVE",
            "market_trade_live_publication.take()",
        ),
        (
            "COMPACT_DOMAIN_CHARACTERS_LEADERS_CABINET_LIVE",
            "characters_leaders_cabinet_live_publication.take()",
        ),
        (
            "COMPACT_DOMAIN_RAIL_REPLAY_STATE_LIVE",
            "std::mem::take(&mut rail_replay_state_publication)",
        ),
        (
            "COMPACT_DOMAIN_COUNTRY_ARMY_UPGRADE_REQUESTS_LIVE",
            "country_army_upgrade_requests_publication.take()",
        ),
        (
            "COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE",
            "diplomacy_intel_operatives_live_publication.take()",
        ),
    ];
    assert_eq!(
        WIRED.len(),
        psd::DELTA_ENABLED_DOMAINS.len(),
        "every delta-enabled domain is wired, and only those"
    );
    let mut prepared_for: Vec<&str> = Vec::new();
    let mut from = 0usize;
    while let Some(hit) = body[from..].find("prepare_span_delta_data_record(") {
        let at = from + hit;
        from = at + 1;
        let call = lines_after(body, at, 6);
        let domain = WIRED
            .iter()
            .map(|(domain, _)| *domain)
            .find(|domain| call.contains(domain))
            .unwrap_or_else(|| panic!("a preparation names its domain:\n{call}"));
        prepared_for.push(domain);
    }
    for (domain, staged) in WIRED {
        assert_eq!(
            prepared_for
                .iter()
                .filter(|prepared| *prepared == domain)
                .count(),
            1,
            "{domain} must be prepared on exactly one worker"
        );
        assert!(
            body.contains(staged),
            "{domain}'s tail must stage the worker's preparation via `{staged}`"
        );
    }
    assert!(
        !body.contains("append_compact_span_delta_data_record("),
        "no domain may wrap inline on the frame builder's serial tail; the inline \
         wrap is reachable only as the prepared append's fallback"
    );
    for helper in [
        "fn spawn_gated_split<",
        "fn spawn_gated_finishing<",
        "fn spawn_gated_preparing<",
    ] {
        let helper_body = item_body(&source, helper);
        let permit_at = helper_body
            .find("drop(permit);")
            .unwrap_or_else(|| panic!("{helper} releases its permit"));
        let prepare_at = helper_body
            .find("prepare(&")
            .unwrap_or_else(|| panic!("{helper} runs its preparation"));
        assert!(
            permit_at < prepare_at,
            "{helper}: the preparation must run after the permit is released, or a \
             queued reader waits on a digest"
        );
        assert!(
            helper_body
                .contains("std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| prepare(&"),
            "{helper}: a panicking preparation must be caught so the tail re-runs the work"
        );
    }
}

/// kills: a `recycle_record_buffer` that keeps the bytes (the next frame would
/// start with last hour's records); one that drops the capacity (a fresh
/// reservation and its page faults every hour); the ceiling ignored; the
/// smaller of two candidates kept.
#[test]
fn a_published_data_frames_buffer_is_the_next_builds_record_buffer() {
    let mut buffers = FrameBuffers::default();
    assert!(buffers.take_record_buffer().is_empty());

    let mut published = Vec::with_capacity(4096);
    published.extend_from_slice(&[0xAB; 3000]);
    let capacity = published.capacity();
    buffers.recycle_record_buffer(published);
    let recycled = buffers.take_record_buffer();
    assert!(
        recycled.is_empty(),
        "the recycled buffer is cleared, so the next frame's bytes start at zero"
    );
    assert_eq!(
        recycled.capacity(),
        capacity,
        "and it keeps the capacity the last frame needed"
    );
    assert!(buffers.recycled.is_none(), "taken once");

    buffers.recycle_record_buffer(Vec::with_capacity(1024));
    buffers.recycle_record_buffer(Vec::with_capacity(8192));
    buffers.recycle_record_buffer(Vec::with_capacity(2048));
    assert_eq!(
        buffers.take_record_buffer().capacity(),
        8192,
        "of two candidates the larger stays"
    );

    buffers.recycle_record_buffer(Vec::with_capacity(COMPACT_FRAME_BYTES_HINT_CEILING + 1));
    assert!(
        buffers.recycled.is_none(),
        "a buffer over the hint ceiling is freed, not held"
    );

    const MIB: usize = 1 << 20;
    for _ in 0..(FRAME_BUFFERS_RETIRED_LIMIT_BYTES / MIB + 6) {
        buffers.retired.push(Vec::with_capacity(MIB));
    }
    buffers.take_record_buffer();
    let held: usize = buffers.retired.iter().map(Vec::capacity).sum();
    assert!(
        held <= FRAME_BUFFERS_RETIRED_LIMIT_BYTES && held + MIB > FRAME_BUFFERS_RETIRED_LIMIT_BYTES,
        "a bin no publisher emptied is bounded in bytes at the next build: {held}"
    );
    assert!(!buffers.take_retired().is_empty() && buffers.retired.is_empty());
}

#[test]
fn a_recycled_buffer_seals_the_same_bytes_as_a_fresh_one() {
    let mut buffers = FrameBuffers::default();
    let mut dirty = vec![0xEE; 5_000];
    dirty.reserve(10_000);
    buffers.recycle_record_buffer(dirty);
    let mut recycled = buffers.take_record_buffer();
    let mut fresh = Vec::new();
    let body = body_of_len(3_000, 9);
    for out in [&mut recycled, &mut fresh] {
        append_compact_record(
            out,
            7,
            FIRST_HOUR,
            D43,
            COMPACT_RECORD_TYPE_DOMAIN_DATA,
            0,
            &body,
        );
        append_compact_frame_end_record(out, 8, FIRST_HOUR, 5, 6, 1, 1, 0);
    }
    assert_eq!(recycled, fresh);
}

#[test]
fn disposal_recycles_a_data_frames_buffer_and_frees_an_h4dg_frames() {
    let _session = hourly_service_capture::start();
    let mut buffers = FrameBuffers::default();
    let data = StagedPublication {
        division_copy_reservation: None,
        bytes: Vec::with_capacity(4096),
        staged_after_frame: 2,
        kind: StagedPublicationKind::Data {
            request: CaptureRequest {
                stamp: crate::read_only_scheduler::ClockStamp {
                    epoch: 4,
                    hour_raw: 100,
                    observation_ordinal: 8,
                },
                force_baseline: false,
            },
            degraded: false,
            service_owner: None,
        },
    };
    let gap = StagedPublication {
        division_copy_reservation: None,
        bytes: Vec::with_capacity(64),
        staged_after_frame: 3,
        kind: StagedPublicationKind::ControlGap { group_len: 1 },
    };
    dispose_staged_publications(
        &mut vec![data, gap],
        PendingStagedServiceCloses::default(),
        &mut buffers,
    );
    assert_eq!(
        buffers.take_record_buffer().capacity(),
        4096,
        "the DATA frame's buffer is the next build's; the H4DG frame's was freed"
    );
}

/// The tail retires into the loop's bin, and the in-frame publisher frees it
/// after its frames -- on its own thread, never on the serial tail.
///
/// kills: the bin dropped in the loop (a free on the sweep thread); the free
/// moved ahead of the frames; the builder reserving its own buffer again.
#[test]
fn the_tail_retires_what_the_publisher_frees() {
    let source = super::tests::collector_source_lf();
    let body = capture_loop_body(&source);
    let taken = body
        .find("let retired = frame_buffers.take_retired();")
        .expect("the retired bin is taken before the publisher closure is built");
    let closure = body[taken..]
        .find("Some(Box::new(move || {")
        .map(|at| taken + at)
        .expect("the publisher closure follows");
    let closure_end = body[closure..]
        .find("}))")
        .map(|at| closure + at)
        .expect("the closure closes");
    let publisher = &body[closure..closure_end];
    let run_at = publisher
        .find("run_staged_publications(")
        .expect("the closure publishes");
    let drop_at = publisher
        .find("drop(retired);")
        .expect("the closure frees the retired bytes");
    assert!(run_at < drop_at, "the frames go first, then the free");
    assert_eq!(
        body.matches("drop(retired);").count(),
        1,
        "the retired bytes are freed by the publisher and nowhere else in the loop"
    );

    let builder = item_body(&source, "fn build_compact_extraction_gate_records(");
    assert_eq!(
        builder
            .matches("commit_delta_pin_transactions(trackers, &mut buffers.retired);")
            .count(),
        1,
        "the PSD1 commit retires into the loop's bin"
    );
    assert_eq!(
        builder.matches("buffers.take_record_buffer()").count(),
        1,
        "the builder's record buffer comes from the loop's buffers"
    );
    assert!(
        !builder.contains("Vec::with_capacity(compact_frame_bytes_hint())"),
        "no fresh reservation in the builder itself"
    );
}

#[test]
fn the_build_tail_parts_partition_the_tail_and_never_wrap() {
    use OutsideBuildSegment::{BuildProbes, BuildReseq, BuildReturn, BuildSeal, BuildTail};
    let timing = CompactFramePhaseTiming {
        build_probes_us: 500,
        build_reseq_us: 300,
        build_seal_us: 1_200,
        ..CompactFramePhaseTiming::default()
    };
    let parts = outside_build_tail_parts(2_500, &timing);
    assert_eq!(
        parts,
        [
            (BuildProbes, 500),
            (BuildReseq, 300),
            (BuildSeal, 1_200),
            (BuildReturn, 500)
        ]
    );
    assert_eq!(
        parts.iter().map(|(_, us)| *us).sum::<u64>(),
        2_500,
        "the four parts are a partition of the tail"
    );
    assert_eq!(
        outside_build_tail_parts(1_000, &timing)[3],
        (BuildReturn, 0),
        "marks past the caller's own measurement leave a zero remainder, never a wrapped u64"
    );
    assert_eq!(
        outside_build_tail_parts(700, &CompactFramePhaseTiming::default())[3],
        (BuildReturn, 700),
        "a build with no marks reports its whole tail as the return"
    );
    let mut outside = OutsideBuildSpans::default();
    outside.add_us(BuildTail, 2_500);
    for (segment, us) in parts {
        outside.add_us(segment, us);
    }
    let parts_sum: u64 = [BuildProbes, BuildReseq, BuildSeal, BuildReturn]
        .iter()
        .map(|segment| outside.us[*segment as usize])
        .sum();
    assert_eq!(parts_sum, outside.us[BuildTail as usize]);
}

/// kills: a mark moved so the three are no longer adjacent from the frame-time
/// stamp to FRAME_END, or so that a probe, the resequencing or the commit falls
/// in another part's interval.
#[test]
fn the_three_build_tail_marks_are_adjacent_from_the_frame_time_stamp_to_frame_end() {
    let source = super::tests::collector_source_lf();
    let builder = item_body(&source, "fn build_compact_extraction_gate_records(");
    let stamp = "let frame_time_us = duration_micros_u64(frame_started.elapsed());";
    let frame_time = builder.rfind(stamp).expect("the frame-time stamp");
    let probes = builder
        .find("let build_probes_end_us = duration_micros_u64(frame_started.elapsed());")
        .expect("the probes mark");
    let reseq = builder
        .find("let build_reseq_end_us = duration_micros_u64(frame_started.elapsed());")
        .expect("the resequencing mark");
    let seal = builder
        .find("let build_seal_end_us = duration_micros_u64(frame_started.elapsed());")
        .expect("the seal mark");
    assert!(frame_time < probes && probes < reseq && reseq < seal);
    let probes_part = &builder[frame_time..probes];
    assert!(
        probes_part.contains("*outside_build = OutsideBuildSpans::default();"),
        "the probes part ends after the lane-wall probe is published"
    );
    let reseq_part = &builder[probes..reseq];
    assert!(
        reseq_part.contains("replace_rejected_domain_records_with_root_dependencies(")
            && reseq_part.contains("division_roots_cache.acquisition.dependencies()")
            && reseq_part.contains("&root_dependencies,")
            && reseq_part.contains("close_unresolved_domain_hours("),
        "the resequencing part holds both passes"
    );
    let seal_part = &builder[reseq..seal];
    assert!(
        seal_part.contains("commit_delta_pin_transactions(")
            && seal_part.contains("append_compact_frame_end_record("),
        "the seal part holds the commit and FRAME_END"
    );
    assert!(
        !builder[seal..].contains("append_compact_record("),
        "nothing is appended after the seal mark"
    );
}

/// A failure-status hour retains the basis on the worker-prepared arm exactly
/// as on the inline arm (capture 07's d53 shape: a 72-byte source-unavailable
/// record between full payloads), and the full hour after it is a delta
/// against the full basis on both.
#[test]
fn a_failure_status_hour_retains_the_basis_on_both_arms() {
    const D53: u16 = COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE;
    let base = body_of_len(20_000, 3);
    let mut changed = base.clone();
    changed[7_000..7_200].fill(0x5A);
    let hours = [
        (FIRST_HOUR, 0u32, payload(10, &base)),
        (
            FIRST_HOUR + 1,
            COMPACT_STATUS_SOURCE_UNAVAILABLE,
            payload(20, &[0u8; 64]),
        ),
        (FIRST_HOUR + 2, 0u32, payload(30, &changed)),
    ];
    let mut inline = CompactPerfGateTrackers::default();
    let mut split = CompactPerfGateTrackers::default();
    for (index, (hour, status, bytes)) in hours.iter().enumerate() {
        let sequence = 60 + index as u64;
        let expected =
            inline_record_with_status(&mut inline, sequence, *hour, D53, *status, bytes);
        let prepared =
            prepare_span_delta_data_record(&split.payload_span_delta, *hour, D53, *status, bytes);
        assert!(prepared.is_some(), "hour {index}: d53 is delta-enabled");
        let actual = prepared_record_with_status(
            &mut split, prepared, sequence, *hour, D53, *status, bytes,
        );
        assert_eq!(actual, expected, "hour {index}: the record must be byte-identical");
        let flags = actual[COMPACT_RECORD_HEADER_LEN as usize + psd::FLAGS_OFFSET];
        let retains = if *status == 0 { 0 } else { psd::FLAG_RETAINS_BASIS };
        assert_eq!(flags, retains, "hour {index}: the flags byte follows the status");
        if index == 2 {
            assert!(
                actual.len() < COMPACT_RECORD_HEADER_LEN as usize + 1024,
                "hour 2 re-sent {} bytes: the failure record became the basis",
                actual.len()
            );
        }
        commit_delta_pin_transactions(&mut inline, &mut Vec::new());
        commit_delta_pin_transactions(&mut split, &mut Vec::new());
        assert_eq!(
            format!("{:?}", split.payload_span_delta),
            format!("{:?}", inline.payload_span_delta),
            "hour {index}: the sealed producers must agree"
        );
    }
}
