use super::*;
use engine_native_memory::Memory;
use crate::cgame_state;
use std::cell::{Cell, RefCell};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Event {
    Copy(usize, usize),
    Hint(usize, usize),
}

struct HintedMemory<'a> {
    source: &'a PresentationMemory,
    events: RefCell<Vec<Event>>,
    fault: Cell<Option<usize>>,
    once: bool,
}

impl<'a> HintedMemory<'a> {
    fn new(source: &'a PresentationMemory, fault: Option<usize>, once: bool) -> Self {
        Self {
            source,
            events: RefCell::new(Vec::new()),
            fault: Cell::new(fault),
            once,
        }
    }

    fn copies(&self) -> Vec<(usize, usize)> {
        self.events
            .borrow()
            .iter()
            .filter_map(|event| match *event {
                Event::Copy(at, len) => Some((at, len)),
                Event::Hint(..) => None,
            })
            .collect()
    }
}

impl Memory for HintedMemory<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.events
            .borrow_mut()
            .push(Event::Copy(address, output.len()));
        if self
            .fault
            .get()
            .is_some_and(|at| address <= at && at - address < output.len())
        {
            if self.once {
                self.fault.set(None);
            }
            output.fill(0xff);
            return false;
        }
        self.source.copy(address, output)
    }

    fn prefetch(&self, address: usize, len: usize) {
        self.events.borrow_mut().push(Event::Hint(address, len));
    }
}

struct Unhinted<'a, 'b>(&'a HintedMemory<'b>);
impl Memory for Unhinted<'_, '_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.0.copy(address, output)
    }

    fn prefetch(&self, _: usize, _: usize) {}
}

fn status_world(count: usize) -> (PresentationMemory, Vec<carmy::DivisionSeed>) {
    use engine_native_parts::{
        acclimatization_data as data,
        acclimatization_entry as entry,
    };
    let mut source = presentation_fixture();
    let mut first = [carmy::DivisionSeed {
        pointer: A1,
        division: presentation_source()[0].division,
        ..Default::default()
    }];
    carmy::capture_division_seeds(&source, &mut first);
    source.bytes.resize(0x20000 + count * 0x300, 0);
    let seeds = (0..count)
        .map(|index| {
            let mut seed = first[0];
            seed.division = handle(9, index as u32 + 1);
            seed.handle = Some(seed.division);
            let mut object = seed.acclimatization.unwrap();
            object[..8].copy_from_slice(&(data::VTABLE_RVA.0 as u64).to_le_bytes());
            object[data::DATA..data::DATA + 8]
                .copy_from_slice(&((0x20000 + index * 0x300) as u64).to_le_bytes());
            object[data::CAPACITY..data::CAPACITY + 4].copy_from_slice(&33_u32.to_le_bytes());
            object[data::COUNT..data::COUNT + 4].copy_from_slice(&33_u32.to_le_bytes());
            seed.acclimatization = Some(object);
            assert_eq!(entry::STRIDE, 16);
            seed
        })
        .collect();
    (source, seeds)
}

fn status(
    memory: &impl Memory,
    seeds: &[carmy::DivisionSeed],
    admitted: usize,
) -> (Outcome, Vec<carmy::StatusRow>) {
    status_holding(memory, seeds, admitted, &[])
}

fn status_holding(
    memory: &impl Memory,
    seeds: &[carmy::DivisionSeed],
    admitted: usize,
    held: &[Option<engine_native_parts::csub_unit_definition::DivisionMaxima>],
) -> (Outcome, Vec<carmy::StatusRow>) {
    let mut packet = status_packet(seeds.len());
    let allocated = packet.allocated_bytes();
    let source = if admitted < seeds.len() {
        Outcome::Truncated {
            len: admitted,
            unread_rows: (seeds.len() - admitted) as u32,
        }
    } else {
        Outcome::Rows { len: admitted }
    };
    let outcome = packet.capture_holding(memory, 0, GS, &seeds[..admitted], source, held, &[]);
    assert_eq!(packet.allocated_bytes(), allocated);
    let mut rows = vec![carmy::StatusRow::default(); admitted];
    assert_eq!(packet.project(&mut rows), outcome);
    rows.truncate(outcome.len());
    (outcome, rows)
}

fn status_packet(count: usize) -> carmy::StatusPacket {
    carmy::StatusPacket::with_capacities(carmy::StatusCapacities {
        membership: crate::corders_group_member::MembershipCapacity {
            queries: count,
            groups: 2,
            roots: 2,
            chunks: 2,
        },
        modifiers: engine_native_parts::modifier_accumulator::SnapshotCapacity {
            nodes: count,
            entries: count,
        },
        thresholds: 3,
        ..carmy::StatusCapacities::new(count)
    })
    .unwrap()
}

