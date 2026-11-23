use super::*;
use std::cell::Cell;

struct Source<'a> {
    bytes: &'a [u8],
    fault: Option<usize>,
    once: bool,
    spent: Cell<bool>,
}
impl Memory for Source<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        if self
            .fault
            .is_some_and(|fault| at <= fault && fault - at < out.len())
            && (!self.once || !self.spent.replace(true))
        {
            out.fill(0xff);
            return false;
        }
        engine_native_memory::memory::SliceMemory {
            base: 0x1000,
            bytes: self.bytes,
        }
        .copy(at, out)
    }
}
struct Reads<'a> {
    source: Source<'a>,
    serial: bool,
    members: Cell<usize>,
}
impl Memory for Reads<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        if out.len() == cname_group_member::INPUT_SPAN {
            self.members.set(self.members.get() + 1);
        }
        self.source.copy(at, out)
    }
    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        out: &mut [u8],
    ) -> usize {
        if layout.stride() == cname_group_member::INPUT_SPAN {
            if self.serial {
                return 0;
            }
            self.members.set(self.members.get() + 1);
        }
        self.source.copy_records(sources, layout, out)
    }
}
fn fixture(count: usize) -> (Vec<u8>, Vec<DivisionSeed>) {
    let mut bytes = Vec::new();
    let seeds = (0..count)
        .map(|index| {
            let pointer = 0x1000 + bytes.len();
            let mut span = member(0, b"NAME", index as i32 - 4);
            let cache = cname_group_member::CACHED_NAME;
            let text = 0x1000 + count * cname_group_member::INPUT_SPAN + index * 32;
            span[cache..cache + 8].copy_from_slice(&(text as u64).to_le_bytes());
            span[cache + 16..cache + 24].copy_from_slice(&20_u64.to_le_bytes());
            span[cache + 24..cache + 32].copy_from_slice(&20_u64.to_le_bytes());
            bytes.extend_from_slice(&span);
            DivisionSeed {
                division: ((index as u64 + 1) << 32) | 9,
                country_ordinal: Some(index as u32),
                name_member: Some(pointer),
                ..Default::default()
            }
        })
        .collect();
    for index in 0..count {
        bytes.extend_from_slice(&[b'A' + (index % 26) as u8; 20]);
        bytes.extend_from_slice(&[0; 12]);
    }
    (bytes, seeds)
}
fn reads(bytes: &[u8], serial: bool, fault: Option<usize>, once: bool) -> Reads<'_> {
    Reads {
        source: Source {
            bytes,
            fault,
            once,
            spent: Cell::new(false),
        },
        serial,
        members: Cell::new(0),
    }
}
fn capture(memory: &impl Memory, seeds: &[DivisionSeed]) -> Captured {
    let mut captured = Captured::with_capacities(&Capacities {
        status: seeds.len(),
        name_bytes: seeds.len() * 20,
        cached_name_bytes: seeds.len() * 20,
        ..Capacities::default()
    });
    captured.capture_naming(memory, 0, seeds);
    captured
}
fn same(actual: &Captured, serial: &Captured) {
    assert_eq!(actual.naming.len, serial.naming.len);
    assert_eq!(actual.naming.required, serial.naming.required);
    for (row, old) in actual.naming.first.iter().zip(&serial.naming.first) {
        assert_eq!(row.division, old.division);
        assert_eq!(row.country_ordinal, old.country_ordinal);
        assert_eq!(row.inputs, old.inputs);
        assert_eq!(row.failure, old.failure);
        assert_eq!(row.cached_name, old.cached_name);
        assert_eq!(row.cached_failure, old.cached_failure);
    }
    assert_eq!(
        &actual.names.first[..actual.names.len],
        &serial.names.first[..serial.names.len]
    );
    assert_eq!(
        &actual.cached_names.first[..actual.cached_names.len],
        &serial.cached_names.first[..serial.cached_names.len]
    );
    assert_eq!(actual.names.required, serial.names.required);
    assert_eq!(actual.cached_names.required, serial.cached_names.required);
}
#[test]
fn batched_naming_persistent_faults_keep_serial_options_and_arenas() {
    let (bytes, seeds) = fixture(65);
    let pointer = seeds[20].name_member.unwrap();
    for offset in [
        0,
        cname_group_member::CACHED_NAME + 7,
        cname_group_member::FLAG_B,
        cname_group_member::OVERRIDE_NAME + 7,
    ] {
        let batch = reads(&bytes, false, Some(pointer + offset), false);
        let serial = reads(&bytes, true, Some(pointer + offset), false);
        let before = engine_native_memory::batch_fallback::count(
            engine_native_memory::batch_fallback::Batch::DivisionNaming,
        );
        let actual = capture(&batch, &seeds);
        assert_eq!(
            engine_native_memory::batch_fallback::count(
                engine_native_memory::batch_fallback::Batch::DivisionNaming
            ) - before,
            1,
        );
        same(&actual, &capture(&serial, &seeds));
    }
}
#[test]
fn batched_naming_one_shot_refuses_no_more_and_counts_fallback() {
    let (bytes, seeds) = fixture(65);
    let at = seeds[20].name_member.unwrap() + cname_group_member::FLAG_B;
    let batch = reads(&bytes, false, Some(at), true);
    let serial = reads(&bytes, true, Some(at), true);
    let before =
        engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionNaming);
    let actual = capture(&batch, &seeds);
    assert_eq!(
        engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionNaming)
            - before,
        1,
    );
    let direct = capture(&serial, &seeds);
    let refused = |capture: &Captured| {
        capture.naming.first[..65]
            .iter()
            .filter(|row| row.inputs.is_none() || row.cached_name.is_none())
            .count()
    };
    assert!(refused(&actual) <= refused(&direct));
    same(&actual, &capture(&reads(&bytes, true, None, false), &seeds));
}
#[test]
fn batched_naming_request_count_is_one_per_sixty_four_rows() {
    let (bytes, seeds) = fixture(129);
    let batch = reads(&bytes, false, None, false);
    assert_eq!(capture(&batch, &seeds).naming.len, 129);
    assert_eq!(
        batch.members.get(),
        3,
        "old code used 129 guarded member requests"
    );
}

