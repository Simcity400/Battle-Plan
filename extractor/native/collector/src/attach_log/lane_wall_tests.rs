//! `lane_wall_tests`, extracted verbatim from `attach_log.rs`.

use std::collections::BTreeSet;

use super::*;

// Gate test (lever #5): the scheduler-assignment REPORT must match what
// actually ran. Inject a fake domain set through the real
// spawn_gated/ReaderGate path and check lane assignment + wall
// accounting consistency.
#[test]
fn lane_assignment_reporting_matches_execution() {
    const PERMITS: usize = 3;
    const FAKE_DOMAINS: u16 = 12;
    let gate = ReaderGate::new(PERMITS);
    let tracker = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    thread::scope(|scope| {
        let gref = Some(&gate);
        let tref = &tracker;
        let handles: Vec<_> = (0..FAKE_DOMAINS)
            .map(|i| {
                spawn_gated(scope, gref, tref, 300 + i, move || {
                    // Busy window so lanes are actually contended.
                    for _ in 0..20_000 {
                        std::hint::spin_loop();
                    }
                    i
                })
            })
            .collect();
        for (i, handle) in handles.into_iter().enumerate() {
            assert_eq!(handle.join().unwrap(), i as u16);
        }
    });
    let spans = tracker.take_spans();
    // Every injected domain reported exactly once.
    assert_eq!(spans.len(), FAKE_DOMAINS as usize);
    let mut tags: Vec<u16> = spans.iter().map(|s| s.tag).collect();
    tags.sort_unstable();
    assert_eq!(tags, (300..300 + FAKE_DOMAINS).collect::<Vec<_>>());
    // Lane ids are real permit slots.
    for span in &spans {
        assert!(
            (span.lane as usize) < PERMITS,
            "lane {} out of range",
            span.lane
        );
        assert!(span.end_us >= span.start_us);
    }
    // A lane is a mutual-exclusion slot: its spans must not overlap.
    for lane in 0..PERMITS as u16 {
        let mut lane_spans: Vec<&LaneSpan> = spans.iter().filter(|s| s.lane == lane).collect();
        lane_spans.sort_by_key(|s| s.start_us);
        for pair in lane_spans.windows(2) {
            assert!(
                pair[1].start_us >= pair[0].end_us,
                "lane {lane} spans overlap: {:?} then {:?}",
                pair[0],
                pair[1]
            );
        }
    }
    // The summary must agree with a manual recount of the same spans.
    let summary = summarize_lane_spans(&spans);
    assert_eq!(summary.main_wall_us, 0);
    assert!(!summary.lanes.is_empty());
    assert!(summary.lanes.len() <= PERMITS);
    let manual_max_wall = summary.lanes.iter().map(|&(_, w, _)| w).max().unwrap();
    assert_eq!(summary.crit_wall_us, manual_max_wall);
    let crit_lane = summary.crit_lane.expect("crit lane exists");
    let manual_crit_busy: u64 = spans
        .iter()
        .filter(|s| s.lane == crit_lane)
        .map(|s| s.end_us - s.start_us)
        .sum();
    assert_eq!(summary.crit_busy_us, manual_crit_busy);
    assert_eq!(
        summary.crit_domains.iter().map(|&(_, b)| b).sum::<u64>(),
        manual_crit_busy,
        "critical-path attribution must cover the whole critical lane"
    );
    assert!(summary.crit_busy_us <= summary.crit_wall_us);
}

#[test]
fn summarize_lane_spans_hand_built_exact() {
    let spans = [
        LaneSpan {
            tag: 20,
            lane: 0,
            start_us: 0,
            end_us: 100,
        },
        LaneSpan {
            tag: 21,
            lane: 0,
            start_us: 120,
            end_us: 200,
        },
        LaneSpan {
            tag: 22,
            lane: 1,
            start_us: 10,
            end_us: 50,
        },
        LaneSpan {
            tag: 63,
            lane: LW_MAIN_LANE,
            start_us: 210,
            end_us: 250,
        },
    ];
    let summary = summarize_lane_spans(&spans);
    assert_eq!(summary.lanes, vec![(0, 200, 180), (1, 40, 40)]);
    assert_eq!(summary.main_wall_us, 40);
    assert_eq!(summary.crit_lane, Some(0));
    assert_eq!(summary.crit_wall_us, 200);
    assert_eq!(summary.crit_busy_us, 180);
    assert_eq!(summary.crit_domains, vec![(20, 100), (21, 80)]);
}

#[test]
fn summarize_lane_spans_tie_prefers_lowest_lane() {
    let spans = [
        LaneSpan {
            tag: 30,
            lane: 2,
            start_us: 0,
            end_us: 90,
        },
        LaneSpan {
            tag: 31,
            lane: 1,
            start_us: 0,
            end_us: 90,
        },
    ];
    let summary = summarize_lane_spans(&spans);
    assert_eq!(summary.crit_lane, Some(1));
    assert_eq!(summary.crit_wall_us, 90);
}

#[test]
fn summarize_lane_spans_empty_is_zero() {
    let summary = summarize_lane_spans(&[]);
    assert_eq!(summary, LaneWallFrameSummary::default());
}

#[test]
fn lw_unit_names_are_stable_and_formatted() {
    let names = lw_lane_unit_names(3);
    assert_eq!(names[0], "lw_l3_wall_us");
    assert_eq!(names[1], "lw_l3_busy_us");
    assert!(std::ptr::eq(names, lw_lane_unit_names(3)));
    assert_eq!(lw_domain_crit_unit_name(48), "lw_d48_crit_us");
    assert_eq!(
        lw_domain_crit_unit_name(LW_TAG_COUNTRY_BUNDLE),
        "lw_d200_crit_us"
    );
}