struct StatusReads<'a, 'b> {
    source: &'a HintedMemory<'b>,
    serial: bool,
    maxima: Cell<usize>,
    entries: Cell<usize>,
    prefetched_entries: Cell<usize>,
    uninit_entries: Cell<usize>,
    entries_end: usize,
}

impl Memory for StatusReads<'_, '_> {
    fn copy_uninit<'a>(
        &self,
        at: usize,
        out: &'a mut [std::mem::MaybeUninit<u8>],
    ) -> Option<&'a mut [u8]> {
        if (0x20000..self.entries_end).contains(&at) && !out.is_empty() {
            self.entries.set(self.entries.get() + 1);
            self.uninit_entries.set(self.uninit_entries.get() + 1);
        }
        self.source.copy_uninit(at, out)
    }

    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        if (0x20000..self.entries_end).contains(&at) && !out.is_empty() {
            self.entries.set(self.entries.get() + 1);
        }
        self.source.copy(at, out)
    }
    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        out: &mut [u8],
    ) -> usize {
        if layout.stride() == 25 {
            if self.serial && sources.len() > 1 {
                return 0;
            }
            self.maxima.set(self.maxima.get() + 1);
        }
        self.source.copy_records(sources, layout, out)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        out: &mut [&mut [u8]; N],
    ) -> usize {
        if N == 64 && out.iter().any(|row| row.len() == 512) {
            if self.serial {
                return 0;
            }
            self.entries.set(self.entries.get() + 1);
        }
        self.source.copy_scattered(addresses, out)
    }

    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        out: &mut [&mut [u8]; N],
    ) -> usize {
        if N == 64 && out.iter().any(|row| row.len() == 512) {
            self.prefetched_entries
                .set(self.prefetched_entries.get() + 1);
        }
        self.copy_scattered(addresses, out)
    }
}

fn distinct_status_world(
    count: usize,
) -> (
    PresentationMemory,
    Vec<carmy::DivisionSeed>,
    Vec<Option<engine_native_parts::csub_unit_definition::DivisionMaxima>>,
) {
    use engine_native_parts::csub_unit_definition as stats;
    let (mut source, mut seeds) = status_world(count);
    let start = source.bytes.len();
    source.bytes.resize(start + count * 0x600, 0);
    let held = seeds
        .iter_mut()
        .enumerate()
        .map(|(index, seed)| {
            let pointer = start + index * 0x600;
            seed.stat_block = Some(pointer);
            p64(&mut source.bytes, pointer, stats::VTABLE_RVA.0 as u64);
            p64(
                &mut source.bytes,
                pointer + stats::MAXIMUM_ORGANISATION,
                100_000 + index as u64,
            );
            p64(
                &mut source.bytes,
                pointer + stats::MAXIMUM_STRENGTH,
                200_000 + index as u64,
            );
            Some(stats::DivisionMaxima {
                pointer,
                organisation: 100_000 + index as i64,
                strength: 200_000 + index as i64,
                can_exfiltrate_from_coast: 0,
            })
        })
        .collect();
    let prototype = source.bytes[C1..C1 + 0x1000].to_vec();
    let countries = 0x4000_0000;
    let links = 0x4100_0000;
    let mut country_slots = vec![0_u8; (count + 1) * 8];
    let mut link_slots = vec![0_u8; (count + 1) * 4];
    for (index, seed) in seeds.iter_mut().enumerate() {
        let pointer = source.bytes.len();
        source.bytes.extend_from_slice(&prototype);
        p32(&mut source.bytes, pointer + ccountry::ID, index as u32 + 1);
        p64(&mut country_slots, (index + 1) * 8, pointer as u64);
        p32(&mut link_slots, (index + 1) * 4, index as u32 + 1);
        seed.roster_owner = Some(index as u32 + 1);
        seed.expeditionary_owner = Some(0);
    }
    array(
        &mut source.bytes,
        GS + cgame_state::COUNTRIES,
        countries,
        count as u32 + 1,
    );
    array(
        &mut source.bytes,
        GS + cgame_state::COUNTRY_LINKS,
        links,
        count as u32 + 1,
    );
    source.globals.insert(countries, country_slots);
    source.globals.insert(links, link_slots);
    (source, seeds, held)
}

fn status_reads<'a, 'b>(
    source: &'a HintedMemory<'b>,
    count: usize,
    serial: bool,
) -> StatusReads<'a, 'b> {
    StatusReads {
        source,
        serial,
        maxima: Cell::new(0),
        entries: Cell::new(0),
        prefetched_entries: Cell::new(0),
        uninit_entries: Cell::new(0),
        entries_end: 0x20000 + count * 0x300,
    }
}

