use super::*;
use std::{
    cell::{Cell, RefCell},
    hash::{BuildHasherDefault, Hasher},
};

#[derive(Default)]
struct PointerHasher(u64);

impl Hasher for PointerHasher {
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }

    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut state = self.0;
        for &byte in bytes {
            state = (state ^ u64::from(byte)).wrapping_mul(0x517c_c1b7_2722_0a95);
        }
        self.0 = state;
    }

    #[inline]
    fn write_usize(&mut self, value: usize) {
        self.0 = (self.0 ^ (value as u64)).wrapping_mul(0x517c_c1b7_2722_0a95);
        self.0 ^= self.0 >> 32;
    }
}

type PointerMap<V> = HashMap<usize, V, BuildHasherDefault<PointerHasher>>;

pub(super) const MAX_WITNESS_BYTES: usize = 48 * 1024 * 1024;
const MAX_COALESCED_SPAN: usize = 4096;

pub(super) struct Witness {
    pub id: u64,
    spans: Box<[(usize, u32, bool)]>,
    bytes: Box<[u8]>,
    longest: usize,
    pub team: (u64, bool),
    stamp: u64,
}
impl Witness {
    pub(super) fn retained_bytes(&self) -> usize {
        self.spans.len() * std::mem::size_of::<(usize, u32, bool)>() + self.bytes.len()
    }
    pub(super) fn unchanged(&self, memory: &impl Memory, scratch: &mut Vec<u8>) -> bool {
        const BATCH: usize = 16;
        let ensure_scratch = |scratch: &mut Vec<u8>, needed: usize| -> bool {
            if scratch.len() < needed {
                let short = needed - scratch.len();
                if engine_allocation::advisory(|| engine_allocation::reserve(scratch, short))
                    .is_err()
                {
                    return false;
                }
                scratch.resize(needed, 0);
            }
            true
        };
        if !ensure_scratch(scratch, self.longest) {
            return false;
        }
        let mut index = 0;
        let mut at = 0;
        while index < self.spans.len() {
            let (address, len, ok) = self.spans[index];
            if !ok || address == 0 {
                let len = len as usize;
                let read = &mut scratch[..len];
                if memory.copy(address, read) != ok {
                    return false;
                }
                if ok {
                    if self.bytes.get(at..at + len) != Some(&*read) {
                        return false;
                    }
                    at += len;
                }
                index += 1;
                continue;
            }
            let mut count = 0;
            let mut total = 0_usize;
            while count < BATCH && index + count < self.spans.len() {
                let (span_addr, span_len, span_ok) = self.spans[index + count];
                if !span_ok || span_addr == 0 {
                    break;
                }
                let Some(next) = total.checked_add(span_len as usize) else {
                    return false;
                };
                total = next;
                count += 1;
            }
            let Some(expected) = self.bytes.get(at..at + total) else {
                return false;
            };
            if count == 1 {
                let len = len as usize;
                let read = &mut scratch[..len];
                if !memory.copy(address, read) || read != expected {
                    return false;
                }
            } else {
                if !ensure_scratch(scratch, total) {
                    return false;
                }
                let spans = &self.spans[index..index + count];
                let out = &mut scratch[..total];
                let copied = match count {
                    2..=4 => copy_scattered_batch::<4>(memory, spans, out),
                    5..=8 => copy_scattered_batch::<8>(memory, spans, out),
                    _ => copy_scattered_batch::<BATCH>(memory, spans, out),
                };
                if !copied || &*out != expected {
                    return false;
                }
            }
            at += total;
            index += count;
        }
        at == self.bytes.len()
    }
    pub(super) fn size(&self) -> (usize, usize) {
        (self.spans.len(), self.bytes.len())
    }
}

fn copy_scattered_batch<const N: usize>(
    memory: &impl Memory,
    spans: &[(usize, u32, bool)],
    out: &mut [u8],
) -> bool {
    let mut addresses = [0_usize; N];
    let mut rest = out;
    let mut destinations: [&mut [u8]; N] = std::array::from_fn(|lane| {
        if let Some(&(span_addr, span_len, _)) = spans.get(lane) {
            addresses[lane] = span_addr;
            let (head, tail) = std::mem::take(&mut rest).split_at_mut(span_len as usize);
            rest = tail;
            head
        } else {
            &mut []
        }
    });
    memory.copy_scattered_prefetched(addresses, &mut destinations) == N
}

