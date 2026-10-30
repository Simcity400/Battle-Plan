//! A family's operators and emit, their types erased for the run, and the chunks its walks
//! seal for its emit.

use super::{Chunk, Hour, Item, Operator, Scratch, Settings, Signal, Sink, Spilled, Start, lock};
use crate::columnar_frames::Segment;
use std::{
    cell::Cell,
    collections::{BTreeMap, VecDeque},
    io,
    mem::size_of,
    sync::{
        Mutex, PoisonError, TryLockError,
        atomic::{AtomicUsize, Ordering},
    },
};

pub(super) type Make<'a, O> = Box<dyn Fn(&Segment) -> io::Result<O> + Sync + 'a>;
pub(super) type Emit<'a, T> = Box<dyn FnMut(Chunk<'_, T>) -> io::Result<()> + Send + 'a>;

/// Projected bytes held resident, as walks project and emits drop them, and, with those,
/// what walks reserved as they started and have not projected yet.
#[derive(Default)]
pub(super) struct Credit {
    held: AtomicUsize,
    committed: AtomicUsize,
    peak: AtomicUsize,
}

impl Credit {
    /// Takes `bytes` projected: the bytes held now.
    pub(super) fn take(&self, bytes: usize) -> usize {
        self.committed.fetch_add(bytes, Ordering::SeqCst);
        let held = self.held.fetch_add(bytes, Ordering::SeqCst) + bytes;
        self.peak.fetch_max(held, Ordering::SeqCst);
        held
    }

    pub(super) fn give(&self, bytes: usize) {
        self.held.fetch_sub(bytes, Ordering::SeqCst);
        self.committed.fetch_sub(bytes, Ordering::SeqCst);
    }

    pub(super) fn held(&self) -> usize {
        self.held.load(Ordering::SeqCst)
    }

    /// The bytes held, and those reserved.
    pub(super) fn committed(&self) -> usize {
        self.committed.load(Ordering::SeqCst)
    }

    /// Reserves `bytes` for a walk to project.
    pub(super) fn reserve(&self, bytes: usize) -> Reserved<'_> {
        self.committed.fetch_add(bytes, Ordering::SeqCst);
        Reserved {
            credit: self,
            left: Cell::new(bytes),
        }
    }

    /// Takes `bytes` about to be held, given back once dropped.
    pub(super) fn hold(&self, bytes: usize) -> Taken<'_> {
        self.take(bytes);
        Taken {
            credit: self,
            bytes,
        }
    }

    /// The `bytes` held already, given back once dropped.
    fn adopt(&self, bytes: usize) -> Taken<'_> {
        Taken {
            credit: self,
            bytes,
        }
    }

    pub(super) fn peak(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }
}

/// What a walk reserved as it started: spent as it projects, taken back as a chunk of items
/// that spill closes, the rest given back once dropped.
pub(super) struct Reserved<'c> {
    credit: &'c Credit,
    left: Cell<usize>,
}

impl Reserved<'_> {
    /// Spends up to `bytes` of what is left: the bytes spent.
    fn spend(&self, bytes: usize) -> usize {
        let spent = bytes.min(self.left.get());
        self.left.set(self.left.get() - spent);
        self.credit.committed.fetch_sub(spent, Ordering::SeqCst);
        spent
    }

    /// Takes back `bytes` it spent on a chunk that closed, for the walk's next.
    fn refill(&self, bytes: usize) {
        self.left.set(self.left.get() + bytes);
        self.credit.committed.fetch_add(bytes, Ordering::SeqCst);
    }
}

impl Drop for Reserved<'_> {
    fn drop(&mut self) {
        self.credit
            .committed
            .fetch_sub(self.left.get(), Ordering::SeqCst);
    }
}

/// Bytes held of the output budget, given back once dropped.
pub(super) struct Taken<'c> {
    credit: &'c Credit,
    bytes: usize,
}

