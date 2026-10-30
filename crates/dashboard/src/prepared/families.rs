//! The publications of the families the segment driver prepares, opened for a query: a
//! branch's series root ([`dashboard_base::prepared::roots`]) read through the store session
//! the query pinned, as of its last segment before the query's first hole
//! ([`super::coverage::declared`]), and its directory ([`Directory`]), read once per
//! publication and kept.

use crate::{Source, products::ContentOptions};
use dashboard_base::prepared::roots;
use dashboard_series::{Budget, Cache, Identity, Snapshot, StoreRead, family::Directory};
use std::{
    collections::HashMap,
    io,
    sync::{Arc, LazyLock, Mutex, PoisonError, atomic::AtomicBool},
};

// Decoded pages every query of these families shares, under one budget.
static BUDGET: Budget = Budget::new(256 << 20);
static CACHE: LazyLock<Cache<'static>> = LazyLock::new(|| Cache::new(&BUDGET));

/// Directories kept, by root range, the generation read and schema; dropped whole past this
/// many bytes.
const DIRECTORY_BYTES: usize = 64 << 20;
type Directories = HashMap<([u64; 3], u64, u64), Arc<Directory>>;
static DIRECTORIES: LazyLock<Mutex<(Directories, usize)>> = LazyLock::new(Mutex::default);

/// A family's publication on a branch, as a query reads it.
pub(crate) struct Opened {
    snapshot: store::Snapshot,
    section: String,
    root: u64,
    identity: Identity,
    /// The last hour before the query's first hole.
    through: Option<i32>,
    directory: Arc<Directory>,
}

/// The series publication of `root`, as of its last segment ending by `through`; None when
/// none does.
fn pinned<'s>(
    store: StoreRead<'s>,
    root: u64,
    identity: Identity,
    through: Option<i32>,
) -> io::Result<Option<Snapshot<'s, StoreRead<'s>>>> {
    let snapshot = Snapshot::open(store, &CACHE, root, identity)?;
    match through {
        Some(through) => snapshot.through(through),
        None => Ok(Some(snapshot)),
    }
}

impl Opened {
    /// `read` of the publication pinned for this query and its directory.
    pub(crate) fn read<T>(
        &self,
        cancelled: &AtomicBool,
        read: impl FnOnce(&mut Snapshot<'_, StoreRead<'_>>, &Directory) -> io::Result<T>,
    ) -> io::Result<T> {
        let store = StoreRead {
            snapshot: &self.snapshot,
            section: &self.section,
        };
        let mut snapshot = pinned(store, self.root, self.identity, self.through)?
            .ok_or_else(|| crate::invalid("A family's pinned segment is missing"))?
            .with_cancellation(cancelled);
        read(&mut snapshot, &self.directory)
    }
}

/// `family`'s publication on `branch`, its keys spelled under `schema`; None before its
/// first, or when none of its segments ends before the query's first hole.
pub(crate) fn open(
    source: &Source,
    content: Option<&ContentOptions>,
    family: &str,
    schema: u64,
    branch: u64,
) -> io::Result<Option<Opened>> {
    let (store, key) = super::files(source, content)?;
    let Some(session) = store.session(&key)? else {
        return Ok(None);
    };
    let snapshot = session.snapshot();
    let Some(published) = roots::read(&mut &snapshot, family, branch)? else {
        return Ok(None);
    };
    let section = roots::root_section(family, branch);
    let through = super::coverage::declared()
        .before
        .map(|before| before.saturating_sub(1));
    let range = snapshot
        .read(&section, published.root)?
        .ok_or_else(|| crate::invalid("A family's root range is missing"))?
        .id()
        .cache_key();
    let store = StoreRead {
        snapshot: &snapshot,
        section: &section,
    };
    let Some(mut series) = pinned(store, published.root, published.identity, through)? else {
        return Ok(None);
    };
    let held = (range, series.publication().generation, schema);
    let kept = DIRECTORIES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .0
        .get(&held)
        .cloned();
    let directory = match kept {
        Some(directory) => directory,
        None => {
            let directory = Arc::new(Directory::read(&mut series, schema)?);
            let mut kept = DIRECTORIES.lock().unwrap_or_else(PoisonError::into_inner);
            if kept.1 + directory.bytes() > DIRECTORY_BYTES {
                *kept = Default::default();
            }
            kept.1 += directory.bytes();
            kept.0.insert(held, Arc::clone(&directory));
            directory
        }
    };
    drop(series);
    Ok(Some(Opened {
        snapshot,
        section,
        root: published.root,
        identity: published.identity,
        through,
        directory,
    }))
}
