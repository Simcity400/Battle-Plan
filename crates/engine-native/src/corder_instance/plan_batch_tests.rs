use super::*;
use engine_native_memory::batch_fallback::{self, Batch};
use std::{cell::RefCell, collections::BTreeMap};

const BASE: usize = 0x1000_0000;
const GROUP: usize = 0x2000_0000;
const FIRST: usize = 0x3000_0000;
const STEP: usize = 0x1000;
const UNIT: usize = 0x4000_0000;
const MEMBERS: usize = 0x5000_0000;

#[derive(Default)]
struct Image(BTreeMap<usize, Vec<u8>>);
impl Image {
    fn write(&mut self, address: usize, bytes: &[u8]) {
        if let Some((&base, block)) = self.0.range_mut(..=address).next_back()
            && let Some(out) = block.get_mut(address - base..address - base + bytes.len())
        {
            out.copy_from_slice(bytes);
            return;
        }
        self.0.insert(address, bytes.to_vec());
    }
    fn u64(&mut self, address: usize, value: u64) {
        self.write(address, &value.to_le_bytes());
    }
}
impl Memory for Image {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let Some((&base, block)) = self.0.range(..=address).next_back() else {
            return false;
        };
        let Some(bytes) = block.get(address - base..address - base + output.len()) else {
            return false;
        };
        output.copy_from_slice(bytes);
        true
    }
}

fn fixture() -> (Image, [corders_group::Root; 1], [Visit; 4]) {
    let mut image = Image::default();
    image.u64(
        BASE + engine_native_parts::creference_registry::INVALID_REFERENCE_RVA.0,
        u64::MAX,
    );
    image.write(UNIT, &[0; 0x100]);
    image.u64(UNIT, (BASE + crate::carmy::VTABLE_RVA.0) as u64);
    image.u64(
        UNIT + crate::carmy::SECONDARY_BASE,
        (BASE + crate::carmy::SECONDARY_VTABLE_RVA.0) as u64,
    );
    image.u64(UNIT + crate::cunit::HANDLE, 19);
    let visits = std::array::from_fn(|index| {
        let pointer = FIRST + STEP * index;
        image.write(pointer, &[0; 0x400]);
        image.u64(pointer, (BASE + VTABLE_RVA.0) as u64);
        image.u64(pointer + OWNER, GROUP as u64);
        image.write(pointer + ID, &(index as u32).to_le_bytes());
        image.write(pointer + TYPE, &TYPE_MOVE.to_le_bytes());
        let members = MEMBERS + index * 16;
        image.u64(pointer + ASSIGNED_MEMBER_DATA, members as u64);
        image.write(pointer + ASSIGNED_MEMBER_DATA + 8, &1_u32.to_le_bytes());
        image.write(pointer + ASSIGNED_MEMBER_DATA + 12, &1_u32.to_le_bytes());
        image.u64(members, (UNIT + crate::cunit::SCHEDULED_MEMBER) as u64);
        for offset in NAME_STRINGS {
            image.write(pointer + offset, &[b'A' + index as u8]);
            image.u64(pointer + offset + engine_native_memory::msvc_string::LENGTH, 1);
            image.u64(pointer + offset + engine_native_memory::msvc_string::CAPACITY, 15);
        }
        Visit {
            pointer,
            owner: 0,
            instance: index as u32,
            sequence: index as u32,
            ..Visit::default()
        }
    });
    (
        image,
        [corders_group::Root {
            pointer: GROUP,
            handle: 7,
            country: 1,
        }],
        visits,
    )
}