impl Taken<'_> {
    /// Holds `bytes` instead.
    fn resize(&mut self, bytes: usize) {
        if bytes > self.bytes {
            self.credit.take(bytes - self.bytes);
        } else {
            self.credit.give(self.bytes - bytes);
        }
        self.bytes = bytes;
    }
}

impl Drop for Taken<'_> {
    fn drop(&mut self) {
        self.credit.give(self.bytes);
    }
}

/// What a run's families share: the output budget and the scratch.
pub(super) struct Shared<'r> {
    pub(super) credit: Credit,
    pub(super) settings: Settings,
    pub(super) scratch: &'r dyn Scratch,
    pub(super) chunks_spilled: AtomicUsize,
    signal: &'r Signal,
}

impl<'r> Shared<'r> {
    pub(super) fn new(settings: Settings, scratch: &'r dyn Scratch, signal: &'r Signal) -> Self {
        Self {
            credit: Credit::default(),
            settings,
            scratch,
            chunks_spilled: AtomicUsize::new(0),
            signal,
        }
    }
}

/// A walk's view of what its families share: the segment it walks, and what it reserved.
#[derive(Clone, Copy)]
pub(super) struct Context<'c> {
    pub(super) shared: &'c Shared<'c>,
    pub(super) at: usize,
    pub(super) reserved: &'c Reserved<'c>,
}

/// How far an emit went through a segment's chunks.
pub(super) enum Emitted {
    /// Through its last.
    Ended,
    /// To a spilled chunk, which reading back takes these bytes.
    Spilled(usize),
}

/// A family, its operator's and items' types erased.
pub(super) trait Erased: Sync {
    fn name(&self) -> &str;

    /// Whether its items spill.
    fn spills(&self) -> bool;

    /// A new operator for `segment`.
    fn fresh<'s>(&'s self, segment: &Segment) -> io::Result<Box<dyn Walking + 's>>;

    /// Hands segment `at`'s sealed chunks to the emit, in order, from the first not yet
    /// handed: a spilled one is read back into `restore`, taken for it, and the next
    /// spilled one stops it.
    fn emit(
        &self,
        at: usize,
        segment: &Segment,
        shared: &Shared<'_>,
        restore: Option<Taken<'_>>,
    ) -> io::Result<Emitted>;
}

/// A family's operator as a walk holds it, with what it projected of the segment.
pub(super) trait Walking: Send {
    fn starts_at(&self, start: Start<'_>) -> bool;
    fn start(&mut self, start: Start<'_>, branch: u64, first: i32) -> io::Result<()>;
    fn cross(&mut self, first: i32) -> io::Result<()>;
    fn hour(&mut self, hour: &Hour<'_>, context: &Context<'_>) -> io::Result<()>;
    fn end(&mut self, context: &Context<'_>) -> io::Result<()>;
    /// Hands what it projected to its family's emit as `context`'s segment's: the bytes it
    /// projected there, spilled or not.
    fn seal(&mut self, context: &Context<'_>) -> io::Result<usize>;
    /// The bytes it holds parked; None when its operator does not say.
    fn carried(&self) -> Option<usize>;
    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool>;
    fn load(&mut self, bytes: &[u8]) -> io::Result<()>;
}

/// A chunk of items, resident with the bytes it holds, or spilled with the bytes reading it
/// back takes: its spill's, and its items' as they were held.
enum Stored<T> {
    Resident(Vec<T>, usize),
    Spilled(Box<dyn Spilled>, usize),
}

/// The bytes `items` hold resident: their allocation, and what each owns.
fn resident<T: Item>(items: &Vec<T>) -> usize {
    items.iter().fold(
        items.capacity().saturating_mul(size_of::<T>()),
        |sum, item| sum.saturating_add(item.heap()),
    )
}

pub(super) struct Typed<'a, O: Operator> {
    name: &'a str,
    make: Make<'a, O>,
    emit: Mutex<Emit<'a, O::Output>>,
    /// Each sealed segment's chunks not yet emitted.
    sealed: Mutex<BTreeMap<usize, VecDeque<Stored<O::Output>>>>,
}

impl<'a, O: Operator> Typed<'a, O> {
    pub(super) fn new(name: &'a str, make: Make<'a, O>, emit: Emit<'a, O::Output>) -> Self {
        Self {
            name,
            make,
            emit: Mutex::new(emit),
            sealed: Mutex::new(BTreeMap::new()),
        }
    }
}

impl<'a, O: Operator + 'a> Erased for Typed<'a, O> {
    fn name(&self) -> &str {
        self.name
    }

    fn spills(&self) -> bool {
        O::Output::SPILLS
    }

    fn fresh<'s>(&'s self, segment: &Segment) -> io::Result<Box<dyn Walking + 's>> {
        Ok(Box::new(Walk {
            lane: self,
            operator: (self.make)(segment)?,
            chunks: Chunks {
                open: Vec::new(),
                held: 0,
                drawn: 0,
                produced: 0,
                closed: Vec::new(),
            },
        }))
    }

