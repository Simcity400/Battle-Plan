//! A family's operators and emit, their types erased for the run, and the chunks its walks
//! file for its emit.

use super::{
    Carry, Chunk, Ending, Fault, Hour, Item, Operation, Operator, Scratch, Settings, Signal, Sink,
    Spilled, Start, caught, invalid, lock,
};
use crate::columnar_frames::Segment;
use std::{
    cell::Cell,
    collections::{BTreeMap, VecDeque},
    io,
    marker::PhantomData,
    mem::size_of,
    sync::{
        Mutex,
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

/// A walk's view of what its families share: what it reserved, and the family whose code runs,
/// which the run's own code it calls back into clears.
#[derive(Clone, Copy)]
pub(super) struct Context<'c> {
    pub(super) shared: &'c Shared<'c>,
    pub(super) reserved: &'c Reserved<'c>,
    pub(super) culprit: &'c Cell<Option<usize>>,
}

/// How far an emit went through a segment's chunks.
pub(super) enum Emitted {
    /// Through its last, which ended the segment short as it says.
    Ended(Option<Ending>),
    /// To a spilled chunk, which reading back takes these bytes.
    Spilled(usize),
}

/// A family, its operator's and items' types erased.
pub(super) trait Erased<W>: Sync {
    fn name(&self) -> &str;

    /// Whether its items spill.
    fn spills(&self) -> bool;

    /// A new operator for `segment`.
    fn fresh<'s>(&'s self, segment: &Segment) -> io::Result<Box<dyn Walking<W> + 's>>;

    /// Hands segment `at`'s filed chunks to the emit, in order, from the first not yet
    /// handed, the last with why the segment ended short: a spilled one is read back into
    /// `restore`, taken for it, and the next spilled one stops it.
    fn emit(
        &self,
        at: usize,
        segment: &Segment,
        shared: &Shared<'_>,
        restore: Option<Taken<'_>>,
    ) -> Result<Emitted, Fault>;

    /// Files segment `at` with no items, ending short.
    fn settle(&self, at: usize, ending: Ending);

    /// Drops what segment `at` filed, giving its resident bytes back to `credit`.
    fn discard(&self, at: usize, credit: &Credit);
}