#[test]
fn lane_wall_probe_record_is_wellformed_probe_domain_perf() {
    let spans = [
        LaneSpan {
            tag: 48,
            lane: 0,
            start_us: 0,
            end_us: 800,
        },
        LaneSpan {
            tag: LW_TAG_COUNTRY_BUNDLE,
            lane: 1,
            start_us: 0,
            end_us: 300,
        },
        LaneSpan {
            tag: 63,
            lane: LW_MAIN_LANE,
            start_us: 810,
            end_us: 860,
        },
    ];
    let summary = summarize_lane_spans(&spans);
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        Some(2),
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        0,
        &OutsideBuildSpans::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let header =
        compact_contract::parse_compact_record_header(&out).expect("well-formed record header");
    assert_eq!(header.domain_id, RC_PROBE_DOMAIN_ID);
    assert_eq!(header.record_type, COMPACT_RECORD_TYPE_DOMAIN_PERF);
    assert_ne!(
        header.record_flags & compact_contract::COMPACT_RECORD_FLAG_PERF_DIAGNOSTIC_TRAILER,
        0
    );
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..];
    assert_eq!(payload.len(), header.payload_len);
    let text = String::from_utf8_lossy(payload);
    for name in [
        "lw_frames",
        "lw_worker_span_us",
        "lw_prewarm_us",
        "lw_hour_label_skew_abs",
        "lw_hour_label_skew_missing",
        "lw_crit_wall_us",
        "lw_crit_busy_us",
        "lw_lmain_wall_us",
        "lw_l0_wall_us",
        "lw_l0_busy_us",
        "lw_l1_wall_us",
        "lw_l1_busy_us",
        "lw_d48_crit_us",
        "lw_prefetch_dist_relation",
        "lw_prefetch_dist_candidates",
    ] {
        assert!(text.contains(name), "missing unit {name}");
    }
}

/// The country bundle's sub-reads reach the wire under their OWN tags, and do
/// so through a channel that is not a lane span.
///
/// WHAT THIS EXISTS FOR. `LW_TAG_COUNTRY_BUNDLE` is one gated worker, so
/// `lw_d200_crit_us` and the lane units price the WHOLE bundle. On t55
/// (2026-08-15, live, speed 5, 1944 world) that one tag was 99.2% of a 238 ms
/// critical path against a 76 ms hour, and no counter anywhere said which of
/// the bundle's seven sub-reads it was. `lw_cb{tag}_us` is that counter.
///
/// The recording is deliberately not a lane span, and this test pins both
/// halves of that: the sub-reads must NOT appear in `spans` (they run inside
/// the parent's span, so counting them there would push a lane's busy time past
/// its wall), and they must still reach the probe record.
///
/// kills: `country_lane_attribution` keying on `specs.last()` instead of
/// `specs.first()` -> the focus+tech read reports as `lw_cb13_us` and the
/// vector equality fails; dropping any returned read -> that tag's entry
/// disappears; last-write-wins instead of summing -> the repeated tag reports
/// 30 rather than 130; recording the sub-reads as lane spans instead ->
/// `spans` is non-empty here and lane busy exceeds lane wall in production;
/// routing them through `lw_domain_crit_unit_name` -> the emitted name is
/// `lw_d12_crit_us`, which the host folds into
/// `per_domain_critical_path_us_total` and double-counts against
/// `lw_d200_crit_us`.
#[test]
fn country_bundle_sub_reads_are_attributed_under_their_own_units() {
    // The shape `read_compact_country_domain_bundles_parallel` returns: one
    // read per sub-lane, each carrying the specs THAT LANE read. The focus+tech
    // lane carries two specs and is identified by the first.
    let specs = compact_country_domain_specs();
    let spec_of = |id: u16| {
        *specs
            .iter()
            .find(|spec| spec.id == id)
            .expect("country spec")
    };
    let read = |spec_ids: &[u16], read_time_us: u64| CompactBundleRead {
        specs: spec_ids.iter().copied().map(spec_of).collect(),
        rows: Vec::new(),
        read_time_us,
        row_count: 0,
        status_flags: 0,
        technology_ids: Arc::new(TechnologyIdDictionary::default()),
    };
    let country_lanes = country_lane_attribution(&[
        read(&[COMPACT_DOMAIN_FUEL], 4_100),
        read(&[COMPACT_DOMAIN_POLITICS], 100),
        // Focus+tech is ONE sub-read over two specs, published under the first.
        read(&[COMPACT_DOMAIN_FOCUS, COMPACT_DOMAIN_TECH], 91_000),
        // A second read under one tag, as the serial arm's shape allows.
        read(&[COMPACT_DOMAIN_POLITICS], 30),
        // A read naming no spec names no sub-lane and must not invent one.
        read(&[], 500_000),
    ]);
    assert_eq!(
        country_lanes,
        vec![
            (COMPACT_DOMAIN_FUEL, 4_100),
            (COMPACT_DOMAIN_FOCUS, 91_000),
            (COMPACT_DOMAIN_POLITICS, 130),
        ]
    );
    // Not a lane span, so no lane's busy time moves and nothing is recorded on
    // a worker's thread.
    let tracker = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    assert!(
        tracker.take_spans().is_empty(),
        "a sub-read runs inside the bundle's own lane span and must not record \
         a second span on it"
    );

    assert_eq!(lw_country_lane_unit_name(12), "lw_cb12_us");
    assert!(std::ptr::eq(
        lw_country_lane_unit_name(12),
        lw_country_lane_unit_name(12)
    ));

    let mut summary = summarize_lane_spans(&[LaneSpan {
        tag: LW_TAG_COUNTRY_BUNDLE,
        lane: 0,
        start_us: 0,
        end_us: 95_000,
    }]);
    summary.country_lanes = country_lanes;
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        Some(2),
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        0,
        &OutsideBuildSpans::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let text = String::from_utf8_lossy(&out[compact_contract::COMPACT_RECORD_HEADER_LEN..])
        .into_owned();
    for name in ["lw_cb10_us", "lw_cb12_us", "lw_cb14_us"] {
        assert!(text.contains(name), "missing unit {name}");
    }
    // The whole is still published under the bundle's own tag, undisturbed.
    assert!(text.contains("lw_d200_crit_us"));
    // And the parts did NOT go to the critical-path family, whose shares are
    // divided by the critical wall.
    for name in ["lw_d10_crit_us", "lw_d12_crit_us", "lw_d14_crit_us"] {
        assert!(!text.contains(name), "sub-read leaked into {name}");
    }
}

