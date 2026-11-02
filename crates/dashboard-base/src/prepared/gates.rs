//! Every gate of a prepared store, decoded once when the store is bound ([`super::files`]) and
//! then once per publication, in the thread that published it, from the publication as it was
//! committed ([`store::on_publication`]): each family's and job's coverage page with its
//! refusals, each family's publication root with its hole summary, and the identities of the pages queries key their
//! answers by. Each store folder holds one current publication, under the key it was committed
//! with, whichever source binds the folder. A query takes the one of its key from memory
//! ([`current`]), or a view publishing nothing when the folder publishes nothing to its key,
//! pins it for its whole duration ([`pin`]) and reads every page through that publication alone
//! ([`Published::scope`]): no reader runs without it. A query finding a file of it removed by a
//! later publication starts again on the later one, whose snapshot was decoded before any of its
//! files could go. Another process's publication, which no hook here sees, is decoded when a
//! query next finds the folder's manifest is not the one decoded. A folder's publication is held
//! while a source binds it ([`lease`]), and goes with the last.

use super::{
    coverage::{self, Page},
    roots,
};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, PoisonError},
};

const IDENTITIES: &str = "country-identities-";

/// The page of a branch's country identity history, whose identity cached answers are keyed by.
pub fn identities_section(branch: u64) -> String {
    format!("{IDENTITIES}{branch}")
}

/// A decoded page, or why it could not be decoded: raised to the queries consulting it.
type Decoded<T> = Result<T, Arc<str>>;

