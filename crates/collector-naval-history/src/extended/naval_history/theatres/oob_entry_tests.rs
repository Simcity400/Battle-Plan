use super::*;
use crate::extended::naval_history::naval_test_kit as kit;
use engine_native::{
    Memory, batch_fallback, carmy, carmy_group, corders_group, corders_group_member, cunit,
    memory::RecordLayout, memory::RecordSources, memory::SliceMemory,
};
use std::cell::{Cell, RefCell};

thread_local! {
    pub(super) static SERIAL: Cell<bool> = const { Cell::new(false) };
}

pub(super) fn copy_oob_root_serial(
    job: &mut NavalJob,
    memory: &impl engine_native::Memory,
    base: usize,
    pointer: usize,
) -> bool {
    let log = &mut job.log;
    let own = copy_oob_root_spans(log, memory, pointer);
    copy_theatre_name(log, memory, pointer.checked_add(corders_group::NAME), 0);
    let is_army_group = own[0].map(|start| log.bytes[start]);
    for (index, offset) in [
        corders_group::THEATRE,
        corders_group::THEATER_GROUP,
        corders_group::PARENT,
        corders_group::LEADER,
        corders_group::HQ_PROVIDER,
    ]
    .into_iter()
    .enumerate()
    {
        let target = own[index + 2].and_then(|s| usize::try_from(npool_u64(log, s)).ok());
        if let Some(target) = target.filter(|&p| p != 0) {
            if offset != corders_group::HQ_PROVIDER {
                let identity = nspan(log, memory, Some(target), 16, OOB_MASK);
                if offset == corders_group::THEATRE
                    && identity.is_some_and(|start| {
                        engine_sites::at(base, ctheatre::VTABLE_RVA)
                            .is_some_and(|expected| npool_u64(log, start) == expected as u64)
                            && npool_u64(log, start + 8) as u32 == ctheatre::REFERENCE_TYPE
                    })
                {
                    job.oob_theatres_required = job.oob_theatres.len().saturating_add(1);
                    {
                        if reserve(&mut job.oob_theatres, 1).is_err() {
                            return false;
                        }
                        let required = job
                            .oob_theatres_required
                            .saturating_sub(job.oob_theatre_positions.len());
                        if reserve(&mut job.oob_theatre_positions, required).is_err() {
                            return false;
                        }
                    }
                    if job.oob_theatres.len() == job.oob_theatres.capacity()
                        || job.oob_theatres.len() == job.oob_theatre_positions.capacity()
                    {
                        log.overflowed = true;
                        return false;
                    }
                    job.oob_theatres.push(target);
                }
            }
            if offset == corders_group::PARENT {
                let _ = nspan(
                    log,
                    memory,
                    target.checked_add(corders_group::IS_ARMY_GROUP),
                    1,
                    OOB_MASK,
                );
            }
            if offset == corders_group::HQ_PROVIDER {
                if let Some(unit) =
                    target.checked_sub(engine_native::corders_group_member::HOST_OFFSET)
                {
                    let _ = nspan(log, memory, Some(unit), 32, OOB_MASK);
                }
                let _ = nspan(
                    log,
                    memory,
                    target.checked_add(engine_native::corders_group_member::GROUP),
                    8,
                    OOB_MASK,
                );
            }
        }
    }
    let Some(members_header) = own[7] else {
        return true;
    };
    let mut members_raw = [0u8; 16];
    members_raw.copy_from_slice(&log.bytes[members_header..members_header + 16]);
    let members = match Array::from_header(members_raw, 8) {
        Some(array) => array,
        None => return true,
    };
    for index in 0..members.count {
        let entry_addr = members.data.checked_add((index as usize).saturating_mul(8));
        let member = match nspan(log, memory, entry_addr, 8, OOB_MASK)
            .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
        {
            Some(member) => member,
            None => continue,
        };
        let unit = match member.checked_sub(engine_native::corders_group_member::HOST_OFFSET) {
            Some(unit) => unit,
            None => continue,
        };
        let _ = nspan(log, memory, Some(unit), 32, OOB_MASK);
        let _ = nspan(
            log,
            memory,
            member.checked_add(engine_native::corders_group_member::GROUP),
            8,
            OOB_MASK,
        );
        if log.overflowed {
            return false;
        }
    }
    if is_army_group.is_some_and(|flag| flag != 0) {
        let children_header = match nspan(
            log,
            memory,
            pointer.checked_add(engine_native::carmy_group::MEMBER_DATA),
            16,
            OOB_MASK,
        ) {
            Some(start) => start,
            None => return true,
        };
        let mut children_raw = [0u8; 16];
        children_raw.copy_from_slice(&log.bytes[children_header..children_header + 16]);
        if let Some(children) = Array::from_header(children_raw, 8) {
            for index in 0..children.count {
                let entry_addr = children
                    .data
                    .checked_add((index as usize).saturating_mul(8));
                let child = match nspan(log, memory, entry_addr, 8, OOB_MASK)
                    .and_then(|s| usize::try_from(npool_u64(log, s)).ok().filter(|&p| p != 0))
                {
                    Some(child) => child,
                    None => continue,
                };
                let _ = nspan(log, memory, Some(child), 16, OOB_MASK);
                if log.overflowed {
                    return false;
                }
            }
        }
    }
    if job.theatres_active {
        let _reads = read_accounting::table(TableId::LandTheatreMembers);
        let theater_group = nspan(
            log,
            memory,
            pointer.checked_add(corders_group::THEATER_GROUP),
            8,
            LAND_MEMBERS_MASK,
        )
        .and_then(|s| usize::try_from(npool_u64(log, s)).ok());
        if let Some(theater_group) = theater_group.filter(|&p| p != 0) {
            let _ = nspan(log, memory, Some(theater_group), 16, LAND_MEMBERS_MASK);
        }
    }
    if log.overflowed {
        return false;
    }
    true
}

