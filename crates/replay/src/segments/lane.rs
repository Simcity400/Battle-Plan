//! A family's operators and emit, their types erased for the run, and the segments its walks
//! file for its emit, spooled ([`spool`]).

use super::{
    Carry, Chunk, Ending, Fault, Hour, Item, Operation, Operator, Sink, Start, caught, lock,
    spool::{self, Folder, Next, Readers, Spooling},
};
use crate::columnar_frames::Segment;
use engine_allocation::labels::{self, Kind, Owner};
use std::{cell::Cell, collections::BTreeMap, io, marker::PhantomData, mem::size_of, sync::Mutex};

pub(super) type Make<'a, O> = Box<dyn Fn(&Segment) -> io::Result<O> + Sync + 'a>;
pub(super) type Emit<'a, T> = Box<dyn FnMut(Chunk<'_, T>) -> io::Result<()> + Send + 'a>;

/// The spooled bytes of a segment's items an emit is handed at once.
const CHUNK_BYTES: usize = 256 << 10;

/// A walk's view of what its families share, on one worker: the spool, and the family whose
/// code runs there, which the run's own code it calls back into clears.
#[derive(Clone, Copy)]
pub(super) struct Context<'c> {
    pub(super) folder: &'c Folder,
    pub(super) culprit: &'c Cell<Option<usize>>,
}

/// A family, its operator's and items' types erased.
pub(super) trait Erased<W>: Sync {
    fn name(&self) -> &str;

    /// A new operator of family `lane` for `segment`, writing in `lineage`, begun again, when
    /// it is one read back from disk, else in a new one.
    fn fresh<'s>(
        &'s self,
        lane: usize,
        segment: &Segment,
        folder: &Folder,
        lineage: Option<u64>,
    ) -> io::Result<Box<dyn Walking<W> + 's>>;

    /// Hands segment `at`'s filed items to the emit, in order, the last chunk with why the
    /// segment ended short: why it did.
    fn emit(&self, at: usize, segment: &Segment) -> Result<Option<Ending>, Fault>;

    /// Files segment `at` with no items, ending short.
    fn settle(&self, at: usize, ending: Ending);

    /// Drops what segment `at` filed.
    fn discard(&self, at: usize);
}

