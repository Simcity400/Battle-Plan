use super::air_bases_test_kit as kit;
use super::*;

const CAPS: [usize; 5] = [8, 8, 256, 16, 64];

#[test]
fn daily_aces_are_excluded_from_hourly_memo_inputs_and_outputs() {
    struct Guard<'a> {
        inner: kit::SyntheticMemory<'a>,
        aces_active: bool,
        census_reads: std::cell::Cell<usize>,
    }
    impl Memory for Guard<'_> {
        fn copy(&self, address: usize, bytes: &mut [u8]) -> bool {
            if address == kit::GAME_STATE + cgame_state::COUNTRIES {
                self.census_reads.set(self.census_reads.get() + 1);
            }
            assert!(self.aces_active || address != kit::C0 + ccountry::ACES);
            self.inner.copy(address, bytes)
        }
    }
    let bytes = kit::fixture();
    let mut memo = AirBasesMemo::default();
    let mut job = AirBasesJob::fixture();
    for (index, hour) in (20..24).enumerate() {
        let active = hour == 21;
        let memory = Guard {
            inner: kit::memory(&bytes),
            aces_active: active,
            census_reads: std::cell::Cell::new(0),
        };
        let copy = copy_air_bases_fixture(
            &memory,
            AirBasesCopyContext {
                base: kit::BASE,
                game_state: kit::GAME_STATE,
                hour,
                force_full: false,
            },
            &mut job,
        );
        assert!(copy.raw_ok);
        assert_eq!(memory.census_reads.get(), 1);
        if hour == 23 {
            job.log.fault[2] = true;
        }
        let private = if active {
            [8, 256]
        } else if hour == 23 {
            [64, 1024]
        } else {
            [0, 0]
        };
        let (mut air, mut fog) = parts([8, private[0], private[1], 16, 64]);
        air.prepare_hour(hour, false);
        let mut ticks = [0; AIR_BASES_N];
        let report = memo.decode(
            &mut air,
            &mut fog,
            &job,
            AirBasesMemoContext {
                hour,
                ..context(index as u64 + 1)
            },
            &mut ticks,
        );
        assert!(report.eligible);
        assert_eq!(report.hit, hour == 23);
        assert!(matches!(air.bases.outcome, Outcome::Rows { .. }));
        assert!(fog.registrations_memo_complete());
        assert_eq!(
            air.aces.outcome,
            if active {
                Outcome::Rows { len: 2 }
            } else {
                SKIPPED
            }
        );
        if !active {
            assert_eq!((copy.copy_ticks[2], ticks[2]), (0, 0));
            assert_eq!(job.log.fault[2], hour == 23);
            assert_eq!((air.names.len, air.names.required), (0, 0));
            air.fail_air_bases_capture("worker failed");
            assert_eq!(air.aces.outcome, SKIPPED);
        }
    }
}

fn context(seq: u64) -> AirBasesMemoContext {
    AirBasesMemoContext {
        owner: 17,
        base: kit::BASE,
        seq,
        hour: 100 + seq as i32,
        force_full: false,
    }
}

fn parts(cap: [usize; 5]) -> (Captured, super::super::fog::Captured) {
    let mut air = Captured::with_capacities(&Capacities {
        rows: [0, cap[0], 0, cap[1], 0, 4096, 1024, 64, 0],
        ace_names: cap[2],
        wings: 8192,
        sorties: Default::default(),
        scores: Default::default(),
    });
    let mut fog = super::super::fog::Captured::with_capacities(&super::super::fog::Capacities {
        rows: [0, 0, 0, 0, 0, cap[3], 0],
        radar_provinces: 0,
        area_provinces: 0,
        wings: cap[4],
    });
    air.bases_raw_mode = true;
    fog.registrations_raw_mode = true;
    (air, fog)
}

