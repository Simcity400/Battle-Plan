//! Lane RZ's proof that cutting d41's worker changed no recorded byte.
//!
//! Six tests. TWO are value-level, over the two structural changes (the
//! frame-shared pool buffer, the `SpecialProjectsCache::rows` split). FOUR are
//! source-scanning pins, for properties no returned value can carry: a hint
//! must stay a hint, every walk exit must stamp its sub-marks, the d47
//! line-hint spelling must stay a delegation, and `rows()` must keep one
//! status predicate and one call order.

use super::*;

// ------------------------------------------------------------------
// 1. The frame-shared pool buffer
// ------------------------------------------------------------------

/// The bulk arm now writes into a buffer the WHOLE FRAME shares, so the
/// question a test has to answer is not "does it decode" but "can a country
/// see the country before it".
///
/// The fixture answers it the only way that is not an inspection: the same
/// buffer takes a FOUR-project pool first and a ONE-project pool second, so
/// the wide pool's records are still sitting under the narrow one's tail, and
/// the narrow decode must equal a decode of that same narrow pool through a
/// never-used buffer — and must equal the scalar arm, which has no buffer at
/// all. Compared as rows AND as the d41 payload bytes those rows build.
///
/// The second assertion is the sharper one, and it is why the decode was split
/// out of `read_country_project_lifecycle_from_bytes` into
/// `decode_country_project_lifecycle_from_bytes`: the SAME dirty buffer is
/// handed to the decode twice, once cut to `byte_len` and once whole, and the
/// two must agree. That is the property a frame-shared buffer actually rests
/// on — the decode is bounded by `count`, never by how much buffer it was
/// given.
///
/// KILLS, stated exactly rather than generously:
/// * any future "the buffer already holds this pool, skip the `read_into`"
///   fast path — the narrow decode then returns the wide pool's `local_id` 10
///   instead of 99 and the first assertion fails;
/// * a decode that derives its project count from `bytes.len() /
///   PROJECT_COUNTRY_STRIDE` instead of from `count` — the whole-buffer arm
///   then walks all 64 stride slots instead of one. It does NOT emit 64 rows
///   of the previous country: only slots 0..4 were ever written, so slot 4
///   fails the vtable check and the country aborts, and `lifecycle_rows`
///   panics on `CountryLifecycle::Aborted`. The test still fails, and fails
///   loudly, but by panic and not by row mismatch — stated exactly because an
///   earlier version of this comment advertised the wrong failure (Codex
///   review 01a03189-575f-7c83-90bf-09eb0f5af6dc, NIT 1). Passing
///   `&pool_scratch[..]` on its own is NOT killed by the first assertion (the
///   decode still reads only record 0), which is precisely why the second one
///   exists.
#[test]
fn a_shared_pool_buffer_cannot_leak_one_country_into_the_next() {
    let _d41_diagnostics = lock_domain41_diagnostics();
    page_cache_new_frame();
    let project_vtable = HEAP_LO + 0x3000;
    let state_machine_vtable = HEAP_LO + 0x4000;
    let prototype_vtable = HEAP_LO + 0x5000;
    let simple_vtable = HEAP_LO + 0x6000;

    let mut states = Vec::new();
    for _ in 0..5 {
        let mut state = AlignedBytes::<PROJECT_STATE_MACHINE_SNAPSHOT_LEN>::new();
        write_project_state_machine(
            &mut state,
            state_machine_vtable,
            prototype_vtable,
            simple_vtable,
        );
        states.push(state);
    }

    let mut wide = AlignedBytes::<PROJECT_POOL_ARRAY_BULK_LEN>::new();
    let mut narrow = AlignedBytes::<PROJECT_POOL_ARRAY_BULK_LEN>::new();
    assert!(is_map_control_heap_pointer(wide.addr()));
    assert!(is_map_control_heap_pointer(narrow.addr()));
    for index in 0..4usize {
        write_project_pool_record(
            &mut wide,
            index,
            project_vtable,
            (index as u32) + 10,
            (index as u32) + 5000,
            index % 2 == 0,
            index % 3 == 0,
        );
        wide.write_u64(
            index * PROJECT_COUNTRY_STRIDE + PROJECT_PROJECT_OFF_STATE_MACHINE_PTR,
            states[index].addr() as u64,
        );
    }
    write_project_pool_record(&mut narrow, 0, project_vtable, 99, 7777, true, true);
    narrow.write_u64(
        PROJECT_PROJECT_OFF_STATE_MACHINE_PTR,
        states[4].addr() as u64,
    );

    let mut shared = [0u8; PROJECT_POOL_ARRAY_BULK_LEN];
    let mut walk = Domain41LifecycleWalkStats::default();
    let wide_rows = lifecycle_rows(
        read_country_project_lifecycle_bulk(
            wide.addr(),
            4,
            project_vtable,
            state_machine_vtable,
            prototype_vtable,
            simple_vtable,
            None,
            &mut shared,
            &mut walk,
        )
        .expect("contiguous four-project bulk read accepted"),
        47,
    );
    assert_eq!(
        wide_rows.len(),
        4,
        "the wide pool must actually fill the buffer past the narrow pool's end"
    );
    let narrow_after_wide = lifecycle_rows(
        read_country_project_lifecycle_bulk(
            narrow.addr(),
            1,
            project_vtable,
            state_machine_vtable,
            prototype_vtable,
            simple_vtable,
            None,
            &mut shared,
            &mut walk,
        )
        .expect("contiguous one-project bulk read accepted"),
        47,
    );

    let mut fresh = [0u8; PROJECT_POOL_ARRAY_BULK_LEN];
    let mut fresh_walk = Domain41LifecycleWalkStats::default();
    let narrow_alone = lifecycle_rows(
        read_country_project_lifecycle_bulk(
            narrow.addr(),
            1,
            project_vtable,
            state_machine_vtable,
            prototype_vtable,
            simple_vtable,
            None,
            &mut fresh,
            &mut fresh_walk,
        )
        .expect("clean-buffer bulk read accepted"),
        47,
    );

    let mut scalar_walk = Domain41LifecycleWalkStats::default();
    let narrow_scalar = lifecycle_rows(
        read_country_project_lifecycle_scalar(
            narrow.addr(),
            1,
            project_vtable,
            state_machine_vtable,
            prototype_vtable,
            simple_vtable,
            None,
            &mut scalar_walk,
        ),
        47,
    );

    assert_eq!(
        narrow_after_wide, narrow_alone,
        "a pool decoded through a dirty shared buffer must equal the same pool \
         decoded through a clean one"
    );
    assert_eq!(
        narrow_after_wide, narrow_scalar,
        "and must equal the arm that has no buffer at all"
    );

    // The decode must be bounded by `count`, not by the buffer it was handed.
    // Same dirty buffer, same count, two slice lengths. A length-derived
    // decoder would walk all 64 stride slots and abort on the first unwritten
    // one, panicking in `lifecycle_rows`; see this test's doc comment.
    let cut = lifecycle_rows(
        decode_country_project_lifecycle_from_bytes(
            narrow.addr(),
            1,
            project_vtable,
            state_machine_vtable,
            prototype_vtable,
            simple_vtable,
            None,
            &shared[..PROJECT_LIFECYCLE_PROJECT_SNAPSHOT_LEN],
        ),
        47,
    );
    let whole = lifecycle_rows(
        decode_country_project_lifecycle_from_bytes(
            narrow.addr(),
            1,
            project_vtable,
            state_machine_vtable,
            prototype_vtable,
            simple_vtable,
            None,
            &shared[..],
        ),
        47,
    );
    assert_eq!(
        cut, whole,
        "the decode is bounded by `count`; handing it the whole 28,672-byte          buffer instead of the `byte_len` prefix must change nothing"
    );
    assert_eq!(
        cut, narrow_alone,
        "and both must agree with the production bulk arm's own result"
    );

    let dirty_payload = compact_projects_data_payload(
        &compact_projects_read_with_lifecycle(narrow_after_wide),
        96,
    );
    let clean_payload =
        compact_projects_data_payload(&compact_projects_read_with_lifecycle(narrow_alone), 96);
    assert_eq!(
        dirty_payload, clean_payload,
        "the d41 payload built from the two must be byte-identical"
    );

    assert_eq!(
        walk.bulk_fallback_countries, 0,
        "both bulk reads were accepted, so no fallback may be counted (only \
         `read_country_project_lifecycle` increments it, and this test drives \
         the two arms directly)"
    );
    assert!(
        walk.projects_ns > 0,
        "the per-project sub-mark must accumulate across BOTH calls sharing \
         this stats value; a zero means the timing wrapper was bypassed"
    );
}

