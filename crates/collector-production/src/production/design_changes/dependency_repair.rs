//! Lost dependency callbacks retain their own keys, not a global log refusal.
//! Only the parked worker follows saved bindings or resolves author handles.
use super::*;
use collector_certificates::watch_registry::WatchRegistry;
use std::collections::HashSet;

struct Marks {
    registry: WatchRegistry,
    pending: AtomicBool,
}
impl Marks {
    const fn new() -> Self {
        Self {
            registry: WatchRegistry::marking(),
            pending: AtomicBool::new(false),
        }
    }
    fn prepare(&self, keys: impl Iterator<Item = usize> + Clone) -> Result<(), &'static str> {
        self.registry.collect();
        self.registry.offer(keys.clone().count());
        self.registry.set_enabled(true);
        self.registry.boundary(true);
        for key in keys {
            self.registry
                .register(key)
                .ok_or("variant dependency repair watch allocation failed")?;
        }
        self.pending.store(false, Ordering::Release);
        Ok(())
    }
    fn mark(&self, key: usize) {
        drop(self.registry.notify(key));
        self.pending.store(true, Ordering::Release);
    }
    fn take(
        &self,
        keys: impl Iterator<Item = usize> + Clone,
    ) -> Result<HashSet<usize>, &'static str> {
        let pending = self.pending.swap(false, Ordering::AcqRel);
        if !pending {
            return Ok(HashSet::new());
        }
        let mut marked = HashSet::new();
        engine_allocation::reserve_set(
            &mut marked,
            keys.clone().count() + self.registry.registered(),
        )
        .map_err(|_| "variant dependency repair allocation failed")?;
        // A newly demanded key may have lost a callback before its slot existed.
        for key in keys {
            let (_, fresh) = self
                .registry
                .register(key)
                .ok_or("variant dependency repair registration failed")?;
            if pending && fresh {
                marked.insert(key);
            }
        }
        self.registry
            .take(|key| {
                marked.insert(key);
            })
            .ok_or("variant dependency repair marks have a running writer or changed epoch")?;
        Ok(marked)
    }
}
static DIRECT: Marks = Marks::new();
static PROVIDERS: Marks = Marks::new();
static REMOVED: Marks = Marks::new();
static AUTHORS: Marks = Marks::new();
static AUTHOR_POINTERS: Marks = Marks::new();
pub(super) static SOURCE_CONTENTIONS: AtomicU64 = AtomicU64::new(0);
pub(super) static AUTHOR_CONTENTIONS: AtomicU64 = AtomicU64::new(0);
fn direct(interests: &sources::Interests) -> impl Iterator<Item = usize> + Clone + '_ {
    interests.entries.keys().filter_map(|key| match key {
        sources::Key::Direct(pointer) => Some(*pointer),
        _ => None,
    })
}
fn providers(interests: &sources::Interests) -> impl Iterator<Item = usize> + Clone + '_ {
    interests.entries.keys().filter_map(|key| match key {
        sources::Key::Provider(pointer) => Some(*pointer),
        _ => None,
    })
}
pub(super) fn prepare(interests: &sources::Interests) -> Result<(), &'static str> {
    liveness::demand(!interests.organisations.is_empty());
    DIRECT.prepare(direct(interests))?;
    REMOVED.prepare(direct(interests))?;
    PROVIDERS.prepare(providers(interests))?;
    AUTHORS.prepare(
        interests
            .organisations
            .keys()
            .map(|handle| *handle as usize),
    )?;
    AUTHOR_POINTERS.prepare(interests.organisation_pointers.keys().copied())
}
pub(super) fn pending() -> bool {
    [&DIRECT, &PROVIDERS, &REMOVED, &AUTHORS, &AUTHOR_POINTERS]
        .iter()
        .any(|marks| marks.pending.load(Ordering::Acquire))
}
fn contention() {
    LOCK_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    RECEIVER_LOCK_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
}
pub(super) fn source(key: sources::Key, retired: bool) {
    if matches!(key, sources::Key::Direct(0) | sources::Key::Provider(0)) {
        lock_contended!();
        return;
    }
    contention();
    SOURCE_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    match key {
        sources::Key::Direct(pointer) => {
            DIRECT.mark(pointer);
            if retired {
                REMOVED.mark(pointer);
            }
        }
        sources::Key::Provider(pointer) => PROVIDERS.mark(pointer),
    }
}
pub(super) fn author(handle: u64, pointer: usize) {
    contention();
    AUTHOR_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    AUTHORS.mark(handle as usize);
    // Reference repair can replace the handle at this receiver. Preserve its
    // preceding demanded handle as well as the post-call hook argument.
    AUTHOR_POINTERS.mark(pointer);
}
pub(super) fn retired_author(pointer: usize) {
    contention();
    AUTHOR_CONTENTIONS.fetch_add(1, Ordering::Relaxed);
    AUTHOR_POINTERS.mark(pointer);
}
pub(super) struct Repairs {
    pub authors: HashSet<u64>,
    pub lost: usize,
}
pub(super) fn take(
    store: &mut Store,
    batch: &mut Batch,
    designs: &mut HashSet<usize>,
) -> Result<Repairs, &'static str> {
    let mut direct = DIRECT.take(direct(&store.interests))?;
    direct.retain(|pointer| {
        store
            .interests
            .entries
            .contains_key(&sources::Key::Direct(*pointer))
    });
    let mut providers = PROVIDERS.take(providers(&store.interests))?;
    providers.retain(|pointer| {
        store
            .interests
            .entries
            .contains_key(&sources::Key::Provider(*pointer))
    });
    let removed = REMOVED.take(self::direct(&store.interests))?;
    // A lost destruction of a demanded source cannot authorize dereferencing
    // that storage. Its membership/lifetime needs a fresh complete census.
    if batch
        .events
        .iter()
        .any(|event| event.kind == Kind::SourceRemoved && direct.contains(&event.pointer))
    {
        return Err("lost design source edit overlaps its observed retirement");
    }
    if removed.iter().any(|pointer| {
        store
            .interests
            .entries
            .contains_key(&sources::Key::Direct(*pointer))
    }) {
        return Err("applied design source retirement was not observed");
    }
    let handles = AUTHORS.take(
        store
            .interests
            .organisations
            .keys()
            .map(|handle| *handle as usize),
    )?;
    let author_pointers =
        AUTHOR_POINTERS.take(store.interests.organisation_pointers.keys().copied())?;
    let mut authors = HashSet::new();
    engine_allocation::reserve_set(&mut authors, handles.len() + author_pointers.len())
        .map_err(|_| "variant author repair allocation failed")?;
    authors.extend(handles.into_iter().map(|handle| handle as u64));
    for pointer in author_pointers {
        if let Some(handle) = store.interests.organisation_pointers.get(&pointer) {
            authors.insert(*handle);
        }
    }
    authors.retain(|handle| store.interests.organisations.contains_key(handle));
    let lost = direct.len() + providers.len() + authors.len();
    if !direct.is_empty() || !providers.is_empty() {
        let affected = sources::affected(&direct, &providers)?;
        engine_allocation::reserve_set(designs, store.watched.len() + batch.events.len())
            .map_err(|_| "variant dependency repair allocation failed")?;
        designs.extend(
            store
                .watched
                .values()
                .filter(|watch| affected.contains(&watch.id))
                .map(|watch| watch.pointer),
        );
        // Logged designs can introduce a binding not yet installed in the
        // worker's dependency index. Refresh these terminal values as well.
        designs.extend(
            batch
                .events
                .iter()
                .filter(|event| {
                    matches!(event.kind, Kind::Design | Kind::Scalar(_)) && !event.removed
                })
                .map(|event| event.pointer),
        );
        for pointer in &direct {
            store.interests.changed(sources::Key::Direct(*pointer));
        }
        for pointer in &providers {
            store.interests.changed(sources::Key::Provider(*pointer));
        }
    }
    for handle in &authors {
        store.interests.author_changed(*handle);
    }
    batch.events.retain(|event| match event.kind {
        Kind::Source => !direct.contains(&event.pointer),
        Kind::Provider(_) | Kind::ProviderReset(_) => !providers.contains(&event.pointer),
        Kind::Organisation(_) => !authors.contains(&event.variant),
        _ => true,
    });
    Ok(Repairs { authors, lost })
}
pub(super) unsafe fn release() {
    for marks in [&DIRECT, &PROVIDERS, &REMOVED, &AUTHORS, &AUTHOR_POINTERS] {
        unsafe { marks.registry.release() };
    }
}