#[test]
fn batched_status_persistent_faults_equal_serial() {
    let (source, seeds, _) = distinct_status_world(65);
    let stats = seeds[20].stat_block.unwrap();
    for fault in [
        stats,
        stats + engine_native_parts::csub_unit_definition::MAXIMUM_STRENGTH,
        stats + engine_native_parts::csub_unit_definition::CAN_EXFILTRATE_FROM_COAST,
        0x20000 + 20 * 0x300 + 7,
        0x20000 + 20 * 0x300 + 32 * 16 + 7,
    ] {
        let actual = HintedMemory::new(&source, Some(fault), false);
        let direct = HintedMemory::new(&source, Some(fault), false);
        assert_eq!(
            status(&status_reads(&actual, 65, false), &seeds, 65),
            status(&status_reads(&direct, 65, true), &seeds, 65),
            "fault {fault:#x}"
        );
    }
}

#[test]
fn batched_status_one_shot_faults_refuse_no_more_and_count_fallbacks() {
    let (source, seeds, _) = distinct_status_world(65);
    for fault in [
        seeds[20].stat_block.unwrap() + engine_native_parts::csub_unit_definition::MAXIMUM_STRENGTH,
        0x20000 + 20 * 0x300 + 7,
    ] {
        let actual = HintedMemory::new(&source, Some(fault), true);
        let direct = HintedMemory::new(&source, Some(fault), true);
        let before = engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionMaxima)
            + engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionAcclimatization);
        let batched = status(&status_reads(&actual, 65, false), &seeds, 65);
        assert_eq!(batched.0, Outcome::Rows { len: 65 });
        assert_eq!(
            engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionMaxima)
                + engine_native_memory::batch_fallback::count(
                    engine_native_memory::batch_fallback::Batch::DivisionAcclimatization
                ),
            before + 1
        );
        let serial = status(&status_reads(&direct, 65, true), &seeds, 65);
        assert!(batched.0.len() >= serial.0.len());
        assert_eq!(batched, status(&source, &seeds, 65));
    }
}

#[test]
fn batched_status_request_counts_and_held_maxima_gates() {
    let (source, seeds, held) = distinct_status_world(65);
    let actual = HintedMemory::new(&source, None, false);
    let batch = status_reads(&actual, 65, false);
    assert_eq!(status(&batch, &seeds, 65).0, Outcome::Rows { len: 65 });
    assert_eq!((batch.maxima.get(), batch.entries.get()), (2, 67));
    let actual = HintedMemory::new(&source, Some(seeds[20].stat_block.unwrap()), false);
    let batch = status_reads(&actual, 65, false);
    assert_eq!(
        status_holding(&batch, &seeds, 65, &held).0,
        Outcome::Rows { len: 65 }
    );
    assert_eq!(batch.maxima.get(), 0);
    assert_eq!(actual.fault.get(), Some(seeds[20].stat_block.unwrap()));
    let actual = HintedMemory::new(&source, None, false);
    let batch = status_reads(&actual, 65, false);
    status(&batch, &seeds, 1);
    assert_eq!((batch.maxima.get(), batch.entries.get()), (1, 2));

    let mut shared = seeds[..2].to_vec();
    shared[1].stat_block = shared[0].stat_block;
    let carried = [held[0], None];
    let pointer = shared[0].stat_block.unwrap();
    let actual = HintedMemory::new(&source, Some(pointer), false);
    let batch = status_reads(&actual, 65, false);
    assert_eq!(
        status_holding(&batch, &shared, 2, &carried).0,
        Outcome::Rows { len: 2 }
    );
    assert_eq!(
        batch.maxima.get(),
        0,
        "an earlier held seed supplies a later memo hit"
    );
    assert_eq!(actual.fault.get(), Some(pointer));
}