const MEMBERS_AT: usize = 0x100_0000;
const CHILDREN_AT: usize = 0x108_0000;
const UNITS_AT: usize = 0x110_0000;
const CHILD_OBJECTS_AT: usize = 0x200_0000;
const UNIT_STRIDE: usize = 0x100;
const CHILD_STRIDE: usize = 0x40;

fn p32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn p64(bytes: &mut [u8], at: usize, value: u64) {
    bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn header(bytes: &mut [u8], at: usize, data: usize, count: u32) {
    p64(bytes, at, data as u64);
    p32(bytes, at + 8, count);
    p32(bytes, at + 12, count);
}

fn unit(index: usize) -> usize {
    UNITS_AT + index * UNIT_STRIDE
}

fn member(index: usize) -> usize {
    unit(index) + corders_group_member::HOST_OFFSET
}

fn entry(index: usize) -> usize {
    MEMBERS_AT + index * 8
}

fn child(index: usize) -> usize {
    CHILD_OBJECTS_AT + index * CHILD_STRIDE
}

fn order_of_battle(members: usize, children: usize) -> Vec<u8> {
    const _: () =
        assert!(corders_group_member::HOST_OFFSET + corders_group_member::GROUP + 8 <= UNIT_STRIDE);
    let mut bytes = kit::fixture();
    let group = kit::OOB_GROUP;
    p64(
        &mut bytes,
        group,
        (kit::BASE + carmy_group::VTABLE_RVA.0) as u64,
    );
    bytes[group + corders_group::IS_ARMY_GROUP] = 1;
    header(
        &mut bytes,
        group + corders_group::MEMBER_DATA,
        MEMBERS_AT,
        members as u32,
    );
    for index in 0..members {
        p64(&mut bytes, entry(index), member(index) as u64);
        let host = unit(index);
        p64(&mut bytes, host, carmy::VTABLE_RVA.0 as u64);
        p64(
            &mut bytes,
            host + carmy::SECONDARY_BASE,
            carmy::SECONDARY_VTABLE_RVA.0 as u64,
        );
        p64(
            &mut bytes,
            host + cunit::HANDLE,
            0x4500_0001u64 + index as u64,
        );
        p64(
            &mut bytes,
            member(index) + corders_group_member::GROUP,
            group as u64,
        );
    }
    header(
        &mut bytes,
        group + carmy_group::MEMBER_DATA,
        CHILDREN_AT,
        children as u32,
    );
    for index in 0..children {
        p64(&mut bytes, CHILDREN_AT + index * 8, child(index) as u64);
        p64(
            &mut bytes,
            child(index),
            (kit::BASE + corders_group::VTABLE_RVA.0) as u64,
        );
        p64(
            &mut bytes,
            child(index) + corders_group::HANDLE,
            ((index as u64 + 7) << 32) | corders_group::REFERENCE_TYPE as u64,
        );
    }
    bytes
}

const ALIAS: (usize, usize) = (5, 40);

fn aliased(members: usize, children: usize, nulls: std::ops::Range<usize>) -> Vec<u8> {
    let mut bytes = nulled(members, children, nulls);
    p64(&mut bytes, entry(11), member(10) as u64);
    p64(&mut bytes, CHILDREN_AT + 8, child(0) as u64);
    let (from, to) = ALIAS;
    let aliasing = entry(to) - corders_group_member::GROUP;
    p64(&mut bytes, entry(from), aliasing as u64);
    bytes
}

fn nulled(members: usize, children: usize, nulls: std::ops::Range<usize>) -> Vec<u8> {
    let mut bytes = order_of_battle(members, children);
    for index in nulls {
        p64(&mut bytes, entry(index), 0);
    }
    bytes
}

struct Faults<'a> {
    memory: SliceMemory<'a>,
    ranges: Vec<std::ops::Range<usize>>,
    once: bool,
    spent: RefCell<Vec<bool>>,
}