    fn emit(
        &self,
        at: usize,
        segment: &Segment,
        shared: &Shared<'_>,
        mut restore: Option<Taken<'_>>,
    ) -> io::Result<Emitted> {
        let mut chunks = lock(&self.sealed).remove(&at).ok_or_else(|| {
            io::Error::other(format!("{} sealed nothing of segment {at}", self.name))
        })?;
        // The schedule hands a family's emit to one worker at a time: never wait for it here.
        let mut emit = match self.emit.try_lock() {
            Ok(emit) => emit,
            Err(TryLockError::Poisoned(poisoned)) => PoisonError::into_inner(poisoned),
            Err(TryLockError::WouldBlock) => {
                return Err(io::Error::other(format!(
                    "{} emitted two segments at once",
                    self.name
                )));
            }
        };
        let emit = &mut *emit;
        if chunks.is_empty() {
            // A segment that projected nothing still ends.
            emit(Chunk {
                segment,
                items: Vec::new(),
                last: true,
            })?;
            return Ok(Emitted::Ended);
        }
        while let Some(chunk) = chunks.pop_front() {
            let (items, held) = match chunk {
                Stored::Resident(items, bytes) => (items, shared.credit.adopt(bytes)),
                Stored::Spilled(spilled, need) => {
                    let Some(mut taken) = restore.take() else {
                        chunks.push_front(Stored::Spilled(spilled, need));
                        lock(&self.sealed).insert(at, chunks);
                        return Ok(Emitted::Spilled(need));
                    };
                    let mut raw = Vec::new();
                    spilled.read(&mut raw)?;
                    drop(spilled);
                    let items = O::Output::load(&raw)?;
                    let held = resident(&items);
                    // Admitted for the read buffer and the bytes the chunk held when it spilled.
                    if held > need.saturating_sub(raw.len()) {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("{} read back more than it held when it spilled", self.name),
                        ));
                    }
                    drop(raw);
                    taken.resize(held);
                    (items, taken)
                }
            };
            let last = chunks.is_empty();
            emit(Chunk {
                segment,
                items,
                last,
            })?;
            drop(held);
        }
        Ok(Emitted::Ended)
    }
}

/// The chunks a walk fills for its family's emit: the one being filled, the bytes it holds
/// resident and those of them it drew from the walk's reservation, the bytes projected into
/// the segment's chunks so far, and those closed.
struct Chunks<T> {
    open: Vec<T>,
    held: usize,
    drawn: usize,
    produced: usize,
    closed: Vec<Stored<T>>,
}