#[test]
fn batched_naming_absent_null_and_duplicate_members_equal_serial() {
    let (bytes, mut seeds) = fixture(65);
    seeds[0].name_member = None;
    seeds[1].name_member = Some(0);
    seeds[63].name_member = seeds[2].name_member;
    seeds[64].name_member = seeds[2].name_member;
    let batch = reads(&bytes, false, None, false);
    let actual = capture(&batch, &seeds);
    same(&actual, &capture(&reads(&bytes, true, None, false), &seeds));
    assert_eq!(
        batch.members.get(),
        2,
        "duplicate members still share each gather"
    );
    assert!(actual.naming.first[0].inputs.is_none());
    assert!(actual.naming.first[1].inputs.is_none());
    let duplicate = |index: usize| {
        let inputs = actual.naming.first[index].inputs.unwrap();
        let (start, len) = inputs.custom;
        (inputs.order, &actual.names.first[start..start + len])
    };
    assert_eq!(duplicate(63), duplicate(64));
}

#[test]
fn batched_naming_zero_and_short_arenas_equal_serial() {
    let (bytes, seeds) = fixture(65);
    for available in [0, 1, 64] {
        let run = |serial| {
            let mut captured = Captured::with_capacities(&Capacities {
                status: available,
                name_bytes: available,
                cached_name_bytes: available,
                ..Capacities::default()
            });
            captured.capture_naming(&reads(&bytes, serial, None, false), 0, &seeds);
            captured
        };
        let actual = run(false);
        same(&actual, &run(true));
        assert_eq!(actual.naming.len, seeds.len());
        assert_eq!(actual.cached_names.len, seeds.len() * 20);
        assert!(
            actual.naming.first[..seeds.len()]
                .iter()
                .all(|row| { row.failure.is_none() && row.cached_failure.is_none() })
        );
    }
}

#[test]
fn batched_naming_heap_faults_equal_serial_and_one_shot_never_refuses_more() {
    let (bytes, seeds) = fixture(65);
    let heap = 0x1000 + seeds.len() * cname_group_member::INPUT_SPAN + 20 * 32;
    for once in [false, true] {
        let actual = capture(&reads(&bytes, false, Some(heap + 7), once), &seeds);
        let serial = capture(&reads(&bytes, true, Some(heap + 7), once), &seeds);
        same(&actual, &serial);
        assert!(actual.naming.first[20].inputs.is_some());
        assert_eq!(actual.naming.first[20].cached_name.is_none(), !once);
        assert!(actual.naming.first[21].cached_name.is_some());
    }
}

