//! `rc_domain_tests`, extracted verbatim from `attach_log.rs`.

use super::*;

#[test]
fn authenticated_early_render_rejects_noncompact_capture_mode() {
    assert!(early_render_capture_mode_values(false, false, false, false));
    assert!(!early_render_capture_mode_values(true, true, true, false));
    assert!(!early_render_capture_mode_values(true, true, false, true));
    assert!(!early_render_capture_mode_values(true, false, true, true));
    assert!(early_render_capture_mode_values(true, true, true, true));
}

fn compact_headers(records: &[u8]) -> Vec<compact_contract::CompactRecordHeader> {
    let mut offset = 0usize;
    let mut headers = Vec::new();
    while offset < records.len() {
        let header = compact_contract::parse_compact_record_header(&records[offset..])
            .expect("valid compact record");
        offset += header.record_len();
        headers.push(header);
    }
    headers
}

#[test]
fn production_render_insert_is_single_before_frame_end_and_hour_bound() {
    let hour_index = (1936i64 * 365 + 10) * 24 + 5;
    let hour = compact_contract::calendar::DATE_EPOCH_CONST + hour_index as i32;
    let mut records = Vec::new();
    append_compact_frame_end_record(&mut records, 7, hour, 0, 0, 1, 1, 0);
    assert!(insert_production_render_payload_before_frame_end(
        &mut records,
        8,
        hour,
        hour_index as u64,
        b"strict-d77-fixture",
    )
    .unwrap());
    let headers = compact_headers(&records);
    assert_eq!(headers.len(), 2);
    assert_eq!((headers[0].domain_id, headers[0].sequence), (77, 7));
    assert_eq!(
        (headers[1].record_type, headers[1].sequence),
        (COMPACT_RECORD_TYPE_FRAME_END, 8)
    );
    assert!(insert_production_render_payload_before_frame_end(
        &mut records,
        9,
        hour,
        hour_index as u64,
        b"duplicate",
    )
    .unwrap_err()
    .contains("already contains"));

    let mut wrong_hour = Vec::new();
    append_compact_frame_end_record(&mut wrong_hour, 1, hour, 0, 0, 1, 1, 0);
    assert!(insert_production_render_payload_before_frame_end(
        &mut wrong_hour,
        2,
        hour,
        hour_index as u64 + 1,
        b"wrong-hour",
    )
    .unwrap_err()
    .contains("enclosing H4CR hour"));
}

#[test]
fn production_render_precedes_scheduler_probe_and_frame_end() {
    let hour_index = (1936i64 * 365 + 11) * 24;
    let hour = compact_contract::calendar::DATE_EPOCH_CONST + hour_index as i32;
    let mut records = Vec::new();
    append_compact_frame_end_record(&mut records, 7, hour, 0, 0, 1, 1, 0);
    assert!(insert_production_render_payload_before_frame_end(
        &mut records,
        8,
        hour,
        hour_index as u64,
        b"strict-d77-fixture",
    )
    .unwrap());
    let telemetry = SchedulerTelemetry {
        observations: 1,
        scheduled_hours: 1,
        accepted_hours: 1,
        ..SchedulerTelemetry::default()
    };
    assert!(insert_compact_scheduler_probe_before_frame_end(
        &mut records,
        9,
        hour,
        0,
        &telemetry,
        &SinkDeliveryState::new(true, true),
        true,
    ));
    let headers = compact_headers(&records);
    assert_eq!(headers.len(), 3);
    assert_eq!((headers[0].domain_id, headers[0].sequence), (77, 7));
    assert_eq!((headers[1].domain_id, headers[1].sequence), (250, 8));
    assert_eq!(headers[2].sequence, 9);
    assert_eq!(headers[2].record_type, COMPACT_RECORD_TYPE_FRAME_END);
}