fn stage(job: &mut AirBasesJob, bytes: &[u8]) {
    copy_air_bases_pass(
        job,
        &kit::memory(bytes),
        kit::BASE,
        kit::GAME_STATE,
        true,
        &mut [0; 3],
    );
    assert!(!job.log.overflowed);
    assert_eq!(job.log.fault, [false; 3]);
}

fn run(
    memo: &mut AirBasesMemo,
    job: &AirBasesJob,
    ctx: AirBasesMemoContext,
    capacities: [usize; 5],
) -> (Captured, super::super::fog::Captured, AirBasesMemoReport) {
    let (mut air, mut fog) = parts(capacities);
    let report = memo.decode(&mut air, &mut fog, job, ctx, &mut [0; 3]);
    (air, fog, report)
}

fn assert_rows(air: &Captured, fog: &super::super::fog::Captured, expected: &kit::Split) {
    assert_eq!(air.bases.outcome, expected.bases_outcome);
    assert_eq!(
        &air.bases.rows[..air.bases.outcome.len()],
        &expected.bases[..expected.bases_outcome.len()]
    );
    assert_eq!(air.aces.outcome, expected.aces_outcome);
    assert_eq!(
        &air.aces.rows[..air.aces.outcome.len()],
        &expected.aces[..expected.aces_outcome.len()]
    );
    assert_eq!(air.names.len, expected.ace_bytes_len);
    assert_eq!(air.names.required, expected.ace_required);
    assert_eq!(
        &air.names.first[..air.names.len],
        &expected.ace_names[..expected.ace_bytes_len]
    );
    let regs = fog.registrations_snapshot();
    assert_eq!(regs.outcome, expected.regs.outcome);
    assert_eq!(regs.rows, expected.regs.rows);
    assert_eq!(regs.wings, expected.regs.wings);
}

