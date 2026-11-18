//! Hour products the map views of one source share. Each is keyed by everything it reads, so a
//! view takes only what its own perspective would have built; the first view to ask builds it,
//! the others take it, and it lives while any view holds an hour built with it.
//!
//! A product is claimed only by a running builder and its build never waits, so a view that
//! finds it in flight waits on running work only, and does so without a thread: the builder
//! delivers to it when the product is ready.

use super::*;
use std::any::Any;

/// A shared product a built hour keeps alive, so a later view of the same hour finds it.
pub type Held = Arc<dyn Any + Send + Sync>;

/// An error every view of a product reports, kept as its kind and wording.
type Refusal = (io::ErrorKind, String);

fn refusal(error: &io::Error) -> Refusal {
    (error.kind(), error.to_string())
}

fn raise((kind, message): &Refusal) -> io::Error {
    io::Error::new(*kind, message.clone())
}

type Waiter<V> = Box<dyn FnOnce(io::Result<Arc<V>>) + Send>;

enum State<V> {
    Building(Vec<Waiter<V>>),
    Built(Result<Arc<V>, Refusal>),
}

/// One product of one hour while a view builds it.
pub(super) struct Cell<V> {
    state: Mutex<State<V>>,
}

impl<V> Cell<V> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::Building(Vec::new())),
        })
    }

    /// Runs `then` with the product: now when it is built, else on the builder's thread.
    pub(super) fn then(&self, then: impl FnOnce(io::Result<Arc<V>>) + Send + 'static) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        match &mut *state {
            State::Building(waiters) => waiters.push(Box::new(then)),
            State::Built(value) => {
                let value = value.clone().map_err(|error| raise(&error));
                drop(state);
                then(value);
            }
        }
    }

    fn fill(&self, value: Result<Arc<V>, Refusal>) {
        let waiters = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            match std::mem::replace(&mut *state, State::Built(value.clone())) {
                State::Building(waiters) => waiters,
                built @ State::Built(_) => {
                    *state = built;
                    return;
                }
            }
        };
        for waiter in waiters {
            waiter(value.clone().map_err(|error| raise(&error)));
        }
    }
}

/// A product found for a view: built, or in another view's builder.
pub(super) enum Found<V> {
    Built(Arc<V>),
    Building(Arc<Cell<V>>),
}

struct Entry<K, V> {
    key: K,
    at: (BranchId, Hour),
    /// Live while a builder holds it, so views asking meanwhile join it.
    cell: Weak<Cell<V>>,
    /// Live while a held hour keeps the product.
    built: Weak<V>,
}

/// One kind of shared product, by key and hour.
pub(super) struct Cells<K, V> {
    entries: Mutex<Vec<Entry<K, V>>>,
}

impl<K, V> Default for Cells<K, V> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(Vec::new()),
        }
    }
}

impl<K: PartialEq, V> Cells<K, V> {
    fn lock(&self) -> MutexGuard<'_, Vec<Entry<K, V>>> {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        entries.retain(|entry| entry.cell.strong_count() != 0 || entry.built.strong_count() != 0);
        entries
    }

    fn found(entries: &[Entry<K, V>], key: &K, at: (BranchId, Hour)) -> Option<Found<V>> {
        entries
            .iter()
            .filter(|entry| entry.at == at && entry.key == *key)
            .find_map(|entry| {
                entry
                    .built
                    .upgrade()
                    .map(Found::Built)
                    .or_else(|| entry.cell.upgrade().map(Found::Building))
            })
    }

    /// The product of `key` at `at`, built or in a builder. Never waits.
    pub(super) fn peek(&self, key: &K, at: (BranchId, Hour)) -> Option<Found<V>> {
        Self::found(&self.lock(), key, at)
    }

    /// [`Self::peek`], else a claim: the caller builds it, and views asking meanwhile join it.
    pub(super) fn claim(&self, key: K, at: (BranchId, Hour)) -> Result<Found<V>, Claim<'_, K, V>> {
        let mut entries = self.lock();
        if let Some(found) = Self::found(&entries, &key, at) {
            return Ok(found);
        }
        let cell = Cell::new();
        entries.push(Entry {
            key,
            at,
            cell: Arc::downgrade(&cell),
            built: Weak::new(),
        });
        Err(Claim {
            cells: self,
            at,
            cell,
            done: false,
        })
    }

    /// Built values of `key` at the nearest earlier and later hours, which an equal value shares.
    pub(super) fn settled(&self, key: &K, (branch, hour): (BranchId, Hour)) -> Vec<Arc<V>> {
        let entries = self.lock();
        let mut earlier: Option<(Hour, Arc<V>)> = None;
        let mut later: Option<(Hour, Arc<V>)> = None;
        for entry in entries.iter() {
            let (at_branch, at_hour) = entry.at;
            if at_branch != branch || at_hour == hour || entry.key != *key {
                continue;
            }
            let Some(value) = entry.built.upgrade() else {
                continue;
            };
            let slot = if at_hour < hour {
                &mut earlier
            } else {
                &mut later
            };
            let nearer = slot
                .as_ref()
                .is_none_or(|(held, _)| at_hour.abs_diff(hour) < held.abs_diff(hour));
            if nearer {
                *slot = Some((at_hour, value));
            }
        }
        earlier
            .into_iter()
            .chain(later)
            .map(|(_, value)| value)
            .collect()
    }
}