#[test]
fn scheduler_probe_is_inserted_before_physical_final_frame_end() {
    let hour = 123;
    let mut records = Vec::new();
    append_compact_frame_end_record(&mut records, 7, hour, 0, 0, 1, 1, 0);
    let telemetry = SchedulerTelemetry {
        observations: 1,
        scheduled_hours: 1,
        accepted_hours: 1,
        ..SchedulerTelemetry::default()
    };
    let sink = SinkDeliveryState::new(true, true);
    assert!(insert_compact_scheduler_probe_before_frame_end(
        &mut records,
        8,
        hour,
        0,
        &telemetry,
        &sink,
        true,
    ));

    let first = compact_contract::parse_compact_record_header(&records).unwrap();
    assert_eq!(first.domain_id, RC_PROBE_DOMAIN_ID);
    assert_eq!(first.record_type, COMPACT_RECORD_TYPE_DOMAIN_PERF);
    assert_eq!(first.sequence, 7);
    let final_offset = first.record_len();
    let final_header =
        compact_contract::parse_compact_record_header(&records[final_offset..]).unwrap();
    assert_eq!(final_header.record_type, COMPACT_RECORD_TYPE_FRAME_END);
    assert_eq!(final_header.sequence, 8);
    assert_eq!(final_offset + final_header.record_len(), records.len());
    assert!(compact_frame_complete_for_scheduler(
        &records,
        hour,
        &compact_contract::domain_outcome::DomainOutcomeLedger::new(),
    ));
}

// The tests below toggle the process-global RC_ENABLED and drain the
// process-global per-domain sink, so they take
// `super::lock_rc_counters()` — the ONE lock over the whole read-counter
// family, whose doc enumerates every entry point.
//
// A module-private lock stood here and did not work. It serialised these
// tests against each other but not against the 36 tests OUTSIDE this
// module that drive a production frame, and frame assembly stores the
// control's perf gate into RC_ENABLED at the top of every frame. The
// residual flip was covered by an `eventually()` helper that re-ran each
// body up to five times; it is gone, along with every retry, because a
// retry loop around a race also swallows a genuine regression that
// breaks the counters four times in five. Every assertion here now runs
// ONCE and must hold on the first attempt.

// Uses domain ids 213/214, which no production code tags, so parallel
// tests bumping the GLOBAL counters cannot contaminate these slots.
#[test]
fn rc_domain_scope_attributes_nested_counts_and_drains() {
    let _rc_counters = super::lock_rc_counters();
    RC_ENABLED.store(true, Ordering::Relaxed);
    // Pre-drain our slots in case of a previous run in this process.
    let _ = rc_domain_take_and_reset();
    {
        let _outer = RcDomainScope::new(213);
        rc_bump(RC_READS, 5);
        rc_bump(RC_SEH_CALLS, 5);
        {
            let _inner = RcDomainScope::new(214);
            rc_bump(RC_READS, 7);
        }
        // back under the outer tag after inner drop
        rc_bump(RC_READS, 3);
    }
    rc_flush_current_thread();
    RC_ENABLED.store(false, Ordering::Relaxed);
    let drained = rc_domain_take_and_reset();
    let find = |id: u16| drained.iter().find(|(d, _)| *d == id).map(|(_, v)| *v);
    let d213 = find(213).expect("slot 213 drained");
    let d214 = find(214).expect("slot 214 drained");
    assert_eq!(
        d213[RC_PD_READS], 8,
        "outer reads: 5 before + 3 after inner"
    );
    assert_eq!(d213[RC_PD_SEH_CALLS], 5);
    assert_eq!(d214[RC_PD_READS], 7, "inner scope attributed separately");
    // drained means drained
    assert!(rc_domain_take_and_reset()
        .iter()
        .all(|(d, _)| *d != 213 && *d != 214));
}

#[test]
fn rc_domain_unit_names_are_stable_and_formatted() {
    let n = rc_domain_unit_names(213);
    assert_eq!(n[RC_PD_READS], "rc_d213_reads");
    assert_eq!(n[RC_PD_BULK_BYTES], "rc_d213_bulk_bytes");
    assert_eq!(n[RC_PD_SEH_CALLS], "rc_d213_seh_calls");
    assert_eq!(n[RC_PD_VQ_CALLS], "rc_d213_vq_calls");
    assert_eq!(n[RC_PD_GUARDED_SPL_NS], "rc_d213_guarded_spl_ns");
    assert_eq!(n[RC_PD_GUARDED_SPL_N], "rc_d213_guarded_spl_n");
    assert_eq!(n[RC_PD_SCAN_GUARDED_NS], "rc_d213_scan_guarded_ns");
    // Same leaked pointers on second lookup (no re-leak per call).
    let again = rc_domain_unit_names(213);
    assert!(std::ptr::eq(n, again));
}

