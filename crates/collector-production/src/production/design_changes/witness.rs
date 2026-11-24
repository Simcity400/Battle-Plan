use super::*;
use std::cell::{Cell, RefCell};

pub(super) const MAX_WITNESS_BYTES: usize = 12 * 1024 * 1024;
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

static WITNESSES: Mutex<Option<HashMap<usize, Witness>>> = Mutex::new(None);
fn witnesses() -> std::sync::MutexGuard<'static, Option<HashMap<usize, Witness>>> {
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
    let held = held.get_or_insert_with(HashMap::new);
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
            held.shrink_to_fit();
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
        let longest = spans.iter().map(|&(_, len, _)| len as usize).max().unwrap_or(0);
        let witness = (|| {
            let copied_spans = engine_allocation::advisory(|| {
                let copy = engine_allocation::copy_vec(&spans)?;
                engine_allocation::boxed_slice(copy)
            })
            .ok()?;
            let copied_bytes = engine_allocation::advisory(|| {
                let copy = engine_allocation::copy_vec(&bytes)?;
                engine_allocation::boxed_slice(copy)
            })
            .ok()?;
            Some(Witness {
                id,
                spans: copied_spans,
                bytes: copied_bytes,
                longest,
                team,
                stamp: 0,
            })
        })();
        (witness, spans, bytes)
    }
    #[cfg(test)]
    pub(super) fn witness(self, id: u64, team: (u64, bool)) -> Option<Witness> {
        if self.refused.get() {
            return None;
        }
        let spans = self.spans.into_inner();
        let bytes = self.bytes.into_inner();
        let longest = spans.iter().map(|&(_, len, _)| len as usize).max().unwrap_or(0);
        let spans = engine_allocation::advisory(|| engine_allocation::boxed_slice(spans)).ok()?;
        let bytes = engine_allocation::advisory(|| engine_allocation::boxed_slice(bytes)).ok()?;
        Some(Witness {
            id,
            spans,
            bytes,
            longest,
            team,
            stamp: 0,
        })
    }
}
impl<M: Memory> Memory for Recording<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ok = self.source.copy(address, output);
        if self.refused.get() {
            return ok;
        }
        let (mut spans, mut bytes) = (self.spans.borrow_mut(), self.bytes.borrow_mut());
        let held = u32::try_from(output.len()).ok().and_then(|len| {
            let merged = spans.last_mut().filter(|last| {
                last.2 == ok
                    && last.0.checked_add(last.1 as usize) == Some(address)
                    && (last.1 as usize).saturating_add(output.len()) <= MAX_COALESCED_SPAN
            });
            if merged.is_none() && spans.len() == spans.capacity() {
                let more = spans.capacity().max(64);
                engine_allocation::reserve(&mut spans, more).ok()?;
            }
            if ok {
                if bytes.capacity() - bytes.len() < output.len() {
                    let more = bytes.capacity().max(output.len()).max(1024);
                    engine_allocation::reserve(&mut bytes, more).ok()?;
                }
                bytes.extend_from_slice(output);
            }
            if let Some(last) = spans.last_mut().filter(|last| {
                last.2 == ok
                    && last.0.checked_add(last.1 as usize) == Some(address)
                    && (last.1 as usize).saturating_add(output.len()) <= MAX_COALESCED_SPAN
            }) {
                last.1 += len;
            } else {
                spans.push((address, len, ok));
            }
            Some(())
        });
        if held.is_none() {
            self.refused.set(true);
        }
        ok
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
}