impl<T: Item> Chunks<T> {
    /// Closes the chunk being filled, of whichever segment: spilled when its items spill and
    /// the budget is overdrawn. True when it spilled.
    fn close(&mut self, context: &Context<'_>) -> io::Result<bool> {
        let drawn = std::mem::take(&mut self.drawn);
        if self.open.is_empty() {
            return Ok(false);
        }
        engine_allocation::reserve(&mut self.closed, 1)?;
        let items = std::mem::take(&mut self.open);
        let bytes = std::mem::take(&mut self.held);
        let shared = context.shared;
        if T::SPILLS {
            // A walk holds a chunk of them at a time: the next takes what this one drew.
            context.reserved.refill(drawn);
            if shared.credit.held() > shared.settings.output_bytes {
                let mut raw = Vec::new();
                T::save(&items, &mut raw)?;
                let spilled = shared.scratch.spill(&raw)?;
                let need = raw.len().saturating_add(bytes);
                drop(items);
                shared.credit.give(bytes);
                shared.chunks_spilled.fetch_add(1, Ordering::SeqCst);
                self.closed.push(Stored::Spilled(spilled, need));
                shared.signal.changed();
                return Ok(true);
            }
        }
        self.closed.push(Stored::Resident(items, bytes));
        Ok(false)
    }
}

/// What an operator pushes into during a walk: each item counted against the output budget,
/// the allocation it grows included, and the chunk closed once full or past the working
/// allowance.
struct Filling<'f, 'c, T> {
    name: &'f str,
    chunks: &'f mut Chunks<T>,
    context: &'f Context<'c>,
}

impl<T: Item> Sink<T> for Filling<'_, '_, T> {
    fn push(&mut self, item: T) -> io::Result<()> {
        let chunks = &mut *self.chunks;
        let shared = self.context.shared;
        let ceiling = shared.settings.ceiling();
        let (capacity, heap) = (chunks.open.capacity(), item.heap());
        Sink::push(&mut chunks.open, item)?;
        let grown = (chunks.open.capacity() - capacity)
            .saturating_mul(size_of::<T>())
            .saturating_add(heap);
        let held = shared.credit.take(grown);
        chunks.drawn = chunks
            .drawn
            .saturating_add(self.context.reserved.spend(grown));
        chunks.held = chunks.held.saturating_add(grown);
        chunks.produced = chunks.produced.saturating_add(grown);
        if (held > ceiling || chunks.held >= shared.settings.chunk_bytes)
            && !chunks.close(self.context)?
            && shared.credit.held() > ceiling
        {
            return Err(io::Error::new(
                io::ErrorKind::OutOfMemory,
                format!(
                    "Projected items held resident exceed the output budget of {} bytes and \
                     its working allowance of {} bytes, and {}'s cannot spill",
                    shared.settings.output_bytes, shared.settings.working_bytes, self.name
                ),
            ));
        }
        Ok(())
    }
}

/// A family's operator in a walk, with the chunks it projected of the segment.
struct Walk<'s, 'a, O: Operator> {
    lane: &'s Typed<'a, O>,
    operator: O,
    chunks: Chunks<O::Output>,
}

impl<O: Operator> Walking for Walk<'_, '_, O> {
    fn starts_at(&self, start: Start<'_>) -> bool {
        self.operator.starts_at(start)
    }

    fn start(&mut self, start: Start<'_>, branch: u64, first: i32) -> io::Result<()> {
        self.operator.start(start, branch, first)
    }

    fn cross(&mut self, first: i32) -> io::Result<()> {
        self.operator.cross(first)
    }

    fn hour(&mut self, hour: &Hour<'_>, context: &Context<'_>) -> io::Result<()> {
        self.operator.hour(
            hour,
            &mut Filling {
                name: self.lane.name,
                chunks: &mut self.chunks,
                context,
            },
        )
    }

    fn end(&mut self, context: &Context<'_>) -> io::Result<()> {
        self.operator.end(&mut Filling {
            name: self.lane.name,
            chunks: &mut self.chunks,
            context,
        })
    }

    fn seal(&mut self, context: &Context<'_>) -> io::Result<usize> {
        self.chunks.close(context)?;
        let chunks = VecDeque::from(std::mem::take(&mut self.chunks.closed));
        lock(&self.lane.sealed).insert(context.at, chunks);
        Ok(std::mem::take(&mut self.chunks.produced))
    }

    fn carried(&self) -> Option<usize> {
        self.operator
            .carried()
            .map(|heap| heap.saturating_add(size_of::<O>()))
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        self.operator.save(out)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.operator.load(bytes)
    }
}