/// The frame builder actually CALLS the attribution, and carries its result to
/// the lane-wall summary.
///
/// The behavioural test above can only reach the pure function; building a real
/// frame here is not possible, so deleting the one production call site — or
/// replacing it with `Vec::new()` — left every assertion in it green (Codex
/// gate finding, round 2). This pins the call site the way this file already
/// pins other single-site production structure: against the collector's own
/// source text.
///
/// It also pins the PLACEMENT, which is the round-2 defect itself. The
/// attribution must be taken from the same drain that emits the bundle's
/// records — after the last clock bracket of the frame — so it cannot delay a
/// bracket opening. A call site that drifts back above the drain fails here.
///
/// kills: deleting the call site, replacing it with an empty vector, assigning
/// `lane_wall.country_lanes` from anything else, or moving the call above the
/// `country_reads.drain(..)` it must sit on.
#[test]
fn the_frame_builder_takes_the_country_attribution_off_the_bundle_drain() {
    let source = super::tests::collector_source_lf();
    let call = "let country_lane_us = country_lane_attribution(&country_reads);";
    assert_eq!(
        source.matches(call).count(),
        1,
        "the frame builder must call the attribution exactly once"
    );
    let assign = "lane_wall.country_lanes = country_lane_us;";
    assert_eq!(
        source.matches(assign).count(),
        1,
        "the summary's country lanes must come from that call and nothing else"
    );
    // ON THE DRAIN, and above it by exactly one statement: the drain consumes
    // `country_reads`, so the attribution cannot follow it, and every clock
    // bracket in the frame has closed by the time the drain runs.
    let drain = "for read in country_reads.drain(..) {";
    assert_eq!(source.matches(drain).count(), 1);
    let call_at = source.find(call).expect("call site");
    let drain_at = source.find(drain).expect("drain site");
    assert!(call_at < drain_at, "the attribution must precede the drain");
    let between = &source[call_at + call.len()..drain_at];
    assert!(
        between.trim().is_empty(),
        "nothing may sit between the attribution and the drain it is taken \
         off; found: {between:?}"
    );
    // And the assignment is downstream of both.
    assert!(source.find(assign).expect("assign site") > drain_at);
}