#[test]
fn memo_ace_presentation_heap_bytes_match_raw_and_typed_with_local_failure() {
    use engine_native::{cace, cpdx_array, msvc_string};
    const GUI: usize = 0x74000;
    const DEFINITION: usize = 0x75000;
    const DEFINITION_BYTES: usize = 0x75100;
    const SPRITE: usize = 0x76000;
    const SPRITE_BYTES: usize = 0x76100;
    const POOLS: usize = 0x77000;
    let mut bytes = kit::fixture();
    let array = |bytes: &mut [u8], at, data, count| {
        kit::put64(bytes, at, data as u64);
        kit::put32(bytes, at + 8, count);
        kit::put32(bytes, at + 12, count);
    };
    let heap = |bytes: &mut [u8], head, data, text: &[u8]| {
        kit::put64(bytes, head, data as u64);
        kit::put64(bytes, head + msvc_string::LENGTH, text.len() as u64);
        kit::put64(bytes, head + msvc_string::CAPACITY, text.len() as u64);
        bytes[data..data + text.len()].copy_from_slice(text);
        bytes[data + text.len()] = 0;
    };
    kit::put64(&mut bytes, kit::GUI_SINGLETON, GUI as u64);
    kit::put64(
        &mut bytes,
        GUI,
        engine_native::cgame_gui::VTABLE_RVA.0 as u64,
    );
    array(&mut bytes, GUI + 0xcb0, POOLS, 3);
    array(&mut bytes, POOLS + cpdx_array::STRIDE, SPRITE, 1);
    heap(
        &mut bytes,
        DEFINITION + cace::TYPE_NAME,
        DEFINITION_BYTES,
        b"authored selected ace definition",
    );
    heap(
        &mut bytes,
        SPRITE,
        SPRITE_BYTES,
        b"GFX_mod_retained_ace_portrait",
    );
    kit::put64(
        &mut bytes,
        kit::A0 + cace::TYPE_DEFINITION,
        DEFINITION as u64,
    );
    bytes[kit::A0 + cace::PORTRAIT_GENDER] = 7;
    kit::put32(&mut bytes, kit::A0 + cace::PORTRAIT_POOL_INDEX, 0);

    let direct = kit::direct(&kit::memory(&bytes));
    let (split, _, job) = kit::split(&kit::memory(&bytes));
    kit::assert_matches_direct(&split, &direct);
    assert_eq!(job.log.fault, [false; 3]);
    let fields = direct.aces[0].presentation.unwrap();
    assert_eq!((fields.gender, fields.pool_index), (7, 0));
    let text = |range: cace::TextRange| &direct.ace_names[range.start..range.start + range.len];
    assert_eq!(
        text(fields.definition.unwrap()),
        b"authored selected ace definition"
    );
    let portrait = fields.portrait.unwrap();
    assert_eq!(
        text(portrait.cached.unwrap()),
        b"GFX_mod_retained_ace_portrait"
    );
    assert_eq!((portrait.culture, portrait.fallback), (None, None));

    let mut memo = AirBasesMemo::default();
    let mut job = AirBasesJob::fixture();
    stage(&mut job, &bytes);
    let (air, fog, report) = run(&mut memo, &job, context(1), CAPS);
    assert!(report.eligible && report.installed && !report.hit);
    assert_rows(&air, &fog, &direct);
    stage(&mut job, &bytes);
    let (air, fog, report) = run(&mut memo, &job, context(2), CAPS);
    assert!(report.compared && report.hit && !report.installed);
    assert_rows(&air, &fog, &direct);

    for (seq, address) in [(3, DEFINITION_BYTES), (4, SPRITE_BYTES)] {
        bytes[address] = bytes[address].wrapping_add(1);
        stage(&mut job, &bytes);
        let direct = kit::direct(&kit::memory(&bytes));
        let (air, fog, report) = run(&mut memo, &job, context(seq), CAPS);
        assert!(report.compared && !report.hit && report.installed);
        assert_rows(&air, &fog, &direct);
    }

    struct Fault<'a> {
        inner: kit::SyntheticMemory<'a>,
    }
    impl Memory for Fault<'_> {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            if address <= DEFINITION_BYTES && DEFINITION_BYTES - address < output.len() {
                return false;
            }
            self.inner.copy(address, output)
        }
    }
    let memory = Fault {
        inner: kit::memory(&bytes),
    };
    let failed = kit::direct(&memory);
    let (split, _, job) = kit::split(&memory);
    kit::assert_matches_direct(&split, &failed);
    assert_eq!(failed.aces_outcome, Outcome::Rows { len: 2 });
    let mut prefix = direct.aces[0];
    prefix.presentation = None;
    assert_eq!(failed.aces[0], prefix);
    assert_eq!(&failed.ace_names[..6], b"AlBoCy");
    assert_eq!(job.log.fault, [false, false, true]);
    let (air, fog, report) = run(&mut memo, &job, context(5), CAPS);
    assert!(!report.eligible && !report.hit && !report.installed);
    assert!(memo.key.is_none() && memo.previous.is_none());
    assert_rows(&air, &fog, &failed);
}

