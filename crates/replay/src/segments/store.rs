//! Operators parked for the segment they go on into, and the scratch they and projected
//! chunks spill to.

use super::{Fault, Lane, Operation, Stats, caught, invalid, lane::Erased, lane::Walking, lock};
use crate::columnar_frames::Segment;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{self, Read, Write},
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

/// Where a run spills what its budgets do not hold resident: parked operators and projected
/// chunks, each read back at most once.
pub trait Scratch: Sync {
    /// Keeps `bytes` until the handle drops.
    fn spill(&self, bytes: &[u8]) -> io::Result<Box<dyn Spilled>>;
}

/// Bytes a [`Scratch`] keeps, freed when dropped.
pub trait Spilled: Send {
    /// Replaces `out` with the bytes spilled.
    fn read(&self, out: &mut Vec<u8>) -> io::Result<()>;
}

/// Spills into files of a folder no one else writes in, one per spill, each removed when its
/// handle drops.
pub struct Folder {
    path: PathBuf,
    next: AtomicU64,
}

impl Folder {
    /// Spills into `path`, created when missing.
    pub fn new(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        std::fs::create_dir_all(&path)?;
        Ok(Self {
            path,
            next: AtomicU64::new(0),
        })
    }
}

impl Scratch for Folder {
    fn spill(&self, bytes: &[u8]) -> io::Result<Box<dyn Spilled>> {
        let path = self.path.join(format!(
            "{}.spill",
            self.next.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let spilled = File {
            path,
            length: bytes.len(),
        };
        file.write_all(bytes)?;
        Ok(Box::new(spilled))
    }
}

/// A spill in a [`Folder`].
struct File {
    path: PathBuf,
    length: usize,
}

impl Spilled for File {
    fn read(&self, out: &mut Vec<u8>) -> io::Result<()> {
        out.clear();
        engine_allocation::reserve_exact(out, self.length)?;
        std::fs::File::open(&self.path)?
            .take(self.length as u64 + 1)
            .read_to_end(out)?;
        if out.len() != self.length {
            return Err(invalid("A spill reads back at another length"));
        }
        Ok(())
    }
}

impl Drop for File {
    fn drop(&mut self) {
        // A spill no longer read back; a file left behind holds no state.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A parked operator: resident with the bytes it holds, taken out to be written with those
/// bytes still counted resident (and left so when that failed, until discarded), or spilled.
enum Carry<'r, W> {
    Resident(Box<dyn Walking<W> + 'r>, usize),
    Writing(usize),
    Spilled(Box<dyn Spilled>),
}

/// A parked operator taken out to be written: its segment and family, the operator, and the
/// bytes it held.
pub(super) type Out<'r, W> = ((usize, usize), Box<dyn Walking<W> + 'r>, usize);

/// What a walk finds parked for its segment.
pub(super) enum Parked<'r, W> {
    /// The operator, resident or read back.
    Ready(Box<dyn Walking<W> + 'r>),
    /// It is being written out, or failed to be: the walk leaves it, to be taken once it
    /// lands, or closed with its family's failure.
    Writing,
}

struct Shelf<'r, W> {
    /// By the segment each goes on into, then family.
    carries: BTreeMap<(usize, usize), Carry<'r, W>>,
    resident: usize,
    /// Of those, the bytes of carries being written out.
    writing: usize,
    /// Resident carries whose operators cannot be spilled.
    pinned: BTreeSet<(usize, usize)>,
    /// The most resident at once, those being parked and written out included.
    peak: usize,
    parked: usize,
    spilled: usize,
    loaded: usize,
}

impl<'r, W> Shelf<'r, W> {
    /// Takes out, to be written, the furthest resident carries from their segment that are not
    /// pinned, for those left resident and not being written to fit `limit`. Past it with
    /// none to take out, the run fails, naming the families that cannot spill.
    fn evict(&mut self, limit: usize, lanes: &[Lane<'_, W>]) -> io::Result<Vec<Out<'r, W>>> {
        let mut excess = (self.resident - self.writing).saturating_sub(limit);
        if excess == 0 {
            return Ok(Vec::new());
        }
        let mut keys = Vec::new();
        for (key, carry) in self.carries.iter().rev() {
            if excess == 0 {
                break;
            }
            if let Carry::Resident(_, bytes) = carry
                && !self.pinned.contains(key)
            {
                engine_allocation::reserve(&mut keys, 1)?;
                keys.push(*key);
                excess = excess.saturating_sub(*bytes);
            }
        }
        if keys.is_empty() {
            let pinned: BTreeSet<&str> = self
                .pinned
                .iter()
                .map(|&(_, lane)| lanes[lane].erased.name())
                .collect();
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                format!(
                    "Operators parked for later segments exceed the continuation budget of \
                     {limit} bytes, and these families cannot spill theirs: {pinned:?}"
                ),
            ));
        }
        // Allocated before any is taken out: an operator never drops while the store is held.
        let mut out = engine_allocation::vec(keys.len())?;
        for key in keys {
            let Some(Carry::Resident(operator, bytes)) = self.carries.remove(&key) else {
                unreachable!("the carry is resident");
            };
            self.carries.insert(key, Carry::Writing(bytes));
            self.writing += bytes;
            out.push((key, operator, bytes));
        }
        Ok(out)
    }
}

/// Bytes of carries being written out or read back, at once and at most.
#[derive(Default)]
struct Scratched {
    now: AtomicUsize,
    peak: AtomicUsize,
}

impl Scratched {
    /// Counts `raw`'s allocation until the count drops.
    fn hold(&self, raw: &Vec<u8>) -> Counted<'_> {
        let bytes = raw.capacity();
        let now = self.now.fetch_add(bytes, Ordering::SeqCst) + bytes;
        self.peak.fetch_max(now, Ordering::SeqCst);
        Counted {
            scratched: self,
            bytes,
        }
    }
}

struct Counted<'s> {
    scratched: &'s Scratched,
    bytes: usize,
}