/// A gated worker's OWN cycles are folded into the frame's split, and the
/// frame's process term covers the fan-out.
///
/// THE MEASUREMENT THIS EXISTS FOR. The capture witness records the target
/// process's total CPU, and the collector is an injected DLL, so that total is
/// the game AND us. A capture can therefore see the process go from 4.9 to 7.2
/// busy cores on attach and still not say whether the game's own half moved --
/// which is the entire question behind "a passive reader should not cost the
/// host wall-clock". `process_cycles - worker_cycles` is the subtraction that
/// answers it, and neither term existed on the wire before.
///
/// The workers below burn a real, unelidable amount of CPU (a volatile
/// accumulator the optimiser cannot drop), so a split that reports zero ours,
/// or that counts the workers but not their cycles, cannot pass.
///
/// MUTATION RUN (`--release`), verbatim:
///  * `lanes.observe_worker_cycles(...)` deleted from `spawn_gated` ->
///    `assertion `left == right` failed: every gated worker must contribute to
///    the split, or the bound it supports covers fewer workers than the wire
///    claims / left: 0 / right: 6`.
///  * `observe_worker_cycles` folded the count but not the cycles
///    (`fetch_add(0, ...)`) -> this test fails at the `worker_cycles > 0`
///    assertion and `a_frame_with_no_process_counter_...` fails
///    `left: 0 / right: 4242`.
///  * `cpu_split` returned a hardcoded `worker_threads: 0` -> this test fails
///    `left: 0 / right: 6` and `a_frame_with_no_process_counter_...` fails
///    `left: 0 / right: 1`.
#[test]
fn the_frame_cpu_split_counts_every_gated_worker_and_its_own_cycles() {
    const PERMITS: usize = 2;
    const WORKERS: u16 = 6;
    let gate = ReaderGate::new(PERMITS);
    let tracker = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    thread::scope(|scope| {
        let gref = Some(&gate);
        let tref = &tracker;
        let handles: Vec<_> = (0..WORKERS)
            .map(|i| {
                spawn_gated(scope, gref, tref, 400 + i, move || {
                    // Real work, not a sleep: a sleeping thread accrues no
                    // cycles, so a sleep here would pass a split that measured
                    // nothing.
                    let mut acc: u64 = 0;
                    for step in 0..2_000_000u64 {
                        acc = acc.wrapping_add(step ^ u64::from(i));
                    }
                    std::hint::black_box(acc);
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    });
    // Production closes the process term on the same statement that closes
    // `worker_read_span_us`; without it the frame is correctly reported as
    // unmeasurable, so the test has to do what production does.
    tracker.close_fanout_process_cycles();
    let split = tracker.cpu_split();
    assert_eq!(
        split.worker_threads,
        u64::from(WORKERS),
        "every gated worker must contribute to the split, or the bound it \
         supports covers fewer workers than the wire claims"
    );
    assert!(
        split.worker_cycles > 0,
        "the workers burned real CPU, so a zero worker term means the split is \
         charging our cycles to the game"
    );
    assert!(
        !split.process_cycles_missing,
        "the process cycle counter must be readable on a supported host"
    );
    // The process ran the same workers plus this thread, so its term cannot be
    // smaller than ours. This is the invariant the subtraction depends on: if
    // it inverts, `process - workers` underflows into a nonsense game term.
    assert!(
        split.process_cycles >= split.worker_cycles,
        "process cycles {} < worker cycles {}; the subtraction that yields the \
         game's own cycles is not defined",
        split.process_cycles,
        split.worker_cycles
    );
}

/// A frame whose process counter was refused contributes NOTHING to either
/// term, and says why.
///
/// THE PAIRING RULE. The host sums the process and worker terms independently
/// across the capture and subtracts the totals to get the game's own cycles.
/// A frame that contributed a real worker term but no process term would shrink
/// that difference by exactly its worker cycles -- with enough such frames it
/// goes negative, and `frames_with_no_counter` can tell you the total is
/// damaged but cannot repair it. So an unmeasurable frame contributes zero to
/// BOTH and raises the flag; the ratio is then over fewer frames, which is a
/// statement the flag makes and the totals no longer have to.
///
/// Three ways a frame can be unmeasurable, all checked: the counter was refused
/// at the origin, the fan-out was never closed, and the counter did not advance
/// across the interval.
///
/// MUTATION RUN (`--release`), verbatim: `cpu_split` changed to return the
/// worker terms alongside a zero process term -> `assertion `left == right`
/// failed: an unmeasurable frame must contribute nothing to the worker term
/// either / left: 4242 / right: 0`.
#[test]
fn a_frame_with_no_process_counter_contributes_to_neither_term() {
    for (label, prepare) in [
        (
            "the origin counter was refused",
            (|t: &mut FrameLaneTracker| {
                t.origin_process_cycles = 0;
                t.fanout_end_process_cycles.store(9_000, Ordering::Relaxed);
            }) as fn(&mut FrameLaneTracker),
        ),
        ("the fan-out was never closed", |t: &mut FrameLaneTracker| {
            t.origin_process_cycles = 1_000;
        }),
        ("the counter went backwards", |t: &mut FrameLaneTracker| {
            t.origin_process_cycles = 9_000;
            t.fanout_end_process_cycles.store(1_000, Ordering::Relaxed);
        }),
    ] {
        let mut tracker = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
        prepare(&mut tracker);
        tracker.observe_worker_cycles(4_242);
        let split = tracker.cpu_split();
        assert!(split.process_cycles_missing, "{label}");
        assert_eq!(split.process_cycles, 0, "{label}");
        assert_eq!(
            split.worker_cycles, 0,
            "{label}: an unmeasurable frame must contribute nothing to the \
             worker term either"
        );
        assert_eq!(split.worker_threads, 0, "{label}");
    }
}

/// The process term is bounded by the FAN-OUT, not by whenever the probe is
/// built.
///
/// In production the probe is appended after the encode and attribution tail,
/// several milliseconds of SERIAL COLLECTOR WORK after the last worker joined.
/// A process term closed at probe time would include those cycles, be divided
/// by a `lw_worker_span_us` that does not cover them, and hand the difference
/// to the game as if the game had burned it. Here the tracker is closed, then
/// a large amount of CPU is burned, then the split is taken: the burn must not
/// appear.
///
/// MUTATION RUN (`--release`), verbatim: `cpu_split` reverted to sampling
/// `process_cycle_counter()` itself instead of reading the stored fan-out close
/// -> `assertion failed: split.process_cycles < burn_cycles / the process term
/// grew by work done after the fan-out closed`.
#[test]
fn the_process_term_is_closed_at_the_fanout_not_at_probe_time() {
    let tracker = FrameLaneTracker::new(Instant::now(), Instant::now(), 0, 0);
    tracker.close_fanout_process_cycles();
    let closed_at = tracker.fanout_end_process_cycles.load(Ordering::Relaxed);
    assert_ne!(closed_at, 0, "the process counter must be readable");
    // Burn enough that a term closed later could not be mistaken for one
    // closed at the fan-out.
    let before = process_cycle_counter().expect("readable");
    let mut acc: u64 = 0;
    for step in 0..20_000_000u64 {
        acc = acc.wrapping_add(step ^ 0x9e37_79b9);
    }
    std::hint::black_box(acc);
    let burn_cycles = process_cycle_counter().expect("readable") - before;
    assert!(burn_cycles > 0, "the burn must move the process counter");
    let split = tracker.cpu_split();
    assert!(
        !split.process_cycles_missing,
        "a closed fan-out with a readable counter is a measurement"
    );
    assert!(
        split.process_cycles < burn_cycles,
        "the process term grew by work done after the fan-out closed: \
         {} cycles reported, {burn_cycles} burned afterwards",
        split.process_cycles
    );
}

/// All four CPU-split units reach the wire, including the flag.
///
/// Red if any is dropped from `append_compact_lane_wall_probe_record` -- a
/// consumer would then read the remaining ones as complete.
#[test]
fn the_cpu_split_units_reach_the_probe() {
    let summary = summarize_lane_spans(&[]);
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        None,
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        0,
        0,
        0,
        &OutsideBuildSpans::default(),
        &summary,
        FrameCpuSplit {
            process_cycles: 987_654_321,
            worker_cycles: 123_456_789,
            worker_threads: 11,
            process_cycles_missing: false,
        },
    );
    // Decoded, not string-matched: the values are little-endian binary in the
    // PDU1 trailer, and a test that only looked for the NAMES would pass with
    // every unit carrying the same wrong number.
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..];
    let trailer = &payload[compact_contract::COMPACT_DOMAIN_PERF_BASE_LEN..];
    assert_eq!(
        &trailer[..4],
        compact_contract::COMPACT_DOMAIN_PERF_DIAGNOSTIC_UNITS_MAGIC
    );
    let (units, _) = compact_contract::diagnostic_channel::decode_units(&trailer[6..])
        .expect("the probe's own trailer must decode");
    for (name, expected) in [
        ("lw_fanout_process_cycles", 987_654_321u64),
        ("lw_fanout_worker_cycles", 123_456_789),
        ("lw_fanout_worker_threads", 11),
        ("lw_fanout_cycles_missing", 0),
    ] {
        let unit = units
            .iter()
            .find(|unit| unit.name == name)
            .unwrap_or_else(|| panic!("missing unit {name}"));
        assert_eq!(unit.value, expected, "unit {name} carries the wrong value");
    }
}

/// The outside-build segments must reach the wire with THEIR OWN values, each
/// under the name its code owns.
///
/// This is the whole point of the lane: the ~27 ms per frame that is not inside
/// `build_compact_extraction_gate_records` was previously knowable only as
/// `frame_period - frame_time_us`, a subtraction of two other numbers. A probe
/// that emitted the segments under the wrong names, or emitted one segment's
/// value twelve times, would look exactly as healthy in a manifest and would
/// misattribute one call's milliseconds to another.
///
/// MUTATION RUN: `value: outside_build.us[index]` in the emission loop was
/// changed to `value: outside_build.us[0]`, and the whole suite run in
/// `--release`. Observed: this test failed with
/// `segment 1 (wait) carries 1000 on the wire but the accumulator holds 2000`
/// — the first segment's value published under every name. Restored.
#[test]
fn every_outside_build_segment_reaches_the_probe_under_its_own_name() {
    // Distinct per segment, and distinct from every other value the probe
    // carries, so a crossed wire cannot pass by coincidence.
    let mut outside = OutsideBuildSpans::default();
    for (index, slot) in outside.us.iter_mut().enumerate() {
        *slot = 1_000 + index as u64 * 1_000;
    }
    let summary = summarize_lane_spans(&[]);
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        Some(2),
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        0,
        &outside,
        &summary,
        FrameCpuSplit::default(),
    );
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..];
    let units = compact_perf_diagnostic_units(payload);
    for (code, name) in compact_contract::diagnostic_channel::OUTSIDE_BUILD_SEGMENTS {
        let wire_name =
            compact_contract::diagnostic_channel::OUTSIDE_BUILD_UNIT_NAMES[usize::from(*code)];
        let unit = units
            .iter()
            .find(|unit| unit.name == wire_name)
            .unwrap_or_else(|| panic!("{wire_name} missing from the probe record"));
        assert_eq!(
            unit.value,
            outside.us[usize::from(*code)],
            "segment {code} ({name}) carries {} on the wire but the accumulator \
             holds {}",
            unit.value,
            outside.us[usize::from(*code)]
        );
    }
}