#[test]
fn batched_naming_admitted_status_prefix_equals_serial() {
    let (bytes, seeds) = fixture(129);
    for admitted in [0, 1, 63, 64, 65, 128] {
        let batch = reads(&bytes, false, None, false);
        let actual = capture(&batch, &seeds[..admitted]);
        same(
            &actual,
            &capture(&reads(&bytes, true, None, false), &seeds[..admitted]),
        );
        assert_eq!(actual.naming.len, admitted);
        assert_eq!(batch.members.get(), admitted.div_ceil(64));
    }
}

#[test]
fn batched_naming_carried_members_and_heap_cache_equal_serial() {
    use collector_division_parts::army_stat_changes::Watches;
    let (bytes, mut seeds) = fixture(65);
    for (index, seed) in seeds.iter_mut().enumerate() {
        seed.pointer = 0x1000_0000 + index * 256;
    }
    let watches = Watches::enabled(256);
    let mut store = collector_division_parts::attack_stats::ArmyStatStore::new();
    let mut actual = capture(&reads(&bytes, false, None, false), &[]);
    for hour in 0..3 {
        store.observations.finish();
        if hour == 2 {
            drop(watches.notify(seeds[20].name_member.unwrap(), 1));
        }
        let memory = reads(&bytes, false, None, false);
        actual.attack.certify(
            &memory,
            0,
            &seeds,
            Outcome::Rows { len: seeds.len() },
            Some(&mut store),
            &watches,
        );
        let lookups = store.name_lookups;
        let expected = if actual.attack.carryable {
            seeds.len()
        } else {
            0
        };
        actual.capture_naming_holding(&memory, 0, &seeds, Some(&mut store), &watches);
        assert_eq!(
            store.name_lookups - lookups,
            expected,
            "each seed's held lookup must serve gathering and decoding"
        );
        store.hold(actual.attack.epoch());
        same(&actual, &capture(&reads(&bytes, true, None, false), &seeds));
        if hour == 0 {
            assert_eq!(memory.members.get(), 2);
        } else if hour == 1 {
            assert_eq!(memory.members.get(), 0, "held members must not be gathered");
            assert_eq!(actual.attack.status_counts.names_reused, 65);
        } else {
            assert_eq!(
                memory.members.get(),
                1,
                "only the touched member is gathered"
            );
            assert_eq!(actual.attack.status_counts.names_reused, 64);
        }
    }
    assert!(!watches.poisoned());
}

#[test]
fn batched_naming_selects_once_with_touched_and_reordered_seeds() {
    use collector_division_parts::army_stat_changes::Watches;
    let (bytes, mut seeds) = fixture(5_000);
    for (index, seed) in seeds.iter_mut().enumerate() {
        seed.pointer = 0x1000_0000 + index * 256;
    }
    let watches = Watches::enabled(2 * seeds.len());
    let mut store = collector_division_parts::attack_stats::ArmyStatStore::new();
    let mut actual = capture(&reads(&bytes, false, None, false), &[]);
    for hour in 0..3 {
        if hour == 1 {
            seeds.rotate_left(23);
            seeds.remove(40);
            seeds[90].name_member = None;
            seeds[91].name_member = Some(0);
            let touched = seeds
                .iter()
                .find_map(|seed| seed.name_member.filter(|&at| at != 0))
                .unwrap();
            drop(watches.notify(touched, 1));
        }
        let memory = reads(&bytes, false, None, false);
        actual.attack.certify(
            &memory,
            0,
            &seeds,
            Outcome::Rows { len: seeds.len() },
            Some(&mut store),
            &watches,
        );
        let before = store.name_lookups;
        let expected = if actual.attack.carryable {
            seeds
                .iter()
                .filter(|seed| seed.name_member.is_some_and(|at| at != 0))
                .count()
        } else {
            0
        };
        actual.capture_naming_holding(&memory, 0, &seeds, Some(&mut store), &watches);
        assert_eq!(store.name_lookups - before, expected);
        let reused = seeds
            .iter()
            .enumerate()
            .filter(|(index, seed)| {
                let Some(pointer) = seed.name_member.filter(|&at| at != 0) else {
                    return false;
                };
                actual.attack.carryable
                    && store
                        .held_name(*index, seed.division)
                        .is_some_and(|at| store.held_name_record(at).member == pointer)
                    && !actual.attack.touched(pointer)
            })
            .count();
        assert_eq!(actual.attack.status_counts.names_reused as usize, reused);
        store.hold(actual.attack.epoch());
        same(&actual, &capture(&reads(&bytes, true, None, false), &seeds));
    }
    assert!(!watches.poisoned());
}
