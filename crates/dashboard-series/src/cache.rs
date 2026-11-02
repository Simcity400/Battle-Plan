use crate::{
    budget::Budget,
    invalid,
    pages::{self, Kind, Page, PageRead},
    types::PageId,
};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex},
};

/// Decoded pages shared by every reader of the process, admitted from one budget.
/// Unpinned pages are evicted oldest first when admission needs room.
pub struct Cache<'a> {
    budget: &'a Budget,
    inner: Mutex<Inner>,
}
#[derive(Default)]
struct Inner {
    pages: HashMap<[u64; 3], Slot>,
    clock: u64,
    held: usize,
}
struct Slot {
    page: Arc<Page>,
    used: u64,
}

impl<'a> Cache<'a> {
    pub fn new(budget: &'a Budget) -> Self {
        Self {
            budget,
            inner: Mutex::new(Inner::default()),
        }
    }
    pub(crate) fn budget(&self) -> &'a Budget {
        self.budget
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub(crate) fn page(
        &self,
        io: &mut impl PageRead,
        id: PageId,
        kind: Kind,
    ) -> io::Result<Arc<Page>> {
        let identity = io.identity(id)?;
        {
            let mut inner = self.lock();
            inner.clock += 1;
            let clock = inner.clock;
            if let Some(slot) = inner.pages.get_mut(&identity) {
                slot.used = clock;
                if slot.page.kind() != kind {
                    return Err(invalid("series page kind"));
                }
                return Ok(Arc::clone(&slot.page));
            }
        }
        let page = Arc::new(pages::open(io, id, kind)?);
        let bytes = page.held();
        let mut inner = self.lock();
        while self.budget.reserve(bytes).is_err() {
            if !self.evict(&mut inner, bytes) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "series page cache admission exhausted",
                ));
            }
        }
        inner.clock += 1;
        let used = inner.clock;
        if let Some(slot) = inner.pages.get(&identity) {
            self.budget.release(bytes);
            return Ok(Arc::clone(&slot.page));
        }
        inner.held += bytes;
        inner.pages.insert(
            identity,
            Slot {
                page: Arc::clone(&page),
                used,
            },
        );
        Ok(page)
    }
    /// Reserves `bytes` of a reader's working memory, evicting the oldest unpinned pages until
    /// they fit. Under the lock no page is added, so each round either fits or evicts one at
    /// least, and it ends refused once no unpinned page is left.
    pub(crate) fn reserve_evicting(&self, bytes: usize) -> io::Result<()> {
        let mut inner = self.lock();
        while self.budget.reserve(bytes).is_err() {
            if !self.evict(&mut inner, bytes) {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "series byte admission exhausted",
                ));
            }
        }
        Ok(())
    }
    /// Frees at least `bytes` (or an eighth of the cache) from the oldest unpinned pages.
    fn evict(&self, inner: &mut Inner, bytes: usize) -> bool {
        let mut unpinned: Vec<([u64; 3], u64)> = inner
            .pages
            .iter()
            .filter(|(_, slot)| Arc::strong_count(&slot.page) == 1)
            .map(|(identity, slot)| (*identity, slot.used))
            .collect();
        if unpinned.is_empty() {
            return false;
        }
        unpinned.sort_unstable_by_key(|(_, used)| *used);
        let goal = bytes.max(inner.held / 8);
        let mut freed = 0;
        for (identity, _) in unpinned {
            if freed >= goal {
                break;
            }
            if let Some(slot) = inner.pages.remove(&identity) {
                let held = slot.page.held();
                inner.held -= held;
                self.budget.release(held);
                freed += held;
            }
        }
        true
    }
}
impl Drop for Cache<'_> {
    fn drop(&mut self) {
        let held = self.lock().held;
        self.budget.release(held);
    }
}