/// A ZERO segment is still emitted. "This call ran and cost nothing" and "this
/// collector does not measure that call" are different facts, and a manifest
/// that cannot tell them apart cannot price a change to that call.
///
/// MUTATION RUN: the emission `units.push(..)` was wrapped in
/// `if outside_build.us[index] != 0 { .. }`, which is the shape
/// `lw_hour_label_skew_abs` legitimately uses. Observed: this test failed with
/// `a segment measured as zero must still be stated: lw_o0_us`. Restored.
#[test]
fn a_zero_outside_build_segment_is_stated_rather_than_omitted() {
    let summary = summarize_lane_spans(&[]);
    let mut out = Vec::new();
    append_compact_lane_wall_probe_record(
        &mut out,
        7,
        123,
        5,
        900,
        125,
        Some(2),
        FrameHourAge::default(),
        &[],
        &BTreeSet::new(),
        MapTopologyShareCounts::default(),
        130,
        700,
        0,
        &OutsideBuildSpans::default(),
        &summary,
        FrameCpuSplit::default(),
    );
    let payload = &out[compact_contract::COMPACT_RECORD_HEADER_LEN..];
    let units = compact_perf_diagnostic_units(payload);
    for name in compact_contract::diagnostic_channel::OUTSIDE_BUILD_UNIT_NAMES {
        let unit = units
            .iter()
            .find(|unit| &unit.name == name)
            .unwrap_or_else(|| panic!("a segment measured as zero must still be stated: {name}"));
        assert_eq!(unit.value, 0);
    }
}

/// `add_since` ADDS. Between two builds the loop can run several iterations —
/// an hour that produced no sweep decision, a re-attempt after an uncertain
/// campaign boundary — and each one's identity re-read and owner polls are
/// time the next build was not running. An overwriting accumulator would
/// report only the last iteration's cost and make the extra iterations free.
///
/// MUTATION RUN: `add_since`'s `slot.saturating_add(..)` was changed to a
/// plain assignment, and this test re-run against it in `--release` after the
/// assertion below was strengthened. Observed: it failed with `64 closes of one
/// ageing span totalled 1001 us, which is no more than the 1001 us that span
/// had aged by the end — that is what assignment gives`. The two figures being
/// equal is the mutation's signature: assignment leaves exactly the last age
/// read. Restored.
#[test]
fn outside_build_segments_accumulate_across_iterations() {
    const PASSES: u64 = 64;
    let mut outside = OutsideBuildSpans::default();
    // ONE instant, aged past a millisecond, then closed against `PASSES`
    // times. The instant keeps ageing, so the two arithmetics separate by a
    // factor of `PASSES` rather than by a margin scheduler noise could cover:
    // accumulation lands at or above `PASSES` ms, assignment lands at whatever
    // the instant reads on the last pass, which is a few ms at most.
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(1) {
        std::hint::spin_loop();
    }
    for _ in 0..PASSES {
        outside.add_since(OutsideBuildSegment::Identity, started);
    }
    let elapsed_after_us = duration_micros_u64(started.elapsed());
    let total = outside.us[OutsideBuildSegment::Identity as usize];
    // The separator, and exactly what it is worth. Under assignment the slot
    // holds the age read on the LAST pass, which a monotonic clock guarantees
    // is no greater than the age read here, one statement later — so the
    // mutation cannot pass this assertion at all. That is the hole a floor
    // alone left open: a stall inflates both sides of `total >= PASSES * 1_000`
    // equally, so a descheduled thread could satisfy it under assignment.
    //
    // It is NOT symmetric, and the residual risk runs the other way. Correct
    // accumulation fails here if this thread is descheduled between the last
    // `add_since` and the reading below for longer than the sum of the first 63
    // ages — about 63 ms. That is a false FAILURE, not a false pass, so it
    // cannot hide the defect; it can only interrupt a build. Two clock reads
    // one statement apart is as close as a test of a clock-reading function
    // gets, and the remaining window is stated rather than called determinism.
    assert!(
        total > elapsed_after_us,
        "add_since must ACCUMULATE across the iterations between two builds: \
         {PASSES} closes of one ageing span totalled {total} us, which is no \
         more than the {elapsed_after_us} us that span had aged by the end — \
         that is what assignment gives"
    );
    assert!(
        total >= PASSES * 1_000,
        "each of the {PASSES} closes covered a span already older than 1 ms, \
         so the accumulated total cannot be under {} us; it was {total}",
        PASSES * 1_000
    );
    // The other segments are untouched: a segment must never collect a
    // neighbour's time.
    for (index, value) in outside.us.iter().enumerate() {
        if index != OutsideBuildSegment::Identity as usize {
            assert_eq!(*value, 0, "segment {index} collected another's time");
        }
    }
}