// Deterministic stride: with the phase pinned to 0, exactly every 64th
// logical read is sampled, counts route to the active domain tag, and
// the sampled-time accumulator moves. Domain id 215 is test-reserved.
#[test]
fn rc_read_probe_samples_every_64th_and_routes_per_domain() {
    let _rc_counters = super::lock_rc_counters();
    RC_ENABLED.store(true, Ordering::Relaxed);
    let _ = rc_domain_take_and_reset();
    // Pin a NONZERO phase (0 would trigger the lazy re-seed).
    RC_LOCAL.with(|r| r.spl_ctr.set(64));
    {
        let _scope = RcDomainScope::new(215);
        for _ in 0..128 {
            let probe = rc_read_probe(16);
            // Give a sampled window a measurable body so elapsed
            // nanos cannot floor to 0 at QPC resolution.
            let mut acc = 0u64;
            for i in 0..2_000u64 {
                acc = std::hint::black_box(acc.wrapping_add(i));
            }
            std::hint::black_box(acc);
            rc_read_probe_end(probe);
        }
    }
    rc_flush_current_thread();
    RC_ENABLED.store(false, Ordering::Relaxed);
    let drained = rc_domain_take_and_reset();
    let d215 = drained
        .iter()
        .find(|(d, _)| *d == 215)
        .map(|(_, v)| *v)
        .expect("slot 215 drained");
    // From phase 64, counts run 65..=192: samples at exactly 128
    // and 192. (A phase-shifted stride would see 3; see the
    // boundary test below for the 63-stride discriminator.)
    assert_eq!(d215[RC_PD_READS], 128);
    assert_eq!(d215[RC_PD_BULK_BYTES], 128 * 16);
    assert_eq!(d215[RC_PD_GUARDED_SPL_N], 2);
    assert!(d215[RC_PD_GUARDED_SPL_NS] > 0);
    assert!(d215[RC_PD_GUARDED_SPL_N] <= d215[RC_PD_READS]);
}

// Exact stride boundary (1R review finding 3: the aggregate-count
// oracle above alone cannot distinguish stride 63 or a phase-shifted
// 64 — this can): from phase 126, read #1 (count 127) must NOT sample
// and read #2 (count 128) MUST. A 63-stride samples neither (127%63=1,
// 128%63=2); a shifted 64 samples the wrong one.
#[test]
fn rc_read_probe_stride_boundary_is_exact() {
    let _rc_counters = super::lock_rc_counters();
    RC_ENABLED.store(true, Ordering::Relaxed);
    let _ = rc_domain_take_and_reset();
    RC_LOCAL.with(|r| r.spl_ctr.set(126));
    let mut sampled = [false; 2];
    {
        let _scope = RcDomainScope::new(215);
        for slot in sampled.iter_mut() {
            let probe = rc_read_probe(0);
            *slot = probe.is_some();
            rc_read_probe_end(probe);
        }
    }
    rc_flush_current_thread();
    RC_ENABLED.store(false, Ordering::Relaxed);
    let drained = rc_domain_take_and_reset();
    let d215 = drained
        .iter()
        .find(|(d, _)| *d == 215)
        .map(|(_, v)| *v)
        .expect("slot 215 drained");
    assert!(!sampled[0], "count 127 must not sample");
    assert!(sampled[1], "count 128 must sample");
    assert_eq!(d215[RC_PD_READS], 2);
    assert_eq!(d215[RC_PD_GUARDED_SPL_N], 1);
}

// Fresh threads must NOT start at phase 0, and DIFFERENT fresh threads
// must get DIFFERENT phases (1R2 finding: a constant shared seed would
// pass a mere nonzero check while preserving frame-wide phase lock).
#[test]
fn rc_read_probe_seeds_fresh_threads_with_distinct_phases() {
    let _rc_counters = super::lock_rc_counters();
    RC_ENABLED.store(true, Ordering::Relaxed);
    let grab = || {
        std::thread::spawn(|| {
            let probe = rc_read_probe(0);
            rc_read_probe_end(probe);
            RC_LOCAL.with(|r| r.spl_ctr.get())
        })
        .join()
        .expect("probe thread")
    };
    let (a, b) = (grab(), grab());
    RC_ENABLED.store(false, Ordering::Relaxed);
    let _ = rc_domain_take_and_reset();
    // Unseeded would leave exactly 1 on both; a constant seed would
    // leave them equal. Sampling depends only on the low 6 bits, so
    // compare PHASES, not whole counters (a seed source stepping by a
    // multiple of 64 would pass a whole-counter check while keeping
    // every thread phase-locked — 1R3 finding). Consecutive golden
    // steps differ by 21 mod 64, so distinct phases are guaranteed.
    assert_ne!(a, 1, "fresh thread ran unseeded at phase 0");
    assert_ne!(a, 0);
    assert_ne!(
        a & RC_SPL_MASK,
        b & RC_SPL_MASK,
        "two fresh threads drew the same low-6-bit sampling phase"
    );
}

