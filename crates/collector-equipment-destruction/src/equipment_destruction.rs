use collector_hooks::hook::Hook;
type Clock = engine_native::cgame_state::Clock<u64>;
use collector_primitives::spill_stack::{Reserved, SpillStack};
use engine_events::equipment_destruction as native;
use engine_native_memory::Memory;
use engine_native_parts::equipment_entry;
use native::SiteId;
mod queue_index;
mod transfers;
use queue_index::FixedIndex;
use std::{
    borrow::Cow,
    cell::Cell,
    collections::HashMap,
    ops::Range,
    sync::{
        LazyLock, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};
use transfers::{Credits, Net, PoolState};

type Row = equipment_entry::Row;
type Read<T> = Result<T, &'static str>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    pub country: u32,
    pub cause: native::Cause,
    pub disposition: native::Disposition,
    pub expeditionary_owner: u32,
    pub division_handle: u64,
    pub variant: u64,
    pub amount_raw: i64,
}

#[derive(Clone, Copy, serde::Serialize)]
struct Refusal {
    site_index: Option<usize>,
    reason: &'static str,
}

fn record_refusal(site_index: Option<usize>, reason: &'static str) -> Refusal {
    let refusal = Refusal { site_index, reason };
    let _ = FIRST_REFUSAL.set(refusal);
    if let Some(site) = site_index {
        SITE_REFUSALS[site].fetch_add(1, Ordering::Relaxed);
    }
    REFUSALS.fetch_add(1, Ordering::Relaxed);
    refusal
}

#[derive(Clone, Copy, Default)]
struct HeldRow {
    destination: usize,
    country: u32,
    row: Row,
}

#[derive(Clone, Copy)]
struct Scope {
    id: u64,
    army: usize,
    handle: u64,
    country: u32,
    ignored: bool,
    merge_source: bool,
    observed: bool,
}
impl Scope {
    fn ignored(army: usize) -> Self {
        Self {
            id: next_id(),
            army,
            handle: 0,
            country: 0,
            ignored: true,
            merge_source: false,
            observed: false,
        }
    }
}

struct Event {
    thread: u32,
    kind: Kind,
}

enum Kind {
    Boundary {
        transition: bool,
        load: bool,
        first: bool,
        skipped_queue: bool,
    },
    Lost,
    Refused {
        site: SiteId,
        reason: &'static str,
    },
    Revoke,
    Skipped {
        scope: Option<Scope>,
        army: Option<usize>,
        identity: Option<u64>,
    },
    SkippedQueue,
    Nested {
        site: SiteId,
        army: usize,
        scope: Scope,
        identity: Read<u64>,
    },
    Open {
        id: u64,
        site: SiteId,
    },
    Basis {
        id: u64,
        site: SiteId,
        pre: Pre,
    },
    Close {
        id: u64,
        post: Post,
    },
    Delete {
        scope: Scope,
        expeditionary_owner: u32,
        classified: Option<(native::Cause, native::Disposition)>,
    },
    Retire {
        army: usize,
        handle: Option<u64>,
        scope: Option<u64>,
    },
    Freeing {
        id: u64,
        army: usize,
    },
    Free(Box<Free>),
    Freed {
        id: u64,
        unwound: bool,
    },
    Queue {
        army: usize,
        handle: u64,
        cause: native::Cause,
        disposition: native::Disposition,
    },
    Transfer {
        site: SiteId,
        entry: Entry,
    },
}

enum Pre {
    Pool(PoolPre),
    Variant(VariantPre),
    Return(Box<ReturnPre>),
    Merge(Box<MergePre>),
}
struct PoolPre {
    cause: native::Cause,
    army: Option<usize>,
    scope: Option<Scope>,
    identity: Read<u64>,
    owner: Read<u32>,
    pool: Read<usize>,
    rows: Read<Vec<Row>>,
}
struct VariantPre {
    army: Read<usize>,
    pool: usize,
    scope: Option<Scope>,
    identity: Read<u64>,
    owner: Read<u32>,
    design: Read<u64>,
    amount: Read<i64>,
}
struct ReturnPre {
    army: Read<usize>,
    pool: usize,
    scope: Option<Scope>,
    excluded: Read<bool>,
    identity: Read<u64>,
    owner: Read<u32>,
    reserves: Read<usize>,
    reserve_owner: Read<u32>,
    sources: Read<Vec<Row>>,
    touched: Read<Vec<Row>>,
}
struct MergePre {
    excluded: Read<bool>,
    head: Read<(usize, u64)>,
    country: Read<u32>,
    armies: Read<Vec<MergeArmy>>,
    rest: Read<MergeRest>,
    pools: Vec<usize>,
}
struct MergeArmy {
    army: usize,
    rows: Vec<Row>,
    identity: u64,
    owner: Read<u32>,
}
struct MergeRest {
    scopes: Vec<MergeScope>,
}
struct MergeScope {
    pool: usize,
    main: bool,
    identity: u32,
    held: Vec<HeldRow>,
    touched: Vec<Row>,
}
impl Pre {
    fn source(&self) -> (Option<Scope>, Option<usize>, Option<u64>) {
        match self {
            Self::Pool(pre) => (pre.scope, pre.army, pre.identity.ok()),
            Self::Variant(pre) => (pre.scope, pre.army.ok(), pre.identity.ok()),
            Self::Return(pre) => (pre.scope, pre.army.ok(), pre.identity.ok()),
            Self::Merge(_) => (None, None, None),
        }
    }
    fn window(&mut self, site: SiteId) -> (Pools, bool) {
        match self {
            Self::Pool(pre) => (
                pre.pool.map_or(Pools::None, Pools::One),
                site == SiteId::ScriptedReserveEquipmentFraction,
            ),
            Self::Variant(pre) => (Pools::One(pre.pool), false),
            Self::Return(pre) => (
                pre.reserves.map_or(Pools::One(pre.pool), |reserves| {
                    Pools::Two(pre.pool, reserves)
                }),
                true,
            ),
            Self::Merge(pre) => (Pools::Many(std::mem::take(&mut pre.pools)), true),
        }
    }
}

enum Pools {
    None,
    One(usize),
    Two(usize, usize),
    Many(Vec<usize>),
}
impl Pools {
    fn contains(&self, pool: usize) -> bool {
        match self {
            Self::None => false,
            Self::One(one) => *one == pool,
            Self::Two(one, two) => *one == pool || *two == pool,
            Self::Many(pools) => pools.contains(&pool),
        }
    }
    fn shared(&self, other: &Self) -> Option<usize> {
        match other {
            Self::None => None,
            Self::One(one) => self.contains(*one).then_some(*one),
            Self::Two(one, two) => [*one, *two].into_iter().find(|&pool| self.contains(pool)),
            Self::Many(pools) => pools.iter().copied().find(|&pool| self.contains(pool)),
        }
    }
}

#[derive(Clone, Copy, serde::Serialize)]
struct Contest {
    reason: &'static str,
    site_index: usize,
    other_site_index: usize,
    pool: usize,
    country: u32,
}

enum Post {
    Unsettled,
    WorldChanged,
    Pool(Read<Vec<Row>>),
    Variant(Read<i64>),
    Return {
        identity: Read<u64>,
        touched: Read<Vec<Row>>,
    },
    Merge {
        identity: Read<u64>,
        rows: Read<Vec<Row>>,
        scopes: Vec<(Read<u32>, Read<Vec<Row>>)>,
    },
}

struct Free {
    id: u64,
    army: usize,
    direct: Option<(Scope, u32)>,
    scope: Option<Scope>,
    deleted: Option<Read<Vec<Row>>>,
    enabled: bool,
    identity: Option<u64>,
    merge_source: bool,
    observing: bool,
}

static LOG: SpillStack<Event> = SpillStack::new();

fn thread_id() -> u32 {
    unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
}
fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}
fn event(kind: Kind) -> Event {
    Event {
        thread: thread_id(),
        kind,
    }
}
fn log(kind: Kind) {
    if LOG.push(event(kind)).is_err() {
        refused_event(None);
    }
}
fn refused_event(site: Option<SiteId>) {
    STORAGE_REFUSALS.fetch_add(1, Ordering::SeqCst);
    record_refusal(site.map(|site| site as usize), EVENT_STORAGE_FAILED);
}
fn boxed<T>(value: T) -> Read<Box<T>> {
    engine_allocation::advisory(|| engine_allocation::boxed(value)).map_err(|_| EVENT_STORAGE_FAILED)
}

#[derive(Clone, Copy)]
struct Queued {
    army: usize,
    handle: u64,
    cause: native::Cause,
    disposition: native::Disposition,
}

#[derive(Clone, Copy)]
struct Deletion {
    army: usize,
    handle: u64,
    country: u32,
    cause: native::Cause,
    disposition: native::Disposition,
    expeditionary_owner: u32,
    returned: bool,
    ignored: bool,
    merge_source: bool,
}
impl Deletion {
    fn ignored(army: usize) -> Self {
        Self {
            army,
            handle: 0,
            country: 0,
            cause: native::Cause::Unknown,
            disposition: native::Disposition::Loss,
            expeditionary_owner: 0,
            returned: false,
            ignored: true,
            merge_source: false,
        }
    }
}

struct Opened {
    site: SiteId,
    thread: u32,
    pools: Pools,
    reserves: bool,
    state: Option<OpenState>,
    since: Option<u64>,
}
enum OpenState {
    Pool {
        country: u32,
        cause: native::Cause,
        rows: Vec<Row>,
    },
    Variant {
        country: u32,
        design: u64,
        before: i64,
    },
    Return {
        army: usize,
        handle: u64,
        country: u32,
        scope: Option<u64>,
        sources: Vec<Row>,
        pre: Vec<Row>,
        bookkeeping: bool,
        revision: u64,
    },
    Merge {
        country: u32,
        survivor: usize,
        handle: u64,
        scopes: Vec<(bool, u32)>,
        net: Vec<Net>,
        prior_credit: bool,
    },
}
impl OpenState {
    fn country(&self) -> u32 {
        match self {
            Self::Pool { country, .. }
            | Self::Variant { country, .. }
            | Self::Return { country, .. }
            | Self::Merge { country, .. } => *country,
        }
    }
}
struct Closed {
    at: u64,
    thread: u32,
    window: Window,
}
enum Window {
    Observation { site: SiteId, pools: Pools },
    Destructor { pool: usize },
}
struct Freeing {
    thread: u32,
    army: usize,
    pool: usize,
    enabled: bool,
    retired: Option<u64>,
    direct: Option<u64>,
}

const SHARED_POOL: &str = "equipment destruction pools observed by two threads at once";
const DESTRUCTOR_OVERLAP: &str =
    "equipment destruction observation overlaps another thread's division destructor";
const UNSETTLED: &str = "equipment observed native call did not settle";
const NO_BASIS: &str = "equipment observation closed without its pre-state";
const NO_WINDOW: &str = "equipment observation pre-state has no open window";
const DRAIN_STORAGE_FAILED: &str = "equipment destruction drain storage allocation failed";