// ------------------------------------------------------------------
// 2. The `SpecialProjectsCache::rows` split
// ------------------------------------------------------------------

/// `rows()` was one function with three loops; the compact reader now calls
/// two of them and never enters the third. The composition must not have moved
/// a row, a cap or a status verdict.
///
/// NON-VACUOUS BY CONSTRUCTION (Codex review 01a03142, SHOULD-FIX 1). An
/// earlier form of this test ran over an EMPTY cache, where all three
/// accessors return empty for every status and the assertions hold whatever
/// the bodies do. This one stages a real `CProject` and a real `ProjectPin`,
/// so the middle loop -- the one the compact path stops calling, and the whole
/// point of the split -- actually materialises a row and the status predicate
/// actually decides something.
///
/// KILLS: a `rows_materialisable` copy that drifts in `legacy_project_rows`
/// (drop `"country_project_pool_partial"` and that status returns 0 rows here
/// while the composition still expects 1); a `rows()` that reorders its tuple;
/// a `legacy_project_rows` that loses its loop.
#[test]
fn the_row_accessors_compose_to_exactly_what_rows_returned() {
    let _d41_diagnostics = lock_domain41_diagnostics();
    page_cache_new_frame();
    let base = HEAP_LO;
    let project_vtable = base + PROJECT_CPROJECT_VTABLE_RVA;
    let mut project = AlignedBytes::<PROJECT_COUNTRY_STRIDE>::new();
    assert!(is_map_control_heap_pointer(project.addr()));
    write_project_pool_record(&mut project, 0, project_vtable, 77, 9001, true, false);
    project.write_u32(PROJECT_PROJECT_OFF_OWNER_TAG, PROJECT_FACILITY_TYPE);
    project.write_u32(PROJECT_PROJECT_OFF_OWNER_FACILITY, 5);
    project.write_u32(PROJECT_PROJECT_OFF_ITERATIONS, 3);
    let pin = ProjectPin {
        addr: project.addr(),
        project_id: 77,
        static_def_handle: 9001,
    };

    for status in [
        "verified",
        "bridge_verified",
        "country_project_pool_verified",
        "country_project_pool_partial",
        "too_many",
        "not_attempted",
    ] {
        for too_many in [false, true] {
            let cache = SpecialProjectsCache {
                status,
                scan_ms: 0,
                bridge_rows_supplied: 0,
                facilities: Vec::new(),
                projects: vec![pin],
                scientists: Vec::new(),
                too_many,
                invalid_program_status_countries: 0,
                invalid_facility_countries: 0,
                invalid_project_pool_countries: 0,
                neutral_program_status_countries: 0,
                neutral_facility_countries: 0,
                neutral_project_pool_countries: 0,
                root_reasons: ProjectRootReasonCounters::default(),
            };
            let admissible = matches!(
                status,
                "verified"
                    | "bridge_verified"
                    | "country_project_pool_verified"
                    | "country_project_pool_partial"
            ) && !too_many;

            let composed = cache.rows(base);
            let facilities = cache.facility_rows(base, &mut Domain41MaterialiseLoss::default());
            let projects = cache.legacy_project_rows(base);
            let scientists = cache.scientist_rows(base, &mut Domain41MaterialiseLoss::default());

            assert_eq!(
                projects.len(),
                usize::from(admissible),
                "the legacy loop must materialise the staged pin exactly when \
                 the cache is materialisable ({status}/too_many={too_many})"
            );
            if admissible {
                assert_eq!(projects[0].id, 77);
                assert_eq!(projects[0].static_def_handle, 9001);
                assert_eq!(projects[0].worked_at_facility, Some(5));
                assert_eq!(projects[0].iterations, 3);
            }
            assert!(
                facilities.is_empty() && scientists.is_empty(),
                "no facility or scientist pin is staged here"
            );
            assert!(
                composed.0 == facilities,
                "facility rows for {status}/too_many={too_many}"
            );
            assert!(
                composed.1 == projects,
                "legacy project rows for {status}/too_many={too_many}"
            );
            assert!(
                composed.2 == scientists,
                "scientist rows for {status}/too_many={too_many}"
            );
        }
    }
}