/// A view's claim on a product it builds. Dropped unfinished, as by a panic, it refuses the
/// product, so no view waits for ever.
pub(super) struct Claim<'a, K, V> {
    cells: &'a Cells<K, V>,
    at: (BranchId, Hour),
    cell: Arc<Cell<V>>,
    done: bool,
}

impl<K: PartialEq, V> Claim<'_, K, V> {
    /// Records what the build made and hands it to every view waiting for it.
    pub(super) fn finish(mut self, value: io::Result<Arc<V>>) -> io::Result<Arc<V>> {
        if let Ok(built) = &value {
            let mut entries = self.cells.lock();
            if let Some(entry) = entries.iter_mut().find(|entry| {
                entry.at == self.at && std::ptr::eq(entry.cell.as_ptr(), Arc::as_ptr(&self.cell))
            }) {
                entry.built = Arc::downgrade(built);
            }
        }
        self.done = true;
        self.cell
            .fill(value.as_ref().map(Arc::clone).map_err(refusal));
        value
    }
}

impl<K, V> Drop for Claim<'_, K, V> {
    fn drop(&mut self) {
        if !self.done {
            self.cell.fill(Err((
                io::ErrorKind::Other,
                "the hour builder panicked".to_owned(),
            )));
        }
    }
}

/// An `Arc` compared by identity: a key part that only the same value matches.
pub(super) struct Same<T>(pub Arc<T>);

impl<T> Clone for Same<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T> PartialEq for Same<T> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// What the political part reads beyond its hour.
#[derive(Clone, PartialEq)]
pub(super) struct TerritoryKey {
    pub mode: viewer_world::map::map_mode::MapMode,
    /// The viewing country.
    pub ours: u32,
    /// The country the mode colours relations from: the diplomacy selection, else `ours`.
    pub selected: u32,
    /// A scripted mode's colours as the preceding hour left them.
    pub colour: Option<Same<viewer_world::map::map_mode::ColourState>>,
}

/// An hour's political fill and the border buckets drawn over it.
pub(super) struct Territory {
    pub political: Arc<Political>,
    pub borders: Arc<Borders>,
}

/// An hour's building field and the type tokens it refused.
pub(super) struct BuildingField {
    pub field: Arc<Shared<PreparedBuildings>>,
    pub warnings: Vec<String>,
}

/// Every shared product of one content and replay.
pub struct HourStore {
    /// The content and replay its products are built from; none for one view's own store.
    source: Option<(Weak<Content>, Weak<Replay>)>,
    pub(super) memos: Arc<Memos>,
    pub(super) territory: Cells<TerritoryKey, Territory>,
    pub(super) names: Cells<viewer_world::map::map_mode::MapMode, Placements>,
    /// Rail geometry is uploaded on the builder, so a device is part of its key.
    pub(super) rails: Cells<Option<eframe::wgpu::Device>, Rails>,
    pub(super) buildings: Cells<perspective::Choice, BuildingField>,
    pub(super) fog: Cells<perspective::Choice, FogPart>,
    pub(super) snow: Cells<(), SnowMud>,
}