#[test]
fn memo_compares_every_span_field_order_alias_and_only_used_bytes() {
    use collector_raw::raw::Span;
    let mut log = RawLog::<3>::new(4, 64);
    log.bytes.extend_from_slice(&[10, 11, 12, 13, 20, 21]);
    log.spans.extend([
        Span {
            addr: 0x1000,
            len: 4,
            ok: true,
            mask: 1,
            start: 0,
            bulk: true,
        },
        Span {
            addr: 0x1001,
            len: 1,
            ok: true,
            mask: 2,
            start: 4,
            bulk: false,
        },
        Span {
            addr: 0x1001,
            len: 1,
            ok: true,
            mask: 4,
            start: 5,
            bulk: false,
        },
    ]);
    log.has_bulk = true;
    let mut memo = AirBasesMemo::default();
    memo.spans.clone_from(&log.spans);
    memo.bytes.extend_from_slice(log.bytes.as_ref());
    memo.has_bulk = true;
    assert!(memo.input_equal(&log));
    let edits: [fn(&mut Span); 6] = [
        |span| span.addr += 1,
        |span| span.len += 1,
        |span| span.ok = false,
        |span| span.mask ^= 4,
        |span| span.start += 1,
        |span| span.bulk = !span.bulk,
    ];
    for edit in edits {
        let old = log.spans[0];
        edit(&mut log.spans[0]);
        assert!(!memo.input_equal(&log));
        log.spans[0] = old;
    }
    log.spans.swap(1, 2);
    assert!(!memo.input_equal(&log));
    log.spans.swap(1, 2);
    for index in 0..log.bytes.len() {
        log.bytes[index] ^= 1;
        assert!(!memo.input_equal(&log), "byte {index}");
        log.bytes[index] ^= 1;
    }
    log.has_bulk = false;
    assert!(!memo.input_equal(&log));
    log.has_bulk = true;
    for slot in 0..3 {
        log.fault[slot] = true;
        assert!(!memo.input_equal(&log));
        log.fault[slot] = false;
    }
    log.overflowed = true;
    assert!(!memo.input_equal(&log));
    log.overflowed = false;
    let spans = log.spans.clone();
    log.clear();
    log.bytes.extend_from_slice(&[99; 32]);
    log.clear();
    log.bytes.extend_from_slice(&memo.bytes);
    log.spans.extend_from_slice(&spans);
    log.has_bulk = true;
    assert!(memo.input_equal(&log));
    log.spans.pop();
    assert!(!memo.input_equal(&log));
}

#[test]
fn memo_reuses_complete_current_capture_in_independently_owned_frame() {
    let mut bytes = kit::fixture();
    kit::add_alias_and_rocket(&mut bytes);
    let expected = kit::direct(&kit::memory(&bytes));
    let mut job = AirBasesJob::fixture();
    stage(&mut job, &bytes);
    let mut memo = AirBasesMemo::default();
    let (mut first, first_fog, report) = run(&mut memo, &job, context(1), CAPS);
    assert!(!report.hit && report.installed && report.eligible);
    assert_rows(&first, &first_fog, &expected);
    first.bases.rows.fill(cair_base::Row::default());
    first.aces.rows.fill(cace::Row::default());
    first.names.first.fill(0);
    drop(first);
    drop(first_fog);
    stage(&mut job, &bytes);
    let (mut air, mut fog) = parts(CAPS);
    let report = memo.decode(&mut air, &mut fog, &job, context(2), &mut [0; 3]);
    assert!(report.hit && report.compared && !report.installed);
    assert_rows(&air, &fog, &expected);
    assert!(!air.bases_raw_mode && !fog.registrations_raw_mode);
}

#[test]
fn memo_changed_fresh_leaf_misses_and_installs_new_complete_result() {
    let mut bytes = kit::fixture();
    let mut job = AirBasesJob::fixture();
    stage(&mut job, &bytes);
    let mut memo = AirBasesMemo::default();
    assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
    kit::put32(&mut bytes, kit::B0 + cair_base::CAPACITY, 123);
    stage(&mut job, &bytes);
    let (air, fog, report) = run(&mut memo, &job, context(2), CAPS);
    assert!(report.compared && !report.hit && report.installed);
    assert_rows(&air, &fog, &kit::direct(&kit::memory(&bytes)));
    assert!(run(&mut memo, &job, context(3), CAPS).2.hit);
}