struct Store {
    _charge: collector_primitives::raw_memory::Charge,
    entries: Vec<Entry>,
    queued: FixedIndex<u64, Queued>,
    queued_by_army: FixedIndex<usize, u64>,
    queued_limit: usize,
    queued_count: usize,
    credits: Credits,
    transition: bool,
    refusal: Option<Refusal>,
    interval_refusal: Option<Refusal>,
    attach_interval: bool,
    unproven_queue: bool,
    poisoned: bool,
    scopes: HashMap<u64, Deletion>,
    open: HashMap<u64, Opened>,
    freeing: HashMap<u64, Freeing>,
    closes: u64,
    closed: Vec<Closed>,
    net: Vec<Net>,
    charged: (u64, u64),
}
impl Store {
    fn new() -> Self {
        Self {
            _charge: Default::default(),
            entries: Vec::new(),
            queued: FixedIndex::new(),
            queued_by_army: FixedIndex::new(),
            queued_limit: 0,
            queued_count: 0,
            credits: Credits::new(),
            transition: true,
            refusal: None,
            interval_refusal: None,
            attach_interval: true,
            unproven_queue: false,
            poisoned: false,
            scopes: HashMap::new(),
            open: HashMap::new(),
            freeing: HashMap::new(),
            closes: 0,
            closed: Vec::new(),
            net: Vec::new(),
            charged: hook_failures(),
        }
    }
    #[cfg(test)]
    fn owned_capacity(&self) -> collector_primitives::raw_memory::Capacity {
        owned_capacity(
            &self.entries,
            &self.queued,
            &self.queued_by_army,
            &self.credits,
        )
    }
    fn append(&mut self, site: SiteId, entry: Entry) -> Result<(), &'static str> {
        append_entry(&mut self.entries, site, entry)
    }
    fn recharge(&mut self) {
        let capacity = owned_capacity(
            &self.entries,
            &self.queued,
            &self.queued_by_army,
            &self.credits,
        );
        STORE_BYTES.store(
            capacity.payload_vectors + capacity.index_vectors,
            Ordering::Relaxed,
        );
        CAPACITY.store(self.entries.capacity(), Ordering::Release);
        self._charge.set(capacity);
    }
    fn refuse(&mut self, site_index: Option<usize>, reason: &'static str) {
        self.refusal
            .get_or_insert(record_refusal(site_index, reason));
        self.poisoned = true;
        UNAVAILABLE.store(true, Ordering::Relaxed);
    }
    fn revoke(&mut self) {
        if !self.open.is_empty() {
            self.credits.advance();
        }
    }
    fn fail(&mut self, site: SiteId, reason: &'static str) {
        self.revoke();
        self.refuse(Some(site as usize), reason);
    }
    fn boundary(&mut self, transition: bool, load: bool, refused: bool) {
        EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
        self.transition = transition;
        self.interval_refusal = None;
        if transition {
            self.entries.clear();
            self.refusal = None;
            self.unproven_queue = false;
            self.attach_interval |= refused;
            if !load && self.credits.has_unsettled() {
                self.refuse(None, "equipment returned source lifetime incomplete");
            }
        } else if refused {
            self.entries.clear();
            self.unproven_queue = false;
            self.attach_interval = true;
            self.interval_refusal = Some(self.refusal.take().unwrap_or(Refusal {
                site_index: None,
                reason: "equipment destruction lost observation; interval unavailable",
            }));
        } else {
            self.refusal = None;
        }
        if load {
            if let Err(reason) = self
                .queued
                .clear()
                .and_then(|()| self.queued_by_army.clear())
            {
                self.refuse(None, reason);
                return;
            }
            if let Err(reason) = self.credits.clear() {
                self.refuse(None, reason);
                return;
            }
            self.queued_count = 0;
            self.unproven_queue = false;
        }
    }

    fn fold(&mut self) {
        let before = hook_failures();
        let taken = LOG.take();
        let after = hook_failures();
        for event in taken {
            self.apply(event);
        }
        if after.0 > self.charged.0 {
            self.credits.advance();
            self.charge(EVENT_STORAGE_FAILED);
        }
        if after.1 > self.charged.1 {
            self.revoke();
            self.charge(UNSETTLED);
        }
        self.charged = before;
        UNAVAILABLE.store(self.poisoned, Ordering::Relaxed);
        self.recharge();
    }
    fn charge(&mut self, reason: &'static str) {
        self.refusal.get_or_insert(Refusal {
            site_index: None,
            reason,
        });
        self.poisoned = true;
    }
    fn apply(&mut self, Event { thread, kind }: Event) {
        match kind {
            Kind::Boundary {
                transition,
                load,
                first,
                skipped_queue,
            } => {
                let refused = std::mem::take(&mut self.poisoned);
                self.attach_interval |= first || load || skipped_queue;
                self.boundary(transition, load, refused);
            }
            Kind::Lost => self.poisoned = true,
            Kind::Refused { site, reason } => self.fail(site, reason),
            Kind::Revoke => self.revoke(),
            Kind::Skipped {
                scope,
                army,
                identity,
            } => self.skip_credit(scope, army, identity),
            Kind::SkippedQueue => self.attach_interval = true,
            Kind::Nested {
                site,
                army,
                scope,
                identity,
            } => self.nested(site, army, scope, identity),
            Kind::Open { id, site } => self.open(thread, id, site),
            Kind::Basis { id, site, pre } => self.basis(thread, id, site, pre),
            Kind::Close { id, post } => self.close(id, post),
            Kind::Delete {
                scope,
                expeditionary_owner,
                classified,
            } => self.delete(scope, expeditionary_owner, classified),
            Kind::Retire {
                army,
                handle,
                scope,
            } => {
                self.retire(army, handle);
                if let Some(id) = scope {
                    self.scopes.remove(&id);
                }
            }
            Kind::Freeing { id, army } => self.begin_free(thread, id, army),
            Kind::Free(free) => self.free(*free),
            Kind::Freed { id, unwound } => self.freed(id, unwound),
            Kind::Queue {
                army,
                handle,
                cause,
                disposition,
            } => {
                if self.poisoned {
                    self.revoke();
                    self.attach_interval = true;
                } else if let Err(reason) = self.record_queue(army, handle, cause, disposition) {
                    self.refuse(Some(SiteId::Queue as usize), reason);
                }
            }
            Kind::Transfer { site, entry } => {
                if self.poisoned {
                    self.revoke();
                } else if let Err(reason) = self.append(site, entry) {
                    self.refuse(Some(site as usize), reason);
                }
            }
        }
    }
    fn skipped_entry(
        &mut self,
        site: SiteId,
        scope: Option<Scope>,
        army: Option<usize>,
        identity: Option<u64>,
    ) {
        self.revoke();
        match site {
            SiteId::Land
            | SiteId::Attrition
            | SiteId::Paradrop
            | SiteId::SetExiled
            | SiteId::VolunteerReturnLoss
            | SiteId::CapitulationEscapeLoss
            | SiteId::ScriptedEquipmentFraction
            | SiteId::Return
            | SiteId::HqReturn => self.skip_credit(scope, army, identity),
            SiteId::Merge => self.skip_credit(None, None, None),
            _ => {}
        }
    }
    fn skip_credit(&mut self, scope: Option<Scope>, army: Option<usize>, identity: Option<u64>) {
        let context = army.and_then(|army| self.scope_of(scope, army));
        if context.is_some_and(|context| context.ignored) {
            return;
        }
        let source = army.and_then(|army| {
            context
                .filter(|context| context.handle != 0)
                .map(|context| context.handle)
                .or(identity)
                .map(|handle| (army, handle))
        });
        if let Some((army, handle)) = source
            && self.credits.skip(handle, army)
        {
            return;
        }
        self.credits.advance();
    }
    fn scope_of(&self, scope: Option<Scope>, army: usize) -> Option<Deletion> {
        scope
            .filter(|scope| scope.army == army)
            .and_then(|scope| self.scopes.get(&scope.id).copied())
    }
    fn context(
        &self,
        scope: Option<Scope>,
        army: usize,
        identity: Read<u64>,
    ) -> Read<Option<Deletion>> {
        let Some(context) = self.scope_of(scope, army) else {
            return Ok(None);
        };
        if context.ignored && context.handle == 0 {
            return Ok(Some(context));
        }
        Ok((identity? == context.handle).then_some(context))
    }
    fn insert_scope(&mut self, id: u64, context: Deletion) {
        if engine_allocation::advisory(|| engine_allocation::reserve_map(&mut self.scopes, 1))
            .is_err()
        {
            self.refuse(Some(SiteId::Delete as usize), DRAIN_STORAGE_FAILED);
            return;
        }
        self.scopes.insert(id, context);
    }
    fn nested(&mut self, site: SiteId, army: usize, scope: Scope, identity: Read<u64>) {
        if self.poisoned {
            self.skipped_entry(site, Some(scope), Some(army), identity.ok());
            return;
        }
        match self.context(Some(scope), army, identity) {
            Err(reason) => self.fail(site, reason),
            Ok(Some(context))
                if context.returned
                    || context.ignored
                    || context.disposition != native::Disposition::Loss => {}
            Ok(_) => self.fail(site, "equipment destruction pool observations overlap"),
        }
    }
    fn transferred_source(
        &self,
        army: usize,
        context: Option<Deletion>,
        identity: Read<u64>,
        owner: Read<u32>,
    ) -> Result<bool, &'static str> {
        if context.is_some_and(|context| {
            context.ignored
                || context.returned
                || matches!(
                    context.disposition,
                    native::Disposition::Merged
                        | native::Disposition::VolunteerTransferred
                        | native::Disposition::ExileTransferred
                )
        }) {
            return Ok(true);
        }
        let pending = self
            .queued_by_army
            .get(&army)
            .and_then(|handle| self.queued.get(handle))
            .filter(|entry| entry.disposition != native::Disposition::Loss);
        if !self.credits.is_empty() {
            let handle = identity?;
            if self.credits.contains(handle) {
                let country = context.map_or(owner, |context| Ok(context.country))?;
                return Ok(self.credits.state(handle, army, country)? == PoolState::Bookkeeping);
            }
        }
        if let Some(pending) = pending {
            return Ok(identity? == pending.handle);
        }
        Ok(context.is_some_and(|context| context.disposition != native::Disposition::Loss))
    }

    fn open(&mut self, thread: u32, id: u64, site: SiteId) {
        if engine_allocation::advisory(|| engine_allocation::reserve_map(&mut self.open, 1))
            .is_err()
        {
            self.fail(site, DRAIN_STORAGE_FAILED);
            return;
        }
        self.open.insert(
            id,
            Opened {
                site,
                thread,
                pools: Pools::None,
                reserves: false,
                state: None,
                since: Some(self.closes),
            },
        );
    }
    fn basis(&mut self, thread: u32, id: u64, site: SiteId, mut pre: Pre) {
        let since = self.open.get_mut(&id).and_then(|opened| opened.since.take());
        let (scope, army, identity) = pre.source();
        let Some(since) = since.filter(|_| !self.poisoned) else {
            self.open.remove(&id);
            self.skipped_entry(site, scope, army, identity);
            if since.is_none() {
                self.refuse(Some(site as usize), NO_WINDOW);
            }
            self.settle();
            return;
        };
        let (pools, reserves) = pre.window(site);
        match self.open_state(site, pre) {
            Ok(Some(state)) => {
                if let Some((reason, other, pool)) = self.conflict(thread, &pools, reserves, since)
                {
                    self.contest(site, reason, other, pool, state.country());
                }
                if let Some(opened) = self.open.get_mut(&id) {
                    opened.pools = pools;
                    opened.reserves = reserves;
                    opened.state = Some(state);
                }
            }
            Ok(None) => {}
            Err(reason) => {
                self.fail(site, reason);
                if matches!(site, SiteId::Return | SiteId::HqReturn) {
                    self.skip_credit(scope, army, identity);
                } else if site == SiteId::Merge {
                    self.skip_credit(None, None, None);
                }
            }
        }
        self.settle();
    }
    fn settle(&mut self) {
        if !self.open.values().any(|opened| opened.since.is_some()) {
            self.closed.clear();
        }
    }
    fn closed(&mut self, thread: u32, window: Window) {
        self.closes += 1;
        if !self
            .open
            .values()
            .any(|opened| opened.since.is_some() && opened.thread != thread)
        {
            return;
        }
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.closed, 1)).is_err()
        {
            let site = match window {
                Window::Observation { site, .. } => site,
                Window::Destructor { .. } => SiteId::DivisionFree,
            };
            self.fail(site, DRAIN_STORAGE_FAILED);
            return;
        }
        self.closed.push(Closed {
            at: self.closes,
            thread,
            window,
        });
    }
    fn conflict(
        &self,
        thread: u32,
        pools: &Pools,
        reserves: bool,
        since: u64,
    ) -> Option<(&'static str, SiteId, usize)> {
        self.open
            .values()
            .filter(|opened| opened.thread != thread)
            .find_map(|opened| {
                opened
                    .pools
                    .shared(pools)
                    .map(|pool| (SHARED_POOL, opened.site, pool))
            })
            .or_else(|| {
                self.freeing
                    .values()
                    .find(|freeing| {
                        freeing.thread != thread && (reserves || pools.contains(freeing.pool))
                    })
                    .map(|freeing| (DESTRUCTOR_OVERLAP, SiteId::DivisionFree, freeing.pool))
            })
            .or_else(|| {
                self.closed
                    .iter()
                    .filter(|closed| closed.at > since && closed.thread != thread)
                    .find_map(|closed| match &closed.window {
                        Window::Observation { site, pools: other } => {
                            other.shared(pools).map(|pool| (SHARED_POOL, *site, pool))
                        }
                        Window::Destructor { pool } => (reserves || pools.contains(*pool))
                            .then_some((DESTRUCTOR_OVERLAP, SiteId::DivisionFree, *pool)),
                    })
            })
    }
    fn contest(
        &mut self,
        site: SiteId,
        reason: &'static str,
        other: SiteId,
        pool: usize,
        country: u32,
    ) {
        CONTESTS.fetch_add(1, Ordering::Relaxed);
        *CONTEST.lock().unwrap_or_else(PoisonError::into_inner) = Some(Contest {
            reason,
            site_index: site as usize,
            other_site_index: other as usize,
            pool,
            country,
        });
        self.fail(site, reason);
    }
    fn open_state(&mut self, site: SiteId, pre: Pre) -> Read<Option<OpenState>> {
        match pre {
            Pre::Pool(pre) => self.pool_open(pre),
            Pre::Variant(pre) => self.variant_open(pre),
            Pre::Return(pre) => self.return_open(site, *pre),
            Pre::Merge(pre) => self.merge_open(*pre),
        }
    }
    fn close(&mut self, id: u64, post: Post) {
        if let Some(site) = self
            .open
            .get(&id)
            .filter(|opened| opened.since.is_some())
            .map(|opened| opened.site)
        {
            self.fail(site, NO_BASIS);
        }
        if matches!(post, Post::WorldChanged)
            && let Some(site) = self
                .open
                .get(&id)
                .filter(|opened| opened.state.is_some())
                .map(|opened| opened.site)
        {
            self.fail(
                site,
                "equipment destruction native world changed during original",
            );
        }
        let Some(Opened {
            site,
            thread,
            pools,
            state,
            ..
        }) = self.open.remove(&id)
        else {
            return;
        };
        if !matches!(pools, Pools::None) {
            self.closed(thread, Window::Observation { site, pools });
        }
        self.settle();
        let Some(state) = state else {
            return;
        };
        let result = match (state, post) {
            (_, Post::WorldChanged) => Ok(()),
            (_, Post::Unsettled) => {
                self.fail(site, UNSETTLED);
                Ok(())
            }
            (
                OpenState::Pool {
                    country,
                    cause,
                    rows,
                },
                Post::Pool(after),
            ) => self.pool_close(site, country, cause, rows, after),
            (
                OpenState::Variant {
                    country,
                    design,
                    before,
                },
                Post::Variant(after),
            ) => self.variant_close(country, design, before, after),
            (state @ OpenState::Return { .. }, post @ Post::Return { .. }) => {
                self.return_close(state, post)
            }
            (state @ OpenState::Merge { .. }, post @ Post::Merge { .. }) => {
                self.merge_close(state, post)
            }
            _ => Err("equipment observation events disagree"),
        };
        if let Err(reason) = result {
            self.refuse(Some(site as usize), reason);
        }
    }

    fn pool_open(&self, pre: PoolPre) -> Read<Option<OpenState>> {
        let PoolPre {
            cause,
            army,
            scope,
            identity,
            owner,
            pool,
            rows,
        } = pre;
        let country = if let Some(army) = army {
            let context = self.context(scope, army, identity)?;
            if self.transferred_source(army, context, identity, owner)? {
                return Ok(None);
            }
            context.map_or(owner, |context| Ok(context.country))?
        } else {
            owner?
        };
        pool?;
        Ok(Some(OpenState::Pool {
            country,
            cause,
            rows: rows?,
        }))
    }
    fn pool_close(
        &mut self,
        site: SiteId,
        country: u32,
        cause: native::Cause,
        mut before: Vec<Row>,
        after: Read<Vec<Row>>,
    ) -> Result<(), &'static str> {
        let mut after = after?;
        let mut failure = None;
        let entries = &mut self.entries;
        native::removals(&mut before, &mut after, |variant, amount_raw| {
            if let Err(reason) = append_entry(
                entries,
                site,
                Entry {
                    country,
                    cause,
                    disposition: native::Disposition::Loss,
                    expeditionary_owner: 0,
                    division_handle: 0,
                    variant,
                    amount_raw,
                },
            ) {
                failure.get_or_insert(reason);
            }
        })?;
        failure.map_or(Ok(()), Err)
    }
    fn variant_open(&self, pre: VariantPre) -> Read<Option<OpenState>> {
        let army = pre.army?;
        let context = self.context(pre.scope, army, pre.identity)?;
        if self.transferred_source(army, context, pre.identity, pre.owner)? {
            return Ok(None);
        }
        let country = context.map_or(pre.owner, |context| Ok(context.country))?;
        let design = pre.design?;
        Ok(Some(OpenState::Variant {
            country,
            design,
            before: pre.amount?,
        }))
    }
    fn variant_close(
        &mut self,
        country: u32,
        design: u64,
        before: i64,
        after: Read<i64>,
    ) -> Result<(), &'static str> {
        let amount_raw = before
            .checked_sub(after?)
            .ok_or("destruction difference overflow")?;
        self.append(
            SiteId::Attrition,
            Entry {
                country,
                cause: native::Cause::Attrition,
                disposition: native::Disposition::Loss,
                expeditionary_owner: 0,
                division_handle: 0,
                variant: design,
                amount_raw,
            },
        )
    }

    fn return_open(&mut self, site: SiteId, pre: ReturnPre) -> Read<Option<OpenState>> {
        let army = pre.army?;
        let context = self.scope_of(pre.scope, army);
        if context.is_some_and(|context| context.ignored) {
            return Ok(None);
        }
        if site == SiteId::Return && context.is_none() {
            return Err("equipment deletion returned-pool identity mismatch");
        }
        if pre.excluded? {
            return Ok(None);
        }
        let handle = pre.identity?;
        if context.is_some_and(|context| context.handle != handle) {
            return Err("equipment returned source handle mismatch");
        }
        let country = context.map_or(pre.owner, |context| Ok(context.country));
        let revision = self.credits.revision();
        let bookkeeping = if self.credits.contains(handle) {
            let prior = country.and_then(|country| self.credits.state(handle, army, country));
            self.credits.invalidate(handle);
            prior? == PoolState::Bookkeeping
        } else {
            self.credits.begin(handle, army, country?, revision)?;
            self.credits.invalidate(handle);
            false
        };
        let country = country?;
        pre.reserves?;
        pre.reserve_owner?;
        Ok(Some(OpenState::Return {
            army,
            handle,
            country,
            scope: pre.scope.filter(|scope| scope.army == army).map(|scope| scope.id),
            sources: pre.sources?,
            pre: pre.touched?,
            bookkeeping,
            revision,
        }))
    }
    fn return_close(&mut self, state: OpenState, post: Post) -> Result<(), &'static str> {
        let (
            OpenState::Return {
                army,
                handle,
                country,
                scope,
                sources,
                pre,
                bookkeeping,
                revision,
            },
            Post::Return { identity, touched },
        ) = (state, post)
        else {
            return Err("equipment observation events disagree");
        };
        if identity? != handle {
            return Err("equipment returned source changed handle");
        }
        let post = touched?;
        let mut net = std::mem::take(&mut self.net);
        net.clear();
        let result = (|| {
            self.credits.visit(handle, army, country, |value| {
                transfers::push(&mut net, value)
            })?;
            if !bookkeeping {
                for row in &sources {
                    transfers::push(
                        &mut net,
                        Net {
                            country: 0,
                            variant: row.variant_handle,
                            amount: i128::from(row.amount_raw),
                            credited: 0,
                        },
                    )?;
                }
            }
            for row in &pre {
                transfers::push(
                    &mut net,
                    Net {
                        country: 0,
                        variant: row.variant_handle,
                        amount: i128::from(row.amount_raw),
                        credited: -i128::from(row.amount_raw),
                    },
                )?;
            }
            for row in &post {
                transfers::push(
                    &mut net,
                    Net {
                        country: 0,
                        variant: row.variant_handle,
                        amount: -i128::from(row.amount_raw),
                        credited: i128::from(row.amount_raw),
                    },
                )?;
            }
            let len = transfers::compact(&mut net)?;
            self.credits.check_continuity(handle)?;
            self.credits.begin(handle, army, country, revision)?;
            self.credits.invalidate(handle);
            for value in &net[..len] {
                if value.amount != 0 || value.credited != 0 {
                    self.credits.push(handle, *value)?;
                }
            }
            self.credits
                .set_state(handle, army, country, PoolState::Bookkeeping)
        })();
        self.net = net;
        result?;
        if let Some(context) = scope
            .and_then(|scope| self.scopes.get_mut(&scope))
            .filter(|context| context.army == army && context.handle == handle)
        {
            context.returned = true;
        }
        Ok(())
    }

    fn merge_open(&mut self, pre: MergePre) -> Read<Option<OpenState>> {
        if pre.excluded? {
            return Ok(None);
        }
        let (survivor, handle) = pre.head?;
        let country = pre.country;
        let prior_credit = self.credits.contains(handle);
        let prior_state = if prior_credit {
            let state = country.and_then(|country| self.credits.state(handle, survivor, country));
            self.credits.invalidate(handle);
            Some(state?)
        } else {
            None
        };
        let country = country?;
        let mut net = Vec::new();
        for army in &pre.armies? {
            let credited = if army.army == survivor {
                prior_state == Some(PoolState::Bookkeeping)
            } else {
                self.credits.contains(army.identity)
                    && self
                        .credits
                        .state(army.identity, army.army, army.owner?)?
                        == PoolState::Bookkeeping
            };
            if !credited {
                for row in &army.rows {
                    transfers::push(
                        &mut net,
                        Net {
                            country,
                            variant: row.variant_handle,
                            amount: i128::from(row.amount_raw),
                            credited: 0,
                        },
                    )?;
                }
            }
        }
        let rest = pre.rest?;
        let mut scopes = Vec::new();
        engine_allocation::advisory(|| engine_allocation::reserve(&mut scopes, rest.scopes.len()))
            .map_err(|_| DRAIN_STORAGE_FAILED)?;
        for scope in &rest.scopes {
            let scope_country = if scope.main { country } else { scope.identity };
            for row in scope
                .held
                .iter()
                .map(|held| held.row)
                .chain(scope.touched.iter().copied())
            {
                transfers::push(
                    &mut net,
                    Net {
                        country: scope_country,
                        variant: row.variant_handle,
                        amount: i128::from(row.amount_raw),
                        credited: 0,
                    },
                )?;
            }
            scopes.push((scope.main, scope.identity));
        }
        Ok(Some(OpenState::Merge {
            country,
            survivor,
            handle,
            scopes,
            net,
            prior_credit,
        }))
    }
    fn merge_close(&mut self, state: OpenState, post: Post) -> Result<(), &'static str> {
        let (
            OpenState::Merge {
                country,
                survivor,
                handle,
                scopes,
                mut net,
                prior_credit,
            },
            Post::Merge {
                identity,
                rows,
                scopes: reads,
            },
        ) = (state, post)
        else {
            return Err("equipment observation events disagree");
        };
        if identity? != handle {
            return Err("merge survivor changed handle");
        }
        for row in &rows? {
            transfers::push(
                &mut net,
                Net {
                    country,
                    variant: row.variant_handle,
                    amount: -i128::from(row.amount_raw),
                    credited: 0,
                },
            )?;
        }
        if reads.len() != scopes.len() {
            return Err("equipment observation events disagree");
        }
        for ((main, identity), (owner, touched)) in scopes.into_iter().zip(reads) {
            if owner? != identity {
                return Err("merge reserve destination changed identity");
            }
            let scope_country = if main { country } else { identity };
            for row in &touched? {
                transfers::push(
                    &mut net,
                    Net {
                        country: scope_country,
                        variant: row.variant_handle,
                        amount: -i128::from(row.amount_raw),
                        credited: 0,
                    },
                )?;
            }
        }
        let len = transfers::compact(&mut net)?;
        for value in &net[..len] {
            let amount_raw = i64::try_from(value.amount)
                .map_err(|_| "merge equipment correction exceeds signed width")?;
            self.append(
                SiteId::Merge,
                Entry {
                    country: value.country,
                    cause: native::Cause::DivisionMerged,
                    disposition: native::Disposition::Loss,
                    expeditionary_owner: 0,
                    division_handle: 0,
                    variant: value.variant,
                    amount_raw,
                },
            )?;
        }
        if prior_credit {
            self.credits
                .set_state(handle, survivor, country, PoolState::Live)?;
        }
        Ok(())
    }

    fn take_queued(&mut self, army: usize, handle: u64) -> Option<Queued> {
        if self
            .queued
            .get(&handle)
            .is_none_or(|entry| entry.army != army)
        {
            return None;
        }
        if self.queued_by_army.get(&army) == Some(&handle) {
            self.queued_by_army.remove(&army);
        }
        let entry = self.queued.remove(&handle);
        if entry.is_some() {
            self.queued_count -= 1;
        }
        entry
    }
    fn retire(&mut self, army: usize, handle: Option<u64>) {
        if let Some(handle) = handle.or_else(|| self.queued_by_army.get(&army).copied()) {
            self.take_queued(army, handle);
        }
    }
    fn record_queue(
        &mut self,
        army: usize,
        handle: u64,
        cause: native::Cause,
        disposition: native::Disposition,
    ) -> Result<(), &'static str> {
        if let Some(previous) = self.queued_by_army.get(&army).copied()
            && previous != handle
        {
            self.take_queued(army, previous);
        }
        if let Some(entry) = self.queued.get_mut(&handle) {
            if entry.army != army {
                return Err("equipment deletion handle has conflicting army address");
            }
            if entry.cause != cause {
                entry.cause = native::Cause::Unknown;
            }
            if disposition != native::Disposition::Loss {
                entry.disposition = disposition;
            }
            return Ok(());
        }
        if self.queued_count == self.queued_limit {
            grow_queue(self)?;
        }
        self.queued.insert(
            handle,
            Queued {
                army,
                handle,
                cause,
                disposition,
            },
        )?;
        if let Err(reason) = self.queued_by_army.insert(army, handle) {
            self.queued.remove(&handle);
            return Err(reason);
        }
        self.queued_count += 1;
        Ok(())
    }
    fn delete(
        &mut self,
        scope: Scope,
        expeditionary_owner: u32,
        classified: Option<(native::Cause, native::Disposition)>,
    ) {
        if self.poisoned {
            self.revoke();
            return;
        }
        let context = if scope.ignored {
            Deletion::ignored(scope.army)
        } else {
            let pending = self.take_queued(scope.army, scope.handle);
            if classified.is_none() && pending.is_none() && self.attach_interval {
                self.unproven_queue = true;
                Deletion::ignored(scope.army)
            } else {
                let (cause, mut disposition) = classified.unwrap_or_else(|| {
                    pending
                        .map(|entry| (entry.cause, entry.disposition))
                        .unwrap_or((native::Cause::Unknown, native::Disposition::Loss))
                });
                if let Some(entry) =
                    pending.filter(|entry| entry.disposition != native::Disposition::Loss)
                {
                    disposition = entry.disposition;
                }
                Deletion {
                    army: scope.army,
                    handle: scope.handle,
                    country: scope.country,
                    cause,
                    disposition,
                    expeditionary_owner,
                    returned: false,
                    ignored: false,
                    merge_source: scope.merge_source,
                }
            }
        };
        self.insert_scope(scope.id, context);
    }
    fn begin_free(&mut self, thread: u32, id: u64, army: usize) {
        let pool = army
            .checked_add(engine_native::carmy::EQUIPMENT_POOL)
            .unwrap_or(usize::MAX);
        if let Some((site, country)) = self.open.values().find_map(|opened| {
            opened
                .state
                .as_ref()
                .filter(|_| {
                    opened.thread != thread && (opened.reserves || opened.pools.contains(pool))
                })
                .map(|state| (opened.site, state.country()))
        }) {
            self.contest(
                SiteId::DivisionFree,
                DESTRUCTOR_OVERLAP,
                site,
                pool,
                country,
            );
        }
        if engine_allocation::advisory(|| engine_allocation::reserve_map(&mut self.freeing, 1))
            .is_err()
        {
            self.fail(SiteId::DivisionFree, DRAIN_STORAGE_FAILED);
            return;
        }
        self.freeing.insert(
            id,
            Freeing {
                thread,
                army,
                pool,
                enabled: false,
                retired: None,
                direct: None,
            },
        );
    }
    fn free(&mut self, free: Free) {
        let Free {
            id,
            army,
            direct,
            scope,
            deleted,
            enabled,
            identity,
            merge_source,
            observing,
        } = free;
        if self.poisoned {
            self.revoke();
        } else {
            if let Some((direct, expeditionary_owner)) = direct {
                let pending = self.take_queued(army, direct.handle);
                self.insert_scope(
                    direct.id,
                    Deletion {
                        army,
                        handle: direct.handle,
                        country: direct.country,
                        cause: pending.map_or(native::Cause::Unknown, |entry| entry.cause),
                        disposition: pending
                            .map_or(native::Disposition::Loss, |entry| entry.disposition),
                        expeditionary_owner,
                        returned: false,
                        ignored: false,
                        merge_source: false,
                    },
                );
            }
            if let Some(rows) = deleted
                && let Some(context) = self.scope_of(scope, army).filter(|c| !c.ignored)
                && let Err(reason) = self.observe_deleted(context, rows, observing)
            {
                self.fail(SiteId::DivisionFree, reason);
            }
        }
        let context = self.scope_of(scope, army);
        let enabled = enabled && !context.is_some_and(|context| context.ignored);
        let retired = context
            .filter(|context| context.handle != 0)
            .map(|context| context.handle)
            .or(identity);
        if enabled {
            match retired {
                Some(handle) => self.credits.invalidate(handle),
                None => self.skip_credit(scope, Some(army), None),
            }
        }
        if observing && !merge_source && context.is_none_or(|context| !context.ignored) {
            self.fail(
                SiteId::DivisionFree,
                "equipment deletion native pools overlap observation",
            );
        }
        if let Some(freeing) = self.freeing.get_mut(&id) {
            freeing.enabled = enabled;
            freeing.retired = retired;
            freeing.direct = direct.map(|(scope, _)| scope.id);
        }
    }
    fn freed(&mut self, id: u64, unwound: bool) {
        let Some(freeing) = self.freeing.remove(&id) else {
            return;
        };
        self.closed(freeing.thread, Window::Destructor { pool: freeing.pool });
        self.settle();
        if !unwound && freeing.enabled {
            self.retire(freeing.army, freeing.retired);
            if let Some(handle) = freeing.retired {
                self.credits.remove(handle);
            }
        }
        if let Some(scope) = freeing.direct {
            self.scopes.remove(&scope);
        }
    }
    fn observe_deleted(
        &mut self,
        context: Deletion,
        rows: Read<Vec<Row>>,
        observing: bool,
    ) -> Result<(), &'static str> {
        let disposition = if context.returned && context.disposition == native::Disposition::Loss
        {
            native::Disposition::Returned
        } else {
            context.disposition
        };
        let cause = if disposition == native::Disposition::Loss {
            count_cause(context.cause)
        } else {
            context.cause
        };
        self.append(
            SiteId::DivisionFree,
            Entry {
                country: context.country,
                cause,
                disposition,
                expeditionary_owner: context.expeditionary_owner,
                division_handle: context.handle,
                variant: 0,
                amount_raw: 0,
            },
        )?;
        if self.credits.contains(context.handle) {
            let state = self
                .credits
                .state(context.handle, context.army, context.country);
            settle_credit(self, context)?;
            if state? == PoolState::Bookkeeping
                || context.merge_source
                || matches!(
                    disposition,
                    native::Disposition::Merged
                        | native::Disposition::VolunteerTransferred
                        | native::Disposition::ExileTransferred
                )
            {
                return Ok(());
            }
        } else {
            if matches!(
                disposition,
                native::Disposition::Returned | native::Disposition::HqTransferred
            ) {
                return Err("equipment returned source has no actual credit witness");
            }
            if disposition != native::Disposition::Loss {
                return Ok(());
            }
        }
        if observing {
            return Err("equipment deletion snapshot overlaps pool observation");
        }
        let rows = rows?;
        let mut net = std::mem::take(&mut self.net);
        net.clear();
        let result = (|| {
            for row in &rows {
                transfers::push(
                    &mut net,
                    Net {
                        country: 0,
                        variant: row.variant_handle,
                        amount: i128::from(row.amount_raw),
                        credited: 0,
                    },
                )?;
            }
            let len = transfers::compact(&mut net)?;
            for value in &net[..len] {
                let amount_raw = i64::try_from(value.amount)
                    .map_err(|_| "equipment deletion correction exceeds signed width")?;
                self.append(
                    SiteId::DivisionFree,
                    Entry {
                        country: context.country,
                        cause,
                        disposition: native::Disposition::Loss,
                        expeditionary_owner: 0,
                        division_handle: 0,
                        variant: value.variant,
                        amount_raw,
                    },
                )?;
            }
            Ok(())
        })();
        self.net = net;
        result
    }
}