/// A family's operator as a walk holds it, with what it projected of the segment.
pub(super) trait Walking<W>: Send {
    fn starts_at(&self, start: Start<'_>) -> bool;
    fn start(&mut self, start: Start<'_>, branch: u64, first: i32) -> io::Result<()>;
    fn cross(&mut self, first: i32) -> io::Result<()>;
    fn hour(&mut self, hour: &Hour<'_, W>, context: &Context<'_>) -> io::Result<()>;
    fn end(&mut self, context: &Context<'_>) -> io::Result<()>;
    /// Closes what it projected of `context`'s segment: the bytes it projected there, spilled
    /// or not. A failure its items' sink met is its [`Self::fault`].
    fn close(&mut self, context: &Context<'_>) -> io::Result<usize>;
    /// Files what it closed as segment `at`'s, for its family's emit, with why it ended short
    /// and what it would hand on past the last segment of its branch.
    fn file(&mut self, at: usize, ending: Option<Ending>, kept: Option<Kept>);
    /// Drops what it projected and has not filed, giving its resident bytes back to `credit`.
    fn discard(&mut self, credit: &Credit);
    /// The failure its items' sink met, which is not its operator's own: the run's, or its
    /// items' spill's.
    fn fault(&mut self) -> Option<Fault>;
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

/// Drops `chunks`, giving the bytes those resident hold back to `credit`.
fn release<T>(chunks: impl IntoIterator<Item = Stored<T>>, credit: &Credit) {
    for chunk in chunks {
        if let Stored::Resident(_, bytes) = chunk {
            credit.give(bytes);
        }
    }
}

/// What a family's operator would hand on past the last segment of its branch.
pub(super) enum Kept {
    Saved(Vec<u8>),
    Unsaved,
}

impl Kept {
    fn carry(&self) -> Carry<'_> {
        match self {
            Self::Saved(bytes) => Carry::Saved(bytes),
            Self::Unsaved => Carry::Unsaved,
        }
    }
}

/// A segment's chunks not yet emitted, why the segment ended short, and what its operator
/// would hand on past it.
struct Filed<T> {
    chunks: VecDeque<Stored<T>>,
    ending: Option<Ending>,
    kept: Option<Kept>,
}

pub(super) struct Typed<'a, O: Operator<W>, W> {
    name: &'a str,
    make: Make<'a, O>,
    /// The emit, taken out while a worker calls it: no lock is held around a family's code.
    emit: Mutex<Option<Emit<'a, O::Output>>>,
    /// Each filed segment's chunks not yet emitted.
    filed: Mutex<BTreeMap<usize, Filed<O::Output>>>,
    world: PhantomData<fn(&W)>,
}

impl<'a, O: Operator<W>, W> Typed<'a, O, W> {
    pub(super) fn new(name: &'a str, make: Make<'a, O>, emit: Emit<'a, O::Output>) -> Self {
        Self {
            name,
            make,
            emit: Mutex::new(Some(emit)),
            filed: Mutex::new(BTreeMap::new()),
            world: PhantomData,
        }
    }

    /// Files `filed` as segment `at`'s; what it would replace drops once the lock is let go.
    fn put(&self, at: usize, filed: Filed<O::Output>) {
        let replaced = lock(&self.filed).insert(at, filed);
        debug_assert!(replaced.is_none(), "a segment is filed once at a time");
        drop(replaced);
    }

    /// Hands segment `at`'s `filed` chunks to `emit`, as [`Erased::emit`] says.
    fn hand(
        &self,
        emit: &mut Emit<'a, O::Output>,
        at: usize,
        segment: &Segment,
        shared: &Shared<'_>,
        mut restore: Option<Taken<'_>>,
        filed: Filed<O::Output>,
    ) -> Result<Emitted, Fault> {
        let Filed {
            mut chunks,
            ending,
            kept,
        } = filed;
        if chunks.is_empty() {
            // A segment that projected nothing still ends.
            caught(Operation::Emit, || {
                emit(Chunk {
                    segment,
                    items: Vec::new(),
                    last: true,
                    ending: ending.as_ref(),
                    carry: kept.as_ref().map(Kept::carry),
                })
            })?;
            return Ok(Emitted::Ended(ending));
        }
        while let Some(chunk) = chunks.pop_front() {
            let (items, held) = match chunk {
                Stored::Resident(items, bytes) => (items, shared.credit.adopt(bytes)),
                Stored::Spilled(spilled, need) => {
                    let Some(mut taken) = restore.take() else {
                        chunks.push_front(Stored::Spilled(spilled, need));
                        self.put(
                            at,
                            Filed {
                                chunks,
                                ending,
                                kept,
                            },
                        );
                        return Ok(Emitted::Spilled(need));
                    };
                    let mut raw = Vec::new();
                    if let Err(error) = spilled.read(&mut raw) {
                        release(chunks, &shared.credit);
                        return Err(Fault::Run(error));
                    }
                    drop(spilled);
                    let loaded = caught(Operation::Load, || {
                        let items = O::Output::load(&raw)?;
                        let held = resident(&items);
                        Ok((items, held))
                    });
                    let (items, held) = match loaded {
                        Ok(loaded) => loaded,
                        Err(fault) => {
                            release(chunks, &shared.credit);
                            return Err(fault);
                        }
                    };
                    // Admitted for the read buffer and the bytes the chunk held when it spilled.
                    if held > need.saturating_sub(raw.len()) {
                        release(chunks, &shared.credit);
                        return Err(Fault::Run(invalid(format!(
                            "{} read back more than it held when it spilled",
                            self.name
                        ))));
                    }
                    drop(raw);
                    taken.resize(held);
                    (items, taken)
                }
            };
            let last = chunks.is_empty();
            let closing = if last { ending.as_ref() } else { None };
            let carry = if last {
                kept.as_ref().map(Kept::carry)
            } else {
                None
            };
            let emitted = caught(Operation::Emit, || {
                emit(Chunk {
                    segment,
                    items,
                    last,
                    ending: closing,
                    carry,
                })
            });
            drop(held);
            if let Err(fault) = emitted {
                release(chunks, &shared.credit);
                return Err(fault);
            }
        }
        Ok(Emitted::Ended(ending))
    }
}

impl<'a, O: Operator<W> + 'a, W> Erased<W> for Typed<'a, O, W> {
    fn name(&self) -> &str {
        self.name
    }

    fn spills(&self) -> bool {
        O::Output::SPILLS
    }

    fn fresh<'s>(&'s self, segment: &Segment) -> io::Result<Box<dyn Walking<W> + 's>> {
        Ok(Box::new(Walk {
            lane: self,
            operator: (self.make)(segment)?,
            chunks: Chunks {
                open: Vec::new(),
                held: 0,
                drawn: 0,
                produced: 0,
                closed: Vec::new(),
                fault: None,
            },
        }))
    }

