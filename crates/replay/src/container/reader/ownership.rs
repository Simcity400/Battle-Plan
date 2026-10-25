use super::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    ptr,
    sync::{Mutex, Weak, atomic::AtomicUsize},
};

#[derive(Clone, Copy, Debug)]
#[repr(usize)]
pub(crate) enum Kind {
    Other,
    Raw,
    Codec,
    Anchors,
    Bootstrap,
    Aliases,
    Frame,
    Hidden,
    Updates,
    Jobs,
    Results,
}
const KINDS: usize = 11;

#[derive(Default)]
struct Count {
    live: AtomicUsize,
    peak: AtomicUsize,
    allocations: AtomicUsize,
}
impl Count {
    fn add(&self, bytes: usize) {
        let live = self.live.fetch_add(bytes, Ordering::Relaxed) + bytes;
        self.peak.fetch_max(live, Ordering::Relaxed);
        self.allocations.fetch_add(1, Ordering::Relaxed);
    }
    fn sub(&self, bytes: usize) {
        self.live.fetch_sub(bytes, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Meter {
    counts: [Count; KINDS],
    total: Count,
}

#[derive(Default)]
pub(crate) struct Audit {
    meter: Arc<Meter>,
    updates: Mutex<Vec<(Hour, Weak<TableUpdate>)>>,
    frames: Mutex<Vec<(Hour, Weak<crate::frame::Frame>)>>,
    peak_update_hours: AtomicUsize,
    views: Mutex<Vec<(TableId, crate::frame::ViewIdentity, bool)>>,
}

thread_local! {
    static OWNER: Cell<*const Audit> = const { Cell::new(ptr::null()) };
    static KIND: Cell<usize> = const { Cell::new(0) };
}

#[derive(Clone, Default)]
pub(crate) struct Context(Option<Arc<Audit>>);
pub(crate) struct Scope {
    owner: *const Audit,
    kind: usize,
    _held: Context,
}
impl Context {
    pub(crate) fn capture() -> Self {
        let pointer = OWNER.try_with(Cell::get).unwrap_or(ptr::null());
        if pointer.is_null() {
            return Self(None);
        }
        // A scope holds this Arc until its thread leaves the allocation context.
        unsafe {
            Arc::increment_strong_count(pointer);
            Self(Some(Arc::from_raw(pointer)))
        }
    }
    pub(crate) fn enter(&self, kind: Kind) -> Scope {
        let held = self.clone();
        let pointer = held.0.as_ref().map_or(ptr::null(), Arc::as_ptr);
        Scope {
            owner: OWNER.with(|slot| slot.replace(pointer)),
            kind: KIND.with(|slot| slot.replace(kind as usize)),
            _held: held,
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        OWNER.with(|slot| slot.set(self.owner));
        KIND.with(|slot| slot.set(self.kind));
    }
}
pub(crate) fn category(kind: Kind) -> Scope {
    Context::capture().enter(kind)
}

impl Audit {
    pub(crate) fn new() -> Arc<Self> {
        drop(crate::frame::not_selected());
        drop(super::unselected_update());
        Arc::new(Self::default())
    }
    pub(crate) fn enter(self: &Arc<Self>) -> Scope {
        Context(Some(Arc::clone(self))).enter(Kind::Other)
    }
    pub(crate) fn live(&self, kind: Kind) -> usize {
        self.meter.counts[kind as usize]
            .live
            .load(Ordering::Relaxed)
    }
    pub(crate) fn peak(&self, kind: Kind) -> usize {
        self.meter.counts[kind as usize]
            .peak
            .load(Ordering::Relaxed)
    }
    pub(crate) fn peak_total(&self) -> usize {
        self.meter.total.peak.load(Ordering::Relaxed)
    }
    pub(crate) fn live_total(&self) -> usize {
        self.meter.total.live.load(Ordering::Relaxed)
    }
    pub(crate) fn peak_update_hours(&self) -> usize {
        self.peak_update_hours.load(Ordering::Relaxed)
    }
    pub(crate) fn live_update_hours(&self) -> usize {
        let updates = self.updates.lock().unwrap();
        let mut hours = Vec::new();
        for (hour, weak) in updates.iter() {
            if weak.strong_count() != 0 && !hours.contains(hour) {
                hours.push(*hour);
            }
        }
        for (hour, weak) in self.frames.lock().unwrap().iter() {
            if weak.strong_count() != 0 && !hours.contains(hour) {
                hours.push(*hour);
            }
        }
        hours.len()
    }
    pub(crate) fn live_views(&self, hidden: bool) -> usize {
        self.views
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, id, h)| *h == hidden && id.is_alive())
            .count()
    }
    pub(crate) fn assert_unique_views(&self) {
        for (_, id, _) in self.views.lock().unwrap().iter() {
            assert!(
                id.owner_count() <= 1,
                "a reconstructed view has an extra owner"
            );
        }
    }
    pub(crate) fn clear_dead_witnesses(&self) {
        self.updates
            .lock()
            .unwrap()
            .retain(|(_, weak)| weak.strong_count() != 0);
        self.frames
            .lock()
            .unwrap()
            .retain(|(_, weak)| weak.strong_count() != 0);
        self.views
            .lock()
            .unwrap()
            .retain(|(_, id, _)| id.is_alive());
    }
}

pub(crate) fn update(hour: Hour, update: &Arc<TableUpdate>) {
    if !matches!(
        **update,
        TableUpdate::Snapshot(_)
            | TableUpdate::Truncated { .. }
            | TableUpdate::Delta { .. }
            | TableUpdate::Unchanged { .. }
    ) {
        return;
    }
    let context = Context::capture();
    let Some(audit) = &context.0 else {
        return;
    };
    let _untracked = Context::default().enter(Kind::Other);
    {
        let mut updates = audit.updates.lock().unwrap();
        updates.retain(|(_, weak)| weak.strong_count() != 0);
        updates.push((hour, Arc::downgrade(update)));
    }
    audit
        .peak_update_hours
        .fetch_max(audit.live_update_hours(), Ordering::Relaxed);
}
pub(crate) fn recorded(frame: &RecordedFrame) {
    let context = Context::capture();
    let Some(audit) = &context.0 else {
        return;
    };
    let Some(identity) = frame.inline_identity() else {
        return;
    };
    let _untracked = Context::default().enter(Kind::Other);
    {
        let mut frames = audit.frames.lock().unwrap();
        frames.retain(|(_, weak)| weak.strong_count() != 0);
        frames.push((frame.hour, identity));
    }
    audit
        .peak_update_hours
        .fetch_max(audit.live_update_hours(), Ordering::Relaxed);
}
pub(crate) fn view(table: TableId, view: &SharedView, hidden: bool) {
    let context = Context::capture();
    let Some(audit) = &context.0 else {
        return;
    };
    let _untracked = Context::default().enter(Kind::Other);
    let mut views = audit.views.lock().unwrap();
    views.retain(|(_, identity, _)| identity.is_alive());
    if !views.iter().any(|(_, id, _)| id.is(view)) {
        views.push((table, view.identity(), hidden));
    }
}

struct Tagged;
#[global_allocator]
static ALLOCATOR: Tagged = Tagged;
#[repr(C)]
struct Header {
    owner: *const Meter,
    kind: usize,
}
fn allocation_layout(layout: Layout) -> Option<(Layout, usize)> {
    Layout::new::<Header>()
        .extend(layout)
        .ok()
        .map(|(layout, offset)| (layout.pad_to_align(), offset))
}

unsafe fn allocate(layout: Layout, owner: *const Meter, kind: usize) -> *mut u8 {
    let Some((full, offset)) = allocation_layout(layout) else {
        return ptr::null_mut();
    };
    let base = unsafe { System.alloc(full) };
    if base.is_null() {
        return base;
    }
    unsafe {
        base.cast::<Header>().write(Header { owner, kind });
        if !owner.is_null() {
            Arc::increment_strong_count(owner);
            (*owner).counts[kind].add(layout.size());
            (*owner).total.add(layout.size());
        }
        base.add(offset)
    }
}
unsafe impl GlobalAlloc for Tagged {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let audit = OWNER.try_with(Cell::get).unwrap_or(ptr::null());
        let kind = KIND.try_with(Cell::get).unwrap_or(0);
        let owner = if audit.is_null() {
            ptr::null()
        } else {
            unsafe { Arc::as_ptr(&(*audit).meter) }
        };
        unsafe { allocate(layout, owner, kind) }
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let (full, offset) = allocation_layout(layout).unwrap();
        unsafe {
            let base = pointer.sub(offset);
            let Header { owner, kind } = base.cast::<Header>().read();
            if !owner.is_null() {
                (*owner).counts[kind].sub(layout.size());
                (*owner).total.sub(layout.size());
            }
            System.dealloc(base, full);
            if !owner.is_null() {
                drop(Arc::from_raw(owner));
            }
        }
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let (_, offset) = allocation_layout(layout).unwrap();
        let header = unsafe { &*pointer.sub(offset).cast::<Header>() };
        let next = unsafe {
            allocate(
                Layout::from_size_align_unchecked(size, layout.align()),
                header.owner,
                header.kind,
            )
        };
        if !next.is_null() {
            unsafe {
                ptr::copy_nonoverlapping(pointer, next, layout.size().min(size));
                self.dealloc(pointer, layout);
            }
        }
        next
    }
}