fn append_entry(entries: &mut Vec<Entry>, site: SiteId, entry: Entry) -> Result<(), &'static str> {
    if entry.division_handle != 0 {
        if entry.variant != 0 || entry.amount_raw != 0 {
            return Err("equipment deletion witness has equipment amount");
        }
    } else if entry.amount_raw == 0 {
        return Ok(());
    }
    let attempted = EVENT_ATTEMPTS
        .fetch_add(1, Ordering::Relaxed)
        .saturating_add(1);
    EVENT_REQUIRED.fetch_max(attempted, Ordering::Relaxed);
    if entries.len() == entries.capacity() {
        count_growth(site);
        engine_allocation::advisory(|| engine_allocation::reserve(entries, 1))
            .map_err(|_| EVENT_STORAGE_FAILED)?;
    }
    entries.push(entry);
    EVENTS.fetch_add(1, Ordering::Relaxed);
    record_hourly_peak(entries.len(), entries.capacity());
    Ok(())
}

fn settle_credit(store: &mut Store, context: Deletion) -> Result<(), &'static str> {
    store
        .credits
        .state(context.handle, context.army, context.country)?;
    let entries = &mut store.entries;
    store
        .credits
        .visit(context.handle, context.army, context.country, |value| {
            let amount_raw = i64::try_from(value.amount)
                .map_err(|_| "equipment deletion correction exceeds signed width")?;
            append_entry(
                entries,
                SiteId::DivisionFree,
                Entry {
                    country: context.country,
                    cause: context.cause,
                    disposition: native::Disposition::Loss,
                    expeditionary_owner: 0,
                    division_handle: 0,
                    variant: value.variant,
                    amount_raw,
                },
            )
        })
}