fn raised(error: &Arc<str>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

/// One gate of a branch, decoded.
#[derive(Clone)]
pub struct Gate {
    /// The coverage page's range identity, which every publication to it changes.
    id: Option<[u64; 3]>,
    page: Decoded<Option<Arc<Page>>>,
    /// The publication root's range identity.
    root: Option<[u64; 3]>,
    publication: Decoded<Option<Arc<roots::Published>>>,
}

impl Default for Gate {
    fn default() -> Self {
        Self {
            id: None,
            page: Ok(None),
            root: None,
            publication: Ok(None),
        }
    }
}

impl Gate {
    /// The coverage page's range identity; None while unpublished.
    pub fn id(&self) -> Option<[u64; 3]> {
        self.id
    }

    pub fn page(&self) -> io::Result<Option<&Page>> {
        match &self.page {
            Ok(page) => Ok(page.as_deref()),
            Err(error) => Err(raised(error)),
        }
    }

    /// The family's publication root on the branch, with its frontier.
    pub fn publication(&self) -> io::Result<Option<&roots::Published>> {
        match &self.publication {
            Ok(publication) => Ok(publication.as_deref()),
            Err(error) => Err(raised(error)),
        }
    }
}

#[derive(Clone, Default)]
struct Branch {
    /// Each family's and each job's, by name.
    named: BTreeMap<String, Gate>,
    identities: Option<[u64; 3]>,
}

/// The gates of one publication of a store, under the key it was committed with.
pub struct Published {
    snapshot: store::Snapshot,
    plan_id: Option<[u64; 3]>,
    plan: Option<Arc<Vec<u8>>>,
    branches: BTreeMap<u64, Branch>,
}

impl Published {
    /// What a folder publishing nothing to `key` publishes to it.
    fn empty(folder: &Path, key: &store::SourceKey) -> Self {
        Self {
            snapshot: store::Snapshot::empty(folder, key.clone()),
            plan_id: None,
            plan: None,
            branches: BTreeMap::new(),
        }
    }

    /// The key it was committed with.
    pub fn key(&self) -> &store::SourceKey {
        self.snapshot.key()
    }

    /// Installs this publication for this thread's reads of its key until the scope drops: every
    /// read of the store under its key reads it alone, and none reads the manifest.
    pub fn scope(&self) -> store::Scope {
        self.snapshot.scope()
    }

    /// The gate of the family or job `name` on `branch`.
    pub fn gate(&self, branch: u64, name: &str) -> Option<&Gate> {
        self.branches.get(&branch)?.named.get(name)
    }

    /// The mark a preparation published under ([`coverage::PLAN`]); None for a store no
    /// preparation marked.
    pub fn plan(&self) -> Option<&[u8]> {
        self.plan.as_deref().map(Vec::as_slice)
    }

    /// The identity of `branch`'s country identity history page.
    pub fn identities(&self, branch: u64) -> Option<[u64; 3]> {
        self.branches.get(&branch)?.identities
    }
}

/// `store`'s current publication, under the key it was committed with, decoding only the pages
/// that changed since `held` was decoded; None while it publishes nothing.
pub fn decode(store: &store::Store, held: Option<&Published>) -> io::Result<Option<Published>> {
    store
        .current()?
        .map(|snapshot| build(snapshot, held))
        .transpose()
}

/// Every gate `snapshot` publishes, as its manifest lists their pages, decoding only the pages
/// that changed since `held` was decoded.
fn build(snapshot: store::Snapshot, held: Option<&Published>) -> io::Result<Published> {
    let mut plan_id = None;
    let mut plan = None;
    let mut built: BTreeMap<u64, Branch> = BTreeMap::new();
    for section in snapshot.section_names() {
        if section == coverage::PLAN {
            plan_id = snapshot.range_id(section, 0)?.map(|id| id.cache_key());
            plan = match held {
                Some(held) if held.plan_id == plan_id => held.plan.clone(),
                _ => match snapshot.read(section, 0)? {
                    Some(mut range) => {
                        let mut bytes = Vec::new();
                        range.read_to_end(&mut bytes)?;
                        Some(Arc::new(bytes))
                    }
                    None => None,
                },
            };
            continue;
        }
        if let Some(branch) = section.strip_prefix(IDENTITIES).and_then(digits) {
            built.entry(branch).or_default().identities =
                snapshot.range_id(section, 0)?.map(|id| id.cache_key());
            continue;
        }
        let Some((name, branch)) = coverage::family_of(section) else {
            continue;
        };
        let previous = held
            .and_then(|held| held.branches.get(&branch))
            .and_then(|gates| gates.named.get(name));
        let gate = decode_gate(&snapshot, section, (name, branch), previous)?;
        built
            .entry(branch)
            .or_default()
            .named
            .insert(name.to_owned(), gate);
    }
    Ok(Published {
        snapshot,
        plan_id,
        plan,
        branches: built,
    })
}

fn digits(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// `section`'s range 0 parsed by `parse`, or why it could not be; None while it holds none.
fn decoded<T>(
    snapshot: &store::Snapshot,
    section: &str,
    parse: impl FnOnce(&[u8]) -> io::Result<T>,
) -> io::Result<Decoded<Option<Arc<T>>>> {
    let Some(mut range) = snapshot.read(section, 0)? else {
        return Ok(Ok(None));
    };
    let mut bytes = Vec::new();
    range.read_to_end(&mut bytes)?;
    Ok(parse(&bytes)
        .map(|value| Some(Arc::new(value)))
        .map_err(|error| Arc::from(error.to_string())))
}

/// The gate whose coverage page is `section` and whose publication is its family's or job's
/// root on its branch, decoded unless `previous` holds the same ranges.
fn decode_gate(
    snapshot: &store::Snapshot,
    section: &str,
    (family, branch): (&str, u64),
    previous: Option<&Gate>,
) -> io::Result<Gate> {
    let mut gate = Gate {
        id: snapshot.range_id(section, 0)?.map(|id| id.cache_key()),
        ..Gate::default()
    };
    gate.page = match previous {
        Some(previous) if previous.id == gate.id => previous.page.clone(),
        _ => decoded(snapshot, section, Page::parse)?,
    };
    let section = roots::root_section(family, branch);
    gate.root = match snapshot.range_count(&section) {
        Some(count) if count > 0 => snapshot
            .range_id(&section, count - 1)?
            .map(|id| id.cache_key()),
        _ => None,
    };
    gate.publication = match previous {
        Some(previous) if previous.root == gate.root => previous.publication.clone(),
        _ if gate.root.is_none() => Ok(None),
        _ => {
            let mut view = snapshot;
            match roots::read(&mut view, family, branch) {
                Ok(published) => Ok(published.map(Arc::new)),
                Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                    Err(Arc::from(error.to_string()))
                }
                Err(error) => return Err(error),
            }
        }
    };
    Ok(gate)
}

/// What a store folder publishes now, as this process last decoded it.
#[derive(Clone)]
enum Held {
    /// No publication: every key reads it as a store without a manifest.
    Nothing,
    Decoded(Arc<Published>),
    /// A publication that could not be decoded: its queries wait for the next.
    Undecoded,
}

/// A store manifest's on-disk identity ([`store::manifest_identity`]); none without one.
type Manifest = Option<(u64, u64, u64)>;

/// A bound store folder: what it publishes now as this process last decoded it, none before
/// its first decoding, the manifest that was decoded from, none while not known, and how many
/// sources bind it.
struct Bound {
    held: Option<Held>,
    manifest: Option<Manifest>,
    leases: usize,
}

/// What each store folder a source of this process binds publishes now.
static CURRENT: Mutex<BTreeMap<PathBuf, Bound>> = Mutex::new(BTreeMap::new());
/// Held while a publication is decoded and made current, so an earlier never replaces a later.
static DECODING: Mutex<()> = Mutex::new(());

fn held(folder: &Path) -> Option<Held> {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(folder)?
        .held
        .clone()
}

/// A source's binding of a store folder: the folder's publication is kept decoded while any
/// source binds it, and released with the last binding.
pub struct Lease {
    folder: PathBuf,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut current = CURRENT.lock().unwrap_or_else(PoisonError::into_inner);
        let released = match current.get_mut(&self.folder) {
            Some(bound) if bound.leases > 1 => {
                bound.leases -= 1;
                None
            }
            _ => current.remove(&self.folder),
        };
        drop(current);
        // Its files close outside the lock.
        drop(released);
    }
}