#[test]
fn status_reuses_entry_scratch_and_prefetches_each_scatter() {
    let (source, seeds, _) = distinct_status_world(65);
    let mut packet = status_packet(seeds.len());
    let scratch = std::ptr::from_ref(packet.acclimatization.as_deref().unwrap());
    let allocated = packet.allocated_bytes();
    let expected = status(&source, &seeds, seeds.len());
    let mut rows = vec![carmy::StatusRow::default(); seeds.len()];
    for fault in [None, Some(0x20000 + 20 * 0x300 + 7), None] {
        packet
            .acclimatization
            .as_deref_mut()
            .unwrap()
            .bytes
            .fill([0xff; 512]);
        let hinted = HintedMemory::new(&source, fault, false);
        let memory = status_reads(&hinted, seeds.len(), false);
        let before =
            engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionAcclimatization);
        let outcome = packet.capture_holding(
            &memory,
            0,
            GS,
            &seeds,
            Outcome::Rows { len: seeds.len() },
            &[],
            &[],
        );
        assert_eq!(outcome, Outcome::Rows { len: seeds.len() });
        assert_eq!(packet.project(&mut rows), outcome);
        assert_eq!(
            memory.prefetched_entries.get(),
            2,
            "every entry batch uses the prefetched API"
        );
        assert_eq!(
            memory.uninit_entries.get(),
            if fault.is_some() { 108 } else { 65 }
        );
        assert_eq!(
            engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionAcclimatization)
                - before,
            u64::from(fault.is_some())
        );
        let serial = HintedMemory::new(&source, fault, false);
        assert_eq!(
            (outcome, rows.clone()),
            status(&status_reads(&serial, 65, true), &seeds, 65)
        );
        if fault.is_none() {
            assert_eq!((outcome, rows.clone()), expected);
        }
        assert_eq!(
            std::ptr::from_ref(packet.acclimatization.as_deref().unwrap()),
            scratch
        );
        assert_eq!(packet.allocated_bytes(), allocated);
    }
}

#[test]
fn status_maxima_reuses_one_probe_per_seed_and_keeps_carried_aliases_unread() {
    let (source, mut seeds, held) = distinct_status_world(129);
    for index in [1, 31, 63, 64, 65, 128] {
        seeds[index].stat_block = seeds[0].stat_block;
    }
    for carried in [false, true] {
        let mut holds = vec![None; seeds.len()];
        if carried {
            holds[0] = held[0];
        }
        let hinted = HintedMemory::new(
            &source,
            carried.then_some(seeds[0].stat_block.unwrap()),
            false,
        );
        let memory = status_reads(&hinted, seeds.len(), false);
        let mut packet = status_packet(seeds.len());
        let outcome = packet.capture_holding(
            &memory,
            0,
            GS,
            &seeds,
            Outcome::Rows { len: seeds.len() },
            &holds,
            &[],
        );
        assert_eq!(outcome, Outcome::Rows { len: seeds.len() });
        assert_eq!(
            packet
                .maxima_probes
                .load(std::sync::atomic::Ordering::Relaxed),
            seeds.len(),
            "selection's slots must serve admission too"
        );
        let mut rows = vec![carmy::StatusRow::default(); seeds.len()];
        packet.project(&mut rows);
        assert_eq!(
            (outcome, rows),
            status_holding(&source, &seeds, seeds.len(), &holds)
        );
        assert_eq!(
            hinted.fault.get(),
            carried.then_some(seeds[0].stat_block.unwrap())
        );
        assert!(
            packet
                .maxima_index
                .iter()
                .all(|&resident| resident <= packet.maxima.len()),
            "speculative keys cannot survive preparation"
        );
    }
}

#[test]
fn status_maxima_collisions_use_serial_slots_and_values() {
    let (source, all, held) = distinct_status_world(5_000);
    let mut packet = status_packet(64);
    let empty = vec![0; packet.maxima_index.len()];
    let home = carmy::status_pointer_slot(&empty, all[0].stat_block.unwrap(), |_| unreachable!());
    let mut seeds: Vec<_> = all
        .iter()
        .copied()
        .filter(|seed| {
            carmy::status_pointer_slot(&empty, seed.stat_block.unwrap(), |_| unreachable!()) == home
        })
        .take(12)
        .collect();
    assert!(
        seeds.len() >= 3,
        "fixture must contain an actual probe chain"
    );
    let first = seeds[0];
    seeds.insert(1, first);
    let mut holds = vec![None; seeds.len()];
    holds[0] = held[0];
    let hinted = HintedMemory::new(&source, Some(first.stat_block.unwrap()), false);
    let outcome = packet.capture_holding(
        &hinted,
        0,
        GS,
        &seeds,
        Outcome::Rows { len: seeds.len() },
        &holds,
        &[],
    );
    assert_eq!(outcome, Outcome::Rows { len: seeds.len() });
    assert_eq!(
        packet
            .maxima_probes
            .load(std::sync::atomic::Ordering::Relaxed),
        seeds.len()
    );
    for (index, seed) in seeds.iter().enumerate() {
        let expected = engine_native_parts::csub_unit_definition::capture_division_maxima(
            &source,
            0,
            seed.stat_block.unwrap(),
        )
        .unwrap();
        assert_eq!(packet.unit_maxima(index), Some(expected));
        let slot = carmy::status_pointer_slot(&packet.maxima_index, expected.pointer, |at| {
            packet.maxima[at].pointer
        });
        assert_eq!(packet.maxima[packet.maxima_index[slot] - 1], expected);
    }
    assert_eq!(hinted.fault.get(), Some(first.stat_block.unwrap()));
}