fn owned_capacity(
    entries: &Vec<Entry>,
    queued: &FixedIndex<u64, Queued>,
    by_army: &FixedIndex<usize, u64>,
    credits: &Credits,
) -> collector_primitives::raw_memory::Capacity {
    use collector_primitives::raw_memory::{Capacity, vector_bytes};
    Capacity {
        payload_vectors: vector_bytes(entries),
        index_vectors: queued.allocated_bytes() as u64
            + by_army.allocated_bytes() as u64
            + credits.allocated_bytes() as u64,
        ..Default::default()
    }
}

pub fn release() -> Result<(), &'static str> {
    if ENABLED.load(Ordering::SeqCst)
        || SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst)
        || ACTIVE
            .iter()
            .any(|active| active.load(Ordering::SeqCst) != 0)
        || OPEN.load(Ordering::SeqCst) != 0
    {
        return Err("equipment event storage release before callback quiescence");
    }
    let storage = {
        let mut store = STORE.lock().unwrap_or_else(|error| error.into_inner());
        CAPACITY.store(0, Ordering::Release);
        QUEUED_CAPACITY.store(0, Ordering::Release);
        STORE_BYTES.store(0, Ordering::Relaxed);
        EVENT_ATTEMPTS.store(0, Ordering::Relaxed);
        CALL_ATTEMPTS.store(0, Ordering::Relaxed);
        EVENT_REQUIRED.store(0, Ordering::Relaxed);
        std::mem::replace(&mut *store, Store::new())
    };
    drop((storage, LOG.take()));
    Ok(())
}

thread_local! {
    static DELETION: Cell<Option<Scope>> = const { Cell::new(None) };
    static WHOLE_MERGE: Cell<bool> = const { Cell::new(false) };
    static OBSERVING: Cell<bool> = const { Cell::new(false) };
    static FREEING: Cell<usize> = const { Cell::new(0) };
}
struct DeletionScope(Option<Scope>);
impl DeletionScope {
    fn enter(context: Option<Scope>) -> Self {
        Self(DELETION.replace(context))
    }
}
impl Drop for DeletionScope {
    fn drop(&mut self) {
        DELETION.set(self.0);
    }
}
static STORE: LazyLock<Mutex<Store>> = LazyLock::new(|| Mutex::new(Store::new()));
static FIRST_REFUSAL: OnceLock<Refusal> = OnceLock::new();
static CLOCK: Mutex<Option<Clock>> = Mutex::new(None);
static WORLD_GENERATION: AtomicU64 = AtomicU64::new(0);
static BASE: AtomicUsize = AtomicUsize::new(0);
static CAPACITY: AtomicUsize = AtomicUsize::new(0);
static EVENT_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static CALL_ATTEMPTS: AtomicUsize = AtomicUsize::new(0);
static EVENT_REQUIRED: AtomicUsize = AtomicUsize::new(0);
static QUEUED_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static STORE_BYTES: AtomicU64 = AtomicU64::new(0);
static HOURLY_PEAK_EVENTS: AtomicUsize = AtomicUsize::new(0);
static HOURLY_PEAK_CAPACITY: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static SUPPLEMENTAL_ENABLED: AtomicBool = AtomicBool::new(false);
static UNAVAILABLE: AtomicBool = AtomicBool::new(false);
static OPEN: AtomicUsize = AtomicUsize::new(0);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static SKIPPED_QUEUE: AtomicBool = AtomicBool::new(false);
static ACTIVE: [AtomicUsize; native::SITES.len()] =
    [const { AtomicUsize::new(0) }; native::SITES.len()];
static ORIGINAL: [AtomicUsize; native::SITES.len()] =
    [const { AtomicUsize::new(0) }; native::SITES.len()];
static INSTALLED: [AtomicBool; native::SITES.len()] =
    [const { AtomicBool::new(false) }; native::SITES.len()];
static SITE_ENABLED: [AtomicBool; native::SITES.len()] =
    [const { AtomicBool::new(false) }; native::SITES.len()];
static CALLS: [AtomicU64; native::SITES.len()] = [const { AtomicU64::new(0) }; native::SITES.len()];
static UNHOOKED: OnceLock<Cow<'static, str>> = OnceLock::new();
static SUPPLEMENTAL_UNHOOKED: OnceLock<Cow<'static, str>> = OnceLock::new();
static SITE_REFUSALS: [AtomicU64; native::SITES.len()] =
    [const { AtomicU64::new(0) }; native::SITES.len()];
static EVENTS: AtomicU64 = AtomicU64::new(0);
static OVERFLOW: AtomicU64 = AtomicU64::new(0);
static SITE_OVERFLOW: [AtomicU64; native::SITES.len()] =
    [const { AtomicU64::new(0) }; native::SITES.len()];
static REFUSALS: AtomicU64 = AtomicU64::new(0);
static CONTESTS: AtomicU64 = AtomicU64::new(0);
static CONTEST: Mutex<Option<Contest>> = Mutex::new(None);
static LOST: AtomicU64 = AtomicU64::new(0);
static DRAINS: AtomicU64 = AtomicU64::new(0);
static HANDED_OVER: AtomicU64 = AtomicU64::new(0);
static STORAGE_REFUSALS: AtomicU64 = AtomicU64::new(0);
static UNSETTLED_CALLS: AtomicU64 = AtomicU64::new(0);
static UNKNOWN_CALLERS: AtomicU64 = AtomicU64::new(0);

fn hook_failures() -> (u64, u64) {
    (
        STORAGE_REFUSALS.load(Ordering::SeqCst),
        UNSETTLED_CALLS.load(Ordering::SeqCst),
    )
}
pub fn active() -> usize {
    ACTIVE.iter().map(|v| v.load(Ordering::SeqCst)).sum()
}
pub fn demand() -> usize {
    HOURLY_PEAK_EVENTS
        .load(Ordering::Relaxed)
        .max(EVENT_REQUIRED.load(Ordering::Acquire))
}
fn record_hourly_peak(events: usize, capacity: usize) {
    if events > HOURLY_PEAK_EVENTS.load(Ordering::Relaxed) {
        HOURLY_PEAK_CAPACITY.store(capacity, Ordering::Relaxed);
        HOURLY_PEAK_EVENTS.store(events, Ordering::Relaxed);
    }
}
fn count_cause(cause: native::Cause) -> native::Cause {
    if cause == native::Cause::Unknown {
        UNKNOWN_CALLERS.fetch_add(1, Ordering::Relaxed);
    }
    cause
}
fn revoke_active_observation() {
    if OPEN.load(Ordering::SeqCst) != 0 {
        log(Kind::Revoke);
    }
}
pub fn lost_drain() {
    LOST.fetch_add(1, Ordering::Relaxed);
    log(Kind::Lost);
}

const EVENT_STORAGE_FAILED: &str = "equipment destruction event storage allocation failed";
const QUEUE_STORAGE_FAILED: &str = "equipment deletion provenance storage allocation failed";

fn count_growth(site: SiteId) {
    OVERFLOW.fetch_add(1, Ordering::Relaxed);
    SITE_OVERFLOW[site as usize].fetch_add(1, Ordering::Relaxed);
}

fn grow_queue(store: &mut Store) -> Result<(), &'static str> {
    count_growth(SiteId::Queue);
    let limit = store
        .queued_limit
        .checked_mul(2)
        .ok_or(QUEUE_STORAGE_FAILED)?
        .max(1);
    let (queued, by_army) = engine_allocation::advisory(|| {
        Ok::<_, std::io::Error>((
            store.queued.grown(limit)?,
            store.queued_by_army.grown(limit)?,
        ))
    })
    .map_err(|_| QUEUE_STORAGE_FAILED)?;
    store.queued = queued;
    store.queued_by_army = by_army;
    store.queued_limit = limit;
    QUEUED_CAPACITY.fetch_max(limit, Ordering::Release);
    Ok(())
}