impl<'a> Faults<'a> {
    fn new(bytes: &'a [u8], ranges: Vec<std::ops::Range<usize>>, once: bool) -> Self {
        let spent = RefCell::new(vec![false; ranges.len()]);
        Self {
            memory: kit::memory(bytes),
            ranges,
            once,
            spent,
        }
    }
}

impl Memory for Faults<'_> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        let end = address.saturating_add(destination.len());
        let mut spent = self.spent.borrow_mut();
        for (range, spent) in self.ranges.iter().zip(spent.iter_mut()) {
            if address < range.end && range.start < end && !*spent {
                *spent = self.once;
                return false;
            }
        }
        self.memory.copy(address, destination)
    }
}

struct Counted<'a, M> {
    inner: &'a M,
    calls: Cell<usize>,
}

impl<M: Memory> Memory for Counted<'_, M> {
    fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy(address, destination)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy_scattered(addresses, destinations)
    }

    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        self.calls.set(self.calls.get() + 1);
        self.inner.copy_records(sources, layout, destination)
    }
}

type Keyed = std::collections::BTreeMap<(usize, u32), Vec<(bool, u32, Vec<u8>)>>;

type Air = (
    Outcome,
    Vec<engine_native::cair_theatre::Row>,
    Outcome,
    Vec<engine_native::cair_group::Row>,
    Outcome,
    Vec<engine_native::cair_group::MemberRow>,
);

#[derive(Debug, PartialEq)]
struct Pass {
    tables: kit::Split,
    names: (Vec<u8>, usize),
    air: Air,
    spans: Keyed,
}

impl Pass {
    fn reads(&self, range: &std::ops::Range<usize>) -> bool {
        self.spans
            .keys()
            .any(|&(address, len)| address < range.end && range.start < address + len as usize)
    }

    fn failures(&self) -> usize {
        self.spans
            .values()
            .flatten()
            .filter(|observation| !observation.0)
            .count()
    }