/// A family's operator as a walk holds it, with what it spooled of the segment.
pub(super) trait Walking<W>: Send {
    fn starts_at(&self, start: Start<'_>) -> bool;
    fn start(&mut self, start: Start<'_>, branch: u64, first: i32) -> io::Result<()>;
    fn cross(&mut self, first: i32) -> io::Result<()>;
    /// Hour `hour` of segment `at`.
    fn hour(&mut self, at: usize, hour: &Hour<'_, W>, context: &Context<'_>) -> io::Result<()>;
    fn end(&mut self, at: usize, context: &Context<'_>) -> io::Result<()>;
    /// Closes what it spooled of the segment: the bytes it spooled there. A failure its items'
    /// sink met is its [`Self::fault`].
    fn close(&mut self, context: &Context<'_>) -> io::Result<usize>;
    /// Files what it closed as segment `at`'s, for its family's emit, with why it ended short,
    /// what it would hand on past the last segment of its branch, and whether its operator
    /// goes on into the segment after.
    fn file(
        &mut self,
        at: usize,
        ending: Option<Ending>,
        kept: Option<Kept>,
        goes_on: bool,
        folder: &Folder,
    );
    /// Drops what it spooled and has not filed.
    fn discard(&mut self, folder: &Folder);
    /// The failure its items' sink met, which is not its operator's own: the run's, or its
    /// items' codec's.
    fn fault(&mut self) -> Option<Fault>;
    /// The bytes it holds parked; None when its operator does not say.
    fn carried(&self) -> Option<usize>;
    /// The lineage it spools in.
    fn lineage(&self) -> u64;
    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool>;
    fn load(&mut self, bytes: &[u8]) -> io::Result<()>;
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

/// A segment's items not yet emitted, why the segment ended short, and what its operator
/// would hand on past it.
struct Filed {
    items: Option<spool::Filed>,
    ending: Option<Ending>,
    kept: Option<Kept>,
}

/// A family's emit, with what it reads its spool by: no lock is held around a family's code.
struct Emitting<'a, T> {
    emit: Emit<'a, T>,
    readers: Readers,
}

pub(super) struct Typed<'a, O: Operator<W>, W> {
    name: &'a str,
    owner: Owner,
    make: Make<'a, O>,
    /// The emit, taken out while a worker calls it.
    emit: Mutex<Option<Emitting<'a, O::Output>>>,
    /// Each filed segment not yet emitted.
    filed: Mutex<BTreeMap<usize, Filed>>,
    world: PhantomData<fn(&W)>,
}

impl<'a, O: Operator<W>, W> Typed<'a, O, W> {
    pub(super) fn new(name: &'a str, make: Make<'a, O>, emit: Emit<'a, O::Output>) -> Self {
        Self {
            name,
            owner: labels::owner(name),
            make,
            emit: Mutex::new(Some(Emitting {
                emit,
                readers: Readers::default(),
            })),
            filed: Mutex::new(BTreeMap::new()),
            world: PhantomData,
        }
    }

    /// Files `filed` as segment `at`'s; what it would replace drops once the lock is let go.
    fn put(&self, at: usize, filed: Filed) {
        let replaced = lock(&self.filed).insert(at, filed);
        debug_assert!(replaced.is_none(), "a segment is filed once at a time");
        drop(replaced);
    }

    /// Hands segment `at`'s `filed` items to `emitting`'s emit, as [`Erased::emit`] says.
    fn hand(
        &self,
        emitting: &mut Emitting<'a, O::Output>,
        segment: &Segment,
        filed: Filed,
    ) -> Result<Option<Ending>, Fault> {
        let _memory = labels::enter(self.owner, Kind::Emit);
        let Filed {
            mut items,
            ending,
            kept,
        } = filed;
        let Emitting { emit, readers } = emitting;
        let mut unspooling = match &mut items {
            Some(items) if items.items > 0 => Some(readers.open(items).map_err(Fault::Run)?),
            Some(items) => {
                // Its lineage is read on, or forgotten, as its items would have it.
                drop(readers.open(items).map_err(Fault::Run)?);
                None
            }
            None => None,
        };
        let Some(unspooling) = unspooling.as_mut() else {
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
            return Ok(ending);
        };
        loop {
            let mut chunk = Vec::new();
            let mut read = 0;
            let _restoring = labels::kind(Kind::Restore);
            while read < CHUNK_BYTES && !unspooling.done() {
                match unspooling.next().map_err(Fault::Run)? {
                    Next::Item => {}
                    Next::End => break,
                }
                read += unspooling.read();
                let item = caught(Operation::Load, || unspooling.decode::<O::Output>())?;
                engine_allocation::reserve(&mut chunk, 1)?;
                chunk.push(item);
            }
            // What its lineage released after its last item is taken before that item's chunk.
            if unspooling.done() {
                match unspooling.next().map_err(Fault::Run)? {
                    Next::End => {}
                    Next::Item => {
                        return Err(Fault::Run(super::invalid(
                            "A spooled segment holds more than its items",
                        )));
                    }
                }
            }
            drop(_restoring);
            let last = unspooling.done();
            let emitted = caught(Operation::Emit, || {
                emit(Chunk {
                    segment,
                    items: chunk,
                    last,
                    ending: if last { ending.as_ref() } else { None },
                    carry: if last {
                        kept.as_ref().map(Kept::carry)
                    } else {
                        None
                    },
                })
            });
            emitted?;
            if last {
                return Ok(ending);
            }
        }
    }
}

impl<'a, O: Operator<W> + 'a, W> Erased<W> for Typed<'a, O, W> {
    fn name(&self) -> &str {
        self.name
    }

    fn fresh<'s>(
        &'s self,
        lane: usize,
        segment: &Segment,
        folder: &Folder,
        lineage: Option<u64>,
    ) -> io::Result<Box<dyn Walking<W> + 's>> {
        let _memory = labels::enter(self.owner, Kind::Operator);
        Ok(Box::new(Walk {
            lane,
            typed: self,
            operator: (self.make)(segment)?,
            spooling: Spooling::new(lineage.unwrap_or_else(|| folder.lineage())),
            fault: None,
        }))
    }

    fn emit(&self, at: usize, segment: &Segment) -> Result<Option<Ending>, Fault> {
        let filed = lock(&self.filed).remove(&at);
        let Some(filed) = filed else {
            return Err(Fault::Run(io::Error::other(format!(
                "{} filed nothing of segment {at}",
                self.name
            ))));
        };
        // The schedule hands a family's emit to one worker at a time: never wait for it here.
        let taken = lock(&self.emit).take();
        let Some(mut emitting) = taken else {
            return Err(Fault::Run(io::Error::other(format!(
                "{} emitted two segments at once",
                self.name
            ))));
        };
        let emitted = self.hand(&mut emitting, segment, filed);
        // An emit a panic unwound through is not called again.
        if !matches!(emitted, Err(Fault::Panic(..))) {
            *lock(&self.emit) = Some(emitting);
        }
        emitted
    }

    fn settle(&self, at: usize, ending: Ending) {
        let filed = Filed {
            items: None,
            ending: Some(ending),
            kept: None,
        };
        self.put(at, filed);
    }

    fn discard(&self, at: usize) {
        let filed = lock(&self.filed).remove(&at);
        drop(filed);
    }
}