pub fn boundary(clock: Option<Clock>) {
    let Some(clock) = clock else {
        lost_drain();
        return;
    };
    let Ok(mut previous) = CLOCK.try_lock() else {
        lost_drain();
        return;
    };
    let generation = collector_lifetime::world_lifetime::generation();
    let old_generation = WORLD_GENERATION.swap(generation, Ordering::SeqCst);
    let world_changed = old_generation != 0 && old_generation != generation;
    let transition = world_changed
        || !previous.is_some_and(|before| {
            before.hour.checked_add(1) == Some(clock.hour)
                && before.campaign_id == clock.campaign_id
                && before.play_session_started_at == clock.play_session_started_at
        });
    let first = previous.is_none();
    let load = world_changed
        || previous.is_some_and(|before| {
            before.campaign_id != clock.campaign_id
                || before.play_session_started_at != clock.play_session_started_at
        });
    *previous = Some(clock);
    drop(previous);
    CALL_ATTEMPTS.store(0, Ordering::Relaxed);
    log(Kind::Boundary {
        transition,
        load,
        first,
        skipped_queue: SKIPPED_QUEUE.swap(false, Ordering::SeqCst),
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drain {
    Rows(usize),
    Partial(&'static str),
}

pub fn drain_interval(output: &mut Vec<Entry>) -> Result<Drain, &'static str> {
    drain_buffer(engine_native_memory::walk::Buffer::Growing(output))
}

#[cfg(test)]
pub fn drain(output: &mut Vec<Entry>) -> Result<usize, &'static str> {
    match drain_interval(output)? {
        Drain::Rows(len) => Ok(len),
        Drain::Partial(reason) => Err(reason),
    }
}

fn drain_buffer(mut output: engine_native_memory::walk::Buffer<'_, Entry>) -> Result<Drain, &'static str> {
    DRAINS.fetch_add(1, Ordering::Relaxed);
    if UNHOOKED.get().is_some() || !ENABLED.load(Ordering::SeqCst) {
        return Err("equipment destruction hooks unavailable (see status unhooked)");
    }
    let mut store = STORE.lock().unwrap_or_else(|error| error.into_inner());
    store.fold();
    if store.poisoned || store.refusal.is_some() || store.interval_refusal.is_some() {
        store.entries.clear();
        return Err(store.refusal.or(store.interval_refusal).map_or(
            "equipment destruction lost observation; interval unavailable",
            |r| r.reason,
        ));
    }
    if store.transition {
        store.unproven_queue = false;
        store.entries.clear();
        return Ok(Drain::Partial(
            replay_data::rows::EQUIPMENT_DESTRUCTION_PARTIAL_REASON,
        ));
    }
    if store.unproven_queue {
        store.unproven_queue = false;
        store.attach_interval = false;
        store.entries.clear();
        return Err("queued division provenance is unavailable; interval is partial");
    }
    let len = store.entries.len();
    output
        .ensure(len)
        .map_err(|_| "equipment destruction drain allocation failed")?;
    if len > output.len() {
        drop(store);
        lost_drain();
        return Err("equipment destruction drain storage too small");
    }
    output[..len].copy_from_slice(&store.entries);
    store.entries.clear();
    store.attach_interval = false;
    HANDED_OVER.fetch_add(len as u64, Ordering::Relaxed);
    Ok(Drain::Rows(len))
}

pub fn deletion_unavailable() -> Option<&'static str> {
    if UNHOOKED.get().is_some() {
        Some("division deletion capture requires original destruction writers")
    } else if let Some(reason) = SUPPLEMENTAL_UNHOOKED.get() {
        Some(reason.as_ref())
    } else if !SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst) {
        Some("supplemental equipment destruction hooks are disabled")
    } else {
        None
    }
}

struct Call {
    site: SiteId,
    unsettled: bool,
    returned: Cell<bool>,
    timing: collector_timing::hook_timing::Span,
}
impl Call {
    fn enter(site: SiteId) -> Self {
        let timing = collector_timing::hook_timing::enter(
            collector_timing::hook_timing::Family::EquipmentDestruction,
        );
        ACTIVE[site as usize].fetch_add(1, Ordering::SeqCst);
        CALLS[site as usize].fetch_add(1, Ordering::Relaxed);
        if ENABLED.load(Ordering::SeqCst) {
            let attempted = CALL_ATTEMPTS
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1);
            EVENT_REQUIRED.fetch_max(attempted, Ordering::Relaxed);
        }
        Self {
            site,
            unsettled: false,
            returned: Cell::new(false),
            timing,
        }
    }
    fn run<T>(&self, original: impl FnOnce() -> T) -> T {
        let result = self.timing.original(original);
        self.returned.set(true);
        result
    }
    fn arm(&mut self) {
        self.unsettled = true;
    }
    fn settle(&mut self) {
        self.unsettled = false;
    }
}
impl Drop for Call {
    fn drop(&mut self) {
        if self.unsettled {
            UNSETTLED_CALLS.fetch_add(1, Ordering::SeqCst);
            record_refusal(Some(self.site as usize), UNSETTLED);
        }
        if !self.returned.get() || std::thread::panicking() {
            return;
        }
        ACTIVE[self.site as usize].fetch_sub(1, Ordering::SeqCst);
    }
}

struct Observing;
impl Observing {
    fn enter() -> Self {
        OBSERVING.set(true);
        OPEN.fetch_add(1, Ordering::SeqCst);
        Self
    }
}
impl Drop for Observing {
    fn drop(&mut self) {
        OBSERVING.set(false);
        OPEN.fetch_sub(1, Ordering::SeqCst);
    }
}
struct InDestructor;
impl InDestructor {
    fn enter() -> Self {
        FREEING.set(FREEING.get() + 1);
        Self
    }
}
impl Drop for InDestructor {
    fn drop(&mut self) {
        FREEING.set(FREEING.get() - 1);
    }
}

struct Closing(Option<Reserved<'static, Event>>);
impl Closing {
    fn finish(mut self, finish: impl FnOnce(&mut Kind)) {
        if let Some(mut node) = self.0.take() {
            finish(&mut node.kind);
            node.push();
        }
    }
}
impl Drop for Closing {
    fn drop(&mut self) {
        if let Some(node) = self.0.take() {
            node.push();
        }
    }
}

fn observe_native<T, C>(
    site: SiteId,
    bail: impl FnOnce(),
    pre: impl FnOnce() -> (Pre, C),
    original: impl FnOnce() -> T,
    post: impl FnOnce(C) -> Post,
) -> T {
    let generation = collector_lifetime::world_lifetime::generation();
    if collector_lifetime::world_lifetime::invalidating() {
        observation_failed(
            site,
            "equipment destruction native world awaits hourly boundary",
        );
        bail();
        return original();
    }
    if OBSERVING.get() {
        observation_failed(site, "equipment destruction pool observations overlap");
        bail();
        return original();
    }
    if FREEING.get() != 0 {
        observation_failed(
            site,
            "equipment observation overlaps active native destructor",
        );
        bail();
        return original();
    }
    let _observing = Observing::enter();
    let id = next_id();
    let reserved = LOG
        .reserve(event(Kind::Close {
            id,
            post: Post::Unsettled,
        }))
        .and_then(|close| {
            let basis = LOG.reserve(event(Kind::Basis {
                id,
                site,
                pre: failed_pool(NO_BASIS),
            }))?;
            Ok((close, basis))
        });
    let Ok((close, mut basis)) = reserved else {
        refused_event(Some(site));
        bail();
        return original();
    };
    if LOG.push(event(Kind::Open { id, site })).is_err() {
        refused_event(Some(site));
        drop((close, basis));
        bail();
        return original();
    }
    let close = Closing(Some(close));
    let (pre, context) = pre();
    #[cfg(test)]
    tests::after_pool_read(site);
    if let Kind::Basis { pre: read, .. } = &mut basis.kind {
        *read = pre;
    }
    basis.push();
    let result = original();
    let read = if generation != collector_lifetime::world_lifetime::generation() {
        Post::WorldChanged
    } else {
        post(context)
    };
    close.finish(|kind| {
        if let Kind::Close { post, .. } = kind {
            *post = read;
        }
    });
    result
}

fn observation_failed(site: SiteId, reason: &'static str) {
    log(Kind::Refused { site, reason });
}

fn recording() -> bool {
    let admitted = ENABLED.load(Ordering::SeqCst)
        && !collector_lifetime::world_lifetime::invalidating()
        && SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst)
        && !collector_memory::terminal::failed();
    if !admitted {
        revoke_active_observation();
    }
    admitted
}

fn skipped_credit_mutation(army: Option<usize>) {
    if !ENABLED.load(Ordering::SeqCst) || !SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let scope = army.and_then(|army| DELETION.get().filter(|scope| scope.army == army));
    if scope.is_some_and(|scope| scope.ignored) {
        return;
    }
    let identity = army.and_then(|army| {
        native::army_identity(
            &unsafe { engine_native_memory::memory::LiveMemory::new() },
            BASE.load(Ordering::Relaxed),
            army,
        )
        .ok()
    });
    log(Kind::Skipped {
        scope,
        army,
        identity,
    });
}

fn pool_rows(memory: &impl Memory, base: usize, pool: usize) -> Read<Vec<Row>> {
    let count = engine_native::cequipment_variant::pool::entries(memory, base, pool)?.count as usize;
    let mut rows = engine_allocation::advisory(|| engine_allocation::default_vec(count))
        .map_err(|_| EVENT_STORAGE_FAILED)?;
    let len = native::snapshot(memory, base, pool, &mut rows)?;
    rows.truncate(len);
    Ok(rows)
}
fn touched_rows(
    memory: &impl Memory,
    base: usize,
    pool: usize,
    selected: impl Fn(usize) -> bool,
) -> Read<Vec<Row>> {
    let count = engine_native::cequipment_variant::pool::entries(memory, base, pool)?.count as usize;
    let mut rows = engine_allocation::advisory(|| engine_allocation::default_vec(count))
        .map_err(|_| EVENT_STORAGE_FAILED)?;
    let len = native::touched_reserves_if(memory, base, pool, selected, &mut rows)?;
    rows.truncate(len);
    Ok(rows)
}
fn variant_amount(memory: &impl Memory, base: usize, pool: usize, variant: usize) -> Read<i64> {
    let count = engine_native::cequipment_variant::pool::entries(memory, base, pool)?.count as usize;
    let bytes = count
        .checked_mul(equipment_entry::STRIDE)
        .ok_or("destruction variant snapshot size overflow")?;
    let mut scratch = engine_allocation::advisory(|| engine_allocation::default_vec(bytes))
        .map_err(|_| EVENT_STORAGE_FAILED)?;
    native::variant_amount(memory, base, pool, variant, &mut scratch)
}
fn selects(sources: &[Row], variant: usize) -> bool {
    let start = sources.partition_point(|row| row.variant_pointer < variant);
    sources[start..]
        .iter()
        .take_while(|row| row.variant_pointer == variant)
        .next()
        .is_some()
}

fn army_receiver(site: SiteId, receiver: usize) -> Result<Option<usize>, &'static str> {
    match site {
        SiteId::Land | SiteId::Paradrop | SiteId::SetExiled => Ok(Some(receiver)),
        SiteId::VolunteerReturnLoss
        | SiteId::CapitulationEscapeLoss
        | SiteId::ScriptedEquipmentFraction => Ok(Some(
            receiver
                .checked_sub(engine_native::carmy::EQUIPMENT_POOL)
                .ok_or("army equipment pool address underflow")?,
        )),
        _ => Ok(None),
    }
}

fn nested_in_merge(site: SiteId, army: Option<usize>) -> bool {
    let Some((army, scope)) = army
        .filter(|_| WHOLE_MERGE.get())
        .and_then(|army| Some((army, DELETION.get().filter(|scope| scope.army == army)?)))
    else {
        return false;
    };
    let identity = if scope.ignored && scope.handle == 0 {
        Ok(0)
    } else {
        native::army_identity(
            &unsafe { engine_native_memory::memory::LiveMemory::new() },
            BASE.load(Ordering::Relaxed),
            army,
        )
    };
    log(Kind::Nested {
        site,
        army,
        scope,
        identity,
    });
    true
}

fn observed_owner(scope: Option<Scope>, identity: Read<u64>, owner: Read<u32>) -> Read<u32> {
    match scope {
        Some(scope) if (scope.ignored && scope.handle == 0) || identity == Ok(scope.handle) => {
            Ok(scope.country)
        }
        _ => owner,
    }
}

fn measure_pool(
    site: SiteId,
    receiver: usize,
    cause: native::Cause,
    strike: bool,
    original: impl FnOnce(),
) {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let army = match army_receiver(site, receiver) {
        Ok(army) => army,
        Err(reason) => {
            observation_failed(site, reason);
            original();
            return;
        }
    };
    if nested_in_merge(site, army) {
        original();
        return;
    }
    let scope = army.and_then(|army| DELETION.get().filter(|scope| scope.army == army));
    observe_native(
        site,
        || {},
        || {
            let identity = army.map_or(Err("pool writer has no army"), |army| {
                native::army_identity(&memory, base, army)
            });
            let owner = match army {
                Some(army) => native::army_owner(&memory, army),
                None => match site {
                    SiteId::Air => native::air_owner(&memory, receiver),
                    SiteId::Garrison => native::garrison_owner(&memory, receiver),
                    SiteId::ScriptedReserveEquipmentFraction => {
                        native::reserve_owner(&memory, base, receiver)
                    }
                    _ => Err("unsupported pool destruction writer"),
                },
            };
            let pool = match site {
                SiteId::Land | SiteId::Paradrop | SiteId::SetExiled => receiver
                    .checked_add(engine_native::carmy::EQUIPMENT_POOL)
                    .ok_or("army equipment pool address overflow"),
                SiteId::VolunteerReturnLoss
                | SiteId::CapitulationEscapeLoss
                | SiteId::ScriptedEquipmentFraction
                | SiteId::ScriptedReserveEquipmentFraction => Ok(receiver),
                _ => native::pool(receiver, site),
            };
            let rows = pool.and_then(|pool| pool_rows(&memory, base, pool));
            let forward = (strike && army.is_none_or(|_| scope.is_none_or(|s| !s.ignored)))
                .then(|| {
                    let country = if army.is_some() {
                        observed_owner(scope, identity, owner)
                    } else {
                        owner
                    };
                    let copy = rows.as_ref().ok().and_then(|rows| {
                        engine_allocation::advisory(|| {
                            engine_allocation::collect_vec(rows.iter().copied())
                        })
                        .ok()
                    });
                    country.ok().zip(copy)
                })
                .flatten();
            let pre = Pre::Pool(PoolPre {
                cause,
                army,
                scope,
                identity,
                owner,
                pool,
                rows,
            });
            (pre, (pool, forward))
        },
        original,
        |(pool, forward): (Read<usize>, Option<(u32, Vec<Row>)>)| {
            let mut after = pool.and_then(|pool| pool_rows(&memory, base, pool));
            if let (Some((country, mut before)), Ok(after)) = (forward, after.as_mut()) {
                let forwarded = native::removals(&mut before, after, |variant, amount_raw| {
                    collector_observers::nuclear_strikes::equipment(
                        site == SiteId::Air,
                        country,
                        variant,
                        amount_raw,
                    );
                });
                if forwarded.is_ok() {
                    collector_observers::nuclear_strikes::equipment_observed();
                }
            }
            Post::Pool(after)
        },
    );
}