    fn refused(&self) -> [bool; 9] {
        let tables = &self.tables;
        [
            &tables.land_outcome,
            &tables.groups_outcome,
            &tables.members_outcome,
            &tables.oob_groups_outcome,
            &tables.oob_members_outcome,
            &tables.oob_children_outcome,
            &self.air.0,
            &self.air.2,
            &self.air.4,
        ]
        .map(|outcome| !matches!(outcome, Outcome::Rows { .. }))
    }
}

fn keyed(job: &NavalJob) -> Keyed {
    let mut keyed = Keyed::new();
    for span in &job.log.spans {
        let bytes = if span.ok {
            job.log.bytes[span.start as usize..][..span.len as usize].to_vec()
        } else {
            Vec::new()
        };
        keyed
            .entry((span.addr, span.len))
            .or_default()
            .push((span.ok, span.mask, bytes));
    }
    keyed
}

fn copy(memory: &impl Memory, serial: bool, log: Option<(usize, usize)>) -> (NavalJob, bool) {
    let ctx = NavalCopyContext {
        hour: 0,
        force_full: true,
        base: kit::BASE,
        game_state: kit::GAME_STATE,
    };
    let mut job = NavalJob::fixture();
    if let Some((spans, bytes)) = log {
        job.log = RawLog::new(spans, bytes);
    }
    kit::stage_wings(memory, &mut job);
    SERIAL.set(serial);
    let report = copy_naval_refresh(memory, ctx, &mut job);
    SERIAL.set(false);
    (job, report.raw_ok)
}

fn pass(memory: &impl Memory, serial: bool) -> Pass {
    let (job, raw_ok) = copy(memory, serial, None);
    assert!(raw_ok);
    let mut captured = Captured::with_capacities(&Capacities::fixture());
    decode_naval_payload(&mut captured, kit::BASE, &job, &mut [0u64; NAVAL_PACK_N]);
    Pass {
        tables: kit::decoded(&captured),
        names: (
            captured.theatre_names.first[..captured.theatre_names.len].to_vec(),
            captured.theatre_names.required,
        ),
        air: (
            captured.air.outcome,
            captured.air.rows.clone(),
            captured.air_groups.outcome,
            captured.air_groups.rows.clone(),
            captured.air_members.outcome,
            captured.air_members.rows.clone(),
        ),
        spans: keyed(&job),
    }
}

fn faults(members: usize, children: usize) -> Vec<(&'static str, std::ops::Range<usize>)> {
    let mut faults = Vec::new();
    for index in [0, 1, 2, 62, 63, 64, 65, 100, 127, 128, members - 1] {
        faults.push(("member entry", entry(index)..entry(index) + 8));
        faults.push(("host identity", unit(index)..unit(index) + 32));
        faults.push(("host identity tail", unit(index) + 31..unit(index) + 32));
        let backlink = member(index) + corders_group_member::GROUP;
        faults.push(("backlink", backlink..backlink + 8));
    }
    faults.push(("entries across a run boundary", entry(60)..entry(70)));
    faults.push(("every entry", entry(0)..entry(members)));
    for index in [0, 1, 63, 64, children - 1] {
        let at = CHILDREN_AT + index * 8;
        faults.push(("child entry", at..at + 8));
        faults.push(("child identity", child(index)..child(index) + 16));
    }
    let (_, to) = ALIAS;
    faults.push(("alias key", entry(to)..entry(to) + 8));
    faults.push(("alias host", entry(16)..entry(20)));
    faults
}

fn stops_a_batch(name: &str) -> bool {
    !matches!(name, "backlink" | "child identity")
}