/// Binds `store`'s folder until the lease drops ([`Lease`]).
pub fn lease(store: &store::Store) -> Lease {
    let folder = store.folder().to_owned();
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(folder.clone())
        .or_insert(Bound {
            held: None,
            manifest: None,
            leases: 0,
        })
        .leases += 1;
    Lease { folder }
}

/// Whether a source binds `folder`, its publication held.
#[cfg(test)]
pub(crate) fn bound(folder: &Path) -> bool {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .contains_key(folder)
}

/// The manifest `folder`'s publication was decoded from; None while it is not bound.
fn decoded_manifest(folder: &Path) -> Option<Option<Manifest>> {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(folder)
        .map(|bound| bound.manifest)
}

fn decoded_of(folder: &Path) -> Option<Arc<Published>> {
    match held(folder) {
        Some(Held::Decoded(published)) => Some(published),
        _ => None,
    }
}

/// Makes `held`, decoded from `manifest`, what a bound `folder` publishes now.
fn make_current(folder: &Path, manifest: Option<Manifest>, held: Held) {
    if let Some(bound) = CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get_mut(folder)
    {
        bound.held = Some(held);
        bound.manifest = manifest;
    }
}

/// Decodes the publication `store`'s folder holds now, whichever key it was committed with, and
/// makes it current: binding a source of another key leaves that publication current for its
/// own key's queries. Only a bound folder's is kept ([`lease`]).
pub fn refresh(store: &store::Store) {
    let _decoding = DECODING.lock().unwrap_or_else(PoisonError::into_inner);
    let folder = store.folder();
    if decoded_manifest(folder).is_none() {
        return;
    }
    // Read before the publication is, so one landing between the two is decoded again.
    let manifest = store.manifest_identity().ok();
    let held = match decode(store, decoded_of(folder).as_deref()) {
        Ok(Some(published)) => Held::Decoded(Arc::new(published)),
        Ok(None) => Held::Nothing,
        // A Clear removed what it read: the Clear's own news makes the folder current.
        Err(error) if store::is_superseded(&error) => return,
        Err(_) => Held::Undecoded,
    };
    make_current(folder, manifest, held);
}

/// Decodes `store`'s publication again when its manifest is not the one decoded: another
/// process published to it, or cleared it, since.
fn refresh_changed(store: &store::Store) {
    let Ok(manifest) = store.manifest_identity() else {
        return;
    };
    if decoded_manifest(store.folder()).is_some_and(|decoded| decoded != Some(manifest)) {
        refresh(store);
    }
}

fn published(folder: &Path, publication: Option<io::Result<store::Snapshot>>) {
    let _decoding = DECODING.lock().unwrap_or_else(PoisonError::into_inner);
    if decoded_manifest(folder).is_none() {
        return;
    }
    // The committing writer holds the store until this returns: none of its files can go, and
    // its manifest is the one committed.
    let manifest = store::manifest_identity(folder).ok();
    let held = match publication {
        None => Held::Nothing,
        Some(Ok(snapshot)) => match build(snapshot, decoded_of(folder).as_deref()) {
            Ok(published) => Held::Decoded(Arc::new(published)),
            Err(_) => Held::Undecoded,
        },
        Some(Err(_)) => Held::Undecoded,
    };
    make_current(folder, manifest, held);
}

/// Decodes, from now on, every publication of this process's stores as it is made.
pub fn decode_publications() {
    store::on_publication(published);
}

thread_local! {
    /// Snapshots the queries this thread runs pinned, innermost last.
    static PINNED: RefCell<Vec<Arc<Published>>> = const { RefCell::new(Vec::new()) };
}

/// Keeps a snapshot pinned on this thread until dropped ([`pin`]).
#[must_use]
pub struct Pinned(());

impl Drop for Pinned {
    fn drop(&mut self) {
        PINNED.with(|pinned| pinned.borrow_mut().pop());
    }
}

/// Pins `published` for this thread's gates of its store until the guard drops: one query
/// reads every supplier it joins from this one publication.
pub fn pin(published: &Arc<Published>) -> Pinned {
    PINNED.with(|pinned| pinned.borrow_mut().push(Arc::clone(published)));
    Pinned(())
}

/// The gates `store` publishes to `key`: the snapshot this thread's query pinned, else the
/// folder's current publication when it is `key`'s, decoded again first when the folder's
/// manifest changed since ([`refresh_changed`]), else a view publishing nothing; None while the
/// folder's publication is not decoded, as when it could not be, or while no source binds it.
pub fn current(store: &store::Store, key: &store::SourceKey) -> Option<Arc<Published>> {
    let folder = store.folder();
    let pinned = PINNED.with(|pinned| {
        pinned
            .borrow()
            .iter()
            .rev()
            .find(|published| published.snapshot.folder() == folder && published.key() == key)
            .cloned()
    });
    if pinned.is_some() {
        return pinned;
    }
    refresh_changed(store);
    match held(folder)? {
        Held::Decoded(published) if published.key() == key => Some(published),
        Held::Decoded(_) | Held::Nothing => Some(Arc::new(Published::empty(folder, key))),
        Held::Undecoded => None,
    }
}
