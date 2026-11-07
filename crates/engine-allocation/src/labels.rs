//! Memory attribution for measurement runs, counted by the allocator. Code says whose work runs
//! on its thread with [`enter`] or [`kind`]; with the `memory-labels` feature, [`Counting`]
//! wraps the global allocator, tags each allocation with the label current on its thread, and
//! charges its free back to that label, wherever it is freed. Each [`phase`] keeps every
//! label's peak, the bytes each held when the tracked total peaked, and, of an operator's bytes
//! held then, those its family's emit has freed since: its output in flight. Without the
//! feature every call here is empty.

use std::marker::PhantomData;

/// Whether this build counts allocations.
pub const ENABLED: bool = cfg!(feature = "memory-labels");

/// What work holds memory. Every kind but [`Kind::Unlabelled`] and [`Kind::Headers`] is set by
/// code; a family's kinds are its own, the others the run's ([`Owner::RUN`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Unlabelled = 0,
    /// A family's operators, and the items they project until the emit frees them.
    Operator = 1,
    /// A family's spilled items read back for its emit.
    Restore = 2,
    /// A family's ordered emit and the branch state it keeps.
    Emit = 3,
    /// Publication: stages, series writers, batches sealing.
    Publish = 4,
    /// Game content resolved and loaded.
    Content = 5,
    /// The world a walk's operators share.
    World = 6,
    /// The walker's pages, arena and tables, and what decoding them takes.
    Walker = 7,
    /// The segment executor's own work: scheduling, filing, parking.
    Executor = 8,
    /// The attribution's own tag on each allocation.
    Headers = 9,
    /// What a series writer keeps of every key it defined: it grows with the publication.
    Keys = 10,
    /// Sealing a family's series segments and batches: scratch, and the stage written.
    Seal = 11,
    /// The walker's tables, as each hour applies and each restart restores them.
    WalkerTables = 12,
    /// zstd contexts, each owned by its holder ([`crate::zstd::Holder`]), not a family.
    Zstd = 13,
    /// What blocks hold past the bytes asked for, rounded up to the allocator's size classes:
    /// counted apart, never in the tracked total.
    Rounding = 14,
    /// What the attribution's tags add to that, pushing blocks into larger size classes, past
    /// the tags themselves: only in a counting build, apart from the tracked total.
    TagRounding = 15,
}

impl Kind {
    pub const ALL: [Kind; 16] = [
        Kind::Unlabelled,
        Kind::Operator,
        Kind::Restore,
        Kind::Emit,
        Kind::Publish,
        Kind::Content,
        Kind::World,
        Kind::Walker,
        Kind::Executor,
        Kind::Headers,
        Kind::Keys,
        Kind::Seal,
        Kind::WalkerTables,
        Kind::Zstd,
        Kind::Rounding,
        Kind::TagRounding,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Unlabelled => "unlabelled",
            Kind::Operator => "operator",
            Kind::Restore => "restore",
            Kind::Emit => "emit",
            Kind::Publish => "publish",
            Kind::Content => "content",
            Kind::World => "world",
            Kind::Walker => "walker",
            Kind::Executor => "executor",
            Kind::Headers => "headers",
            Kind::Keys => "keys",
            Kind::Seal => "seal",
            Kind::WalkerTables => "walker tables",
            Kind::Zstd => "zstd",
            Kind::Rounding => "size classes",
            Kind::TagRounding => "tag size classes",
        }
    }

    /// Whether its allocations are a family's output until its emit frees them.
    pub fn projects(self) -> bool {
        matches!(self, Kind::Operator | Kind::Restore)
    }
}

/// Whose work: a family, by name ([`owner`]), or the run's. Empty without the feature.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Owner(#[cfg(feature = "memory-labels")] u16);

impl Owner {
    #[cfg(feature = "memory-labels")]
    pub const RUN: Self = Self(0);
    #[cfg(not(feature = "memory-labels"))]
    pub const RUN: Self = Self();
}

/// A thread's label, for work it hands to threads it starts ([`resume`]). Empty without the
/// feature.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Label(#[cfg(feature = "memory-labels")] u32);

/// A label held until dropped, on the thread that set it.
#[must_use]
pub struct Scope {
    #[cfg(feature = "memory-labels")]
    previous: u32,
    thread: PhantomData<*const ()>,
}