#[test]
fn status_exercise_control_memo_matches_native_path_and_first_partial_faults() {
    use crate::cgame_state as state;
    for counts in [2, 4] {
        let mut source = presentation_fixture();
        array(&mut source.bytes, GS + state::HUMAN_STATUS, 0x15000, counts);
        source.bytes[0x15011] = 1;
        source.bytes[0x15002] = 1;
        for once in [false, true] {
            for fault in [
                None,
                Some(GS + state::HUMAN_STATUS + 7),
                Some(GS + state::AUTONOMOUS_STATUS + 7),
                Some(0x15001),
                Some(0x15012),
                Some(GS + state::PLAYER_RECORDS + 7),
            ] {
                let actual = HintedMemory::new(&source, fault, once);
                let direct = HintedMemory::new(&source, fault, once);
                let countries = crate::ccountry::Reader::new(&actual, 0, GS).unwrap();
                let mut packet =
                    carmy::StatusPacket::with_capacities(carmy::StatusCapacities::new(8)).unwrap();
                let allocated = packet.allocated_bytes();
                for raw in [3, 3, 1, 2, 2, 0, 1] {
                    let result = packet.exercise_autonomous(&actual, 0, GS, &countries, raw);
                    let reference = state::autonomous_country(&direct, 0, GS, raw);
                    assert_eq!(
                        result, reference,
                        "counts {counts}, fault {fault:?}, once {once}, raw {raw}"
                    );
                    if result.is_err() {
                        break;
                    }
                }
                assert_eq!(packet.allocated_bytes(), allocated);
            }
        }
        let memory = HintedMemory::new(&source, None, false);
        let countries = crate::ccountry::Reader::new(&memory, 0, GS).unwrap();
        let mut packet =
            carmy::StatusPacket::with_capacities(carmy::StatusCapacities::new(1)).unwrap();
        assert_eq!(
            packet.exercise_autonomous(&memory, 0, GS, &countries, 3),
            Ok(true)
        );
        packet.clear();
        drop(memory);
        source.bytes[0x15011] = 0;
        let countries = crate::ccountry::Reader::new(&source, 0, GS).unwrap();
        assert_eq!(
            packet.exercise_autonomous(&source, 0, GS, &countries, 3),
            Ok(false)
        );
    }
}

fn movement_world(count: usize) -> (PresentationMemory, Vec<carmy::DivisionSeed>) {
    let (mut source, sources) = movement_combat_fixture();
    let mut original = [carmy::DivisionSeed {
        pointer: sources[0].pointer,
        division: sources[0].division,
        ..Default::default()
    }];
    carmy::capture_division_seeds(&source, &mut original);
    source.bytes.resize(0x20000 + count * 0x1000, 0);
    array(
        &mut source.bytes,
        GS + crate::cgame_state::PROVINCES,
        PROVINCES,
        (2 + count * 2) as u32,
    );
    let seeds = (0..count)
        .map(|index| {
            let at = 0x20000 + index * 0x1000;
            let mut seed = original[0];
            seed.division = handle(9, index as u32 + 1);
            seed.handle = Some(seed.division);
            let mut header = [0; 16];
            array(&mut header, 0, at, 1);
            seed.ordinary_route = Some(header);
            array(&mut header, 0, at + 8, 1);
            seed.alternate_route = Some(header);
            array(&mut header, 0, at + 0x20, 1);
            seed.combat = Some(header);
            p32(&mut source.bytes, at, 1);
            p32(&mut source.bytes, at + 8, 1);
            p64(&mut source.bytes, at + 0x20, 0x16000);
            let stat = at + 0x100;
            seed.stat_block = Some(stat);
            p64(
                &mut source.bytes,
                stat,
                engine_native_parts::csub_unit_definition::VTABLE_RVA.0 as u64,
            );
            p64(
                &mut source.bytes,
                stat + engine_native_parts::csub_unit_definition::MAXIMUM_SPEED,
                400_000 + index as u64,
            );
            for (offset, id) in [(0x800, 2 + index * 2), (0x900, 3 + index * 2)] {
                let pointer = at + offset;
                p64(&mut source.bytes, pointer, cprovince::VTABLE_RVA.0 as u64);
                p32(&mut source.bytes, pointer + cprovince::ID, id as u32);
                p64(&mut source.bytes, PROVINCES + id * 8, pointer as u64);
            }
            seed.movement_target = Some(at + 0x800);
            seed.movement_source = Some(at + 0x900);
            seed
        })
        .collect();
    (source, seeds)
}