static WITNESSES: Mutex<Option<PointerMap<Witness>>> = Mutex::new(None);
fn witnesses() -> std::sync::MutexGuard<'static, Option<PointerMap<Witness>>> {
    WITNESSES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
pub(super) fn take(watches: &[Watch], removed: &[usize]) -> Vec<Option<Witness>> {
    let mut held = witnesses();
    let mut out = Vec::new();
    let room =
        engine_allocation::advisory(|| engine_allocation::reserve(&mut out, watches.len())).is_ok();
    for watch in watches {
        let witness = held
            .as_mut()
            .and_then(|held| held.remove(&watch.pointer))
            .filter(|witness| witness.id == watch.id && !removed.contains(&watch.pointer));
        if room {
            out.push(witness);
        }
    }
    out
}
static WITNESS_STAMP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub(super) fn keep_all(kept: Vec<(usize, Witness)>) {
    let mut held = witnesses();
    let held = held.get_or_insert_with(PointerMap::default);
    let stamp = WITNESS_STAMP.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if engine_allocation::advisory(|| engine_allocation::reserve_map(held, kept.len())).is_ok() {
        for (pointer, mut witness) in kept {
            witness.stamp = stamp;
            held.insert(pointer, witness);
        }
    }
    let mut total: usize = held.values().map(Witness::retained_bytes).sum();
    if total > MAX_WITNESS_BYTES {
        let mut order: Vec<(u64, usize, usize)> = Vec::new();
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut order, held.len())).is_ok()
        {
            for (&pointer, witness) in held.iter() {
                order.push((witness.stamp, pointer, witness.retained_bytes()));
            }
            order.sort_unstable_by_key(|&(stamp, _, _)| stamp);
            for (_, pointer, bytes) in order {
                if total <= MAX_WITNESS_BYTES {
                    break;
                }
                if held.remove(&pointer).is_some() {
                    total = total.saturating_sub(bytes);
                }
            }
        }
    }
}
pub(super) fn remove(pointer: usize) {
    if let Some(held) = witnesses().as_mut() {
        held.remove(&pointer);
    }
}
pub(super) fn release() {
    drop(witnesses().take());
    super::snapshots::speculative::release();
}

fn normalize(
    spans: &[(usize, u32, bool)],
    bytes: &[u8],
) -> Option<(Vec<(usize, u32, bool)>, Vec<u8>)> {
    let mut ok_spans: Vec<(usize, u32, usize)> = Vec::new();
    let mut other_spans: Vec<(usize, u32, bool, usize)> = Vec::new();
    engine_allocation::reserve(&mut ok_spans, spans.len()).ok()?;
    let mut at = 0_usize;
    for &(address, len, ok) in spans {
        let len_usize = len as usize;
        if ok {
            if at.checked_add(len_usize).is_none_or(|end| end > bytes.len()) {
                return None;
            }
            if address != 0 && len != 0 && address.checked_add(len_usize).is_some() {
                ok_spans.push((address, len, at));
            } else {
                engine_allocation::reserve(&mut other_spans, 1).ok()?;
                other_spans.push((address, len, true, at));
            }
            at += len_usize;
        } else {
            engine_allocation::reserve(&mut other_spans, 1).ok()?;
            other_spans.push((address, len, false, 0));
        }
    }
    if at != bytes.len() {
        return None;
    }
    ok_spans.sort_unstable_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));
    let mut out_spans: Vec<(usize, u32, bool)> = Vec::new();
    let mut out_bytes: Vec<u8> = Vec::new();
    engine_allocation::reserve(&mut out_spans, ok_spans.len() + other_spans.len()).ok()?;
    engine_allocation::reserve(&mut out_bytes, bytes.len()).ok()?;
    let mut prev_byte_start = 0_usize;
    for (address, len, byte_off) in ok_spans {
        let len_usize = len as usize;
        let data = &bytes[byte_off..byte_off + len_usize];
        if let Some(last) = out_spans.last_mut() {
            let prev_start = last.0;
            let prev_len = last.1 as usize;
            let prev_end = prev_start.checked_add(prev_len)?;
            if address < prev_end {
                let overlap_off = address - prev_start;
                let overlap_len = (prev_end - address).min(len_usize);
                if out_bytes[prev_byte_start + overlap_off
                    ..prev_byte_start + overlap_off + overlap_len]
                    != data[..overlap_len]
                {
                    return None;
                }
                if len_usize <= overlap_len {
                    continue;
                }
                let tail = &data[overlap_len..];
                if prev_len.saturating_add(tail.len()) <= MAX_COALESCED_SPAN {
                    last.1 += tail.len() as u32;
                    out_bytes.extend_from_slice(tail);
                } else {
                    prev_byte_start = out_bytes.len();
                    out_spans.push((prev_end, tail.len() as u32, true));
                    out_bytes.extend_from_slice(tail);
                }
                continue;
            }
            if address == prev_end && prev_len.saturating_add(len_usize) <= MAX_COALESCED_SPAN {
                last.1 += len;
                out_bytes.extend_from_slice(data);
                continue;
            }
        }
        prev_byte_start = out_bytes.len();
        out_spans.push((address, len, true));
        out_bytes.extend_from_slice(data);
    }
    for (address, len, ok, byte_off) in other_spans {
        if ok {
            out_bytes.extend_from_slice(&bytes[byte_off..byte_off + len as usize]);
        }
        out_spans.push((address, len, ok));
    }
    Some((out_spans, out_bytes))
}