/// What a label held over a phase.
#[derive(Clone, Debug)]
pub struct Charge {
    pub owner: &'static str,
    pub kind: Kind,
    /// The most it held at once.
    pub peak: i64,
    /// What it held when the tracked total peaked.
    pub at_peak: i64,
    /// What it held as the phase ended.
    pub end: i64,
    /// Of what it held at the peak, what its family's emit has freed since: output then in
    /// flight. Only for kinds that project ([`Kind::projects`]).
    pub in_flight: Option<i64>,
}

/// The process's memory as the system counts it, and what of it the wrapped allocator holds.
#[derive(Clone, Copy, Debug)]
pub struct Os {
    pub commit: usize,
    pub working: usize,
    /// The most since the process started.
    pub peak_commit: usize,
    pub peak_working: usize,
    /// What the wrapped allocator holds committed, as it counts it ([`measure_allocator`]).
    pub allocator: Option<usize>,
}

/// What a phase of the process held, from one [`phase`] call to the next.
#[derive(Clone, Debug)]
pub struct Phase {
    pub name: &'static str,
    /// The most the allocator was asked to hold at once, the attribution's headers included.
    pub tracked_peak: i64,
    /// The tracked total when the labels were last read at a peak, within about a thousandth
    /// of [`Self::tracked_peak`].
    pub tracked_at_peak: i64,
    /// The process's memory when the labels were last read at a peak.
    pub os_at_peak: Option<Os>,
    pub os_started: Option<Os>,
    pub os_ended: Option<Os>,
    /// Families named past the labels' room, counted as "other".
    pub overflowed: usize,
    /// Every label that held anything.
    pub labels: Vec<Charge>,
}

#[cfg(feature = "memory-labels")]
pub use on::*;

#[cfg(not(feature = "memory-labels"))]
mod off {
    use super::*;

    #[inline(always)]
    pub fn owner(_: &str) -> Owner {
        Owner::RUN
    }

    #[inline(always)]
    pub fn enter(_: Owner, _: Kind) -> Scope {
        Scope {
            thread: PhantomData,
        }
    }

    #[inline(always)]
    pub fn kind(_: Kind) -> Scope {
        Scope {
            thread: PhantomData,
        }
    }

    #[inline(always)]
    pub fn current() -> Label {
        Label()
    }

    #[inline(always)]
    pub fn resume(_: Label) -> Scope {
        Scope {
            thread: PhantomData,
        }
    }

    #[inline(always)]
    pub fn phase(_: &'static str) {}

    #[inline(always)]
    pub fn measure_allocator(_: fn() -> Option<usize>) {}

    #[inline(always)]
    pub fn measure_sizes(_: fn(usize) -> usize) {}

    #[inline(always)]
    pub fn phases() -> Vec<Phase> {
        Vec::new()
    }
}

#[cfg(not(feature = "memory-labels"))]
pub use off::*;

#[cfg(feature = "memory-labels")]
mod on {
    use super::*;
    use std::{
        alloc::{GlobalAlloc, Layout},
        cell::{Cell, UnsafeCell},
        sync::{
            Mutex, MutexGuard, PoisonError, RwLock,
            atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU32, AtomicUsize, Ordering},
        },
    };

    const KINDS: usize = 32;
    const OWNERS: usize = 256;
    const LABELS: usize = OWNERS * KINDS;
    const OTHER: usize = OWNERS - 1;
    const HEADERS: u32 = Kind::Headers as u32;
    const ROUNDING: u32 = Kind::Rounding as u32;
    const TAG_ROUNDING: u32 = Kind::TagRounding as u32;
    /// Marks a batch entry as bytes of the label's output its emit freed ([`IN_FLIGHT`]).
    const FLIGHT: u32 = 1 << 31;
    /// The tag before each allocation: its label, and the peak reading it was made after.
    const HEADER: usize = 8;
    /// A thread's counts go to the shared ones once they moved this much, or it held this many
    /// labels: the shared counts lag each thread by no more.
    const FLUSH_BYTES: u64 = 256 << 10;
    const ENTRIES: usize = 16;
    const MISSING: usize = usize::MAX;
    const READINGS: usize = 5;

