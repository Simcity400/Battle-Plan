use super::*;

pub(super) fn item_body<'a>(source: &'a str, signature: &str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} must exist"));
    let end = source[start..]
        .find("\n}\n")
        .map(|at| start + at + 3)
        .unwrap_or_else(|| panic!("{signature} must close on a column-zero brace"));
    &source[start..end]
}

pub(super) fn code_only(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn d53_clock_bracket_calls_only_the_bounded_capture_function() {
    let source = super::collector_source_lf();
    let anchor = source
        .find("COMPACT_DOMAIN_DIPLOMACY_INTEL_OPERATIVES_LIVE,\n                move || {\n                    capture_compact_diplomacy_intel_operatives_live_snapshot(")
        .expect("d53 must put its snapshot capture in the bracket");
    let spawn = source[..anchor]
        .rfind("spawn_gated_finishing(")
        .expect("d53 must use the split finishing helper");
    let tail = &source[anchor..];
    let finish = tail
        .find("finish_compact_diplomacy_intel_operatives_live_snapshot,")
        .expect("d53 must name its snapshot decoder as the finish half");
    let next_spawn = tail[1..]
        .find("spawn_gated")
        .map(|at| at + 1)
        .unwrap_or(tail.len());
    assert!(spawn < anchor && finish < next_spawn);

    let capture = code_only(item_body(
        &source,
        "fn capture_compact_diplomacy_intel_operatives_live_snapshot<'t>(",
    ));
    assert!(capture.contains("plan.capture_with("));
    assert!(capture.contains("seh_read_exact("));
    for forbidden in [
        "Domain53InProcessMemory",
        "read_compact_diplomacy_intel_operatives_live_observe",
        "extract_rows_from_current_roots",
        "Vec::",
        "Box::",
        "Mutex",
        "Instant::",
        "read_into(",
        "read_bytes(",
    ] {
        assert!(
            !capture.contains(forbidden),
            "the bracket capture reached `{forbidden}`:\n{capture}"
        );
    }
}

#[test]
fn d53_allocation_and_walk_are_before_or_after_the_bracket() {
    let source = super::collector_source_lf();
    let arm = source
        .find("let snapshot_arm = arm_compact_diplomacy_intel_operatives_live_snapshot(")
        .expect("d53 must arm its preallocated plan");
    let spawn = source[arm..]
        .find("spawn_gated_finishing(")
        .map(|at| arm + at)
        .expect("d53 must spawn after arming");
    assert!(arm < spawn, "allocation and trace walk must precede the bracket");

    let finish = code_only(item_body(
        &source,
        "fn finish_compact_diplomacy_intel_operatives_live_snapshot(",
    ));
    assert!(finish.contains("let snapshot = plan.view();"));
    assert!(finish.contains("read_compact_diplomacy_intel_operatives_live_observe("));
    assert!(finish.contains("snapshot.first_miss().is_some()"));
    assert!(finish.contains("DOMAIN53_SNAPSHOT_REGION_MISS_CAUSE"));
    assert!(
        finish.contains("tracker.snapshot_plan = Some(plan);"),
        "a successful frame must reuse its preallocated plan instead of live-walking to arm every hour"
    );
    assert!(
        !finish.contains("Domain53InProcessMemory"),
        "the decoder must not regain a live engine-memory handle"
    );
}

#[test]
fn d53_snapshot_receipt_is_emitted_in_the_existing_perf_channel() {
    let source = super::collector_source_lf();
    let units = code_only(item_body(&source, "fn domain53_diag_units("));
    for name in [
        "domain53_bulk_snapshot_regions",
        "domain53_bulk_snapshot_bytes",
        "domain53_bulk_snapshot_copied_regions",
        "domain53_bulk_snapshot_copied_bytes",
        "domain53_bulk_snapshot_copy_refused",
        "domain53_bulk_snapshot_region_miss",
    ] {
        assert!(units.contains(name), "missing d53 snapshot receipt unit {name}");
    }
}

#[test]
fn a30_reads_only_through_the_selected_domain53_memory() {
    let source = super::collector_source_lf();
    let observing = code_only(item_body(
        &source,
        "fn read_a30_domain53_engine_authority<",
    ));
    for read in [".read_i32", ".read_u64", ".read_u32"] {
        assert!(observing.contains(read), "A30 must take `{read}` from the snapshot");
    }
    for live in [
        "read_i32_stable",
        "read_u64_stable",
        "read_u32_stable",
        "Domain53InProcessMemory",
    ] {
        assert!(
            !observing.contains(live),
            "A30 may not bypass snapshot custody through `{live}`"
        );
    }
}

#[test]
fn d53_direct_bulk_copy_bench_receipt() {
    const BYTES: usize = 32 * 1024 * 1024;
    let mut source = vec![0u8; BYTES];
    for (index, byte) in source.iter_mut().enumerate() {
        *byte = (index as u8).wrapping_mul(17).wrapping_add(3);
    }
    let address = source.as_ptr() as u64;
    let trace = diplomacy53::snapshot_memory::Domain53AccessTrace {
        reads: vec![diplomacy53::snapshot_memory::Domain53ReadTrace {
            address,
            len: BYTES,
            succeeded: true,
        }],
        ..diplomacy53::snapshot_memory::Domain53AccessTrace::default()
    };
    let mut plan = diplomacy53::snapshot_memory::Domain53SnapshotPlan::from_trace_with_limits(
        1,
        trace,
        0,
        1,
        BYTES,
    )
    .expect("one preallocated bulk region");
    let mut samples = Vec::new();
    for _ in 0..9 {
        let started = Instant::now();
        let receipt = plan.capture_with(|address, out| unsafe {
            seh_read_exact(address as usize, out.as_mut_ptr(), out.len())
        });
        samples.push(started.elapsed().as_nanos() as u64);
        assert_eq!(receipt.failure_region_ordinal, None);
        assert_eq!(receipt.copied_byte_count, BYTES as u64);
    }
    samples.sort_unstable();
    let copied = diplomacy53::extractor::Domain53Memory::read_bytes(&plan.view(), address, BYTES)
        .expect("captured image");
    assert_eq!(copied[0], source[0]);
    assert_eq!(copied[BYTES - 1], source[BYTES - 1]);
    eprintln!(
        "D53_BULK_COPY_BENCH bytes={BYTES} regions=1 samples=9 p50_ns={} p95_ns={}",
        samples[4], samples[8]
    );
}