#[test]
fn memo_declines_current_fault_and_admission_states() {
    let bytes = kit::fixture();
    let mut job = AirBasesJob::fixture();
    for case in [0, 1, 2, 9] {
        stage(&mut job, &bytes);
        let mut memo = AirBasesMemo::default();
        assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
        let (mut air, mut fog) = parts(CAPS);
        match case {
            0 => job.log.overflowed = true,
            1 => job.log.fault[0] = true,
            2 => job.log.spans[0].ok = false,
            9 => air.bases_raw_mode = false,
            _ => unreachable!(),
        }
        let report = memo.decode(&mut air, &mut fog, &job, context(2), &mut [0; 3]);
        assert!(
            !report.hit && !report.installed && !report.eligible,
            "case {case}"
        );
        assert!(memo.key.is_none() && memo.previous.is_none());
    }
}

#[test]
fn memo_context_capacity_branch_and_missing_refresh_never_reuse_old_entry() {
    let bytes = kit::fixture();
    let mut job = AirBasesJob::fixture();
    stage(&mut job, &bytes);
    for case in 0..11 {
        let mut memo = AirBasesMemo::default();
        assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
        let mut ctx = context(2);
        let mut cap = CAPS;
        match case {
            0 => ctx.owner += 1,
            1 => ctx.base += 1,
            2 => ctx.force_full = true,
            3 => ctx.hour += 1,
            4 => ctx.seq += 1,
            5..=9 => cap[case - 5] += 1,
            10 => memo.invalidate(),
            _ => unreachable!(),
        }
        assert!(!run(&mut memo, &job, ctx, cap).2.hit, "case {case}");
    }
    let mut memo = AirBasesMemo::default();
    assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
    job.game_state += 8;
    assert!(!run(&mut memo, &job, context(2), CAPS).2.hit);
}

#[test]
fn memo_grows_registrations_before_keying_and_restoring_another_small_frame() {
    let bytes = kit::fixture();
    let expected = kit::direct(&kit::memory(&bytes));
    let mut job = AirBasesJob::fixture();
    stage(&mut job, &bytes);
    for initial in [0, 1] {
        let mut cap = CAPS;
        cap[3] = initial;
        cap[4] = initial;
        let mut memo = AirBasesMemo::default();
        let (air, fog, report) = run(&mut memo, &job, context(1), cap);
        assert!(report.installed && !report.hit);
        assert_eq!(memo.key.unwrap().capacities[3..], [2, 2]);
        assert_rows(&air, &fog, &expected);
        stage(&mut job, &bytes);
        let (air, fog, report) = run(&mut memo, &job, context(2), cap);
        assert!(report.hit && report.compared && !report.installed);
        assert_rows(&air, &fog, &expected);
        assert!(!air.bases_raw_mode && !fog.registrations_raw_mode);
    }
}

#[test]
fn memo_capacity_accounting_follows_growth_invalidation_move_and_drop() {
    static LOCAL: collector_primitives::raw_memory::Tracker =
        collector_primitives::raw_memory::Tracker::new();
    let mut bytes = kit::fixture();
    let mut job = AirBasesJob::fixture();
    stage(&mut job, &bytes);
    let mut memo = AirBasesMemo::on_tracker(&LOCAL);
    assert!(run(&mut memo, &job, context(1), CAPS).2.installed);
    assert_eq!(LOCAL.snapshot().current, memo.capacity());
    assert!(memo.capacity().payload_vectors > 0 && memo.capacity().auxiliary_vectors > 0);
    let before = memo.capacity();
    kit::add_alias_and_rocket(&mut bytes);
    stage(&mut job, &bytes);
    assert!(run(&mut memo, &job, context(2), CAPS).2.installed);
    assert_eq!(LOCAL.snapshot().current, memo.capacity());
    assert!(memo.capacity().payload_vectors >= before.payload_vectors);
    memo.invalidate();
    assert_eq!(LOCAL.snapshot().current, memo.capacity());
    let moved = Some(memo);
    assert!(LOCAL.snapshot().current.payload_vectors > 0);
    drop(moved);
    assert_eq!(
        LOCAL.snapshot().current,
        collector_primitives::raw_memory::Capacity::default()
    );
    assert!(LOCAL.snapshot().peak_total_lower_bound > 0);
}