struct Source<'a> {
    image: &'a Image,
    faults: [usize; 2],
    once: bool,
    serial: bool,
    spent: RefCell<[bool; 2]>,
    gathers: RefCell<Vec<(usize, Vec<usize>)>>,
    reads: RefCell<Vec<(usize, usize)>>,
}
impl Memory for Source<'_> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        self.reads.borrow_mut().push((address, output.len()));
        let mut spent = self.spent.borrow_mut();
        for (index, fault) in self.faults.iter().enumerate() {
            if address <= *fault && *fault < address + output.len() && !spent[index] {
                spent[index] = self.once;
                return false;
            }
        }
        self.image.copy(address, output)
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        output: &mut [u8],
    ) -> usize {
        self.gathers.borrow_mut().push((
            layout.stride(),
            (0..sources.len())
                .map(|row| sources.pointer(row).unwrap())
                .collect(),
        ));
        if self.serial && matches!(layout.stride(), PLAN_STRIDE | NAME_STRIDE) {
            return 0;
        }
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields() {
                let at = row * layout.stride() + field.destination_offset;
                if !self.copy(
                    sources.pointer(row).unwrap() + field.source_offset,
                    &mut output[at..at + field.len],
                ) {
                    return completed;
                }
                completed += 1;
            }
        }
        completed
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Captured {
    outcome: Outcome,
    rows: Vec<PlanRow>,
    assigned: Vec<u64>,
    links: Vec<u32>,
    names: Vec<u8>,
    demand: usize,
}
fn capture(memory: &Source<'_>, groups: &[corders_group::Root], visits: &[Visit]) -> Captured {
    let (mut rows, mut assigned, mut links, mut names) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let walk = walk_plans(
        memory,
        BASE,
        groups,
        visits,
        Outcome::Rows { len: visits.len() },
        &mut rows,
        &mut assigned,
        &mut links,
        &mut names,
    );
    assigned.truncate(walk.assigned);
    links.truncate(walk.links);
    names.truncate(walk.names);
    Captured {
        outcome: walk.outcome,
        rows,
        assigned,
        links,
        names,
        demand: walk.required_assigned,
    }
}

fn check_suffix(names: bool, once: bool) {
    let (image, groups, visits) = fixture();
    let offset = if names { NAMES.0 } else { SEGMENT_START };
    let source = |serial| Source {
        image: &image,
        faults: [FIRST + STEP + offset, FIRST + 3 * STEP + offset],
        once,
        serial,
        spent: RefCell::new([false; 2]),
        gathers: RefCell::default(),
        reads: RefCell::default(),
    };
    let batched = source(false);
    let before = batch_fallback::count(Batch::OrderPlans);
    let actual = capture(&batched, &groups, &visits);
    assert_eq!(batch_fallback::count(Batch::OrderPlans) - before, 1);
    assert!(actual.demand > 0 && !actual.assigned.is_empty());
    let reference = capture(&source(true), &groups, &visits);
    if once {
        assert!(
            actual
                .rows
                .iter()
                .filter(|row| row.owner_group != 0)
                .count()
                >= reference
                    .rows
                    .iter()
                    .filter(|row| row.owner_group != 0)
                    .count()
        );
        assert!(actual.names.len() >= reference.names.len());
        assert!(
            batched.spent.borrow()[1],
            "the later suffix fault belongs to the scalar decode"
        );
        let later = FIRST + 3 * STEP + offset;
        let width = if names { engine_native_memory::msvc_string::STRIDE } else { 4 };
        assert_eq!(
            batched
                .reads
                .borrow()
                .iter()
                .filter(|&&read| read == (later, width))
                .count(),
            1
        );
    } else {
        assert_eq!(
            actual, reference,
            "persistent faults preserve rows, first reason, arenas and demand"
        );
    }
    let stride = if names { NAME_STRIDE } else { PLAN_STRIDE };
    let gathers = batched.gathers.borrow();
    let matching: Vec<_> = gathers
        .iter()
        .filter(|(found, _)| *found == stride)
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "the incomplete suffix must never be gathered again"
    );
    assert_eq!(
        matching[0].1,
        (0..4).map(|index| FIRST + index * STEP).collect::<Vec<_>>()
    );
    let prefix_read = if names {
        (FIRST + NAMES.0, NAME_STRIDE)
    } else {
        (FIRST + SEGMENT_START, 8)
    };
    assert_eq!(
        batched
            .reads
            .borrow()
            .iter()
            .filter(|&&read| read == prefix_read)
            .count(),
        1
    );
}

#[test]
fn plan_middle_record_fault_keeps_prefix_and_reads_suffix_once() {
    for once in [false, true] {
        check_suffix(false, once);
    }
}

#[test]
fn plan_middle_name_fault_keeps_prefix_and_reads_suffix_once() {
    for once in [false, true] {
        check_suffix(true, once);
    }
}