    static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
    static OVERFLOWED: AtomicUsize = AtomicUsize::new(0);
    static LIVE: [AtomicI64; LABELS] = [const { AtomicI64::new(0) }; LABELS];
    static PEAK: [AtomicI64; LABELS] = [const { AtomicI64::new(0) }; LABELS];
    static AT_PEAK: [AtomicI64; LABELS] = [const { AtomicI64::new(0) }; LABELS];
    static IN_FLIGHT: [AtomicI64; LABELS] = [const { AtomicI64::new(0) }; LABELS];
    static TOTAL: AtomicI64 = AtomicI64::new(0);
    static TOTAL_PEAK: AtomicI64 = AtomicI64::new(0);
    static AT_PEAK_TOTAL: AtomicI64 = AtomicI64::new(0);
    /// The tracked total past which the labels are read again.
    static NEXT: AtomicI64 = AtomicI64::new(0);
    /// Bumped at each reading: an allocation tagged before the current one was held then.
    static EPOCH: AtomicU32 = AtomicU32::new(1);
    /// Shared by calls counted without a slot and by each slot listed, alone by each reading,
    /// which also locks every slot: no call is counted while a reading is under way.
    static GATE: RwLock<()> = RwLock::new(());
    static OS_AT_PEAK: [AtomicUsize; READINGS] = [const { AtomicUsize::new(MISSING) }; READINGS];
    static COMMITTED: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
    static SIZES: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
    static OPEN: Mutex<Option<(&'static str, Option<Os>)>> = Mutex::new(Some(("startup", None)));
    static PHASES: Mutex<Vec<Phase>> = Mutex::new(Vec::new());

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What one allocator call adds, before it goes to its thread's counts.
    #[derive(Default)]
    struct Tally {
        keys: [u32; 8],
        deltas: [i64; 8],
        used: usize,
        moved: u64,
    }

    impl Tally {
        #[inline]
        fn add(&mut self, key: u32, bytes: i64) {
            self.moved += bytes.unsigned_abs();
            for at in 0..self.used {
                if self.keys[at] == key {
                    self.deltas[at] += bytes;
                    return;
                }
            }
            self.keys[self.used] = key;
            self.deltas[self.used] = bytes;
            self.used += 1;
        }

        fn entries(&self) -> impl Iterator<Item = (u32, i64)> + '_ {
            (0..self.used).map(|at| (self.keys[at], self.deltas[at]))
        }
    }

    /// A thread's counts not added to the shared ones yet.
    struct Counts {
        /// The reading the counts were all made after.
        epoch: u32,
        keys: [u32; ENTRIES],
        deltas: [i64; ENTRIES],
        used: usize,
        moved: u64,
    }

    impl Counts {
        /// Whether the total passed the next reading.
        #[inline]
        fn add(&mut self, key: u32, bytes: i64) -> bool {
            for at in (0..self.used).rev() {
                if self.keys[at] == key {
                    self.deltas[at] += bytes;
                    return false;
                }
            }
            let peaked = self.used == ENTRIES && self.flush();
            self.keys[self.used] = key;
            self.deltas[self.used] = bytes;
            self.used += 1;
            peaked
        }

        /// Adds them to the shared ones: whether the total passed the next reading.
        fn flush(&mut self) -> bool {
            let used = std::mem::replace(&mut self.used, 0);
            self.moved = 0;
            used != 0
                && apply(
                    self.epoch,
                    (0..used).map(|at| (self.keys[at], self.deltas[at])),
                )
        }
    }

    /// A thread's [`Counts`], and the lock under which it calls the allocator and counts what
    /// it did. A reading holds every slot's lock, so no call falls between its parts. Leaked,
    /// and listed from [`SLOTS`]: a slot its thread left is taken by the next.
    struct Slot {
        busy: AtomicBool,
        owned: AtomicBool,
        next: *const Slot,
        counts: UnsafeCell<Counts>,
    }

    // SAFETY: `counts` is reached only with `busy` held; `next` is set before it is listed.
    unsafe impl Sync for Slot {}

    impl Slot {
        #[inline]
        fn lock(&self) {
            while self.busy.swap(true, Ordering::Acquire) {
                std::hint::spin_loop();
            }
        }

        #[inline]
        fn unlock(&self) {
            self.busy.store(false, Ordering::Release);
        }