#[test]
fn healthy_runs_log_the_per_entry_spans_at_scale() {
    const MEMBERS: usize = 5_000;
    const CHILDREN: usize = 130;
    let bytes = order_of_battle(MEMBERS, CHILDREN);
    let healthy = Faults::new(&bytes, Vec::new(), false);
    let counted = Counted {
        inner: &healthy,
        calls: Cell::new(0),
    };
    let fallbacks = batch_fallback::count(batch_fallback::Batch::OrderOfBattle);
    let batched = pass(&counted, false);
    let batched_calls = counted.calls.replace(0);
    assert_eq!(
        batch_fallback::count(batch_fallback::Batch::OrderOfBattle),
        fallbacks
    );
    let serial = pass(&counted, true);
    let serial_calls = counted.calls.get();
    assert_eq!(batched, serial);
    assert_eq!(
        batched.tables.oob_members_outcome,
        Outcome::Rows { len: MEMBERS }
    );
    assert_eq!(
        batched.tables.oob_children_outcome,
        Outcome::Rows { len: CHILDREN }
    );
    assert!(
        batched_calls * 2 < serial_calls,
        "{batched_calls} guarded calls against the per-entry {serial_calls}"
    );
}

#[test]
fn null_duplicate_and_aliased_entries_log_the_per_entry_spans() {
    for nulls in [0..0, 64..128, 70..90] {
        let bytes = aliased(150, 70, nulls.clone());
        let healthy = Faults::new(&bytes, Vec::new(), false);
        assert_eq!(pass(&healthy, false), pass(&healthy, true), "{nulls:?}");
    }
}

#[test]
fn a_lasting_fault_logs_the_per_entry_spans() {
    const MEMBERS: usize = 150;
    const CHILDREN: usize = 70;
    for bytes in [
        order_of_battle(MEMBERS, CHILDREN),
        aliased(MEMBERS, CHILDREN, 0..0),
    ] {
        let healthy = pass(&Faults::new(&bytes, Vec::new(), false), true);
        for (name, range) in faults(MEMBERS, CHILDREN) {
            let fallbacks = batch_fallback::count(batch_fallback::Batch::OrderOfBattle);
            let batched = pass(&Faults::new(&bytes, vec![range.clone()], false), false);
            let counted = batch_fallback::count(batch_fallback::Batch::OrderOfBattle) - fallbacks;
            let serial = pass(&Faults::new(&bytes, vec![range.clone()], false), true);
            assert_eq!(batched, serial, "{name} {range:x?}");
            if !healthy.reads(&range) {
                assert_eq!(batched.failures(), healthy.failures(), "{name} {range:x?}");
                continue;
            }
            assert!(batched.failures() > 0, "{name} {range:x?}");
            if !name.starts_with("alias") {
                assert_eq!(counted > 0, stops_a_batch(name), "{name} {range:x?}");
            }
        }
    }
}

#[test]
fn a_one_shot_fault_is_never_harsher_than_the_per_entry_copy() {
    const MEMBERS: usize = 150;
    const CHILDREN: usize = 70;
    for bytes in [
        order_of_battle(MEMBERS, CHILDREN),
        aliased(MEMBERS, CHILDREN, 0..0),
    ] {
        for (name, range) in faults(MEMBERS, CHILDREN) {
            let fallbacks = batch_fallback::count(batch_fallback::Batch::OrderOfBattle);
            let batched = pass(&Faults::new(&bytes, vec![range.clone()], true), false);
            let counted = batch_fallback::count(batch_fallback::Batch::OrderOfBattle) - fallbacks;
            let serial = pass(&Faults::new(&bytes, vec![range.clone()], true), true);
            assert!(batched.failures() <= serial.failures(), "{name} {range:x?}");
            for (table, (batched, serial)) in batched
                .refused()
                .into_iter()
                .zip(serial.refused())
                .enumerate()
            {
                assert!(!batched || serial, "{name} {range:x?} table {table}");
            }
            if !name.starts_with("alias") {
                assert_eq!(counted > 0, stops_a_batch(name), "{name} {range:x?}");
            }
        }
    }
}