impl Drop for Counted<'_> {
    fn drop(&mut self) {
        self.scratched.now.fetch_sub(self.bytes, Ordering::SeqCst);
    }
}

/// Operators parked for the segment they go on into: resident within a budget, the
/// furthest from their segment spilled past it. No family's code runs while it is locked:
/// operators are written out, read back and dropped once it is let go.
pub(super) struct Store<'r, W> {
    shelf: Mutex<Shelf<'r, W>>,
    limit: usize,
    scratched: Scratched,
}

impl<'r, W> Store<'r, W> {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            shelf: Mutex::new(Shelf {
                carries: BTreeMap::new(),
                resident: 0,
                writing: 0,
                pinned: BTreeSet::new(),
                peak: 0,
                parked: 0,
                spilled: 0,
                loaded: 0,
            }),
            limit,
            scratched: Scratched::default(),
        }
    }

    /// Parks `operator` of family `lane`, holding `bytes`, for segment `at`, taking out at once
    /// the parked operators to write out ([`Self::write`]) for the rest to fit the budget.
    pub(super) fn park(
        &self,
        at: usize,
        lane: usize,
        operator: Box<dyn Walking<W> + 'r>,
        bytes: usize,
        lanes: &[Lane<'_, W>],
    ) -> io::Result<Vec<Out<'r, W>>> {
        let parked = {
            let mut held = lock(&self.shelf);
            let shelf = &mut *held;
            if shelf.carries.contains_key(&(at, lane)) {
                Err(operator)
            } else {
                shelf
                    .carries
                    .insert((at, lane), Carry::Resident(operator, bytes));
                shelf.resident = shelf.resident.saturating_add(bytes);
                shelf.peak = shelf.peak.max(shelf.resident);
                shelf.parked += 1;
                Ok(shelf.evict(self.limit, lanes))
            }
        };
        match parked {
            Ok(out) => out,
            Err(operator) => {
                drop(operator);
                Err(invalid(format!(
                    "{} parked two operators for segment {at}",
                    lanes[lane].erased.name()
                )))
            }
        }
    }

    /// Takes out the parked operators to write out for those left to fit the budget, as
    /// [`Self::park`] does: none once they fit.
    pub(super) fn evict(&self, lanes: &[Lane<'_, W>]) -> io::Result<Vec<Out<'r, W>>> {
        lock(&self.shelf).evict(self.limit, lanes)
    }

    /// Writes out the operators taken out, `out`, spilling each to `scratch`, or keeping it
    /// resident when it cannot be: the families whose operator failed as it was written, each
    /// with its segment, which fail. One whose family no longer walks that segment drops.
    /// One that failed stays out of reach as being written, to every walk, those holding its
    /// segment already included, until its family's failure, recorded, closes that segment
    /// and discards it ([`Self::discard`]).
    pub(super) fn write(
        &self,
        out: Vec<Out<'r, W>>,
        scratch: &dyn Scratch,
    ) -> io::Result<Vec<(usize, usize, Fault)>> {
        let mut failed = Vec::new();
        for (key, operator, bytes) in out {
            let mut raw = Vec::new();
            let saved = caught(Operation::Save, || operator.save(&mut raw));
            let _counted = self.scratched.hold(&raw);
            let written = match saved {
                Ok(true) => {
                    drop(operator);
                    Written::Spilled(scratch.spill(&raw)?)
                }
                Ok(false) => Written::Kept(operator),
                // Its family's own failure: its operator goes, and the family with it.
                Err(fault) => {
                    drop(operator);
                    engine_allocation::reserve(&mut failed, 1)?;
                    failed.push((key.0, key.1, fault));
                    continue;
                }
            };
            drop(self.land(key, bytes, written));
        }
        Ok(failed)
    }

    /// Puts back carry `key`, of `bytes`, as `written` left it: what to drop once the store is
    /// let go, when it was discarded meanwhile.
    fn land(
        &self,
        key: (usize, usize),
        bytes: usize,
        written: Written<'r, W>,
    ) -> Option<Written<'r, W>> {
        let mut held = lock(&self.shelf);
        let shelf = &mut *held;
        // Discarded while written out, its bytes given back then.
        if !matches!(shelf.carries.get(&key), Some(Carry::Writing(_))) {
            return Some(written);
        }
        shelf.writing -= bytes;
        match written {
            Written::Spilled(spilled) => {
                shelf.carries.insert(key, Carry::Spilled(spilled));
                shelf.resident -= bytes;
                shelf.spilled += 1;
            }
            Written::Kept(operator) => {
                shelf.carries.insert(key, Carry::Resident(operator, bytes));
                shelf.pinned.insert(key);
            }
        }
        None
    }

    /// Drops the operator of family `lane` parked for segment `at`, if any: it will not walk
    /// that segment.
    pub(super) fn discard(&self, at: usize, lane: usize) {
        let carry = {
            let mut held = lock(&self.shelf);
            let shelf = &mut *held;
            shelf.pinned.remove(&(at, lane));
            let carry = shelf.carries.remove(&(at, lane));
            match &carry {
                Some(Carry::Resident(_, bytes)) => shelf.resident -= bytes,
                Some(Carry::Writing(bytes)) => {
                    shelf.resident -= bytes;
                    shelf.writing -= bytes;
                }
                Some(Carry::Spilled(_)) | None => {}
            }
            carry
        };
        drop(carry);
    }

    /// The operator of family `lane` parked for segment `at`, if any: a spilled one read back
    /// into a new operator of `erased`, the family's.
    pub(super) fn take(
        &self,
        at: usize,
        lane: usize,
        segment: &Segment,
        erased: &'r (dyn Erased<W> + 'r),
    ) -> Result<Option<Parked<'r, W>>, Fault> {
        let spilled = {
            let mut held = lock(&self.shelf);
            let shelf = &mut *held;
            if let Some(Carry::Writing(_)) = shelf.carries.get(&(at, lane)) {
                return Ok(Some(Parked::Writing));
            }
            shelf.pinned.remove(&(at, lane));
            match shelf.carries.remove(&(at, lane)) {
                None => return Ok(None),
                Some(Carry::Resident(operator, bytes)) => {
                    shelf.resident -= bytes;
                    return Ok(Some(Parked::Ready(operator)));
                }
                Some(Carry::Spilled(spilled)) => {
                    shelf.loaded += 1;
                    spilled
                }
                Some(Carry::Writing(_)) => unreachable!("a carry being written is left"),
            }
        };
        let mut raw = Vec::new();
        spilled.read(&mut raw)?;
        drop(spilled);
        let _counted = self.scratched.hold(&raw);
        let mut operator = caught(Operation::Make, || erased.fresh(segment))?;
        caught(Operation::Load, || operator.load(&raw))?;
        Ok(Some(Parked::Ready(operator)))
    }

    /// Refuses an operator no walk took: its family decided otherwise at a segment's end
    /// than at the start of the segment after.
    pub(super) fn finish(&self, lanes: &[Lane<'_, W>]) -> io::Result<()> {
        let left = lock(&self.shelf).carries.keys().next().copied();
        match left {
            Some((at, lane)) => Err(invalid(format!(
                "{} parked an operator for segment {at}, which started without it",
                lanes[lane].erased.name()
            ))),
            None => Ok(()),
        }
    }

    pub(super) fn count(&self, stats: &mut Stats) {
        let shelf = lock(&self.shelf);
        stats.carry_bytes = shelf.peak;
        stats.carry_scratch = self.scratched.peak.load(Ordering::SeqCst);
        stats.parked = shelf.parked;
        stats.spilled = shelf.spilled;
        stats.loaded = shelf.loaded;
    }
}

/// What writing out a parked operator left of it.
enum Written<'r, W> {
    Spilled(Box<dyn Spilled>),
    /// It cannot be spilled, and stays resident.
    Kept(Box<dyn Walking<W> + 'r>),
}