// ------------------------------------------------------------------
// 3. Source pins for the properties no returned value can carry
// ------------------------------------------------------------------

// `pub(super)` so lane TB's source pin uses THESE and does not become a third
// copy of the same two helpers (there is already one in
// `attach_log/domain_scoped_reset_tests.rs`).
pub(super) fn attach_log_source() -> String {
    // Normalised to LF: the working copy is CRLF, and the multi-line signature
    // one of the pins below searches for would never match otherwise.
    std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/attach_log.rs"
    ))
    .expect("attach_log.rs is readable from the collector crate root")
    .replace("\r\n", "\n")
}

/// The text between the braces of the first item whose header is `signature`.
pub(super) fn function_body<'a>(source: &'a str, signature: &'a str) -> &'a str {
    let start = source
        .find(signature)
        .unwrap_or_else(|| panic!("{signature} must exist in attach_log.rs"));
    let after = &source[start..];
    let open = after.find('{').expect("function body opens");
    let mut depth = 0usize;
    for (index, byte) in after.as_bytes().iter().copied().enumerate().skip(open) {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return &after[open + 1..index];
                }
            }
            _ => {}
        }
    }
    panic!("{signature} body never closes");
}

/// A hint must stay a hint.
///
/// The whole byte-identity argument for lane RZ's prefetching is that
/// `PREFETCHh` issues no read: it does not enter `range_confirmed_good`, does
/// not move a read counter, and is not seen by `domain47_witness_note`, whose
/// output d47 replays. The tempting edit — "warm the object with a real
/// `read_u64` instead" — breaks all three silently, because these helpers
/// return nothing and no assertion over a result can exist.
///
/// KILLS: replacing a `prefetch_read` / `prefetch_object_lines` call in any of
/// the three with `read_u64`, `read_u32`, `read_u16`, `read_u8`, `read_scalar`,
/// `read_into`, `read_bytes` or an `ObjectSnapshot` read.
#[test]
fn the_d41_prefetch_helpers_issue_no_guarded_read() {
    let source = attach_log_source();
    for signature in [
        "fn prefetch_project_country_root(rows: &[CountryObjectRow], index: usize)",
        "fn prefetch_project_state_machine(bytes: &[u8], index: usize)",
        "fn prefetch_object_lines(addr: usize, span: usize)",
    ] {
        let body = function_body(&source, signature);
        for forbidden in [
            "read_u64(",
            "read_u32(",
            "read_u16(",
            "read_u8(",
            "read_scalar",
            "read_into(",
            "read_bytes(",
            "ObjectSnapshot",
        ] {
            assert!(
                !body.contains(forbidden),
                "{signature} must issue no guarded read; found {forbidden}"
            );
        }
        assert!(
            body.contains("prefetch_read(") || body.contains("prefetch_object_lines("),
            "{signature} must actually hint something"
        );
    }
    // `read_u64_from_slice` reads the caller's ALREADY-COPIED bytes and is not
    // a guarded read; the state-machine hint is allowed exactly that one, and
    // required to use it — taking the address out of the process instead would
    // be the very read this rule forbids.
    let state_machine = function_body(
        &source,
        "fn prefetch_project_state_machine(bytes: &[u8], index: usize)",
    );
    assert!(
        state_machine.contains("read_u64_from_slice(bytes, field)"),
        "the state-machine hint must take its address out of the pool COPY"
    );
}

