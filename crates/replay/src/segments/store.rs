//! Operators parked for the segment they go on into, and the scratch they and projected
//! chunks spill to.

use super::{Lane, Stats, invalid, lane::Erased, lane::Walking, lock};
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

/// A parked operator: resident with the bytes it holds, or spilled.
enum Carry<'r> {
    Resident(Box<dyn Walking + 'r>, usize),
    Spilled(Box<dyn Spilled>),
}

#[derive(Default)]
struct Parked<'r> {
    /// By the segment each goes on into, then family.
    carries: BTreeMap<(usize, usize), Carry<'r>>,
    resident: usize,
    /// Resident carries whose operators cannot be spilled.
    pinned: BTreeSet<(usize, usize)>,
    /// The most resident at once, one being parked before the furthest spill included.
    peak: usize,
    parked: usize,
    spilled: usize,
    loaded: usize,
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
/// furthest from their segment spilled past it.
pub(super) struct Store<'r> {
    parked: Mutex<Parked<'r>>,
    limit: usize,
    scratched: Scratched,
}

impl<'r> Store<'r> {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            parked: Mutex::default(),
            limit,
            scratched: Scratched::default(),
        }
    }

    /// Parks `operator` of family `lane` for segment `at`.
    pub(super) fn park(
        &self,
        at: usize,
        lane: usize,
        operator: Box<dyn Walking + 'r>,
        lanes: &[Lane<'_>],
        scratch: &dyn Scratch,
    ) -> io::Result<()> {
        let name = |lane: usize| lanes[lane].0.name().to_owned();
        let bytes = operator.carried().ok_or_else(|| {
            invalid(format!(
                "{} cannot be parked for segment {at}: its operator does not say what it holds",
                name(lane)
            ))
        })?;
        let mut held = lock(&self.parked);
        let parked = &mut *held;
        if parked
            .carries
            .insert((at, lane), Carry::Resident(operator, bytes))
            .is_some()
        {
            return Err(invalid(format!(
                "{} parked two operators for segment {at}",
                name(lane)
            )));
        }
        parked.resident = parked.resident.saturating_add(bytes);
        parked.peak = parked.peak.max(parked.resident);
        parked.parked += 1;
        while parked.resident > self.limit {
            let victim = parked
                .carries
                .iter()
                .rev()
                .find(|&(key, carry)| {
                    matches!(carry, Carry::Resident(..)) && !parked.pinned.contains(key)
                })
                .map(|(key, _)| *key);
            let Some(key) = victim else {
                let pinned: BTreeSet<String> =
                    parked.pinned.iter().map(|&(_, lane)| name(lane)).collect();
                return Err(io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    format!(
                        "Operators parked for later segments exceed the continuation budget of \
                         {} bytes, and these families cannot spill theirs: {pinned:?}",
                        self.limit
                    ),
                ));
            };
            let Some(Carry::Resident(operator, bytes)) = parked.carries.remove(&key) else {
                unreachable!("the victim is resident");
            };
            let mut raw = Vec::new();
            let saved = operator.save(&mut raw);
            let _counted = self.scratched.hold(&raw);
            if saved? {
                drop(operator);
                parked
                    .carries
                    .insert(key, Carry::Spilled(scratch.spill(&raw)?));
                parked.resident -= bytes;
                parked.spilled += 1;
            } else {
                parked.carries.insert(key, Carry::Resident(operator, bytes));
                parked.pinned.insert(key);
            }
        }
        Ok(())
    }

    /// The operator of family `lane` parked for segment `at`, if any: a spilled one read back
    /// into a new operator of `erased`, the family's.
    pub(super) fn take(
        &self,
        at: usize,
        lane: usize,
        segment: &Segment,
        erased: &'r (dyn Erased + 'r),
    ) -> io::Result<Option<Box<dyn Walking + 'r>>> {
        let spilled = {
            let mut held = lock(&self.parked);
            let parked = &mut *held;
            parked.pinned.remove(&(at, lane));
            match parked.carries.remove(&(at, lane)) {
                None => return Ok(None),
                Some(Carry::Resident(operator, bytes)) => {
                    parked.resident -= bytes;
                    return Ok(Some(operator));
                }
                Some(Carry::Spilled(spilled)) => {
                    parked.loaded += 1;
                    spilled
                }
            }
        };
        let mut raw = Vec::new();
        spilled.read(&mut raw)?;
        drop(spilled);
        let _counted = self.scratched.hold(&raw);
        let mut operator = erased.fresh(segment)?;
        operator.load(&raw)?;
        Ok(Some(operator))
    }

    /// Refuses an operator no walk took: its family decided otherwise at a segment's end
    /// than at the start of the segment after.
    pub(super) fn finish(&self, lanes: &[Lane<'_>]) -> io::Result<()> {
        match lock(&self.parked).carries.keys().next() {
            Some(&(at, lane)) => Err(invalid(format!(
                "{} parked an operator for segment {at}, which started without it",
                lanes[lane].0.name()
            ))),
            None => Ok(()),
        }
    }

    pub(super) fn count(&self, stats: &mut Stats) {
        let parked = lock(&self.parked);
        stats.carry_bytes = parked.peak;
        stats.carry_scratch = self.scratched.peak.load(Ordering::SeqCst);
        stats.parked = parked.parked;
        stats.spilled = parked.spilled;
        stats.loaded = parked.loaded;
    }
}