        /// With `busy` held; `work` must not allocate.
        #[inline]
        fn with<R>(&self, work: impl FnOnce(&mut Counts) -> R) -> R {
            self.lock();
            // SAFETY: `busy` is held.
            let result = work(unsafe { &mut *self.counts.get() });
            self.unlock();
            result
        }
    }

    /// The newest slot; each names the one listed before it. Listed under [`GATE`] shared, so
    /// a reading, holding it alone, finds the same slots to lock and to let go.
    static SLOTS: AtomicPtr<Slot> = AtomicPtr::new(std::ptr::null_mut());

    fn slots() -> impl Iterator<Item = &'static Slot> {
        let mut at = SLOTS.load(Ordering::Acquire).cast_const();
        std::iter::from_fn(move || {
            // SAFETY: slots are never freed.
            let slot = unsafe { at.as_ref() }?;
            at = slot.next;
            Some(slot)
        })
    }

    /// Every slot locked, for a reading: with [`GATE`] held alone, nothing is counted until it
    /// drops. Nothing may allocate meanwhile.
    struct Stilled;

    impl Stilled {
        fn new() -> Self {
            slots().for_each(Slot::lock);
            Self
        }

        /// Adds every thread's counts to the shared ones.
        fn drain(&self) {
            for slot in slots() {
                // SAFETY: `busy` is held, by this.
                unsafe { &mut *slot.counts.get() }.flush();
            }
            // What was drained can take the total past the next reading: read it now.
            let total = TOTAL.load(Ordering::Relaxed);
            if total >= NEXT.load(Ordering::Relaxed) {
                mark(total);
            }
        }
    }

    impl Drop for Stilled {
        fn drop(&mut self) {
            slots().for_each(Slot::unlock);
        }
    }

    /// A thread's label and slot. No destructor: the allocator may not use thread-locals that
    /// have one.
    struct Local {
        label: Cell<u32>,
        /// Null until labelling code runs on the thread, and once it ends: its calls are then
        /// counted under [`GATE`] shared.
        slot: Cell<*const Slot>,
        gone: Cell<bool>,
    }

    thread_local! {
        static LOCAL: Local = const {
            Local {
                label: Cell::new(0),
                slot: Cell::new(std::ptr::null()),
                gone: Cell::new(false),
            }
        };
        // Registered from labelling code, never from the allocator.
        static GUARD: Guard = const { Guard };
    }

    /// Adds a thread's counts as it ends and frees its slot.
    struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            let _ = LOCAL.try_with(|local| {
                local.gone.set(true);
                // SAFETY: slots are never freed.
                if let Some(slot) = unsafe { local.slot.replace(std::ptr::null()).as_ref() } {
                    let peaked = slot.with(Counts::flush);
                    slot.owned.store(false, Ordering::Release);
                    if peaked {
                        read_peak();
                    }
                }
            });
        }
    }

    impl Local {
        /// Gives the thread a slot and the guard that frees it: from labelling code only,
        /// never from the allocator.
        fn guard(&self) {
            if self.slot.get().is_null() && !self.gone.get() {
                let _ = GUARD.try_with(|_| ());
                self.slot.set(claim());
            }
        }
    }

    fn claim() -> &'static Slot {
        let free = slots().find(|slot| {
            slot.owned
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
        });
        if let Some(slot) = free {
            return slot;
        }
        // Made before the gate is taken: making it is counted under the gate too.
        let slot = Box::into_raw(Box::new(Slot {
            busy: AtomicBool::new(false),
            owned: AtomicBool::new(true),
            next: std::ptr::null(),
            counts: UnsafeCell::new(Counts {
                epoch: 0,
                keys: [0; ENTRIES],
                deltas: [0; ENTRIES],
                used: 0,
                moved: 0,
            }),
        }));
        let _gate = GATE.read().unwrap_or_else(PoisonError::into_inner);
        let mut head = SLOTS.load(Ordering::Relaxed);
        loop {
            // SAFETY: not yet listed, so this thread's alone.
            unsafe { (*slot).next = head.cast_const() };
            match SLOTS.compare_exchange_weak(head, slot, Ordering::Release, Ordering::Relaxed) {
                Ok(_) => break,
                Err(now) => head = now,
            }
        }
        // SAFETY: leaked, never freed.
        unsafe { &*slot }
    }

    /// Adds counts made after reading `epoch` to the shared ones, with no reading under way:
    /// under a slot's lock or [`GATE`]. Made before the latest reading, they were held then
    /// too, and what of them its emit freed was not since. Whether the total passed the next
    /// reading.
    fn apply(epoch: u32, batch: impl IntoIterator<Item = (u32, i64)>) -> bool {
        let before = epoch != EPOCH.load(Ordering::Relaxed);
        let mut sum = 0;
        for (key, delta) in batch {
            if delta == 0 {
                continue;
            }
            let label = (key & !FLIGHT) as usize;
            if key & FLIGHT != 0 {
                if !before {
                    IN_FLIGHT[label].fetch_add(delta, Ordering::Relaxed);
                }
                continue;
            }
            let live = LIVE[label].fetch_add(delta, Ordering::Relaxed) + delta;
            PEAK[label].fetch_max(live, Ordering::Relaxed);
            if before {
                AT_PEAK[label].fetch_add(delta, Ordering::Relaxed);
            }
            if key != ROUNDING && key != TAG_ROUNDING {
                sum += delta;
            }
        }
        sum != 0 && {
            let total = TOTAL.fetch_add(sum, Ordering::Relaxed) + sum;
            TOTAL_PEAK.fetch_max(total, Ordering::Relaxed);
            if before {
                AT_PEAK_TOTAL.fetch_add(sum, Ordering::Relaxed);
            }
            total >= NEXT.load(Ordering::Relaxed)
        }
    }

    /// Does `call`, an allocator call, and counts what it reports, with no reading between:
    /// under this thread's slot, or [`GATE`] shared when it has none. `call` is given the
    /// reading it is made after.
    #[inline]
    fn counted<R>(call: impl FnOnce(u32, &mut Tally) -> R) -> R {
        let slot = LOCAL
            .try_with(|local| local.slot.get())
            .unwrap_or(std::ptr::null());
        let mut tally = Tally::default();
        // SAFETY: slots are never freed.
        let (result, peaked) = match unsafe { slot.as_ref() } {
            Some(slot) => slot.with(|counts| {
                let epoch = EPOCH.load(Ordering::Relaxed);
                let mut peaked = false;
                if counts.epoch != epoch {
                    peaked |= counts.flush();
                    counts.epoch = epoch;
                }
                let result = call(epoch, &mut tally);
                for (key, delta) in tally.entries() {
                    peaked |= counts.add(key, delta);
                }
                counts.moved += tally.moved;
                if counts.moved >= FLUSH_BYTES {
                    peaked |= counts.flush();
                }
                (result, peaked)
            }),
            None => {
                let _gate = GATE.read().unwrap_or_else(PoisonError::into_inner);
                let epoch = EPOCH.load(Ordering::Relaxed);
                let result = call(epoch, &mut tally);
                (result, apply(epoch, tally.entries()))
            }
        };
        if peaked {
            read_peak();
        }
        result
    }

    fn step(total: i64) -> i64 {
        (total / 1024).max(1 << 20)
    }

    /// Reads every label at a new peak of the tracked total. Each count taking the total past
    /// the next reading comes here, so growth during a reading is read after it.
    fn read_peak() {
        let _gate = GATE.write().unwrap_or_else(PoisonError::into_inner);
        let _stilled = Stilled::new();
        let total = TOTAL.load(Ordering::Relaxed);
        if total >= NEXT.load(Ordering::Relaxed) {
            mark(total);
        }
    }

    /// Takes `total` as the peak: what every label holds now, none of it freed since. Only
    /// with [`GATE`] held alone and every slot [`Stilled`]; it allocates nothing.
    fn mark(total: i64) {
        EPOCH.fetch_add(1, Ordering::Relaxed);
        for label in 0..LABELS {
            AT_PEAK[label].store(LIVE[label].load(Ordering::Relaxed), Ordering::Relaxed);
            IN_FLIGHT[label].store(0, Ordering::Relaxed);
        }
        AT_PEAK_TOTAL.store(total, Ordering::Relaxed);
        let os = os();
        for (at, value) in OS_AT_PEAK.iter().zip(match os {
            Some(os) => [
                os.commit,
                os.working,
                os.peak_commit,
                os.peak_working,
                os.allocator.unwrap_or(MISSING),
            ],
            None => [MISSING; READINGS],
        }) {
            at.store(value, Ordering::Relaxed);
        }
        NEXT.store(total + step(total), Ordering::Relaxed);
    }

    #[inline]
    fn label() -> u32 {
        LOCAL.try_with(|local| local.label.get()).unwrap_or(0)
    }

    #[inline]
    fn of(label: u32) -> (usize, u32) {
        (label as usize / KINDS, label % KINDS as u32)
    }

    /// Counts `bytes` of `label`, tagged after reading `tagged`, freed after reading `epoch`.
    #[inline]
    fn freed(tally: &mut Tally, epoch: u32, label: u32, tagged: u32, bytes: usize, header: usize) {
        let (owner, kind) = of(label);
        let projects = kind == Kind::Operator as u32 || kind == Kind::Restore as u32;
        tally.add(label, -(bytes as i64));
        tally.add(HEADERS, -(header as i64));
        rounded(tally, bytes, header, -1);
        if projects && tagged < epoch {
            let (by, freeing) = of(self::label());
            let emits = freeing == Kind::Emit as u32 || freeing == Kind::Publish as u32;
            if by == owner && emits {
                tally.add(label | FLIGHT, bytes as i64);
            }
        }
    }

    fn set(label: u32) -> Scope {
        let previous = LOCAL
            .try_with(|local| {
                local.guard();
                local.label.replace(label)
            })
            .unwrap_or(0);
        Scope {
            previous,
            thread: PhantomData,
        }
    }

    impl Drop for Scope {
        #[inline]
        fn drop(&mut self) {
            let _ = LOCAL.try_with(|local| local.label.set(self.previous));
        }
    }

    /// Has each reading ask `committed` what the wrapped allocator holds committed: a function
    /// that allocates nothing, as the readings are taken inside the allocator.
    pub fn measure_allocator(committed: fn() -> Option<usize>) {
        COMMITTED.store(committed as *mut (), Ordering::Release);
    }

    /// Has each block counted at the size class `good` gives what is asked of the wrapped
    /// allocator ([`Kind::Rounding`], [`Kind::TagRounding`]): a function that allocates nothing.
    pub fn measure_sizes(good: fn(usize) -> usize) {
        SIZES.store(good as *mut (), Ordering::Release);
    }

    /// Counts, by `sign`, the size classes' rounding of a block of `size` bytes after a tag of
    /// `tag` bytes.
    #[inline]
    fn rounded(tally: &mut Tally, size: usize, tag: usize, sign: i64) {
        let good = SIZES.load(Ordering::Relaxed);
        if good.is_null() {
            return;
        }
        // SAFETY: only `measure_sizes` stores here, a function of this type.
        let good = unsafe { std::mem::transmute::<*mut (), fn(usize) -> usize>(good) };
        let alone = good(size);
        let tagged = good(size + tag);
        tally.add(ROUNDING, sign * (alone - size) as i64);
        tally.add(
            TAG_ROUNDING,
            sign * (tagged as i64 - alone as i64 - tag as i64),
        );
    }

    fn committed() -> Option<usize> {
        let committed = COMMITTED.load(Ordering::Acquire);
        if committed.is_null() {
            return None;
        }
        // SAFETY: only `measure_allocator` stores here, a function of this type.
        unsafe { std::mem::transmute::<*mut (), fn() -> Option<usize>>(committed)() }
    }

    /// The owner named `name`, the same each time it is named.
    pub fn owner(name: &str) -> Owner {
        let mut names = lock(&NAMES);
        if names.is_empty() {
            names.push("run");
        }
        if let Some(at) = names.iter().position(|named| *named == name) {
            return Owner(at as u16);
        }
        if names.len() >= OTHER {
            OVERFLOWED.fetch_add(1, Ordering::Relaxed);
            return Owner(OTHER as u16);
        }
        let named: &'static str = Box::leak(Box::<str>::from(name));
        names.push(named);
        Owner((names.len() - 1) as u16)
    }

    /// Labels this thread's allocations `owner`'s `kind` until the scope drops.
    #[inline]
    pub fn enter(owner: Owner, kind: Kind) -> Scope {
        set(u32::from(owner.0) * KINDS as u32 + kind as u32)
    }

    /// Labels this thread's allocations `kind`, of the owner already labelled.
    #[inline]
    pub fn kind(kind: Kind) -> Scope {
        let (owner, _) = of(label());
        set(owner as u32 * KINDS as u32 + kind as u32)
    }

    /// This thread's label.
    #[inline]
    pub fn current() -> Label {
        Label(label())
    }

    /// Labels this thread's allocations as `label` until the scope drops: a thread started for
    /// another's work takes its label.
    #[inline]
    pub fn resume(label: Label) -> Scope {
        set(label.0)
    }

    /// Ends the phase under way and begins `name`'s: from here, its peaks.
    pub fn phase(name: &'static str) {
        close(Some(name));
    }

    /// Every phase, the one under way ended.
    pub fn phases() -> Vec<Phase> {
        close(None);
        std::mem::take(&mut *lock(&PHASES))
    }

    fn close(next: Option<&'static str>) {
        let os = os();
        let mut open = lock(&OPEN);
        let ended = match next {
            Some(name) => open.replace((name, os)),
            None => open.take(),
        };
        drop(open);
        if let Some((name, started)) = ended {
            let report = report(name, started, os);
            lock(&PHASES).push(report);
        }
    }

    /// Reads the phase, every thread's counts added, and starts the next one's peaks, all with
    /// every slot [`Stilled`], so no count or reading falls between.
    fn report(name: &'static str, started: Option<Os>, ended: Option<Os>) -> Phase {
        let names = lock(&NAMES).clone();
        // Allocated first: nothing may allocate while the slots are stilled.
        let mut held = Vec::with_capacity(LABELS);
        let gate = GATE.write().unwrap_or_else(PoisonError::into_inner);
        let stilled = Stilled::new();
        stilled.drain();
        for label in 0..LABELS {
            let live = LIVE[label].load(Ordering::Relaxed);
            held.push([
                PEAK[label].load(Ordering::Relaxed),
                AT_PEAK[label].load(Ordering::Relaxed),
                live,
                IN_FLIGHT[label].load(Ordering::Relaxed),
            ]);
            PEAK[label].store(live, Ordering::Relaxed);
        }
        let tracked_peak = TOTAL_PEAK.load(Ordering::Relaxed);
        let tracked_at_peak = AT_PEAK_TOTAL.load(Ordering::Relaxed);
        let at_peak: [usize; READINGS] =
            std::array::from_fn(|at| OS_AT_PEAK[at].load(Ordering::Relaxed));
        let total = TOTAL.load(Ordering::Relaxed);
        TOTAL_PEAK.store(total, Ordering::Relaxed);
        mark(total);
        drop(stilled);
        drop(gate);
        let kinds = Kind::ALL;
        let mut labels = Vec::new();
        for (label, [peak, at_peak, end, in_flight]) in held.into_iter().enumerate() {
            if peak == 0 && at_peak == 0 && end == 0 {
                continue;
            }
            let (owner, kind) = (label / KINDS, label % KINDS);
            let Some(&kind) = kinds.get(kind) else {
                continue;
            };
            let owner = match owner {
                OTHER => "other",
                0 => "run",
                at => names.get(at).copied().unwrap_or("unnamed"),
            };
            labels.push(Charge {
                owner,
                kind,
                peak,
                at_peak,
                end,
                in_flight: kind.projects().then_some(in_flight),
            });
        }
        Phase {
            name,
            tracked_peak,
            tracked_at_peak,
            os_at_peak: (at_peak[0] != MISSING).then(|| Os {
                commit: at_peak[0],
                working: at_peak[1],
                peak_commit: at_peak[2],
                peak_working: at_peak[3],
                allocator: (at_peak[4] != MISSING).then_some(at_peak[4]),
            }),
            os_started: started,
            os_ended: ended,
            overflowed: OVERFLOWED.load(Ordering::Relaxed),
            labels,
        }
    }

    /// The process's memory counters; it allocates nothing.
    #[cfg(windows)]
    fn os() -> Option<Os> {
        #[repr(C)]
        struct Counters {
            size: u32,
            faults: u32,
            peak_working: usize,
            working: usize,
            peak_paged: usize,
            paged: usize,
            peak_nonpaged: usize,
            nonpaged: usize,
            pagefile: usize,
            peak_pagefile: usize,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn K32GetProcessMemoryInfo(
                process: *mut std::ffi::c_void,
                counters: *mut Counters,
                size: u32,
            ) -> i32;
        }
        let size = size_of::<Counters>() as u32;
        let mut counters = Counters {
            size,
            faults: 0,
            peak_working: 0,
            working: 0,
            peak_paged: 0,
            paged: 0,
            peak_nonpaged: 0,
            nonpaged: 0,
            pagefile: 0,
            peak_pagefile: 0,
        };
        // The current process's pseudo-handle is -1.
        let process = std::ptr::without_provenance_mut(usize::MAX);
        // SAFETY: the pseudo-handle of this process and an owned block of `size` bytes.
        let read = unsafe { K32GetProcessMemoryInfo(process, &mut counters, size) };
        (read != 0).then(|| Os {
            commit: counters.pagefile,
            working: counters.working,
            peak_commit: counters.peak_pagefile,
            peak_working: counters.peak_working,
            allocator: committed(),
        })
    }

    #[cfg(not(windows))]
    fn os() -> Option<Os> {
        let _ = committed;
        None
    }

    /// The allocator `A`, every allocation tagged with its thread's label ahead of it: 8 bytes,
    /// or its alignment when larger, counted apart ([`Kind::Headers`]).
    pub struct Counting<A>(pub A);

    #[inline]
    fn widened(layout: Layout) -> Option<(Layout, usize)> {
        let offset = layout.align().max(HEADER);
        let size = layout.size().checked_add(offset)?;
        Layout::from_size_align(size, offset)
            .ok()
            .map(|outer| (outer, offset))
    }

    /// Tags the block at `base`, of `size` bytes after `offset`, with this thread's label and
    /// `epoch`, and counts it: its user's pointer.
    #[inline]
    unsafe fn tagged(
        tally: &mut Tally,
        epoch: u32,
        base: *mut u8,
        offset: usize,
        size: usize,
    ) -> *mut u8 {
        if base.is_null() {
            return base;
        }
        let label = label();
        // SAFETY: the block holds `offset` bytes, at least 8 and a multiple of 8, before `size`.
        let pointer = unsafe { base.add(offset) };
        unsafe {
            pointer
                .sub(HEADER)
                .cast::<u64>()
                .write(u64::from(epoch) << 32 | u64::from(label));
        }
        tally.add(label, size as i64);
        tally.add(HEADERS, offset as i64);
        rounded(tally, size, offset, 1);
        pointer
    }

    /// The label and epoch tagging `pointer`'s block.
    #[inline]
    unsafe fn tag(pointer: *mut u8) -> (u32, u32) {
        // SAFETY: every block this allocator hands out is tagged just before it.
        let word = unsafe { pointer.sub(HEADER).cast::<u64>().read() };
        (word as u32, (word >> 32) as u32)
    }

    unsafe impl<A: GlobalAlloc> GlobalAlloc for Counting<A> {
        #[inline]
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let Some((outer, offset)) = widened(layout) else {
                return std::ptr::null_mut();
            };
            counted(|epoch, tally| unsafe {
                tagged(tally, epoch, self.0.alloc(outer), offset, layout.size())
            })
        }

        #[inline]
        unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
            let Some((outer, offset)) = widened(layout) else {
                return std::ptr::null_mut();
            };
            counted(|epoch, tally| unsafe {
                tagged(
                    tally,
                    epoch,
                    self.0.alloc_zeroed(outer),
                    offset,
                    layout.size(),
                )
            })
        }

        #[inline]
        unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
            let Some((outer, offset)) = widened(layout) else {
                return;
            };
            let (label, was) = unsafe { tag(pointer) };
            counted(|epoch, tally| {
                unsafe { self.0.dealloc(pointer.sub(offset), outer) };
                freed(tally, epoch, label, was, layout.size(), offset);
            });
        }

        #[inline]
        unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
            let Some((outer, offset)) = widened(layout) else {
                return std::ptr::null_mut();
            };
            let Some(grown) = size
                .checked_add(offset)
                .filter(|&grown| Layout::from_size_align(grown, offset).is_ok())
            else {
                return std::ptr::null_mut();
            };
            let (label, was) = unsafe { tag(pointer) };
            counted(|epoch, tally| {
                let base = unsafe { self.0.realloc(pointer.sub(offset), outer, grown) };
                if base.is_null() {
                    // The block stays as it was: nothing to count.
                    return base;
                }
                freed(tally, epoch, label, was, layout.size(), offset);
                unsafe { tagged(tally, epoch, base, offset, size) }
            })
        }
    }
}