    fn emit(
        &self,
        at: usize,
        segment: &Segment,
        shared: &Shared<'_>,
        restore: Option<Taken<'_>>,
    ) -> Result<Emitted, Fault> {
        let filed = lock(&self.filed).remove(&at);
        let Some(filed) = filed else {
            return Err(Fault::Run(io::Error::other(format!(
                "{} filed nothing of segment {at}",
                self.name
            ))));
        };
        // The schedule hands a family's emit to one worker at a time: never wait for it here.
        let taken = lock(&self.emit).take();
        let Some(mut emit) = taken else {
            release(filed.chunks, &shared.credit);
            return Err(Fault::Run(io::Error::other(format!(
                "{} emitted two segments at once",
                self.name
            ))));
        };
        let emitted = self.hand(&mut emit, at, segment, shared, restore, filed);
        // An emit a panic unwound through is not called again.
        if !matches!(emitted, Err(Fault::Panic(..))) {
            *lock(&self.emit) = Some(emit);
        }
        emitted
    }

    fn settle(&self, at: usize, ending: Ending) {
        let filed = Filed {
            chunks: VecDeque::new(),
            ending: Some(ending),
            kept: None,
        };
        self.put(at, filed);
    }

    fn discard(&self, at: usize, credit: &Credit) {
        let filed = lock(&self.filed).remove(&at);
        if let Some(filed) = filed {
            release(filed.chunks, credit);
        }
    }
}

/// The chunks a walk fills for its family's emit: the one being filled, the bytes it holds
/// resident and those of them it drew from the walk's reservation, the bytes projected into
/// the segment's chunks so far, those closed, and a failure the filling met that is not the
/// operator's own.
struct Chunks<T> {
    open: Vec<T>,
    held: usize,
    drawn: usize,
    produced: usize,
    closed: Vec<Stored<T>>,
    fault: Option<Fault>,
}

impl<T: Item> Chunks<T> {
    /// Closes the chunk being filled, of whichever segment: spilled when its items spill and
    /// the budget is overdrawn. True when it spilled.
    fn close(&mut self, context: &Context<'_>) -> Result<bool, Fault> {
        let drawn = std::mem::take(&mut self.drawn);
        if self.open.is_empty() {
            return Ok(false);
        }
        engine_allocation::reserve(&mut self.closed, 1)?;
        let shared = context.shared;
        if T::SPILLS {
            // A walk holds a chunk of them at a time: the next takes what this one drew.
            context.reserved.refill(drawn);
            if shared.credit.held() > shared.settings.output_bytes {
                let mut raw = Vec::new();
                // The items stay held until they are written out, by their family's code.
                caught(Operation::Save, || T::save(&self.open, &mut raw))?;
                // The scratch is the run's own: its panic is not the family's.
                let culprit = context.culprit.replace(None);
                let spilled = shared.scratch.spill(&raw);
                context.culprit.set(culprit);
                let spilled = spilled?;
                let bytes = std::mem::take(&mut self.held);
                let need = raw.len().saturating_add(bytes);
                drop(std::mem::take(&mut self.open));
                shared.credit.give(bytes);
                shared.chunks_spilled.fetch_add(1, Ordering::SeqCst);
                self.closed.push(Stored::Spilled(spilled, need));
                shared.signal.changed();
                return Ok(true);
            }
        }
        let items = std::mem::take(&mut self.open);
        let bytes = std::mem::take(&mut self.held);
        self.closed.push(Stored::Resident(items, bytes));
        Ok(false)
    }