type ArmyFn =
    unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize, usize, usize, usize);
core::arch::global_asm!(
    ".text", ".globl destruction_land_entry", ".seh_proc destruction_land_entry",
    "destruction_land_entry:", "sub rsp, 0x78", ".seh_stackalloc 0x78", ".seh_endprologue",
    "mov rax, [rsp + 0xa0]", "mov [rsp + 0x20], rax",
    "mov rax, [rsp + 0xa8]", "mov [rsp + 0x28], rax",
    "mov rax, [rsp + 0xb0]", "mov [rsp + 0x30], rax",
    "mov rax, [rsp + 0xb8]", "mov [rsp + 0x38], rax",
    "mov rax, [rsp + 0x78]", "mov [rsp + 0x40], rax",
    "call {body}", "add rsp, 0x78", "ret", ".seh_endproc", body = sym land);
unsafe extern "system-unwind" {
    fn destruction_land_entry();
}

#[allow(clippy::too_many_arguments)]
unsafe extern "system-unwind" fn land(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
    f: usize,
    g: usize,
    h: usize,
    caller: usize,
) {
    let _call = Call::enter(SiteId::Land);
    let _experience = collector_division_parts::army_stat_changes::experience_damage(a);
    let _strike = (b != 0)
        .then(|| collector_observers::nuclear_strikes::equipment_call(None))
        .flatten();
    let original: ArmyFn =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Land as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(a, b, c, d, e, f, g, h) });
    if !ENABLED.load(Ordering::SeqCst) || collector_memory::terminal::failed() || b == 0 {
        if b != 0 {
            revoke_active_observation();
            skipped_credit_mutation(Some(a));
        }
        run();
        return;
    }
    let base = BASE.load(Ordering::Relaxed);
    let cause = count_cause(native::land_cause(base, caller));
    measure_pool(SiteId::Land, a, cause, _strike.is_some(), run);
}

#[unsafe(naked)]
unsafe extern "system-unwind" fn air_entry() {
    core::arch::naked_asm!("mov r9, [rsp]", "jmp {body}", body = sym air);
}
unsafe extern "system-unwind" fn air(a: usize, b: usize, c: usize, caller: usize) {
    let _call = Call::enter(SiteId::Air);
    let _wing = collector_observers::wing_changes::kill_airplanes(a);
    let _crew = collector_observers::crew_changes::notify(a);
    let _strike = (b as u32 != 0)
        .then(|| collector_observers::nuclear_strikes::equipment_call(Some(a)))
        .flatten();
    let original: unsafe extern "system-unwind" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Air as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(a, b, c) });
    if !ENABLED.load(Ordering::SeqCst) || collector_memory::terminal::failed() || b as u32 == 0 {
        if b as u32 != 0 {
            revoke_active_observation();
        }
        run();
        return;
    }
    let base = BASE.load(Ordering::Relaxed);
    let cause = count_cause(native::air_cause(base, caller));
    measure_pool(SiteId::Air, a, cause, _strike.is_some(), run);
}
unsafe extern "system-unwind" fn garrison(a: usize, b: usize, c: usize) {
    let _call = Call::enter(SiteId::Garrison);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Garrison as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(a, b, c) });
    if !ENABLED.load(Ordering::SeqCst) || collector_memory::terminal::failed() {
        revoke_active_observation();
        run();
        return;
    }
    measure_pool(SiteId::Garrison, a, native::Cause::Garrison, false, run);
}

unsafe extern "system-unwind" fn attrition(pool: usize, variant: usize, quantity: usize) {
    let _call = Call::enter(SiteId::Attrition);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize) = unsafe {
        std::mem::transmute(ORIGINAL[SiteId::Attrition as usize].load(Ordering::Acquire))
    };
    let run = || _call.run(|| unsafe { original(pool, variant, quantity) });
    let army = pool.checked_sub(engine_native::carmy::EQUIPMENT_POOL);
    if !ENABLED.load(Ordering::SeqCst) || collector_memory::terminal::failed() {
        revoke_active_observation();
        skipped_credit_mutation(army);
        run();
        return;
    }
    if nested_in_merge(SiteId::Attrition, army) {
        run();
        return;
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let scope = army.and_then(|army| DELETION.get().filter(|scope| scope.army == army));
    observe_native(
        SiteId::Attrition,
        || {},
        || {
            let pre = VariantPre {
                army: army.ok_or("attrition army address underflow"),
                pool,
                scope,
                identity: army.map_or(Err("attrition army address underflow"), |army| {
                    native::army_identity(&memory, base, army)
                }),
                owner: army.map_or(Err("attrition army address underflow"), |army| {
                    native::army_owner(&memory, army)
                }),
                design: engine_native_parts::equipment_variant_identity::identity(&memory, base, variant),
                amount: variant_amount(&memory, base, pool, variant),
            };
            (Pre::Variant(pre), ())
        },
        run,
        |()| Post::Variant(variant_amount(&memory, base, pool, variant)),
    );
}

fn deletion_scope(
    memory: &impl Memory,
    base: usize,
    state: usize,
    army: usize,
    caller: usize,
) -> Read<Option<(Scope, u32, Option<(native::Cause, native::Disposition)>)>> {
    let classified = native::deletion_context(base, caller);
    let merge_source = classified == native::DeletionContext::Transfer(native::Disposition::Merged);
    let classified = match classified {
        native::DeletionContext::Ignore => return Ok(Some((Scope::ignored(army), 0, None))),
        native::DeletionContext::Queue => None,
        native::DeletionContext::Loss(cause) => Some((cause, native::Disposition::Loss)),
        native::DeletionContext::Transfer(disposition) => {
            Some((native::Cause::Unknown, disposition))
        }
    };
    native::current_state(memory, base, state)?;
    if native::cleanup(memory, base)? {
        return Ok(Some((Scope::ignored(army), 0, None)));
    }
    if !native::is_army(memory, base, army)? {
        return Ok(None);
    }
    if native::fake_intel_division(memory, army)? {
        return Ok(Some((Scope::ignored(army), 0, None)));
    }
    let handle = native::army_identity(memory, base, army)?;
    let country = native::army_owner(memory, army)?;
    let expeditionary_owner = native::army_expeditionary_owner(memory, army)?;
    Ok(Some((
        Scope {
            id: next_id(),
            army,
            handle,
            country,
            ignored: false,
            merge_source,
            observed: false,
        },
        expeditionary_owner,
        classified,
    )))
}

#[unsafe(naked)]
unsafe extern "system-unwind" fn delete_entry() {
    core::arch::naked_asm!("mov rax, [rsp]", "jmp {body}", body = sym destruction_delete_shim);
}
core::arch::global_asm!(
    ".text", ".globl destruction_delete_shim", ".seh_proc destruction_delete_shim",
    "destruction_delete_shim:", "sub rsp, 0x38", ".seh_stackalloc 0x38", ".seh_endprologue",
    "mov [rsp + 0x20], rax", "call {body}", "add rsp, 0x38", "ret", ".seh_endproc", body = sym delete);
unsafe extern "system-unwind" {
    fn destruction_delete_shim();
}
unsafe extern "system-unwind" fn delete(
    state: usize,
    unit: usize,
    return_equipment: usize,
    behavior: usize,
    caller: usize,
) {
    let mut call = Call::enter(SiteId::Delete);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Delete as usize].load(Ordering::Acquire)) };
    let scope = if recording() {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        match deletion_scope(&memory, BASE.load(Ordering::Relaxed), state, unit, caller) {
            Ok(Some((scope, expeditionary_owner, classified))) => {
                log(Kind::Delete {
                    scope,
                    expeditionary_owner,
                    classified,
                });
                Some(scope)
            }
            Ok(None) => None,
            Err(reason) => {
                observation_failed(SiteId::Delete, reason);
                None
            }
        }
    } else {
        None
    };
    if scope.is_some_and(|scope| !scope.ignored) {
        call.arm();
    }
    let _scope = DeletionScope::enter(scope);
    call.run(|| unsafe { original(state, unit, return_equipment, behavior) });
    call.settle();
    if ENABLED.load(Ordering::SeqCst) && SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst) {
        log(Kind::Retire {
            army: unit,
            handle: scope
                .filter(|scope| scope.handle != 0)
                .map(|scope| scope.handle),
            scope: scope.map(|scope| scope.id),
        });
    }
}

/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn returned_at(site: SiteId, reserves: usize, pool: usize) {
    let _call = Call::enter(site);
    let original: unsafe extern "system-unwind" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[site as usize].load(Ordering::Acquire)) };
    let army = pool.checked_sub(engine_native::carmy::EQUIPMENT_POOL);
    let run = || _call.run(|| unsafe { original(reserves, pool) });
    if !recording() {
        skipped_credit_mutation(army);
        run();
        return;
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let reserve_pool = native::reserve_pool(reserves);
    observe_native(
        site,
        || skipped_credit_mutation(army),
        || {
            let scope = army.and_then(|army| DELETION.get().filter(|scope| scope.army == army));
            let army = army.ok_or("returned army address underflow");
            let sources = pool_rows(&memory, base, pool).map(|mut rows| {
                rows.sort_unstable_by_key(|row| row.variant_pointer);
                rows
            });
            let selected = sources.as_ref().ok().and_then(|rows| {
                engine_allocation::advisory(|| engine_allocation::collect_vec(rows.iter().copied()))
                    .ok()
            });
            let touched = reserve_pool.and_then(|reserves| {
                let sources = sources.as_ref().map_err(|reason| *reason)?;
                touched_rows(&memory, base, reserves, |variant| selects(sources, variant))
            });
            let pre = ReturnPre {
                army,
                pool,
                scope,
                excluded: army.and_then(|army| {
                    Ok(native::cleanup(&memory, base)?
                        || native::fake_intel_division(&memory, army)?)
                }),
                identity: army.and_then(|army| native::army_identity(&memory, base, army)),
                owner: army.and_then(|army| native::army_owner(&memory, army)),
                reserves: reserve_pool,
                reserve_owner: reserve_pool
                    .and_then(|reserves| native::reserve_owner(&memory, base, reserves)),
                sources,
                touched,
            };
            (boxed(pre).map_or_else(failed_pool, Pre::Return), selected)
        },
        run,
        |selected: Option<Vec<Row>>| Post::Return {
            identity: army
                .ok_or("returned army address underflow")
                .and_then(|army| native::army_identity(&memory, base, army)),
            touched: reserve_pool.and_then(|reserves| {
                let sources = selected.as_deref().ok_or(EVENT_STORAGE_FAILED)?;
                touched_rows(&memory, base, reserves, |variant| selects(sources, variant))
            }),
        },
    );
}
unsafe extern "system-unwind" fn returned(reserves: usize, pool: usize) {
    unsafe {
        returned_at(SiteId::Return, reserves, pool);
    }
}
unsafe extern "system-unwind" fn hq_returned(reserves: usize, pool: usize) {
    unsafe {
        returned_at(SiteId::HqReturn, reserves, pool);
    }
}

fn direct_scope(memory: &impl Memory, base: usize, army: usize) -> Read<Option<(Scope, u32)>> {
    if native::cleanup(memory, base)? || native::fake_intel_division(memory, army)? {
        return Ok(None);
    }
    let handle = native::army_identity(memory, base, army)?;
    let country = native::army_owner(memory, army)?;
    let expeditionary_owner = native::army_expeditionary_owner(memory, army)?;
    Ok(Some((
        Scope {
            id: next_id(),
            army,
            handle,
            country,
            ignored: false,
            merge_source: false,
            observed: false,
        },
        expeditionary_owner,
    )))
}

#[unsafe(naked)]
unsafe extern "system-unwind" fn division_free_entry() {
    core::arch::naked_asm!("jmp {body}", body = sym division_free);
}
unsafe extern "system-unwind" fn division_free(army: usize, flags: usize) -> usize {
    let mut call = Call::enter(SiteId::DivisionFree);
    let _freeing = InDestructor::enter();
    let original: unsafe extern "system-unwind" fn(usize, usize) -> usize = unsafe {
        std::mem::transmute(ORIGINAL[SiteId::DivisionFree as usize].load(Ordering::Acquire))
    };
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let admitted = recording();
    let id = next_id();
    let mut freed = None;
    if ENABLED.load(Ordering::SeqCst) {
        match LOG.reserve(event(Kind::Freed { id, unwound: true })) {
            Ok(node) if LOG.push(event(Kind::Freeing { id, army })).is_ok() => {
                freed = Some(Closing(Some(node)));
            }
            _ => refused_event(Some(SiteId::DivisionFree)),
        }
    }
    let mut direct = None;
    let mut direct_scope_guard = None;
    if admitted && DELETION.get().is_none_or(|scope| scope.army != army) {
        match direct_scope(&memory, base, army) {
            Ok(Some(found)) => {
                direct_scope_guard = Some(DeletionScope::enter(Some(found.0)));
                direct = Some(found);
            }
            Ok(None) => {}
            Err(reason) => observation_failed(SiteId::DivisionFree, reason),
        }
    }
    let mut deleted = None;
    if admitted
        && let Some(mut scope) = DELETION.get()
        && scope.army == army
        && !scope.ignored
        && !scope.observed
    {
        scope.observed = true;
        DELETION.set(Some(scope));
        deleted = Some(
            army.checked_add(engine_native::carmy::EQUIPMENT_POOL)
                .ok_or("deleted army equipment pool address overflow")
                .and_then(|pool| pool_rows(&memory, base, pool)),
        );
    }
    #[cfg(test)]
    tests::after_pool_read(SiteId::DivisionFree);
    let scope = DELETION.get().filter(|scope| scope.army == army);
    let identity = native::army_identity(&memory, base, army);
    let free = Free {
        id,
        army,
        direct,
        scope,
        deleted,
        enabled: ENABLED.load(Ordering::SeqCst)
            && SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst)
            && !scope.is_some_and(|scope| scope.ignored),
        identity: identity.ok(),
        merge_source: WHOLE_MERGE.get()
            && scope.is_some_and(|scope| scope.merge_source && identity == Ok(scope.handle)),
        observing: OBSERVING.get(),
    };
    if freed.is_some()
        && boxed(free)
            .and_then(|free| {
                LOG.push(event(Kind::Free(free)))
                    .map_err(|_| EVENT_STORAGE_FAILED)
            })
            .is_err()
    {
        refused_event(Some(SiteId::DivisionFree));
    }
    if DELETION.get().is_some_and(|scope| !scope.ignored) {
        call.arm();
    }
    let result = call.run(|| unsafe { original(army, flags) });
    call.settle();
    if let Some(freed) = freed {
        freed.finish(|kind| {
            if let Kind::Freed { unwound, .. } = kind {
                *unwound = false;
            }
        });
    }
    drop(direct_scope_guard);
    result
}

