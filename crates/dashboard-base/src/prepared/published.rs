//! The publications of the families the segment driver prepares, opened for a query: a
//! branch's series root ([`dashboard_base::prepared::roots`]) read through the store session
//! the query pinned, as of its last segment before the query's first hole
//! ([`super::coverage::declared`]), and its directory ([`Directory`]), read once per
//! publication and shared while any query holds it open. An open publication holds its root and
//! interior nodes ([`Pinned`]), which each of its reads starts from.

use super::{coverage, files, roots};
use crate::{Source, products::ContentOptions};
use dashboard_series::{Identity, Pinned, Snapshot, StoreRead, family::Directory, shared::Shared};
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// Each open publication's directory, by root range, the generation read and schema.
static DIRECTORIES: Shared<([u64; 3], u64, u64), Directory> = Shared::new();

/// A family's publication on a branch, as a query reads it.
pub struct Opened {
    snapshot: store::Snapshot,
    section: String,
    /// The last hour before the query's first hole.
    through: Option<i32>,
    pinned: Pinned,
    directory: Arc<Directory>,
    /// The root's range and the generation read: equal stamps, equal publications.
    stamp: ([u64; 3], u64),
}

/// The series publication of `root`, as of its last segment ending by `through`; None when
/// none does.
fn pinned<'s>(
    store: StoreRead<'s>,
    root: u64,
    identity: Identity,
    through: Option<i32>,
) -> io::Result<Option<Snapshot<'s, StoreRead<'s>>>> {
    let snapshot = Snapshot::open(store, root, identity)?;
    match through {
        Some(through) => snapshot.through(through),
        None => Ok(Some(snapshot)),
    }
}

impl Opened {
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_snapshot(
        snapshot: store::Snapshot,
        section: String,
        root: u64,
        identity: Identity,
        schema: u64,
        through: Option<i32>,
    ) -> io::Result<Option<Self>> {
        Self::new(snapshot, section, root, identity, schema, through)
    }

    fn new(
        snapshot: store::Snapshot,
        section: String,
        root: u64,
        identity: Identity,
        schema: u64,
        through: Option<i32>,
    ) -> io::Result<Option<Self>> {
        let range = snapshot
            .read(&section, root)?
            .ok_or_else(|| crate::invalid("A family's root range is missing"))?
            .id()
            .cache_key();
        let store = StoreRead {
            snapshot: &snapshot,
            section: &section,
        };
        let Some(mut series) = pinned(store, root, identity, through)? else {
            return Ok(None);
        };
        let held = (range, series.publication().generation, schema);
        let directory = DIRECTORIES.get_or_make(held, || Directory::read(&mut series, schema))?;
        let pinned = series.pin()?;
        drop(series);
        Ok(Some(Self {
            snapshot,
            section,
            through,
            pinned,
            directory,
            stamp: (held.0, held.1),
        }))
    }

    /// What identifies the publication read: equal stamps, equal publications.
    pub fn stamp(&self) -> ([u64; 3], u64) {
        self.stamp
    }

    /// The last hour before the query's first hole it is read through, which [`Self::stamp`]
    /// does not name.
    pub fn through(&self) -> Option<i32> {
        self.through
    }

    /// `read` of the publication pinned for this query and its directory.
    pub fn read<T>(
        &self,
        cancelled: &AtomicBool,
        read: impl FnOnce(&mut Snapshot<'_, StoreRead<'_>>, &Directory) -> io::Result<T>,
    ) -> io::Result<T> {
        let store = StoreRead {
            snapshot: &self.snapshot,
            section: &self.section,
        };
        let mut snapshot = Snapshot::open_pinned(store, &self.pinned).with_cancellation(cancelled);
        read(&mut snapshot, &self.directory)
    }
}

/// `family`'s publication on `branch`, its keys spelled under `schema`; None before its
/// first, or when none of its segments ends before the query's first hole.
pub fn open(
    source: &Source,
    content: Option<&ContentOptions>,
    family: &str,
    schema: u64,
    branch: u64,
) -> io::Result<Option<Opened>> {
    open_through(source, content, family, schema, branch, None)
}

/// [`open`], as of its last segment ending by `limit` too: what a query joining it with another
/// family's coverage reads, never past it.
pub fn open_through(
    source: &Source,
    content: Option<&ContentOptions>,
    family: &str,
    schema: u64,
    branch: u64,
    limit: Option<i32>,
) -> io::Result<Option<Opened>> {
    let (store, key) = files(source, content)?;
    let Some(session) = store.session(&key)? else {
        return Ok(None);
    };
    let snapshot = session.snapshot();
    let Some(published) = roots::read(&mut &snapshot, family, branch)? else {
        return Ok(None);
    };
    if !coverage::accepts_publication(family, &published) {
        return Ok(None);
    }
    let section = roots::root_section(family, branch);
    let hole = coverage::declared()
        .before
        .map(|before| before.saturating_sub(1));
    let through = match (hole, limit) {
        (Some(hole), Some(limit)) => Some(hole.min(limit)),
        (hole, limit) => hole.or(limit),
    };
    Opened::new(
        snapshot,
        section,
        published.root,
        published.identity,
        schema,
        through,
    )
}