    /// Keeps `fault`, met filling the chunks, for the walk: the operator gets its echo.
    fn met(&mut self, fault: Fault) -> io::Error {
        let echo = fault.echo();
        self.fault.get_or_insert(fault);
        echo
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
        // The run's own failure, kept before the item it cannot hold drops.
        if let Err(error) = engine_allocation::reserve(&mut chunks.open, 1) {
            let echo = chunks.met(Fault::Run(error));
            drop(item);
            return Err(echo);
        }
        chunks.open.push(item);
        let grown = (chunks.open.capacity() - capacity)
            .saturating_mul(size_of::<T>())
            .saturating_add(heap);
        let held = shared.credit.take(grown);
        chunks.drawn = chunks
            .drawn
            .saturating_add(self.context.reserved.spend(grown));
        chunks.held = chunks.held.saturating_add(grown);
        chunks.produced = chunks.produced.saturating_add(grown);
        if held > ceiling || chunks.held >= shared.settings.chunk_bytes {
            let spilled = match chunks.close(self.context) {
                Ok(spilled) => spilled,
                Err(fault) => return Err(chunks.met(fault)),
            };
            if !spilled && shared.credit.held() > ceiling {
                let error = io::Error::new(
                    io::ErrorKind::OutOfMemory,
                    format!(
                        "Projected items held resident exceed the output budget of {} bytes and \
                         its working allowance of {} bytes, and {}'s cannot spill",
                        shared.settings.output_bytes, shared.settings.working_bytes, self.name
                    ),
                );
                return Err(chunks.met(Fault::Run(error)));
            }
        }
        Ok(())
    }
}

/// A family's operator in a walk, with the chunks it projected of the segment.
struct Walk<'s, 'a, O: Operator<W>, W> {
    lane: &'s Typed<'a, O, W>,
    operator: O,
    chunks: Chunks<O::Output>,
}

impl<O: Operator<W>, W> Walking<W> for Walk<'_, '_, O, W> {
    fn starts_at(&self, start: Start<'_>) -> bool {
        self.operator.starts_at(start)
    }

    fn start(&mut self, start: Start<'_>, branch: u64, first: i32) -> io::Result<()> {
        self.operator.start(start, branch, first)
    }

    fn cross(&mut self, first: i32) -> io::Result<()> {
        self.operator.cross(first)
    }

    fn hour(&mut self, hour: &Hour<'_, W>, context: &Context<'_>) -> io::Result<()> {
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

    fn close(&mut self, context: &Context<'_>) -> io::Result<usize> {
        if let Some(fault) = &self.chunks.fault {
            // An operator going on past what its sink met has its walk end there.
            return Err(fault.echo());
        }
        if let Err(fault) = self.chunks.close(context) {
            return Err(self.chunks.met(fault));
        }
        Ok(std::mem::take(&mut self.chunks.produced))
    }

    fn file(&mut self, at: usize, ending: Option<Ending>, kept: Option<Kept>) {
        let filed = Filed {
            chunks: VecDeque::from(std::mem::take(&mut self.chunks.closed)),
            ending,
            kept,
        };
        self.lane.put(at, filed);
    }

    fn discard(&mut self, credit: &Credit) {
        drop(std::mem::take(&mut self.chunks.open));
        credit.give(std::mem::take(&mut self.chunks.held));
        release(std::mem::take(&mut self.chunks.closed), credit);
        self.chunks.drawn = 0;
        self.chunks.produced = 0;
    }

    fn fault(&mut self) -> Option<Fault> {
        self.chunks.fault.take()
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