/// The `OutsideBuildSegment` discriminants ARE the wire codes. The producer
/// indexes `OUTSIDE_BUILD_UNIT_NAMES` by `segment as usize`, so a discriminant
/// that drifts from the contract table publishes one segment's milliseconds
/// under another segment's name — silently, and in a manifest that reads as
/// healthy.
///
/// MUTATION RUN: the contract entry `(3, "prepoll")` was changed to
/// `(3, "prepub")`. Observed: this test failed with
/// `OutsideBuildSegment::Prepoll has discriminant 3 but the contract gives
/// code 3 to "prepub"`. Restored.
///
/// A discriminant mutation such as `Prepub = 5` is NOT the equivalent check:
/// `Complete` already holds 5, and Rust rejects duplicate discriminants before
/// the test can run. The mutation that reaches this assertion is a change to
/// the contract table, which is where a real drift would happen anyway.
#[test]
fn outside_build_segment_discriminants_match_the_contract() {
    const DECLARED: &[(OutsideBuildSegment, &str)] = &[
        (OutsideBuildSegment::Total, "total"),
        (OutsideBuildSegment::Wait, "wait"),
        (OutsideBuildSegment::Identity, "identity"),
        (OutsideBuildSegment::Prepoll, "prepoll"),
        (OutsideBuildSegment::Prepub, "prepub"),
        (OutsideBuildSegment::Complete, "complete"),
        (OutsideBuildSegment::DegNotice, "degnotice"),
        (OutsideBuildSegment::DegLog, "deglog"),
        (OutsideBuildSegment::OpenPub, "openpub"),
        (OutsideBuildSegment::A30Insert, "a30insert"),
        (OutsideBuildSegment::HookStage, "hookstage"),
        (OutsideBuildSegment::TailInsert, "tailinsert"),
        (OutsideBuildSegment::BuildTail, "buildtail_previous_frame"),
    ];
    assert_eq!(
        DECLARED.len(),
        compact_contract::diagnostic_channel::OUTSIDE_BUILD_SEGMENT_COUNT,
        "a segment was added to the contract without a discriminant, or the \
         other way round"
    );
    for (segment, expected) in DECLARED {
        let code = *segment as usize;
        let contract = compact_contract::diagnostic_channel::outside_build_segment_name(code as u16)
            .unwrap_or_else(|| panic!("{segment:?} has discriminant {code}, outside the contract"));
        assert_eq!(
            contract, *expected,
            "OutsideBuildSegment::{segment:?} has discriminant {code} but the \
             contract gives code {code} to {contract:?}"
        );
    }
}

// Micro-cost proof (lever #5): run with
//   cargo test -p collector --release lane_wall_micro_cost -- --ignored --nocapture
// Budget: < 100 us/frame for the WHOLE added path (48 span records +
// summary + probe-record encode).

/// `BuildTail` is a SUBTRACTION, and the subtraction must never wrap.
///
/// `frame_time_us` is stamped inside `build_compact_extraction_gate_records`
/// and can therefore only be smaller than the call's own wall time — but "can
/// only be" is an argument about a monotonic clock and a call that returns,
/// and a `-` would turn any violation of it into ~18 quintillion microseconds
/// published as a frame segment. The whole `lw_o*` family exists to be
/// believed, so the one arithmetic operation in it that can underflow is
/// pinned.
///
/// MUTATION RUN: `outside_build_tail_us`'s `saturating_sub` was changed to a
/// plain `-`. Observed under `--release`, which is what the gate runs: this
/// test failed with
/// `left: 18446744073709548616 right: 0`. Restored.
#[test]
fn a_build_tail_shorter_than_its_own_frame_time_records_zero() {
    assert_eq!(
        outside_build_tail_us(90_000, 93_000),
        0,
        "a build whose wall time reads shorter than the frame_time_us it \
         returned must record zero, never a wrapped u64"
    );
    assert_eq!(outside_build_tail_us(94_500, 93_000), 1_500);
    assert_eq!(outside_build_tail_us(0, 0), 0);

    let mut outside = OutsideBuildSpans::default();
    outside.add_us(OutsideBuildSegment::BuildTail, outside_build_tail_us(90_000, 93_000));
    outside.add_us(OutsideBuildSegment::BuildTail, outside_build_tail_us(94_500, 93_000));
    assert_eq!(outside.us[OutsideBuildSegment::BuildTail as usize], 1_500);
}