fn movement(
    memory: &impl Memory,
    seeds: &[carmy::DivisionSeed],
    admitted: usize,
    paths: usize,
) -> (carmy::MovementWalk, Vec<carmy::MovementRow>, Vec<u32>) {
    let mut geometry = crate::cprovince::GeometryPacket::with_capacities(
        crate::cprovince::GeometryCapacities::default(),
    );
    geometry.begin(memory, 0, GS).unwrap();
    let mut packet = carmy::MovementPacket::new(admitted, paths);
    let allocated = packet.allocated_bytes();
    let source = if admitted < seeds.len() {
        Outcome::Truncated {
            len: admitted,
            unread_rows: (seeds.len() - admitted) as u32,
        }
    } else {
        Outcome::Rows { len: admitted }
    };
    let result = carmy::capture_movement_inputs(
        memory,
        0,
        GS,
        &seeds[..admitted],
        source,
        &mut geometry,
        &mut packet,
    );
    let mut output = vec![carmy::MovementRow::default(); admitted];
    let mut route = vec![0; result.required_paths];
    assert_eq!(
        carmy::project_movement(&packet, &mut output, &mut route),
        result
    );
    if result.required_paths <= paths {
        assert_eq!(packet.allocated_bytes(), allocated);
    }
    output.truncate(result.outcome.len());
    route.truncate(result.paths_len);
    (result, output, route)
}

#[test]
fn movement_route_and_combat_payload_hints_still_lead_their_reads() {
    let (source, seeds) = movement_world(9);
    let memory = HintedMemory::new(&source, None, false);
    let (walk, rows, _) = movement(&memory, &seeds, 9, 18);
    assert_eq!(walk.outcome, Outcome::Rows { len: 9 });
    let events = memory.events.borrow();
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.base_speed, Some(400_000 + index as i64));
        let at = 0x20000 + index * 0x1000;
        let prior = if index < 4 { 0x20000 } else { at - 0x1000 };
        let prior_route = events
            .iter()
            .position(|&event| event == Event::Copy(prior, 4))
            .unwrap();
        for (pointer, len) in [(at, 4), (at + 8, 4), (at + 0x20, 8)] {
            let hint = events
                .iter()
                .position(|&event| event == Event::Hint(pointer, len))
                .unwrap();
            assert!(hint < prior_route, "division {index}, hint {pointer:x}");
        }
    }
}

#[test]
fn movement_hints_preserve_copy_order_source_prefixes_route_growth_and_partial_faults() {
    let (source, mut seeds) = movement_world(9);
    for rows in [0, 1, 4, 5, 9] {
        for paths in [0, 2, 6, 18] {
            for once in [false, true] {
                for fault in [
                    None,
                    Some(0x20003),
                    Some(0x2000b),
                    Some(0x20027),
                    Some(0x20800 + cprovince::ID + 2),
                    Some(PROVINCES + 2 * 8 + 7),
                    Some(0x20100 + engine_native_parts::csub_unit_definition::MAXIMUM_SPEED + 3),
                ] {
                    let actual = HintedMemory::new(&source, fault, once);
                    let direct = HintedMemory::new(&source, fault, once);
                    assert_eq!(
                        movement(&actual, &seeds, rows, paths),
                        movement(&Unhinted(&direct), &seeds, rows, paths),
                        "rows {rows}, paths {paths}, fault {fault:?}, once {once}"
                    );
                    assert_eq!(actual.copies(), direct.copies());
                }
            }
        }
    }
    seeds[0].ordinary_route = None;
    let actual = HintedMemory::new(&source, Some(0x21003), true);
    let direct = HintedMemory::new(&source, Some(0x21003), true);
    assert_eq!(
        movement(&actual, &seeds, 9, 18),
        movement(&Unhinted(&direct), &seeds, 9, 18)
    );
    assert_eq!(actual.copies(), direct.copies());
    assert_eq!(actual.fault.get(), Some(0x21003));
    assert!(!actual.events.borrow().iter().any(|&event| {
        event == Event::Hint(0x20100 + engine_native_parts::csub_unit_definition::MAXIMUM_SPEED, 8)
    }));
}

const BATCHED_PROVINCE_SLOTS: usize = 0x4200_0000;