/// What an operator pushes into during a walk of segment `at`: each item spooled as it
/// comes.
struct Filling<'f, 'c> {
    spooling: &'f mut Spooling,
    fault: &'f mut Option<Fault>,
    lane: usize,
    at: usize,
    context: &'f Context<'c>,
}

impl Filling<'_, '_> {
    /// Keeps `fault`, met spooling an item, for the walk: the operator gets its echo.
    fn met(&mut self, fault: Fault) -> io::Error {
        let echo = fault.echo();
        self.fault.get_or_insert(fault);
        echo
    }
}

impl<T: Item> Sink<T> for Filling<'_, '_> {
    fn push(&mut self, item: T) -> io::Result<()> {
        // The item's codec is its family's own code; its writing out is the run's.
        let spooling = &mut *self.spooling;
        let encoded = caught(Operation::Save, || spooling.encode(&item));
        drop(item);
        if let Err(fault) = encoded {
            return Err(self.met(fault));
        }
        let culprit = self.context.culprit.replace(None);
        let put = self.spooling.put(self.context.folder, self.lane, self.at);
        self.context.culprit.set(culprit);
        put.map_err(|error| self.met(Fault::Run(error)))
    }
}

/// A family's operator in a walk, with what it spooled of the segment.
struct Walk<'s, 'a, O: Operator<W>, W> {
    lane: usize,
    typed: &'s Typed<'a, O, W>,
    operator: O,
    spooling: Spooling,
    /// A failure its items' sink met that is not the operator's own.
    fault: Option<Fault>,
}

impl<O: Operator<W>, W> Walk<'_, '_, O, W> {
    /// Keeps `fault`, met closing what it spooled, for the walk: the operator gets its echo.
    fn met(&mut self, fault: Fault) -> io::Error {
        let echo = fault.echo();
        self.fault.get_or_insert(fault);
        echo
    }
}

impl<O: Operator<W>, W> Walking<W> for Walk<'_, '_, O, W> {
    fn starts_at(&self, start: Start<'_>) -> bool {
        self.operator.starts_at(start)
    }

    fn start(&mut self, start: Start<'_>, branch: u64, first: i32) -> io::Result<()> {
        let _memory = labels::enter(self.typed.owner, Kind::Operator);
        self.operator.start(start, branch, first)
    }

    fn cross(&mut self, first: i32) -> io::Result<()> {
        let _memory = labels::enter(self.typed.owner, Kind::Operator);
        self.operator.cross(first)
    }

    fn hour(&mut self, at: usize, hour: &Hour<'_, W>, context: &Context<'_>) -> io::Result<()> {
        let _memory = labels::enter(self.typed.owner, Kind::Operator);
        self.operator.hour(
            hour,
            &mut Filling {
                spooling: &mut self.spooling,
                fault: &mut self.fault,
                lane: self.lane,
                at,
                context,
            },
        )
    }

    fn end(&mut self, at: usize, context: &Context<'_>) -> io::Result<()> {
        let _memory = labels::enter(self.typed.owner, Kind::Operator);
        self.operator.end(&mut Filling {
            spooling: &mut self.spooling,
            fault: &mut self.fault,
            lane: self.lane,
            at,
            context,
        })
    }

    fn close(&mut self, _: &Context<'_>) -> io::Result<usize> {
        let _memory = labels::enter(self.typed.owner, Kind::Operator);
        if let Some(fault) = &self.fault {
            // An operator going on past what its sink met has its walk end there.
            return Err(fault.echo());
        }
        if let Err(error) = self.spooling.flush() {
            return Err(self.met(Fault::Run(error)));
        }
        Ok(usize::try_from(self.spooling.length()).unwrap_or(usize::MAX))
    }

    fn file(
        &mut self,
        at: usize,
        ending: Option<Ending>,
        kept: Option<Kept>,
        goes_on: bool,
        folder: &Folder,
    ) {
        let filed = Filed {
            items: Some(self.spooling.file(folder, goes_on)),
            ending,
            kept,
        };
        self.typed.put(at, filed);
    }

    fn discard(&mut self, folder: &Folder) {
        self.spooling.discard(folder);
    }

    fn fault(&mut self) -> Option<Fault> {
        self.fault.take()
    }

    fn carried(&self) -> Option<usize> {
        self.operator
            .carried()
            .map(|heap| heap.saturating_add(size_of::<O>()))
    }

    fn lineage(&self) -> u64 {
        self.spooling.lineage()
    }

    fn save(&self, out: &mut Vec<u8>) -> io::Result<bool> {
        let _memory = labels::enter(self.typed.owner, Kind::Operator);
        self.operator.save(out)
    }

    fn load(&mut self, bytes: &[u8]) -> io::Result<()> {
        let _memory = labels::enter(self.typed.owner, Kind::Operator);
        self.operator.load(bytes)
    }
}