// Integration: the probe must fire from read_bytes_scan ITSELF (1R2
// finding: probing rc_scan_probe directly leaves the wiring untested —
// deleting the probe calls inside read_bytes_scan must fail a test).
// Reads our own live buffer via the real RPM path.
#[test]
fn read_bytes_scan_integrates_scan_probe() {
    let _rc_counters = super::lock_rc_counters();
    RC_ENABLED.store(true, Ordering::Relaxed);
    let _ = rc_domain_take_and_reset();
    let src = vec![0xA5u8; 4096];
    let mut scratch: Vec<u8> = Vec::with_capacity(4096);
    {
        let _scope = RcDomainScope::new(218);
        let got = read_bytes_scan(src.as_ptr() as usize, src.len(), &mut scratch);
        assert!(
            matches!(got, Some(b) if b.len() == src.len() && b[0] == 0xA5),
            "the scan must return the whole buffer it was asked for"
        );
    }
    rc_flush_current_thread();
    RC_ENABLED.store(false, Ordering::Relaxed);
    let drained = rc_domain_take_and_reset();
    let d218 = drained
        .iter()
        .find(|(d, _)| *d == 218)
        .map(|(_, v)| *v)
        .expect("slot 218 drained");
    // Unsampled: a 4 KiB guarded scan is microseconds, three orders above
    // QPC's ~100 ns tick, so this cannot floor to zero.
    assert!(
        d218[RC_PD_SCAN_GUARDED_NS] > 0,
        "read_bytes_scan must charge its own scan probe"
    );
}

// Scan probes are unsampled: EVERY call accumulates. Draining after
// each call (1R review finding 3: a single aggregate >0 assert would
// pass even if 2 of 3 calls were dropped) proves per-call coverage.
#[test]
fn rc_scan_probe_accumulates_every_call() {
    let _rc_counters = super::lock_rc_counters();
    RC_ENABLED.store(true, Ordering::Relaxed);
    let _ = rc_domain_take_and_reset();
    for call in 0..3 {
        {
            let _scope = RcDomainScope::new(216);
            let probe = rc_scan_probe();
            assert!(probe.is_some(), "call {call}: gate is ON, probe must arm");
            let mut acc = 0u64;
            for i in 0..2_000u64 {
                acc = std::hint::black_box(acc.wrapping_add(i));
            }
            std::hint::black_box(acc);
            rc_scan_probe_end(probe);
        }
        rc_flush_current_thread();
        let drained = rc_domain_take_and_reset();
        let d216 = drained
            .iter()
            .find(|(d, _)| *d == 216)
            .map(|(_, v)| *v)
            .unwrap_or_else(|| panic!("call {call}: slot 216 drained"));
        assert!(
            d216[RC_PD_SCAN_GUARDED_NS] > 0,
            "call {call} accumulated no guarded scan time"
        );
    }
    RC_ENABLED.store(false, Ordering::Relaxed);
}

// Gate OFF is the production path: no probe, no counts, no timing —
// the primitives pay only the relaxed-load early-return.
#[test]
fn rc_probes_are_inert_when_gate_off() {
    let _rc_counters = super::lock_rc_counters();
    RC_ENABLED.store(false, Ordering::Relaxed);
    let _ = rc_domain_take_and_reset();
    {
        let _scope = RcDomainScope::new(217);
        for read in 0..70 {
            let probe = rc_read_probe(8);
            assert!(probe.is_none(), "read {read}: gate is OFF, probe must not arm");
            rc_read_probe_end(probe);
        }
        assert!(rc_scan_probe().is_none(), "gate is OFF, scan must not arm");
    }
    rc_flush_current_thread();
    // Gate OFF must accumulate nothing, not even read counts.
    assert!(
        rc_domain_take_and_reset().iter().all(|(d, _)| *d != 217),
        "slot 217 accumulated with the gate OFF"
    );
}
