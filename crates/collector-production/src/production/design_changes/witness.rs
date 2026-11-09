//! A marked design's read witness: every span its last complete walk read, with the bytes or
//! the failure each returned. The walk is a function of those reads alone: each address it
//! reads comes from a constant site or from bytes an earlier read returned. So when every span
//! reads back the same, in order, the walk would read the same spans and return the same rows,
//! pointers and layout included, and the design needs no walk, decode or effect.
use super::*;
use std::cell::{Cell, RefCell};

/// One design's witness, keyed by its pointer in [`WITNESSES`].
pub(super) struct Witness {
    pub id: u64,
    /// Address, length and whether the read succeeded; a successful read's bytes follow in order.
    spans: Vec<(usize, u32, bool)>,
    bytes: Vec<u8>,
    longest: usize,
    /// The design's own team and its liveness, which its read re-asserts every hour.
    pub team: (u64, bool),
}
impl Witness {
    /// Whether every span reads back as recorded.
    pub(super) fn unchanged(&self, memory: &impl Memory, scratch: &mut Vec<u8>) -> bool {
        if scratch.len() < self.longest {
            let short = self.longest - scratch.len();
            if engine_allocation::advisory(|| engine_allocation::reserve(scratch, short)).is_err() {
                return false;
            }
            scratch.resize(self.longest, 0);
        }
        let mut at = 0;
        for &(address, len, ok) in &self.spans {
            let (len, read) = (len as usize, &mut scratch[..len as usize]);
            if memory.copy(address, read) != ok {
                return false;
            }
            if ok {
                if self.bytes.get(at..at + len) != Some(&*read) {
                    return false;
                }
                at += len;
            }
        }
        at == self.bytes.len()
    }
    /// The span and byte counts, to presize the next recording of this design.
    pub(super) fn size(&self) -> (usize, usize) {
        (self.spans.len(), self.bytes.len())
    }
}

/// The marked designs' witnesses; only the drain and its lanes touch them.
static WITNESSES: Mutex<Option<HashMap<usize, Witness>>> = Mutex::new(None);
fn witnesses() -> std::sync::MutexGuard<'static, Option<HashMap<usize, Witness>>> {
    WITNESSES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
/// The witnesses of `watches` that name the same design, moved out for the lanes; none for a
/// receiver retired this hour (`removed`), whose held state its tombstone drops. Every watched
/// receiver's witness leaves the map, so only this boundary's walks can put one back.
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
/// Keeps each witness as its pointer's; a refused allocation only loses the shortcut.
pub(super) fn keep_all(kept: Vec<(usize, Witness)>) {
    let mut held = witnesses();
    let held = held.get_or_insert_with(HashMap::new);
    if engine_allocation::advisory(|| engine_allocation::reserve_map(held, kept.len())).is_ok() {
        held.extend(kept);
    }
}
pub(super) fn remove(pointer: usize) {
    if let Some(held) = witnesses().as_mut() {
        held.remove(&pointer);
    }
}
/// Drops every witness: the held state they prove may be replaced without their designs' walks.
pub(super) fn release() {
    drop(witnesses().take());
}

/// Records every read a walk makes through it.
pub(super) struct Recording<'a, M> {
    pub source: &'a M,
    spans: RefCell<Vec<(usize, u32, bool)>>,
    bytes: RefCell<Vec<u8>>,
    refused: Cell<bool>,
}
impl<'a, M> Recording<'a, M> {
    /// Presized to `(spans, bytes)`, the design's last witness, when it has one.
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
    /// The walk's witness; none when a span could not be held or one is too long to name.
    pub(super) fn witness(self, id: u64, team: (u64, bool)) -> Option<Witness> {
        let spans = self.spans.into_inner();
        let longest = spans.iter().map(|&(_, len, _)| len as usize).max().unwrap_or(0);
        (!self.refused.get()).then(|| Witness {
            id,
            spans,
            bytes: self.bytes.into_inner(),
            longest,
            team,
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
        // Storage doubles as it fills, so a long walk reserves a handful of times.
        let held = u32::try_from(output.len()).ok().and_then(|len| {
            if spans.len() == spans.capacity() {
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
            spans.push((address, len, ok));
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
    /// A walk-like read: a pointer at 0, then 4 bytes where it points, then an unreadable span.
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
        // A field byte the walk read.
        world.0.borrow_mut()[6] = 9;
        assert!(!held.unchanged(&world, &mut scratch));
        world.0.borrow_mut()[6] = 3;
        assert!(held.unchanged(&world, &mut scratch));
        // A pointer the walk followed: the layout moved, so its first read differs.
        world.0.borrow_mut()[0] = 3;
        assert!(!held.unchanged(&world, &mut scratch));
        world.0.borrow_mut()[0] = 4;
        // A read that failed now succeeds.
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
}