fn batched_movement_world(countries: usize) -> (PresentationMemory, Vec<carmy::DivisionSeed>) {
    let (mut source, mut seeds) = movement_world(countries);
    let mut slots = vec![0_u8; (2 + countries * 2) * 8];
    let country_slots = 0x4300_0000;
    let link_slots = 0x4400_0000;
    let mut countries_bytes = vec![0_u8; (countries + 1) * 8];
    let mut links_bytes = vec![0_u8; (countries + 1) * 4];
    for (index, seed) in seeds.iter_mut().enumerate() {
        for (id, pointer) in [
            (2 + index * 2, seed.movement_target.unwrap()),
            (3 + index * 2, seed.movement_source.unwrap()),
        ] {
            p64(&mut slots, id * 8, pointer as u64);
        }
        seed.combat = Some([0; 16]);
        seed.logical_owner = Some(index as u32 + 1);
        seed.roster_owner = Some(index as u32 + 1);
        let country = source.bytes.len();
        source.bytes.resize(country + 0x1000, 0);
        p64(&mut source.bytes, country, ccountry::VTABLE_RVA.0 as u64);
        p32(&mut source.bytes, country + ccountry::ID, index as u32 + 1);
        p64(&mut countries_bytes, (index + 1) * 8, country as u64);
        p32(&mut links_bytes, (index + 1) * 4, index as u32 + 1);
    }
    array(
        &mut source.bytes,
        GS + cgame_state::PROVINCES,
        BATCHED_PROVINCE_SLOTS,
        (2 + countries * 2) as u32,
    );
    array(
        &mut source.bytes,
        GS + cgame_state::COUNTRIES,
        country_slots,
        countries as u32 + 1,
    );
    array(
        &mut source.bytes,
        GS + cgame_state::COUNTRY_LINKS,
        link_slots,
        countries as u32 + 1,
    );
    source.globals.insert(BATCHED_PROVINCE_SLOTS, slots);
    source.globals.insert(country_slots, countries_bytes);
    source.globals.insert(link_slots, links_bytes);
    (source, seeds)
}

struct MovementReads<'a, 'b> {
    source: &'a HintedMemory<'b>,
    serial: bool,
    province: Cell<usize>,
    slots: Cell<usize>,
    speed: Cell<usize>,
    slot_end: usize,
}
impl Memory for MovementReads<'_, '_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        if out.len() == 8 && (BATCHED_PROVINCE_SLOTS..self.slot_end).contains(&at) {
            self.slots.set(self.slots.get() + 1);
        }
        self.source.copy(at, out)
    }
    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        out: &mut [u8],
    ) -> usize {
        let counter = match (layout.stride(), layout.field_count()) {
            (12, 2) => Some(&self.province),
            (8, 1) => Some(&self.slots),
            (16, 2) => Some(&self.speed),
            _ => None,
        };
        if let Some(counter) = counter {
            if self.serial {
                return 0;
            }
            counter.set(counter.get() + 1);
        }
        self.source.copy_records(sources, layout, out)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        out: &mut [&mut [u8]; N],
    ) -> usize {
        if N == 2 {
            if addresses[0].checked_add(cprovince::ID) == Some(addresses[1]) && out[1].len() == 4 {
                self.province.set(self.province.get() + 1);
            } else if addresses[0].checked_add(engine_native_parts::csub_unit_definition::MAXIMUM_SPEED)
                == Some(addresses[1])
                && out[1].len() == 8
            {
                self.speed.set(self.speed.get() + 1);
            }
        }
        self.source.copy_scattered(addresses, out)
    }
}
fn movement_reads<'a, 'b>(
    source: &'a HintedMemory<'b>,
    count: usize,
    serial: bool,
) -> MovementReads<'a, 'b> {
    MovementReads {
        source,
        serial,
        province: Cell::new(0),
        slots: Cell::new(0),
        speed: Cell::new(0),
        slot_end: BATCHED_PROVINCE_SLOTS + (2 + count * 2) * 8,
    }
}