// ---------------------------------------------------------------------------
// Outside-build cost bench.
//
//   cargo test -p collector --release outside_build_preboundary_cost \
//       -- --ignored --nocapture
//
// Ignored because it is a MEASUREMENT, not an assertion: it prices the parts of
// the pre-boundary publication path that are pure CPU and are therefore
// reproducible without a live game. The frame shape is transcribed from the
// third mid-capture frame of `runs/live_unlock/t19` (read-only), decoded with
// the H4CR header contract:
//
//   4,586,802 bytes / 121 records
//   51 DOMAIN_PERF (13 with a non-zero status word), 48 DOMAIN_DATA,
//   20 DOMAIN_GAP, 1 FRAME_DEGRADED, 1 FRAME_END
//   largest records: d47 953 KB, d24 439 KB, d26 419 KB, d43 415 KB
//
// The shape is the point. "Each `insert_*_before_frame_end` memmoves a ~7 MB
// record buffer" was the standing hypothesis for where the outside-build time
// goes, and both halves of it are testable here: the walk visits 121 headers
// spread over 4.6 MB (one cache line each, not 4.6 MB of traffic), and the
// splice point is the LAST record in the buffer, so the tail it moves is one
// FRAME_END and not the frame.
//
// Set `BATTLE_PLAN_LANE_AE_FRAME_BLOB` to a file holding a real frame's records to
// price those bytes instead of the synthetic ones.
//
// MEASURED, six interleaved runs (three synthetic, three on t19's real frame
// bytes) on the development machine, with every buffer and argument prepared
// outside the clock. Medians of the real-bytes runs, with the observed range:
//
//   completeness walk                  32.1 us/frame   (31.9 - 32.3)
//   H4FD insert, 64 KiB spare            8.6 us/frame   (8.2 - 8.7)
//   H4FD insert, zero spare            513.6 us/frame   (512.8 - 516.4)
//   degraded-frame log append          133.9 us/frame   (120.1 - 140.7)
//   bare 4.59 MB clone                 476.2 us         (472.5 - 483.1)
//
// The two H4FD arms differ ONLY in spare capacity, and the run states which
// side of the growth condition each landed on rather than assuming it: with
// 65536 bytes spare against a 69-byte insertion, splice need not grow, and it
// costs 8.6 us. With `shrink_to_fit` having left exactly zero spare, splice
// must grow, and it costs 513.6 us — 37.4 us above the bare clone of the same
// bytes, 476.2 us, which already includes its own allocation. That is the
// reallocating case measured rather than argued, and it is still 0.5 ms.
//
// Production takes the first arm: `records` reaches these functions from a
// `Vec` grown by `extend_from_slice`, whose doubling leaves spare capacity.
//
// THE HYPOTHESIS IS REFUTED, and the refutation is what this bench exists for.
// The frame holds 121 records, not thousands, so a walk of every header touches
// 121 cache lines and costs 32 us. The splice moves one FRAME_END record and
// costs 8.6 us.
//
// Summed, the whole offline-measurable pre-boundary path is 0.17 ms/frame
// against a ~27 ms outside-build interval: 0.6% of it. Even three inserts all
// forced to reallocate — the worst case that can be constructed here — would be
// 1.5 ms, under 6%. The outside-build time is therefore NOT record-buffer
// handling, and the `lw_o*` segments in the next capture are what will say
// where it is.
#[test]
#[ignore]
fn outside_build_preboundary_cost() {
    const HOUR: i32 = 61_000_000;
    // The twelve largest DATA payloads of that frame, in bytes.
    const DATA_PAYLOADS: &[usize] = &[
        953_474, 439_056, 418_872, 414_556, 396_512, 383_408, 245_973, 231_936, 163_984, 158_500,
        146_437, 89_981,
    ];

    let records = match std::env::var("BATTLE_PLAN_LANE_AE_FRAME_BLOB").or_else(|_| std::env::var("HOI4_LANE_AE_FRAME_BLOB")) {
        Ok(path) => {
            let bytes = std::fs::read(&path).expect("frame blob");
            println!("frame blob {path}: {} bytes", bytes.len());
            bytes
        }
        Err(_) => {
            let mut out: Vec<u8> = Vec::new();
            let mut sequence = 0u64;
            // 51 PERF records; 13 carry a non-zero status word at +32, which is
            // what puts a domain on the completeness walk's offender path.
            for index in 0..51u16 {
                let mut payload = vec![0u8; 64];
                if index < 13 {
                    payload[32..36].copy_from_slice(&0x0000_0002u32.to_le_bytes());
                }
                append_compact_record(
                    &mut out,
                    sequence,
                    HOUR,
                    10 + index,
                    COMPACT_RECORD_TYPE_DOMAIN_PERF,
                    0,
                    &payload,
                );
                sequence += 1;
            }
            // 48 DATA records: the twelve measured large ones, then 36 more
            // sized so the frame lands on t19's 4.59 MB.
            let large: usize = DATA_PAYLOADS.iter().sum();
            let remainder = 4_586_802usize.saturating_sub(large) / 36;
            for (index, len) in DATA_PAYLOADS
                .iter()
                .copied()
                .chain(std::iter::repeat(remainder).take(36))
                .enumerate()
            {
                append_compact_record(
                    &mut out,
                    sequence,
                    HOUR,
                    10 + index as u16,
                    COMPACT_RECORD_TYPE_DOMAIN_DATA,
                    0,
                    &vec![0x5Au8; len],
                );
                sequence += 1;
            }
            for index in 0..20u16 {
                append_compact_record(
                    &mut out,
                    sequence,
                    HOUR,
                    10 + index,
                    compact_contract::COMPACT_RECORD_TYPE_DOMAIN_GAP,
                    0,
                    &vec![0u8; 48],
                );
                sequence += 1;
            }
            append_compact_record(
                &mut out,
                sequence,
                HOUR,
                compact_contract::COMPACT_DOMAIN_FRAME,
                COMPACT_RECORD_TYPE_FRAME_END,
                0,
                &vec![0u8; 64],
            );
            out
        }
    };

    let mut count = 0usize;
    let mut offset = 0usize;
    let mut last_sequence = 0u64;
    let mut last_hour = 0i32;
    while offset < records.len() {
        let header = compact_contract::parse_compact_record_header(&records[offset..]).unwrap();
        last_sequence = header.sequence;
        last_hour = header.game_hour_raw;
        offset += header.record_len();
        count += 1;
    }
    println!(
        "frame: {} bytes, {count} records, {:.1} KB/record",
        records.len(),
        records.len() as f64 / count as f64 / 1024.0
    );

    let outcomes = compact_contract::domain_outcome::DomainOutcomeLedger::new();
    const REPS: u32 = 40;

    // 1. The completeness walk: one pass over every header in the frame.
    let started = Instant::now();
    let mut findings = 0usize;
    for _ in 0..REPS {
        findings += compact_frame_completeness_findings(&records, last_hour, &outcomes).len();
    }
    println!(
        "completeness walk: {:.1} us/frame ({findings} findings over {REPS} reps)",
        started.elapsed().as_secs_f64() * 1e6 / f64::from(REPS)
    );

    // 2. A bare clone of the frame: the price of one full copy of these bytes,
    //    which is what a "~7 MB memmove" would cost. Measured first so the
    //    setup cost inside 3 and 4 is subtractable.
    let started = Instant::now();
    for _ in 0..REPS {
        let buffer = records.clone();
        std::hint::black_box(&buffer);
    }
    let clone_us = started.elapsed().as_secs_f64() * 1e6 / f64::from(REPS);
    println!("bare {} byte clone: {clone_us:.1} us", records.len());

    let domains = vec![compact_contract::frame_degraded::FrameDegradedDomain {
        domain_id: 47,
        status_flags: 2,
        non_tolerated_flags: 2,
        reason: "bench".to_string(),
    }];

    // 3 and 4. The H4FD insertion, with the SETUP OUTSIDE THE CLOCK.
    //
    // The earlier form of these arms timed `buffer = records.clone()` (or
    // `Vec::with_capacity` + `extend_from_slice`) together with the insertion
    // and then subtracted a standalone clone, which folded the setup's
    // allocator behaviour into the reported figure. Here each buffer is
    // prepared first and only the `insert_*` call is inside the timer, so what
    // is printed is the insertion and nothing else.
    //
    // The two arms differ ONLY in spare capacity, and the condition is stated
    // as the code's own: `splice` grows when `capacity - len < encoded.len()`,
    // not merely when `capacity == len`. `shrink_to_fit` is a request the
    // allocator may decline, so the actual spare capacity of each prepared
    // buffer is printed rather than assumed.
    let notice_len = compact_contract::COMPACT_RECORD_HEADER_LEN
        + compact_contract::frame_degraded::FrameDegradedRecordV1 {
            degraded_hour: last_hour,
            domains: domains.clone(),
        }
        .encode()
        .expect("bench notice encodes")
        .len();
    for (label, prepare) in [
        (
            "spare capacity",
            (|records: &Vec<u8>| {
                let mut buffer = Vec::with_capacity(records.len() + 65_536);
                buffer.extend_from_slice(records);
                buffer
            }) as fn(&Vec<u8>) -> Vec<u8>,
        ),
        ("shrunk to fit", |records: &Vec<u8>| {
            let mut buffer = records.clone();
            buffer.shrink_to_fit();
            buffer
        }),
    ] {
        let mut buffers: Vec<Vec<u8>> = (0..REPS).map(|_| prepare(&records)).collect();
        // The argument list is built OUTSIDE the timer too. A `domains.clone()`
        // written at the call site is argument evaluation, and argument
        // evaluation happens after the clock is read — so the clone was being
        // counted as insertion cost.
        let mut arguments: Vec<Vec<compact_contract::frame_degraded::FrameDegradedDomain>> =
            (0..REPS).map(|_| domains.clone()).collect();
        let spare: Vec<usize> = buffers
            .iter()
            .map(|buffer| buffer.capacity() - buffer.len())
            .collect();
        let min_spare = spare.iter().copied().min().unwrap_or(0);
        let max_spare = spare.iter().copied().max().unwrap_or(0);
        let mut total = Duration::ZERO;
        for (buffer, argument) in buffers.iter_mut().zip(arguments.drain(..)) {
            let mut next = last_sequence + 1;
            let started = Instant::now();
            insert_frame_degraded_notice_before_frame_end(buffer, &mut next, last_hour, argument)
                .unwrap();
            total += started.elapsed();
        }
        // Strongest claim first. `max_spare < notice_len` implies
        // `min_spare < notice_len`, so testing the minimum first made the mixed
        // case unreachable and reported "MUST" for a set in which only some
        // buffers had to grow.
        let growth = if max_spare < notice_len {
            "MUST"
        } else if min_spare < notice_len {
            "may (mixed)"
        } else {
            "need not"
        };
        println!(
            "H4FD insert, {label}: {:.1} us/frame (insert only); spare capacity \
             {min_spare}..{max_spare} bytes vs a {notice_len}-byte insertion, so \
             splice {growth} grow",
            total.as_secs_f64() * 1e6 / f64::from(REPS),
        );
    }

    // 5. The degraded-frame line appended to `collector_worker_error.log`: an
    //    open/append/close per degraded frame, on the sweep thread. Every frame
    //    in t19 was degraded, so this ran 480 times out of 480.
    let started = Instant::now();
    for index in 0..REPS {
        record_attach_worker_error(
            "lane_ae_bench",
            &format!("hour {index} published with degraded domains: d47:0x00000002[bench]"),
        );
    }
    println!(
        "degraded-frame log append: {:.1} us/frame",
        started.elapsed().as_secs_f64() * 1e6 / f64::from(REPS)
    );
}