#[unsafe(naked)]
unsafe extern "system-unwind" fn queue_entry() {
    core::arch::naked_asm!("mov r8, [rsp]", "jmp {body}", body = sym queue);
}
unsafe extern "system-unwind" fn queue(state: usize, unit: usize, caller: usize) {
    let mut call = Call::enter(SiteId::Queue);
    let original: unsafe extern "system-unwind" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Queue as usize].load(Ordering::Acquire)) };
    if recording() {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Relaxed);
        let result = (|| {
            let classified = native::queued_context(base, caller);
            let (cause, disposition) = match classified {
                native::QueuedContext::Ignore => return Ok(None),
                native::QueuedContext::Loss(cause) => (cause, native::Disposition::Loss),
                native::QueuedContext::Transfer(disposition) => {
                    (native::Cause::Unknown, disposition)
                }
            };
            native::current_state(&memory, base, state)?;
            if native::cleanup(&memory, base)?
                || !native::is_army(&memory, base, unit)?
                || native::fake_intel_division(&memory, unit)?
            {
                return Ok(None);
            }
            let handle = native::army_identity(&memory, base, unit)?;
            Ok(Some((handle, cause, disposition)))
        })();
        match result {
            Ok(Some((handle, cause, disposition))) => {
                log(Kind::Queue {
                    army: unit,
                    handle,
                    cause,
                    disposition,
                });
                call.arm();
            }
            Ok(None) => {}
            Err(reason) => observation_failed(SiteId::Queue, reason),
        }
    } else if ENABLED.load(Ordering::SeqCst) {
        log(Kind::SkippedQueue);
    } else {
        SKIPPED_QUEUE.store(true, Ordering::SeqCst);
    }
    call.run(|| unsafe { original(state, unit) });
    call.settle();
}

unsafe extern "system-unwind" fn paradrop(
    army: usize,
    source: usize,
    equipment: usize,
    manpower: usize,
    organisation: usize,
) {
    let _call = Call::enter(SiteId::Paradrop);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Paradrop as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(army, source, equipment, manpower, organisation) });
    if !recording() {
        skipped_credit_mutation(Some(army));
        run();
        return;
    }
    measure_pool(
        SiteId::Paradrop,
        army,
        native::Cause::ParadropDisruption,
        false,
        run,
    );
}

fn transfer_before(
    memory: &impl Memory,
    base: usize,
    army: usize,
    disposition: native::Disposition,
) -> Result<Option<Entry>, &'static str> {
    if native::cleanup(memory, base)? || native::fake_intel_division(memory, army)? {
        return Ok(None);
    }
    Ok(Some(Entry {
        country: native::army_owner(memory, army)?,
        expeditionary_owner: native::army_expeditionary_owner(memory, army)?,
        division_handle: native::army_identity(memory, base, army)?,
        disposition,
        ..Default::default()
    }))
}

/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn transfer_roster(
    site: SiteId,
    country: usize,
    army: usize,
    disposition: native::Disposition,
) -> usize {
    let mut call = Call::enter(site);
    let original: unsafe extern "system-unwind" fn(usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[site as usize].load(Ordering::Acquire)) };
    let pending = if recording() {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        transfer_before(&memory, BASE.load(Ordering::Relaxed), army, disposition)
    } else {
        Ok(None)
    };
    if pending.as_ref().is_ok_and(|entry| entry.is_some()) {
        call.arm();
    }
    if let Err(reason) = pending {
        observation_failed(site, reason);
    }
    let result = call.run(|| unsafe { original(country, army) });
    if let Ok(Some(entry)) = pending
        && result as u8 != 0
    {
        log(Kind::Transfer { site, entry });
    }
    call.settle();
    result
}
unsafe extern "system-unwind" fn volunteer_transfer(country: usize, army: usize) -> usize {
    unsafe {
        transfer_roster(
            SiteId::VolunteerTransfer,
            country,
            army,
            native::Disposition::VolunteerTransferred,
        )
    }
}
unsafe extern "system-unwind" fn exile_transfer(country: usize, army: usize) -> usize {
    unsafe {
        transfer_roster(
            SiteId::ExileTransfer,
            country,
            army,
            native::Disposition::ExileTransferred,
        )
    }
}

/// # Safety
/// The arguments are ones the selected site's published original accepts with its ABI, and
/// every object they reach stays valid for the reads its site makes; one the original destroys
/// is an identity only once it returns.
unsafe fn pool_scale(site: SiteId, receiver: usize, factor: usize, cause: native::Cause) {
    let _call = Call::enter(site);
    let original: unsafe extern "system-unwind" fn(usize, usize) =
        unsafe { std::mem::transmute(ORIGINAL[site as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(receiver, factor) });
    if recording() {
        measure_pool(site, receiver, cause, false, run);
    } else {
        if let Ok(Some(army)) = army_receiver(site, receiver) {
            skipped_credit_mutation(Some(army));
        }
        run();
    }
}
unsafe extern "system-unwind" fn set_exiled(army: usize, exiled: usize) {
    unsafe {
        pool_scale(
            SiteId::SetExiled,
            army,
            exiled,
            native::Cause::ExileEquipment,
        );
    }
}
unsafe extern "system-unwind" fn volunteer_return_loss(pool: usize, factor: usize) {
    unsafe {
        pool_scale(
            SiteId::VolunteerReturnLoss,
            pool,
            factor,
            native::Cause::VolunteerReturnEquipment,
        );
    }
}
unsafe extern "system-unwind" fn capitulation_escape_loss(pool: usize, factor: usize) {
    unsafe {
        pool_scale(
            SiteId::CapitulationEscapeLoss,
            pool,
            factor,
            native::Cause::CapitulationEscapeEquipment,
        );
    }
}
unsafe extern "system-unwind" fn scripted_equipment_fraction(pool: usize, factor: usize) {
    unsafe {
        pool_scale(
            SiteId::ScriptedEquipmentFraction,
            pool,
            factor,
            native::Cause::ScriptedEquipmentFraction,
        );
    }
}
unsafe extern "system-unwind" fn scripted_reserve_equipment_fraction(pool: usize, factor: usize) {
    unsafe {
        pool_scale(
            SiteId::ScriptedReserveEquipmentFraction,
            pool,
            factor,
            native::Cause::ScriptedStockpileFraction,
        );
    }
}

struct WholeMergeScope(bool);
impl WholeMergeScope {
    fn enter() -> Self {
        Self(WHOLE_MERGE.replace(true))
    }
}
impl Drop for WholeMergeScope {
    fn drop(&mut self) {
        WHOLE_MERGE.set(self.0);
    }
}

fn merge_selected(sources: &[Row], held: &[HeldRow], main: bool, variant: usize) -> bool {
    let start = sources.partition_point(|row| row.variant_pointer < variant);
    (main
        && sources[start..]
            .iter()
            .take_while(|row| row.variant_pointer == variant)
            .any(|row| row.amount_raw > 0))
        || held
            .binary_search_by_key(&variant, |held| held.row.variant_pointer)
            .is_ok()
}

#[derive(Default)]
struct MergeReads {
    survivor: usize,
    sources: Vec<Row>,
    scopes: Vec<(usize, bool, Vec<HeldRow>)>,
}

fn merge_pre(memory: &impl Memory, base: usize, argument: usize) -> (Pre, MergeReads) {
    let mut reads = MergeReads::default();
    let mut pools = Vec::new();
    let excluded = native::cleanup(memory, base);
    let failed = |reason| MergePre {
        excluded,
        head: Err(reason),
        country: Err(reason),
        armies: Err(reason),
        rest: Err(reason),
        pools: Vec::new(),
    };
    let pre = (|| {
        if excluded != Ok(false) {
            return failed("merge observation excluded");
        }
        let head = (|| {
            let inputs = native::merge_input_count(memory, argument)?;
            let mut armies = engine_allocation::advisory(|| {
                engine_allocation::default_vec(inputs.saturating_add(1))
            })
            .map_err(|_| EVENT_STORAGE_FAILED)?;
            let (survivor, len) = native::merge_armies(memory, base, argument, &mut armies)?;
            armies.truncate(len);
            Ok((
                survivor,
                armies,
                native::army_identity(memory, base, survivor)?,
            ))
        })();
        let (survivor, armies, handle) = match head {
            Ok(head) => head,
            Err(reason) => return failed(reason),
        };
        reads.survivor = survivor;
        let country = native::merge_owner(memory, survivor);
        let read_armies = (|| {
            let mut read = engine_allocation::advisory(|| engine_allocation::vec(armies.len()))
                .map_err(|_| EVENT_STORAGE_FAILED)?;
            for &army in &armies {
                let pool = army
                    .checked_add(engine_native::carmy::EQUIPMENT_POOL)
                    .ok_or("merge pool address overflow")?;
                let rows = pool_rows(memory, base, pool)?;
                let identity = native::army_identity(memory, base, army)?;
                read.push(MergeArmy {
                    army,
                    rows,
                    identity,
                    owner: native::army_owner(memory, army),
                });
            }
            Ok(read)
        })();
        let rest = read_armies
            .as_ref()
            .map_err(|reason| *reason)
            .and_then(|read| {
                let total = read.iter().map(|army| army.rows.len()).sum();
                let sources = &mut reads.sources;
                engine_allocation::advisory(|| {
                    engine_allocation::reserve(sources, total)
                        .and_then(|()| engine_allocation::reserve(&mut pools, read.len()))
                })
                .map_err(|_| EVENT_STORAGE_FAILED)?;
                for army in read {
                    sources.extend_from_slice(&army.rows);
                    pools.push(
                        army.army
                            .saturating_add(engine_native::carmy::EQUIPMENT_POOL),
                    );
                }
                sources.sort_unstable_by_key(|row| row.variant_pointer);
                merge_rest(memory, base, &armies, survivor, &mut reads, &mut pools)
            });
        MergePre {
            excluded,
            head: Ok((survivor, handle)),
            country,
            armies: read_armies,
            rest,
            pools: std::mem::take(&mut pools),
        }
    })();
    let pre = boxed(pre).map_or_else(failed_pool, Pre::Merge);
    (pre, reads)
}
fn failed_pool(reason: &'static str) -> Pre {
    Pre::Pool(PoolPre {
        cause: native::Cause::Unknown,
        army: None,
        scope: None,
        identity: Err(reason),
        owner: Err(reason),
        pool: Err(reason),
        rows: Err(reason),
    })
}
fn merge_rest(
    memory: &impl Memory,
    base: usize,
    armies: &[usize],
    survivor: usize,
    reads: &mut MergeReads,
    pools: &mut Vec<usize>,
) -> Read<MergeRest> {
    let sources = &reads.sources;
    let main_reserve = if sources.iter().any(|row| row.amount_raw > 0) {
        native::merge_reserves(memory, base, survivor)?
    } else {
        0
    };
    let mut held: Vec<HeldRow> = Vec::new();
    for &army in armies {
        let mut destination = None;
        native::canceled_delivery_pools(memory, base, army, army != survivor, |pool| {
            let rows = pool_rows(memory, base, pool)?;
            if rows.is_empty() {
                return Ok(());
            }
            let (destination, identity) = match destination {
                Some(destination) => destination,
                None => {
                    let reserve = native::merge_reserves(memory, base, army)?;
                    let identity = native::reserve_owner(memory, base, reserve)?;
                    destination = Some((reserve, identity));
                    (reserve, identity)
                }
            };
            engine_allocation::advisory(|| {
                engine_allocation::reserve(&mut held, rows.len())
                    .and_then(|()| engine_allocation::reserve(pools, 1))
            })
            .map_err(|_| EVENT_STORAGE_FAILED)?;
            pools.push(pool);
            held.extend(rows.into_iter().map(|row| HeldRow {
                destination,
                country: identity,
                row,
            }));
            Ok(())
        })?;
    }
    held.sort_unstable_by_key(|held| (held.destination, held.row.variant_pointer));
    let mut scopes: Vec<MergeScope> = Vec::new();
    let mut start = 0;
    while start < held.len() {
        let first = held[start];
        let mut end = start + 1;
        while end < held.len() && held[end].destination == first.destination {
            if held[end].country != first.country {
                return Err("merge reserve destination changed identity");
            }
            end += 1;
        }
        let group = engine_allocation::advisory(|| {
            engine_allocation::collect_vec(held[start..end].iter().copied())
        })
        .map_err(|_| EVENT_STORAGE_FAILED)?;
        engine_allocation::advisory(|| engine_allocation::reserve(&mut scopes, 1))
            .map_err(|_| EVENT_STORAGE_FAILED)?;
        scopes.push(MergeScope {
            pool: first.destination,
            main: first.destination == main_reserve,
            identity: first.country,
            held: group,
            touched: Vec::new(),
        });
        start = end;
    }
    if main_reserve != 0 && !scopes.iter().any(|scope| scope.pool == main_reserve) {
        engine_allocation::advisory(|| engine_allocation::reserve(&mut scopes, 1))
            .map_err(|_| EVENT_STORAGE_FAILED)?;
        scopes.push(MergeScope {
            pool: main_reserve,
            main: true,
            identity: native::reserve_owner(memory, base, main_reserve)?,
            held: Vec::new(),
            touched: Vec::new(),
        });
    }
    engine_allocation::advisory(|| {
        engine_allocation::reserve(&mut reads.scopes, scopes.len())
            .and_then(|()| engine_allocation::reserve(pools, scopes.len()))
    })
    .map_err(|_| EVENT_STORAGE_FAILED)?;
    for scope in &mut scopes {
        scope.touched = touched_rows(memory, base, scope.pool, |variant| {
            merge_selected(&reads.sources, &scope.held, scope.main, variant)
        })?;
        let held = engine_allocation::advisory(|| {
            engine_allocation::collect_vec(scope.held.iter().copied())
        })
        .map_err(|_| EVENT_STORAGE_FAILED)?;
        reads.scopes.push((scope.pool, scope.main, held));
        pools.push(scope.pool);
    }
    Ok(MergeRest { scopes })
}
fn merge_post(memory: &impl Memory, base: usize, reads: MergeReads) -> Post {
    let MergeReads {
        survivor,
        sources,
        scopes,
    } = reads;
    let mut read = Vec::new();
    if engine_allocation::advisory(|| engine_allocation::reserve(&mut read, scopes.len())).is_err() {
        return Post::Merge {
            identity: Err(EVENT_STORAGE_FAILED),
            rows: Err(EVENT_STORAGE_FAILED),
            scopes: Vec::new(),
        };
    }
    for (pool, main, held) in &scopes {
        read.push((
            native::reserve_owner(memory, base, *pool),
            touched_rows(memory, base, *pool, |variant| {
                merge_selected(&sources, held, *main, variant)
            }),
        ));
    }
    Post::Merge {
        identity: native::army_identity(memory, base, survivor),
        rows: survivor
            .checked_add(engine_native::carmy::EQUIPMENT_POOL)
            .ok_or("merge pool address overflow")
            .and_then(|pool| pool_rows(memory, base, pool)),
        scopes: read,
    }
}
unsafe extern "system-unwind" fn merge(argument: usize) {
    let _call = Call::enter(SiteId::Merge);
    let original: unsafe extern "system-unwind" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[SiteId::Merge as usize].load(Ordering::Acquire)) };
    let run = || _call.run(|| unsafe { original(argument) });
    if !recording() {
        skipped_credit_mutation(None);
        run();
        return;
    }
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    let base = BASE.load(Ordering::Relaxed);
    let _scope = WholeMergeScope::enter();
    observe_native(
        SiteId::Merge,
        || skipped_credit_mutation(None),
        || merge_pre(&memory, base, argument),
        run,
        |reads| merge_post(&memory, base, reads),
    );
}