/// Every exit of the lifecycle walk must stamp the sub-marks.
///
/// `read_project_lifecycle_rows` returns from five places: two guard clauses
/// before the walk begins (where `out` is freshly zeroed and the walk never
/// ran) and three truncation caps inside the country loop, plus the tail
/// expression. A `return out;` added back inside the loop would publish
/// `d41_phase_lifecycle_decode_us = 0` on exactly the frames that did the
/// most work, and would change no payload byte, so no value-level test can see
/// it. A whitelist over the source can.
///
/// KILLS: adding a bare `return out;` to the loop, or a new early exit that
/// forgets the stamp.
#[test]
fn the_lifecycle_walk_stamps_its_sub_marks_on_every_exit() {
    let source = attach_log_source();
    let body = function_body(
        &source,
        "fn read_project_lifecycle_rows(state: &CollectorState) -> ProjectLifecycleRead",
    );
    let mut pre_walk_guards = 0usize;
    let mut stamped = 0usize;
    for line in body.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("return") {
            continue;
        }
        if trimmed == "return out;" {
            pre_walk_guards += 1;
        } else if trimmed.starts_with("return stamp_project_lifecycle_walk(") {
            stamped += 1;
        } else {
            panic!("unstamped return in read_project_lifecycle_rows: {trimmed}");
        }
    }
    assert_eq!(
        pre_walk_guards, 2,
        "only the two guard clauses that run BEFORE the walk may return `out` \
         bare; a third means a walk exit lost its stamp"
    );
    assert_eq!(
        stamped, 3,
        "the three truncation caps must all stamp"
    );
    assert!(
        body.trim_end()
            .ends_with("stamp_project_lifecycle_walk(out, walk, sort_ns)"),
        "the tail must stamp with the measured sort span"
    );
}