#[test]
fn batched_movement_persistent_faults_keep_serial_prefix_and_optional_speed() {
    let (source, seeds) = batched_movement_world(65);
    for fault in [
        seeds[20].movement_target.unwrap(),
        seeds[20].movement_source.unwrap() + cprovince::ID,
        BATCHED_PROVINCE_SLOTS + 42 * 8,
        seeds[20].stat_block.unwrap() + engine_native_parts::csub_unit_definition::MAXIMUM_SPEED,
    ] {
        let actual = HintedMemory::new(&source, Some(fault), false);
        let direct = HintedMemory::new(&source, Some(fault), false);
        assert_eq!(
            movement(&movement_reads(&actual, 65, false), &seeds, 65, 130),
            movement(&movement_reads(&direct, 65, true), &seeds, 65, 130),
            "fault {fault:#x}"
        );
    }
}
#[test]
fn batched_movement_one_shot_refuses_no_more_and_counts_fallbacks() {
    let (source, seeds) = batched_movement_world(65);
    for fault in [
        seeds[20].movement_source.unwrap() + cprovince::ID,
        BATCHED_PROVINCE_SLOTS + 42 * 8,
        seeds[20].stat_block.unwrap() + engine_native_parts::csub_unit_definition::MAXIMUM_SPEED,
    ] {
        let actual = HintedMemory::new(&source, Some(fault), true);
        let direct = HintedMemory::new(&source, Some(fault), true);
        let before = engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionProvinces)
            + engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionMovementSpeed);
        let batch = movement(&movement_reads(&actual, 65, false), &seeds, 65, 130);
        assert_eq!(
            engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionProvinces)
                + engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionMovementSpeed),
            before + 1
        );
        let serial = movement(&movement_reads(&direct, 65, true), &seeds, 65, 130);
        assert!(batch.0.outcome.len() >= serial.0.outcome.len());
        assert_eq!(batch, movement(&source, &seeds, 65, 130));
    }
}
#[test]
fn batched_movement_request_counts_and_admitted_speed_gate() {
    let (source, seeds) = batched_movement_world(65);
    let actual = HintedMemory::new(&source, None, false);
    let batch = movement_reads(&actual, 65, false);
    assert_eq!(
        movement(&batch, &seeds, 65, 130).0.outcome,
        Outcome::Rows { len: 65 }
    );
    assert_eq!(
        (batch.province.get(), batch.slots.get(), batch.speed.get()),
        (3, 3, 2)
    );
    let actual = HintedMemory::new(&source, None, false);
    let batch = movement_reads(&actual, 65, false);
    movement(&batch, &seeds, 1, 2);
    assert_eq!(batch.speed.get(), 1);
    let actual = HintedMemory::new(&source, None, false);
    let batch = movement_reads(&actual, 65, false);
    movement(&batch, &seeds, 0, 0);
    assert_eq!(batch.speed.get(), 0);
}

#[test]
fn batched_status_capacity_alias_and_partial_fault_matrix_equal_serial() {
    let (source, mut seeds, held) = distinct_status_world(65);
    seeds[1].stat_block = seeds[0].stat_block;
    seeds[64].stat_block = seeds[0].stat_block;
    let mut carried = vec![None; seeds.len()];
    carried[0] = held[0];
    for capacity in [0, 1, 2, 20, 21, 63, 64, 65] {
        for fault in [
            None,
            Some(seeds[20].stat_block.unwrap()),
            Some(
                seeds[20].stat_block.unwrap()
                    + engine_native_parts::csub_unit_definition::MAXIMUM_ORGANISATION
                    + 7,
            ),
            Some(0x20000 + 20 * 0x300 + 7),
            Some(0x20000 + 20 * 0x300 + 32 * 16 + 7),
        ] {
            let actual = HintedMemory::new(&source, fault, false);
            let direct = HintedMemory::new(&source, fault, false);
            assert_eq!(
                status_holding(
                    &status_reads(&actual, 65, false),
                    &seeds,
                    capacity,
                    &carried
                ),
                status_holding(&status_reads(&direct, 65, true), &seeds, capacity, &carried),
                "capacity {capacity}, fault {fault:x?}"
            );
        }
    }
}

#[test]
fn batched_movement_prefix_growth_duplicates_shared_null_and_stationary_equal_serial() {
    let (source, mut seeds) = batched_movement_world(65);
    for index in [1, 63, 64] {
        seeds[index].division = seeds[0].division;
        seeds[index].handle = seeds[0].handle;
        seeds[index].stat_block = seeds[0].stat_block;
        seeds[index].movement_source = seeds[0].movement_source;
        seeds[index].movement_target = seeds[0].movement_target;
    }
    seeds[2].movement_source = Some(0);
    seeds[3].movement_target = Some(0);
    seeds[4].ordinary_route = Some([0; 16]);
    seeds[5].alternate_route = Some([0; 16]);
    for rows in [0, 1, 2, 20, 21, 63, 64, 65] {
        for paths in [0, 1, 2, 39, 40, 126, 130] {
            for fault in [
                None,
                Some(seeds[20].movement_target.unwrap() + cprovince::ID),
                Some(seeds[0].stat_block.unwrap() + engine_native_parts::csub_unit_definition::MAXIMUM_SPEED),
            ] {
                let actual = HintedMemory::new(&source, fault, false);
                let direct = HintedMemory::new(&source, fault, false);
                assert_eq!(
                    movement(&movement_reads(&actual, 65, false), &seeds, rows, paths),
                    movement(&movement_reads(&direct, 65, true), &seeds, rows, paths),
                    "rows {rows}, paths {paths}, fault {fault:x?}"
                );
            }
        }
    }
}

mod split_tests;