fn locate_sites(
    base: usize,
    supplemental: bool,
) -> Result<Vec<(&'static native::Site, engine_sites::Code)>, Cow<'static, str>> {
    let mut located = engine_allocation::vec(native::SITES.len())
        .map_err(|_| Cow::Borrowed("collector allocation failed"))?;
    for site in native::SITES
        .into_iter()
        .filter(|site| site.id.is_supplemental() == supplemental)
    {
        match engine_sites::hook(base, site.rva) {
            Ok(code) => located.push((site, code)),
            Err(cause) => {
                collector_hooks::hook::installation::refused(
                    collector_hooks::hook::installation::Kind::Sentinel,
                    None,
                    cause,
                );
                use std::fmt::Write;
                let mut reason = String::new();
                engine_allocation::reserve_string(&mut reason, site.name.len() + cause.len() + 2)
                    .map_err(|_| Cow::Borrowed("collector allocation failed"))?;
                write!(reason, "{}: {cause}", site.name)
                    .expect("writing a reserved String cannot fail");
                return Err(Cow::Owned(reason));
            }
        }
    }
    Ok(located)
}

fn callback(id: native::SiteId) -> usize {
    use native::SiteId;
    match id {
        SiteId::Land => destruction_land_entry as *const () as usize,
        SiteId::Attrition => attrition as *const () as usize,
        SiteId::Air => air_entry as *const () as usize,
        SiteId::Garrison => garrison as *const () as usize,
        SiteId::Delete => delete_entry as *const () as usize,
        SiteId::Return => returned as *const () as usize,
        SiteId::HqReturn => hq_returned as *const () as usize,
        SiteId::Merge => merge as *const () as usize,
        SiteId::DivisionFree => division_free_entry as *const () as usize,
        SiteId::Queue => queue_entry as *const () as usize,
        SiteId::Paradrop => paradrop as *const () as usize,
        SiteId::VolunteerTransfer => volunteer_transfer as *const () as usize,
        SiteId::ExileTransfer => exile_transfer as *const () as usize,
        SiteId::SetExiled => set_exiled as *const () as usize,
        SiteId::VolunteerReturnLoss => volunteer_return_loss as *const () as usize,
        SiteId::CapitulationEscapeLoss => capitulation_escape_loss as *const () as usize,
        SiteId::ScriptedEquipmentFraction => scripted_equipment_fraction as *const () as usize,
        SiteId::ScriptedReserveEquipmentFraction => {
            scripted_reserve_equipment_fraction as *const () as usize
        }
    }
}

fn set_sites_enabled(hooks: &mut [(native::SiteId, Hook)], enabled: bool) -> Result<bool, String> {
    let complete = Hook::set_enabled_iter(hooks.iter_mut().map(|(_, hook)| hook), enabled)?;
    if complete {
        for (id, _) in hooks {
            SITE_ENABLED[*id as usize].store(enabled, Ordering::SeqCst);
        }
    }
    Ok(complete)
}

pub struct Hooks {
    original: Vec<(native::SiteId, Hook)>,
    publication_attempted: bool,
    supplemental: Vec<(native::SiteId, Hook)>,
}
impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Self {
        LazyLock::force(&STORE);
        BASE.store(base, Ordering::Relaxed);
        let install = |supplemental| -> Result<Vec<(native::SiteId, Hook)>, Cow<'static, str>> {
            native::classification_sites(base).map_err(Cow::Borrowed)?;
            let located = locate_sites(base, supplemental)?;
            let mut hooks = engine_allocation::vec(located.len())
                .map_err(|_| Cow::Borrowed("collector allocation failed"))?;
            for (site, code) in located {
                let original = code.sentinel::<16>().map_err(Cow::Borrowed)?;
                let target = match site.kind {
                    native::SiteKind::Entry => None,
                    native::SiteKind::Call { target } => Some(
                        engine_sites::hook(base, target)
                            .map(|target| target.address)
                            .map_err(Cow::Borrowed)?,
                    ),
                };
                let hook = match site.kind {
                    native::SiteKind::Entry => unsafe {
                        Hook::new(code.address, callback(site.id), original)
                    },
                    native::SiteKind::Call { .. } => unsafe {
                        Hook::new_call(code.address, callback(site.id), original)
                    },
                }
                .map_err(Cow::Owned)?;
                ORIGINAL[site.id as usize].store(
                    target.unwrap_or_else(|| hook.trampoline()),
                    Ordering::Release,
                );
                hooks.push((site.id, hook));
            }
            for &(id, _) in &hooks {
                INSTALLED[id as usize].store(true, Ordering::Release);
            }
            Ok(hooks)
        };
        let original = install(false).unwrap_or_else(|reason| {
            collector_hooks::hook::installation::refused(
                collector_hooks::hook::installation::Kind::Construction,
                None,
                &reason,
            );
            let _ = UNHOOKED.set(reason);
            Vec::new()
        });
        let supplemental = if original.is_empty() {
            let _ = SUPPLEMENTAL_UNHOOKED
                .set("supplemental skipped: original writers unavailable".into());
            Vec::new()
        } else {
            install(true).unwrap_or_else(|reason| {
                collector_hooks::hook::installation::refused(
                    collector_hooks::hook::installation::Kind::Construction,
                    None,
                    &reason,
                );
                let _ = SUPPLEMENTAL_UNHOOKED.set(reason);
                Vec::new()
            })
        };
        Self {
            original,
            supplemental,
            publication_attempted: false,
        }
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        self.publication_attempted |= enabled;
        if !enabled {
            collector_division_parts::army_stat_changes::experience_forwarder(false);
            collector_observers::wing_changes::observe_kills(false);
            collector_observers::nuclear_strikes::forwarder(
                replay_data::rows::NuclearStrikePart::Equipment,
                false,
            );
            ENABLED.store(false, Ordering::SeqCst);
            SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
            let original = set_sites_enabled(&mut self.original, false);
            let supplemental = set_sites_enabled(&mut self.supplemental, false);
            return match (original, supplemental) {
                (Err(reason), _) | (_, Err(reason)) => Err(reason),
                (Ok(a), Ok(b)) => Ok(a && b),
            };
        }
        if !set_sites_enabled(&mut self.original, enabled)? {
            return Ok(false);
        }
        collector_division_parts::army_stat_changes::experience_forwarder(
            self.original.iter().any(|(id, _)| *id == SiteId::Land),
        );
        collector_observers::wing_changes::observe_kills(
            self.original
                .iter()
                .any(|(id, _)| *id == native::SiteId::Air),
        );
        let original_count = native::SITES
            .iter()
            .filter(|site| !site.id.is_supplemental())
            .count();
        ENABLED.store(
            enabled && self.original.len() == original_count,
            Ordering::SeqCst,
        );
        collector_observers::nuclear_strikes::forwarder(
            replay_data::rows::NuclearStrikePart::Equipment,
            ENABLED.load(Ordering::SeqCst),
        );
        let supplemental_enabled =
            ENABLED.load(Ordering::SeqCst) && SUPPLEMENTAL_UNHOOKED.get().is_none();
        match set_sites_enabled(&mut self.supplemental, supplemental_enabled) {
            Ok(false) => return Ok(false),
            Err(reason) if supplemental_enabled => {
                collector_hooks::hook::installation::refused(
                    collector_hooks::hook::installation::Kind::Publication,
                    None,
                    &reason,
                );
                SUPPLEMENTAL_ENABLED.store(false, Ordering::SeqCst);
                let _ = SUPPLEMENTAL_UNHOOKED.set(Cow::Owned(reason));
                if !set_sites_enabled(&mut self.supplemental, false).inspect_err(|reason| {
                    collector_hooks::hook::installation::refused(
                        collector_hooks::hook::installation::Kind::Rollback,
                        None,
                        reason,
                    );
                })? {
                    return Ok(false);
                }
            }
            Err(reason) => return Err(reason),
            Ok(true) => {}
        }
        let supplemental_count = native::SITES
            .iter()
            .filter(|site| site.id.is_supplemental())
            .count();
        SUPPLEMENTAL_ENABLED.store(
            ENABLED.load(Ordering::SeqCst)
                && SUPPLEMENTAL_UNHOOKED.get().is_none()
                && self.supplemental.len() == supplemental_count,
            Ordering::SeqCst,
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.original
            .iter()
            .chain(&self.supplemental)
            .flat_map(|(_, hook)| hook.executable_ranges())
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        if !self.publication_attempted {
            let _ = release();
        }
    }
}
pub fn status() -> impl serde::Serialize {
    let refusal = FIRST_REFUSAL.get().copied();
    let contested = *CONTEST.lock().unwrap_or_else(PoisonError::into_inner);
    let hooks = std::array::from_fn::<_, { native::SITES.len() }, _>(|i| {
        let site = &native::SITES[i];
        collector_status::status::status!({
            "site_index": i, "name": site.name, "rva": site.rva.0, "group": if site.id.is_supplemental() { "supplemental" } else { "original_writers" },
            "installed": INSTALLED[i].load(Ordering::Acquire),
            "enabled": SITE_ENABLED[i].load(Ordering::SeqCst),
            "active": ACTIVE[i].load(Ordering::SeqCst),
            "calls": CALLS[i].load(Ordering::Relaxed),
            "refusals": SITE_REFUSALS[i].load(Ordering::Relaxed),
            "overflow": SITE_OVERFLOW[i].load(Ordering::Relaxed),
        })
    });
    collector_status::status::status!({"hooks":collector_status::status::sequence(hooks.into_iter()),"unhooked":UNHOOKED.get(),"supplemental_unhooked":SUPPLEMENTAL_UNHOOKED.get(),"supplemental_enabled":SUPPLEMENTAL_ENABLED.load(Ordering::SeqCst),"branch_unavailable":UNAVAILABLE.load(Ordering::Relaxed),
        "events":EVENTS.load(Ordering::Relaxed),"overflow":OVERFLOW.load(Ordering::Relaxed),"refusals":REFUSALS.load(Ordering::Relaxed),"refusal":refusal,
        "contested_pools":CONTESTS.load(Ordering::Relaxed),"contested_pool":contested,
        "lost_drains":LOST.load(Ordering::Relaxed),"drains":DRAINS.load(Ordering::Relaxed),"storage_refusals":STORAGE_REFUSALS.load(Ordering::Relaxed),
        "handed_over":HANDED_OVER.load(Ordering::Relaxed),"event_capacity":CAPACITY.load(Ordering::Acquire),"queue_capacity":QUEUED_CAPACITY.load(Ordering::Acquire),"store_bytes":STORE_BYTES.load(Ordering::Relaxed),
        "unknown_callers":UNKNOWN_CALLERS.load(Ordering::Relaxed),
        "hourly_peak_events":HOURLY_PEAK_EVENTS.load(Ordering::Relaxed),"hourly_peak_capacity":HOURLY_PEAK_CAPACITY.load(Ordering::Relaxed),
        "scope":"cumulative since attach; stable causes; unknown callers counted; peak includes partial or refused intervals; refusals recover at clean boundaries; first and branch intervals are partial"})
}

#[cfg(any(test, feature = "test-support"))]
pub mod tests;

#[cfg(any(test, feature = "test-support"))]
pub fn synthetic_restore_hooks(original: Hook, supplemental: Hook) -> Hooks {
    Hooks {
        original: vec![(native::SiteId::Land, original)],
        supplemental: vec![(native::SiteId::Paradrop, supplemental)],
        publication_attempted: false,
    }
}