fn build_witness(
    spans: &[(usize, u32, bool)],
    bytes: &[u8],
    id: u64,
    team: (u64, bool),
) -> Option<Witness> {
    let normalized = engine_allocation::advisory(|| normalize(spans, bytes));
    let (copied_spans, copied_bytes) = if let Some((norm_spans, norm_bytes)) = normalized {
        let copied_spans =
            engine_allocation::advisory(|| engine_allocation::boxed_slice(norm_spans)).ok()?;
        let copied_bytes =
            engine_allocation::advisory(|| engine_allocation::boxed_slice(norm_bytes)).ok()?;
        (copied_spans, copied_bytes)
    } else {
        let copied_spans = engine_allocation::advisory(|| {
            let copy = engine_allocation::copy_vec(spans)?;
            engine_allocation::boxed_slice(copy)
        })
        .ok()?;
        let copied_bytes = engine_allocation::advisory(|| {
            let copy = engine_allocation::copy_vec(bytes)?;
            engine_allocation::boxed_slice(copy)
        })
        .ok()?;
        (copied_spans, copied_bytes)
    };
    let longest = copied_spans
        .iter()
        .map(|&(_, len, _)| len as usize)
        .max()
        .unwrap_or(0);
    Some(Witness {
        id,
        spans: copied_spans,
        bytes: copied_bytes,
        longest,
        team,
        stamp: 0,
    })
}

