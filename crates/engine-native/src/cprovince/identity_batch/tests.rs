use super::*;
use crate::cprovince::{GeometryCapacities, GeometryPacket};
use engine_native_memory::memory::RecordSources;
use std::cell::Cell;

const STATE: usize = 0x1000;
const SLOTS: usize = 0x2000;
const OBJECTS: usize = 0x20000;

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
            base: 0,
            bytes: self.bytes,
        }
        .copy(at, out)
    }
}
struct Reads<'a> {
    source: Source<'a>,
    requests: Cell<usize>,
}
impl Memory for Reads<'_> {
    fn copy(&self, at: usize, out: &mut [u8]) -> bool {
        self.requests.set(self.requests.get() + 1);
        self.source.copy(at, out)
    }
    fn copy_records(
        &self,
        sources: RecordSources<'_>,
        layout: &RecordLayout<'_>,
        out: &mut [u8],
    ) -> usize {
        self.requests.set(self.requests.get() + 1);
        self.source.copy_records(sources, layout, out)
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        out: &mut [&mut [u8]; N],
    ) -> usize {
        self.requests.set(self.requests.get() + 1);
        self.source.copy_scattered(addresses, out)
    }
}

fn world(count: usize) -> (Vec<u8>, Vec<usize>) {
    let mut bytes = vec![0; OBJECTS + count * 0x100];
    let at = STATE + crate::cgame_state::PROVINCES;
    bytes[at..at + 8].copy_from_slice(&(SLOTS as u64).to_le_bytes());
    for offset in [8, 12] {
        bytes[at + offset..at + offset + 4].copy_from_slice(&(count as u32 + 1).to_le_bytes());
    }
    let pointers = (0..count)
        .map(|index| {
            let pointer = OBJECTS + index * 0x100;
            bytes[pointer..pointer + 8]
                .copy_from_slice(&(crate::cprovince::VTABLE_RVA.0 as u64).to_le_bytes());
            let at = pointer + crate::cprovince::ID;
            bytes[at..at + 4].copy_from_slice(&(index as u32 + 1).to_le_bytes());
            let at = SLOTS + (index + 1) * 8;
            bytes[at..at + 8].copy_from_slice(&(pointer as u64).to_le_bytes());
            pointer
        })
        .collect();
    (bytes, pointers)
}

struct Capture {
    ids: Vec<u32>,
    failure: Option<&'static str>,
    packet: GeometryPacket,
    requests: usize,
}
fn capture(
    bytes: &[u8],
    pointers: &[usize],
    batched: bool,
    fault: Option<usize>,
    once: bool,
) -> Capture {
    capture_capacity(bytes, pointers, batched, fault, once, pointers.len() + 1)
}
fn capture_capacity(
    bytes: &[u8],
    pointers: &[usize],
    batched: bool,
    fault: Option<usize>,
    once: bool,
    capacity: usize,
) -> Capture {
    let memory = Reads {
        source: Source {
            bytes,
            fault,
            once,
            spent: Cell::new(false),
        },
        requests: Cell::new(0),
    };
    let mut packet = GeometryPacket::try_with_capacities(GeometryCapacities {
        provinces: capacity,
        neighbour_entries: 0,
    })
    .unwrap();
    packet.begin(&memory, 0, STATE).unwrap();
    let mut ids = Vec::new();
    let mut failure = None;
    let mut batch = IdentityBatch::default();
    'walk: for chunk in pointers.chunks(64) {
        if batched {
            packet.stage_province_ids(&memory, chunk, &mut batch);
        }
        for &pointer in chunk {
            let read = if batched {
                packet.acquire_province_id_batched(&memory, pointer, &batch)
            } else {
                packet.acquire_province_id(&memory, pointer)
            };
            match read {
                Ok(id) => ids.push(id),
                Err(reason) => {
                    failure = Some(reason);
                    break 'walk;
                }
            }
        }
    }
    Capture {
        ids,
        failure,
        packet,
        requests: memory.requests.get(),
    }
}

fn same(actual: &Capture, serial: &Capture) {
    assert_eq!(actual.ids, serial.ids);
    assert_eq!(actual.failure, serial.failure);
    assert_eq!(actual.packet.counters(), serial.packet.counters());
    assert_eq!(actual.packet.slots, serial.packet.slots);
    assert_eq!(actual.packet.indices, serial.packet.indices);
    let fields = |packet: &GeometryPacket| {
        packet
            .provinces
            .iter()
            .map(|p| (p.pointer, p.vtable, p.id, p.authenticated_id))
            .collect::<Vec<_>>()
    };
    assert_eq!(fields(&actual.packet), fields(&serial.packet));
    assert_eq!(
        actual.packet.required_capacities(),
        serial.packet.required_capacities()
    );
}

#[test]
fn batched_province_persistent_fault_keeps_serial_reason_prefix_and_memos() {
    let (bytes, pointers) = world(65);
    for at in [
        pointers[20],
        pointers[20] + crate::cprovince::ID,
        SLOTS + 21 * 8,
    ] {
        same(
            &capture(&bytes, &pointers, true, Some(at), false),
            &capture(&bytes, &pointers, false, Some(at), false),
        );
    }
}

#[test]
fn batched_province_one_shot_refuses_no_more_and_counts_fallback() {
    let (bytes, pointers) = world(65);
    for at in [pointers[20] + crate::cprovince::ID, SLOTS + 21 * 8] {
        let before = engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionProvinces);
        let actual = capture(&bytes, &pointers, true, Some(at), true);
        let serial = capture(&bytes, &pointers, false, Some(at), true);
        assert_eq!(
            engine_native_memory::batch_fallback::count(engine_native_memory::batch_fallback::Batch::DivisionProvinces),
            before + 1
        );
        assert!(actual.ids.len() >= serial.ids.len());
        same(&actual, &capture(&bytes, &pointers, false, None, false));
    }
}

#[test]
fn batched_province_capacity_alias_and_null_locations_equal_serial() {
    let (bytes, mut pointers) = world(65);
    pointers[1] = pointers[0];
    pointers[63] = pointers[0];
    pointers[64] = pointers[0];
    for null in [false, true] {
        if null {
            pointers[20] = 0;
        }
        for capacity in [0, 1, 2, 20, 21, 63, 64, 65, 66] {
            for fault in [
                None,
                Some(pointers[10] + crate::cprovince::ID),
                Some(SLOTS + 11 * 8),
            ] {
                same(
                    &capture_capacity(&bytes, &pointers, true, fault, false, capacity),
                    &capture_capacity(&bytes, &pointers, false, fault, false, capacity),
                );
            }
        }
    }
}
