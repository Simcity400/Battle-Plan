use super::*;
use collector_certificates::watch_registry::WatchRegistry;

static MARKS: WatchRegistry = WatchRegistry::marks_only();
static PENDING: AtomicBool = AtomicBool::new(false);
static REPAIRING: AtomicBool = AtomicBool::new(false);
static ADDED: AtomicBool = AtomicBool::new(false);
static DIRTY: Mutex<Option<std::collections::HashSet<usize>>> = Mutex::new(None);
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub(super) fn pending() -> bool {
    PENDING.load(Ordering::Acquire) || REPAIRING.load(Ordering::Acquire)
}
pub(super) fn added() {
    ADDED.store(true, Ordering::Release);
}
pub(super) fn prepare(watches: &HashMap<usize, Watch>) -> Result<(), &'static str> {
    MARKS.collect();
    MARKS.offer(watches.capacity());
    MARKS.set_enabled(true);
    MARKS.boundary(true);
    for pointer in watches.keys() {
        MARKS
            .register(*pointer)
            .ok_or("variant repair watch allocation failed")?;
    }
    PENDING.store(false, Ordering::Release);
    ADDED.store(false, Ordering::Release);
    Ok(())
}
pub(super) fn mark(pointer: usize) {
    if pointer == 0 || STORE.get().is_none() {
        unknown_receiver!();
        return;
    }
    MARKS.touch(pointer);
    PENDING.store(true, Ordering::Release);
}
pub(super) struct Repairs {
    admitted: bool,
    pub watches: Vec<Watch>,
    pub lost: usize,
}
impl Drop for Repairs {
    fn drop(&mut self) {
        if !self.admitted {
            unobserved!();
        }
        REPAIRING.store(false, Ordering::Release);
    }
}
pub(super) fn take(batch: &mut Batch) -> Result<Repairs, &'static str> {
    let mut repairs = Repairs {
        admitted: false,
        watches: Vec::new(),
        lost: 0,
    };
    REPAIRING.store(true, Ordering::Release);
    let pending = PENDING.swap(false, Ordering::AcqRel);
    let added = ADDED.swap(false, Ordering::AcqRel);
    if !pending && !added {
        repairs.admitted = true;
        return Ok(repairs);
    }
    let mut store = STORE
        .get()
        .ok_or("variant change observer not prepared")?
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if store.epoch != EPOCH.load(Ordering::Acquire) {
        return Err("variant repair epoch changed");
    }
    let mut pointers = lock(&DIRTY).take().unwrap_or_default();
    pointers.clear();
    engine_allocation::reserve_set(&mut pointers, MARKS.registered() + store.watched.len())
        .map_err(|_| "variant repair allocation failed")?;
    let mut published = std::mem::take(&mut store.published.pointers);
    if std::mem::take(&mut store.published.overflowed) {
        published.clear();
        engine_allocation::reserve(&mut published, store.watched.len())
            .map_err(|_| "variant repair allocation failed")?;
        published.extend(store.watched.keys().copied());
    }
    for &pointer in &published {
        if !store.watched.contains_key(&pointer) {
            continue;
        }
        let (_, fresh) = MARKS
            .register(pointer)
            .ok_or("variant repair receiver registration failed")?;
        if pending && fresh {
            pointers.insert(pointer);
        }
    }
    published.clear();
    store.published.pointers = published;
    MARKS
        .take(|pointer| {
            pointers.insert(pointer);
        })
        .ok_or("variant repair marks have a running writer or changed epoch")?;
    repairs.lost = pointers.len();

    repairs.watches =
        engine_allocation::vec(pointers.len()).map_err(|_| "variant repair allocation failed")?;
    for pointer in &pointers {
        if let Some(watch) = store.watched.get_mut(pointer) {
            watch.dirty = true;
            repairs.watches.push(*watch);
        }
    }
    batch.events.retain(|event| {
        !matches!(event.kind, Kind::Design)
            || event.removed
            || !pointers.contains(&event.pointer)
    });
    repairs.watches.sort_unstable_by_key(|watch| watch.id);
    *lock(&DIRTY) = Some(pointers);
    repairs.admitted = true;
    Ok(repairs)
}
pub(super) fn settle(watches: &[Watch]) {
    let Some(store) = STORE.get() else {
        return;
    };
    let mut store = store
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for watch in watches {
        if let Some(current) = store
            .watched
            .get_mut(&watch.pointer)
            .filter(|current| current.id == watch.id)
        {
            current.dirty = false;
        }
    }
}
pub(super) struct Surviving<'a, M> {
    pub source: &'a M,
    pub removed: &'a [usize],
    pub receiver: usize,
    pub consumed: std::cell::Cell<bool>,
}
impl<M: Memory> Surviving<'_, M> {
    #[inline]
    fn overlaps_removed(&self, address: usize, len: usize) -> bool {
        len != 0
            && self.removed.iter().any(|&pointer| {
                pointer != self.receiver
                    && (address
                        .checked_sub(pointer)
                        .is_some_and(|offset| offset < ROOT_BYTES)
                        || pointer
                            .checked_sub(address)
                            .is_some_and(|offset| offset < len))
            })
    }

    #[inline]
    fn scattered<const N: usize, const PREFETCH: bool>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        if self.removed.is_empty()
            || !addresses
                .iter()
                .zip(destinations.iter())
                .any(|(&address, destination)| self.overlaps_removed(address, destination.len()))
        {
            return if PREFETCH {
                self.source
                    .copy_scattered_prefetched(addresses, destinations)
            } else {
                self.source.copy_scattered(addresses, destinations)
            };
        }
        for (completed, (address, destination)) in addresses
            .into_iter()
            .zip(destinations.iter_mut())
            .enumerate()
        {
            if destination.is_empty() {
                continue;
            }
            if address == 0
                || address.checked_add(destination.len()).is_none()
                || !self.copy(address, destination)
            {
                return completed;
            }
        }
        N
    }
}
impl<M: Memory> Memory for Surviving<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        if self.overlaps_removed(address, output.len()) {
            self.consumed.set(true);
            return false;
        }
        self.source.copy(address, output)
    }

    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.scattered::<N, false>(addresses, destinations)
    }

    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        destinations: &mut [&mut [u8]; N],
    ) -> usize {
        self.scattered::<N, true>(addresses, destinations)
    }

    fn copy_records(
        &self,
        sources: engine_native_memory::memory::RecordSources<'_>,
        layout: &engine_native_memory::memory::RecordLayout<'_>,
        destination: &mut [u8],
    ) -> usize {
        if self.removed.is_empty() {
            return self.source.copy_records(sources, layout, destination);
        }
        let Some(count) = layout.count(sources, destination.len()) else {
            return 0;
        };
        let mut completed = 0;
        for row in 0..sources.len() {
            for field in layout.fields() {
                if field.len != 0 {
                    let Some(address) = sources
                        .pointer(row)
                        .filter(|&pointer| pointer != 0)
                        .and_then(|pointer| pointer.checked_add(field.source_offset))
                        .filter(|&address| address.checked_add(field.len).is_some())
                    else {
                        return completed;
                    };
                    let start = row * layout.stride() + field.destination_offset;
                    if !self.copy(address, &mut destination[start..start + field.len]) {
                        return completed;
                    }
                }
                completed += 1;
            }
        }
        count
    }
}
pub(super) fn snapshot(
    memory: &impl Memory,
    base: usize,
    hour: i32,
    pointer: usize,
    removed: &[usize],
    walk: Option<(snapshots::speculative::WalkSummary, Box<snapshots::Decoded>)>,
) -> Result<snapshots::Decoded, &'static str> {
    let surviving = Surviving {
        source: memory,
        removed,
        receiver: pointer,
        consumed: std::cell::Cell::new(false),
    };
    let result = match walk {
        Some((summary, decoded)) => super::complete_decoded_parts(
            &surviving,
            base,
            true,
            summary.details,
            summary.stats_len,
            &summary.modules,
            summary.first_upgrade.as_ref(),
            *decoded,
        ),
        None => super::decode_snapshot(&surviving, base, hour, pointer, true),
    };
    if surviving.consumed.get() {
        unobserved!();
        return Err("variant parent identity has a consumed saved binding");
    }
    result
}
pub(super) fn complete(
    watch: &Watch,
    snapshot: &rows::VariantSnapshot,
) -> Result<(), &'static str> {
    let state = snapshot
        .definition
        .design
        .require("VariantChanges.design")
        .map_err(|_| "variant repair lacks design state")?;
    let mut store = STORE
        .get()
        .unwrap()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let current = store
        .watched
        .get_mut(&watch.pointer)
        .filter(|current| current.id == watch.id)
        .ok_or("variant repair binding changed during acquisition")?;
    current.flags = [
        state.can_upgrade_type,
        state.can_upgrade_variant,
        state.can_upgrade_modules,
    ];
    Ok(())
}
/// # Safety
/// Hooks and acquisition have stopped before their owner releases storage.
pub(super) unsafe fn release() {
    unsafe {
        MARKS.release();
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn prepared_marks_cover_receivers_without_callback_growth() {
        if !isolated() {
            return;
        }
        let watches = (1..=5)
            .map(|pointer| {
                (
                    pointer,
                    Watch {
                        id: pointer as u64,
                        pointer,
                        flags: [0; 3],
                        dirty: false,
                    },
                )
            })
            .collect();
        prepare(&watches).unwrap();
        assert!(
            STORE
                .set(Mutex::new(Store {
                    batch: Batch::empty(),
                    spare: Batch::empty(),
                    watched: watches,
                    published: Published::default(),
                    interests: sources::Interests::default(),
                    epoch: EPOCH.load(Ordering::Acquire),
                }))
                .is_ok()
        );
        collector_test_support::with_allocations_denied(|| {
            for pointer in 1..=5 {
                mark(pointer);
                mark(pointer);
            }
        });
        let repairs = take(&mut Batch::empty()).unwrap();
        assert_eq!(repairs.lost, 5);
        assert_eq!(repairs.watches.len(), 5);
        assert!(pending());
        drop(repairs);
        assert!(!pending());
    }
}