pub(super) struct Recording<'a, M> {
    pub source: &'a M,
    spans: RefCell<Vec<(usize, u32, bool)>>,
    bytes: RefCell<Vec<u8>>,
    refused: Cell<bool>,
}
impl<'a, M> Recording<'a, M> {
    #[cfg(test)]
    pub(super) fn new(source: &'a M, (spans, bytes): (usize, usize)) -> Self {
        let (mut span_storage, mut byte_storage) = (Vec::new(), Vec::new());
        let refused = engine_allocation::reserve(&mut span_storage, spans).is_err()
            || engine_allocation::reserve(&mut byte_storage, bytes).is_err();
        Self {
            source,
            spans: RefCell::new(span_storage),
            bytes: RefCell::new(byte_storage),
            refused: Cell::new(refused),
        }
    }
    pub(super) fn with_scratch(
        source: &'a M,
        mut spans: Vec<(usize, u32, bool)>,
        mut bytes: Vec<u8>,
        (want_spans, want_bytes): (usize, usize),
    ) -> Self {
        spans.clear();
        bytes.clear();
        let refused = (spans.capacity() < want_spans
            && engine_allocation::reserve(&mut spans, want_spans).is_err())
            || (bytes.capacity() < want_bytes
                && engine_allocation::reserve(&mut bytes, want_bytes).is_err());
        Self {
            source,
            spans: RefCell::new(spans),
            bytes: RefCell::new(bytes),
            refused: Cell::new(refused),
        }
    }
    pub(super) fn finish(
        self,
        id: u64,
        team: (u64, bool),
    ) -> (Option<Witness>, Vec<(usize, u32, bool)>, Vec<u8>) {
        let spans = self.spans.into_inner();
        let bytes = self.bytes.into_inner();
        if self.refused.get() {
            return (None, spans, bytes);
        }
        let witness = build_witness(&spans, &bytes, id, team);
        (witness, spans, bytes)
    }
    #[cfg(test)]
    pub(super) fn witness(self, id: u64, team: (u64, bool)) -> Option<Witness> {
        if self.refused.get() {
            return None;
        }
        let spans = self.spans.into_inner();
        let bytes = self.bytes.into_inner();
        build_witness(&spans, &bytes, id, team)
    }
    fn record_span(
        spans: &mut Vec<(usize, u32, bool)>,
        bytes: &mut Vec<u8>,
        address: usize,
        data: &[u8],
        ok: bool,
    ) -> bool {
        let Ok(len) = u32::try_from(data.len()) else {
            return false;
        };
        let coalesces = spans.last().is_some_and(|last| {
            last.2 == ok
                && last.0.checked_add(last.1 as usize) == Some(address)
                && (last.1 as usize).saturating_add(data.len()) <= MAX_COALESCED_SPAN
        });
        if !coalesces && spans.len() == spans.capacity() {
            let more = spans.capacity().max(64);
            if engine_allocation::reserve(spans, more).is_err() {
                return false;
            }
        }
        if ok {
            if bytes.capacity() - bytes.len() < data.len() {
                let more = bytes.capacity().max(data.len()).max(1024);
                if engine_allocation::reserve(bytes, more).is_err() {
                    return false;
                }
            }
            bytes.extend_from_slice(data);
        }
        if coalesces {
            spans.last_mut().unwrap().1 += len;
        } else {
            spans.push((address, len, ok));
        }
        true
    }