impl HourStore {
    /// A store the views of `content` and `replay` share.
    pub(super) fn shared(content: &Arc<Content>, replay: &Arc<Replay>) -> Arc<Self> {
        Self::with_source(Some((Arc::downgrade(content), Arc::downgrade(replay))))
    }

    /// A store of one view's own.
    pub(super) fn private() -> Arc<Self> {
        Self::with_source(None)
    }

    fn with_source(source: Option<(Weak<Content>, Weak<Replay>)>) -> Arc<Self> {
        Arc::new(Self {
            source,
            memos: Arc::new(Memos::new()),
            territory: Cells::default(),
            names: Cells::default(),
            rails: Cells::default(),
            buildings: Cells::default(),
            fog: Cells::default(),
            snow: Cells::default(),
        })
    }

    pub(super) fn serves(&self, content: &Arc<Content>, replay: &Arc<Replay>) -> bool {
        self.source
            .as_ref()
            .is_some_and(|(held_content, held_replay)| {
                held_content
                    .upgrade()
                    .is_some_and(|held| Arc::ptr_eq(&held, content))
                    && held_replay
                        .upgrade()
                        .is_some_and(|held| Arc::ptr_eq(&held, replay))
            })
    }
}

/// Where a part arriving later goes: the target that asked for it.
#[derive(Clone)]
pub(super) struct Deliver {
    pub events: Sender<Event>,
    pub ticket: u64,
    pub epoch: u64,
    pub branch: BranchId,
    pub hour: Hour,
    pub name: &'static str,
    pub trace: trace::TargetTrace,
}

impl Deliver {
    pub(super) fn send(&self, part: Part, held: Option<Held>) {
        self.trace.finish(self.name, "shared");
        let _ = self.events.send(Event::Part {
            ticket: self.ticket,
            epoch: self.epoch,
            branch: self.branch,
            hour: self.hour,
            name: self.name,
            timing: PartTiming {
                ms: 0.0,
                cpu_ms: Some(0.0),
            },
            children: Vec::new(),
            part: Box::new(part),
            held,
        });
    }
}

/// Makes a view's part from a shared product. `wait` says whether it may wait on the view's
/// own neighbouring hours; a delivery on another view's builder never does.
pub(super) type Adopt<V> = Arc<dyn Fn(io::Result<Arc<V>>, bool) -> Part + Send + Sync>;

/// What a part's work hands back: the part, or nothing when a shared product delivers it later.
pub(super) enum Delivery {
    Now(Part, Option<Held>),
    Later,
}

impl From<Part> for Delivery {
    fn from(part: Part) -> Self {
        Self::Now(part, None)
    }
}

fn held<V: Send + Sync + 'static>(value: &io::Result<Arc<V>>) -> Option<Held> {
    value.as_ref().ok().map(|value| Arc::clone(value) as Held)
}

/// Delivers `adopt`'s part once `cell` is built, on the builder's thread; a panic still delivers.
pub(super) fn deliver_when<V: Send + Sync + 'static>(
    cell: &Cell<V>,
    deliver: Deliver,
    adopt: Adopt<V>,
) {
    cell.then(move |value| {
        let kept = held(&value);
        let part = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| adopt(value, false)))
            .unwrap_or_else(|_| Part::failed(deliver.name));
        deliver.send(part, kept);
    });
}

/// A part a view's builder takes from `cells`: built, joined, or built here with `build`.
pub(super) fn take_shared<K: PartialEq, V: Send + Sync + 'static>(
    cells: &Cells<K, V>,
    key: K,
    at: (BranchId, Hour),
    deliver: Deliver,
    adopt: Adopt<V>,
    build: impl FnOnce() -> io::Result<Arc<V>>,
) -> Delivery {
    let value = match cells.claim(key, at) {
        Ok(Found::Built(value)) => Ok(value),
        Ok(Found::Building(cell)) => {
            deliver_when(&cell, deliver, adopt);
            return Delivery::Later;
        }
        Err(claim) => claim.finish(build()),
    };
    let kept = held(&value);
    Delivery::Now(adopt(value, true), kept)
}

/// The values of the neighbouring slots: waited for, or only those already there.
pub(super) fn near_values<T>(slots: &[Arc<Slot<T>>], wait: bool) -> Vec<Arc<T>> {
    slots
        .iter()
        .filter(|slot| wait || slot.filled())
        .filter_map(|slot| slot.wait())
        .collect()
}