#[test]
fn a_fixed_log_overflows_only_where_the_per_entry_copy_does() {
    const MEMBERS: usize = 70;
    const CHILDREN: usize = 3;
    let host_faults = unit(0)..unit(MEMBERS);
    for (name, bytes, ranges) in [
        ("healthy", order_of_battle(MEMBERS, CHILDREN), Vec::new()),
        ("null run", nulled(MEMBERS, CHILDREN, 0..64), Vec::new()),
        (
            "nulls behind a failing host",
            nulled(MEMBERS, CHILDREN, 1..64),
            std::iter::once(unit(0)..unit(0) + 32).collect::<Vec<_>>(),
        ),
        (
            "duplicates and an alias",
            aliased(MEMBERS, CHILDREN, 0..0),
            Vec::new(),
        ),
        (
            "every entry failing",
            order_of_battle(MEMBERS, CHILDREN),
            std::iter::once(entry(0)..entry(MEMBERS)).collect::<Vec<_>>(),
        ),
        (
            "every host failing",
            order_of_battle(MEMBERS, CHILDREN),
            vec![host_faults],
        ),
    ] {
        let memory = Faults::new(&bytes, ranges, false);
        let (job, raw_ok) = copy(&memory, true, None);
        assert!(raw_ok, "{name}");
        let (spans, pool) = (job.log.spans.len(), job.log.bytes.len());
        let span_caps = (spans.saturating_sub(300)..=spans + 1).map(|cap| (cap, pool + 64));
        let byte_caps = (pool.saturating_sub(1_200)..=pool + 8)
            .step_by(4)
            .chain([pool - 1, pool, pool + 1])
            .map(|cap| (spans + 4, cap));
        for capacity in span_caps.chain(byte_caps).chain([(spans, pool)]) {
            let (batched, batched_ok) = copy(&memory, false, Some(capacity));
            let (serial, serial_ok) = copy(&memory, true, Some(capacity));
            assert_eq!(batched_ok, serial_ok, "{name} {capacity:?}");
            assert_eq!(
                batched.log.overflowed, serial.log.overflowed,
                "{name} {capacity:?}"
            );
            if serial_ok {
                assert_eq!(keyed(&batched), keyed(&serial), "{name} {capacity:?}");
            }
        }
    }
}

#[test]
fn a_terminal_run_behind_a_failing_host_completes_where_the_per_entry_copy_does() {
    let mut bytes = nulled(64, 0, 1..64);
    p64(
        &mut bytes,
        kit::OOB_GROUP,
        (kit::BASE + corders_group::VTABLE_RVA.0) as u64,
    );
    bytes[kit::OOB_GROUP + corders_group::IS_ARMY_GROUP] = 0;
    let memory = Faults::new(
        &bytes,
        std::iter::once(unit(0)..unit(0) + 32).collect(),
        false,
    );
    let root = |serial: bool, log: RawLog<NAVAL_PACK_N>| {
        let mut job = NavalJob::fixture();
        job.log = log;
        job.theatres_active = false;
        SERIAL.set(serial);
        let copied = copy_oob_root(&mut job, &memory, kit::BASE, kit::OOB_GROUP);
        SERIAL.set(false);
        (job, copied)
    };
    let (measured, copied) = root(true, RawLog::new(1 << 12, 1 << 16));
    assert!(copied && !measured.log.overflowed);
    let spans = &measured.log.spans;
    let run = spans.len() - 66;
    assert_eq!((spans[run].addr, spans[run].len), (entry(0), 8));
    assert_eq!((spans[run + 1].addr, spans[run + 1].ok), (unit(0), false));
    assert_eq!(
        spans.last().map(|span| (span.addr, span.len)),
        Some((entry(63), 8))
    );
    let pool = measured.log.bytes.len();
    assert_eq!(pool - spans[run].start as usize, 520);
    let (serial, serial_ok) = root(true, RawLog::new(spans.len(), pool));
    assert!(serial_ok && !serial.log.overflowed);
    let (batched, batched_ok) = root(false, RawLog::new(spans.len(), pool));
    assert!(batched_ok && !batched.log.overflowed);
    assert_eq!(keyed(&batched), keyed(&serial));
}