    fn record_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &[&mut [u8]; N],
        completed: usize,
    ) {
        if self.refused.get() {
            return;
        }
        let (mut spans, mut bytes) = (self.spans.borrow_mut(), self.bytes.borrow_mut());
        for (index, (&address, destination)) in addresses.iter().zip(destinations.iter()).enumerate()
        {
            if index == completed {
                if !destination.is_empty()
                    && address != 0
                    && address.checked_add(destination.len()).is_some()
                    && !Self::record_span(&mut spans, &mut bytes, address, destination, false)
                {
                    self.refused.set(true);
                }
                break;
            }
            if destination.is_empty() {
                continue;
            }
            if !Self::record_span(&mut spans, &mut bytes, address, destination, true) {
                self.refused.set(true);
                break;
            }
        }
    }
}
impl<M: Memory> Memory for Recording<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ok = self.source.copy(address, output);
        if self.refused.get() {
            return ok;
        }
        let (mut spans, mut bytes) = (self.spans.borrow_mut(), self.bytes.borrow_mut());
        if !Self::record_span(&mut spans, &mut bytes, address, output, ok) {
            self.refused.set(true);
        }
        ok
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let completed = self.source.copy_scattered(addresses, destinations);
        self.record_scattered(addresses, destinations, completed);
        completed
    }

    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        let completed = self
            .source
            .copy_scattered_prefetched(addresses, destinations);
        self.record_scattered(addresses, destinations, completed);
        completed
    }

    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        let Some(total) = layout.count(sources, destination.len()) else {
            return 0;
        };
        let completed = self.source.copy_records(sources, layout, destination);
        if self.refused.get() || total == 0 || layout.fields().is_empty() {
            return completed;
        }
        let (mut spans, mut bytes) = (self.spans.borrow_mut(), self.bytes.borrow_mut());
        let fields = layout.fields();
        let stride = layout.stride();
        let mut index = 0_usize;
        'rows: for row in 0..sources.len() {
            let pointer = sources.pointer(row);
            let row_base = row * stride;
            for field in fields {
                if index == completed {
                    if field.len != 0 {
                        let address = pointer
                            .filter(|&pointer| pointer != 0)
                            .and_then(|pointer| pointer.checked_add(field.source_offset))
                            .filter(|&address| address.checked_add(field.len).is_some());
                        let start = row_base + field.destination_offset;
                        if let Some(address) = address
                            && !Self::record_span(
                                &mut spans,
                                &mut bytes,
                                address,
                                &destination[start..start + field.len],
                                false,
                            )
                        {
                            self.refused.set(true);
                        }
                    }
                    break 'rows;
                }
                if field.len != 0 {
                    let Some(address) = pointer
                        .filter(|&pointer| pointer != 0)
                        .and_then(|pointer| pointer.checked_add(field.source_offset))
                        .filter(|&address| address.checked_add(field.len).is_some())
                    else {
                        self.refused.set(true);
                        break 'rows;
                    };
                    let start = row_base + field.destination_offset;
                    if !Self::record_span(
                        &mut spans,
                        &mut bytes,
                        address,
                        &destination[start..start + field.len],
                        true,
                    ) {
                        self.refused.set(true);
                        break 'rows;
                    }
                }
                index += 1;
            }
        }
        completed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct World(std::cell::RefCell<Vec<u8>>);
    impl Memory for World {
        fn copy(&self, address: usize, output: &mut [u8]) -> bool {
            let bytes = self.0.borrow();
            match bytes.get(address..address + output.len()) {
                Some(source) => {
                    output.copy_from_slice(source);
                    true
                }
                None => false,
            }
        }
    }
    fn walk(memory: &impl Memory) {
        let mut pointer = [0; 1];
        assert!(memory.copy(0, &mut pointer));
        let mut field = [0; 4];
        memory.copy(pointer[0] as usize, &mut field);
        assert!(!memory.copy(1_000, &mut [0; 2]));
    }
    fn witness(world: &World) -> Witness {
        let recording = Recording::new(world, (0, 0));
        walk(&recording);
        recording.witness(7, (3, true)).unwrap()
    }
    #[test]
    fn a_witness_holds_while_every_read_returns_the_same_and_fails_on_any_byte_or_status_change() {
        let world = World(std::cell::RefCell::new(vec![4, 0, 0, 0, 1, 2, 3, 4]));
        let held = witness(&world);
        assert_eq!((held.id, held.team, held.size()), (7, (3, true), (3, 5)));
        let mut scratch = Vec::new();
        assert!(held.unchanged(&world, &mut scratch));
        world.0.borrow_mut()[6] = 9;
        assert!(!held.unchanged(&world, &mut scratch));
        world.0.borrow_mut()[6] = 3;
        assert!(held.unchanged(&world, &mut scratch));
        world.0.borrow_mut()[0] = 3;
        assert!(!held.unchanged(&world, &mut scratch));
        world.0.borrow_mut()[0] = 4;
        world.0.borrow_mut().resize(1_002, 0);
        assert!(!held.unchanged(&world, &mut scratch));
    }
    #[test]
    fn a_refused_recording_gives_no_witness() {
        let world = World(std::cell::RefCell::new(vec![4, 0, 0, 0, 1, 2, 3, 4]));
        let recording = Recording::new(&world, (0, 0));
        recording.refused.set(true);
        walk(&recording);
        assert!(recording.witness(7, (0, false)).is_none());
    }
    #[test]
    fn multi_span_witness_batches_reads_through_surviving_guard() {
        struct Counted {
            world: World,
            scattered: Cell<usize>,
        }
        impl Memory for Counted {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.world.copy(address, output)
            }
            fn copy_scattered_prefetched<const N: usize>(
                &self,
                addresses: [usize; N],
                destinations: &mut [&mut [u8]; N],
            ) -> usize {
                self.scattered.set(self.scattered.get() + 1);
                self.world.copy_scattered(addresses, destinations)
            }
        }
        let counted = Counted {
            world: World(std::cell::RefCell::new((0u8..64).collect())),
            scattered: Cell::new(0),
        };
        let recording = Recording::new(&counted, (0, 0));
        let mut buf = [0u8; 4];
        for offset in [4, 12, 24, 40] {
            assert!(recording.copy(offset, &mut buf));
        }
        assert!(!recording.copy(1_000, &mut buf[..2]));
        let held = recording.witness(9, (1, false)).unwrap();
        let guard = super::repair::Surviving {
            source: &counted,
            removed: &[],
            receiver: 4,
            consumed: Cell::new(false),
        };
        let mut scratch = Vec::new();
        assert!(held.unchanged(&guard, &mut scratch));
        assert_eq!(counted.scattered.get(), 1);
        counted.world.0.borrow_mut()[25] ^= 0xff;
        assert!(!held.unchanged(&guard, &mut scratch));
        counted.world.0.borrow_mut()[25] ^= 0xff;
        let removed_guard = super::repair::Surviving {
            source: &counted,
            removed: &[24],
            receiver: 4,
            consumed: Cell::new(false),
        };
        assert!(!held.unchanged(&removed_guard, &mut scratch));
        assert!(removed_guard.consumed.get());
    }
    #[test]
    fn recording_batches_scattered_and_record_reads_with_exact_witness_verification() {
        use engine_native_memory::memory::{RecordField, RecordLayout, RecordSources};
        struct BatchCounted {
            world: World,
            scattered: Cell<usize>,
            records: Cell<usize>,
        }
        impl Memory for BatchCounted {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.world.copy(address, output)
            }
            fn copy_scattered<const N: usize>(
                &self,
                addresses: [usize; N],
                destinations: &mut [&mut [u8]; N],
            ) -> usize {
                self.scattered.set(self.scattered.get() + 1);
                self.world.copy_scattered(addresses, destinations)
            }
            fn copy_scattered_prefetched<const N: usize>(
                &self,
                addresses: [usize; N],
                destinations: &mut [&mut [u8]; N],
            ) -> usize {
                self.scattered.set(self.scattered.get() + 1);
                self.world.copy_scattered_prefetched(addresses, destinations)
            }
            fn copy_records(
                &self,
                sources: RecordSources<'_>,
                layout: &RecordLayout<'_>,
                destination: &mut [u8],
            ) -> usize {
                self.records.set(self.records.get() + 1);
                self.world.copy_records(sources, layout, destination)
            }
        }
        const FIELDS: [RecordField; 2] = [
            RecordField {
                source_offset: 0,
                destination_offset: 0,
                len: 4,
            },
            RecordField {
                source_offset: 8,
                destination_offset: 4,
                len: 4,
            },
        ];
        const LAYOUT: RecordLayout<'static> = RecordLayout::new(&FIELDS, 8).unwrap();
        let counted = BatchCounted {
            world: World(std::cell::RefCell::new((0u8..96).collect())),
            scattered: Cell::new(0),
            records: Cell::new(0),
        };
        let recording = Recording::new(&counted, (0, 0));
        let mut a = [0u8; 4];
        let mut b = [0u8; 4];
        let mut c = [0u8; 4];
        assert_eq!(
            recording.copy_scattered([4, 20, 1_000], &mut [&mut a, &mut b, &mut c]),
            2
        );
        let mut rec_buf = [0u8; 24];
        assert_eq!(
            recording.copy_records(
                RecordSources::Pointers(&[32, 48, 2_000]),
                &LAYOUT,
                &mut rec_buf,
            ),
            4
        );
        assert_eq!(counted.scattered.get(), 1);
        assert_eq!(counted.records.get(), 1);
        let held = recording.witness(11, (2, true)).unwrap();
        let mut scratch = Vec::new();
        assert!(held.unchanged(&counted.world, &mut scratch));
        counted.world.0.borrow_mut()[57] ^= 1;
        assert!(!held.unchanged(&counted.world, &mut scratch));
        counted.world.0.borrow_mut()[57] ^= 1;
        assert!(held.unchanged(&counted.world, &mut scratch));
        counted.world.0.borrow_mut().resize(1_010, 0);
        assert!(!held.unchanged(&counted.world, &mut scratch));
    }
    #[test]
    fn witness_normalizes_out_of_order_duplicate_and_overlapping_spans() {
        let world = World(std::cell::RefCell::new((0u8..64).collect()));
        let recording = Recording::new(&world, (0, 0));
        let mut a = [0u8; 8];
        let mut b = [0u8; 8];
        let mut c = [0u8; 12];
        assert!(recording.copy(16, &mut a));
        assert!(recording.copy(8, &mut b));
        assert!(recording.copy(12, &mut c));
        assert!(recording.copy(8, &mut b));
        let held = recording.witness(15, (0, false)).unwrap();
        assert_eq!(held.size(), (1, 16));
        let mut scratch = Vec::new();
        assert!(held.unchanged(&world, &mut scratch));
        world.0.borrow_mut()[20] ^= 0xff;
        assert!(!held.unchanged(&world, &mut scratch));
        world.0.borrow_mut()[20] ^= 0xff;
        assert!(held.unchanged(&world, &mut scratch));
    }
}