/// The d47 spelling of the line-hint helper must stay a delegation.
///
/// Lane RZ promoted the body to `prefetch_object_lines` and left
/// `domain47_prefetch_object` as a one-line call, so d47's call sites and its
/// cost bench are untouched and the two cannot come apart. A copy of the body
/// pasted back would pass every other test in the tree and would then be free
/// to drift from the alignment reasoning documented on the shared function.
#[test]
fn the_domain47_line_hint_is_a_delegation_and_not_a_copy() {
    let source = attach_log_source();
    let body = function_body(
        &source,
        "fn domain47_prefetch_object(addr: usize, span: usize)",
    );
    assert_eq!(
        body.trim(),
        "prefetch_object_lines(addr, span);",
        "domain47_prefetch_object must be exactly a delegation"
    );
}

/// `rows()` must stay the same three calls in the same order, and the status
/// verdict must live in ONE place.
///
/// The value test above can only exercise the loop it stages a pin for; these
/// two properties are about the other two loops and about the shape of the
/// split, and a source pin is the only thing that reaches them. The order
/// matters because Rust evaluates tuple elements left to right and that
/// ordering IS the guarded-read order the raw-JSON caller had before the
/// split; the single predicate matters because three copies of a status list
/// are three chances to drift.
///
/// KILLS: reordering the tuple in `rows()`; inlining the `matches!(self.status,
/// ..)` list back into any accessor; an accessor that forgets to gate at all.
#[test]
fn the_rows_split_keeps_one_predicate_and_one_order() {
    let source = attach_log_source();

    let rows = function_body(
        &source,
        "fn rows(\n        &self,\n        base: usize,\n    ) -> (",
    );
    let calls: Vec<&str> = rows
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("self."))
        .collect();
    assert_eq!(
        calls,
        vec![
            "self.facility_rows(base, &mut Domain41MaterialiseLoss::default()),",
            "self.legacy_project_rows(base),",
            "self.scientist_rows(base, &mut Domain41MaterialiseLoss::default()),",
        ],
        "rows() must be exactly the three accessors in the original \
         guarded-read order"
    );

    for signature in [
        "fn facility_rows(
        &self,
        base: usize,
        loss: &mut Domain41MaterialiseLoss,
    ) -> Vec<ProjectFacilityRow>",
        "fn legacy_project_rows(&self, base: usize) -> Vec<ProjectRow>",
        "fn scientist_rows(
        &self,
        base: usize,
        loss: &mut Domain41MaterialiseLoss,
    ) -> Vec<ProjectScientistRow>",
    ] {
        let body = function_body(&source, signature);
        assert_eq!(
            body.matches("self.rows_materialisable()").count(),
            1,
            "{signature} must gate on the shared predicate exactly once"
        );
        assert!(
            !body.contains("matches!("),
            "{signature} must not carry its own copy of the status list"
        );
    }
}
