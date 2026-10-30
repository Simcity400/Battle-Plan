//! Every gate of a prepared store, decoded once when the store is bound ([`super::files`]) and
//! then once per publication, in the thread that published it, from the publication as it was
//! committed ([`store::on_publication`]): the hourly walk's and each family's and job's coverage
//! page with its refusals, the version stamps the row pages were published under, each family's
//! publication root with its hole summary, and the identities of the pages queries key their
//! answers by. Each store folder holds one current publication, under the key it was committed
//! with, whichever source binds the folder. A query takes the one of its key from memory
//! ([`current`]), or a view publishing nothing when the folder publishes nothing to its key,
//! pins it for its whole duration ([`pin`]) and reads every page through that publication alone
//! ([`Published::scope`]): no reader runs without it. A query finding a file of it removed by a
//! later publication starts again on the later one, whose snapshot was decoded before any of its
//! files could go.

use super::{
    coverage::{self, Page, Versions},
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
    walk: Gate,
    /// Each family's and each job's, by name.
    named: BTreeMap<String, Gate>,
    identities: Option<[u64; 3]>,
}

/// The gates of one publication of a store, under the key it was committed with.
pub struct Published {
    snapshot: store::Snapshot,
    versions_id: Option<[u64; 3]>,
    versions: Decoded<Option<Arc<Versions>>>,
    branches: BTreeMap<u64, Branch>,
}

impl Published {
    /// What a folder publishing nothing to `key` publishes to it.
    fn empty(folder: &Path, key: &store::SourceKey) -> Self {
        Self {
            snapshot: store::Snapshot::empty(folder, key.clone()),
            versions_id: None,
            versions: Ok(None),
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

    /// The hourly walk's gate on `branch`.
    pub fn walk(&self, branch: u64) -> Option<&Gate> {
        self.branches.get(&branch).map(|gates| &gates.walk)
    }

    /// The gate of the family or job `name` on `branch`.
    pub fn gate(&self, branch: u64, name: &str) -> Option<&Gate> {
        self.branches.get(&branch)?.named.get(name)
    }

    /// The version stamps the row preparation's pages were published under; None when no row
    /// preparation published any.
    pub fn versions(&self) -> io::Result<Option<&Versions>> {
        match &self.versions {
            Ok(versions) => Ok(versions.as_deref()),
            Err(error) => Err(raised(error)),
        }
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
    let mut versions_id = None;
    let mut versions = Ok(None);
    let mut built: BTreeMap<u64, Branch> = BTreeMap::new();
    for section in snapshot.section_names() {
        if section == coverage::VERSIONS {
            versions_id = snapshot.range_id(section, 0)?.map(|id| id.cache_key());
            versions = match held {
                Some(held) if held.versions_id == versions_id => held.versions.clone(),
                _ => decoded(&snapshot, section, |bytes| {
                    serde_json::from_slice::<Versions>(bytes).map_err(io::Error::from)
                })?,
            };
            continue;
        }
        if let Some(branch) = section.strip_prefix(IDENTITIES).and_then(digits) {
            built.entry(branch).or_default().identities =
                snapshot.range_id(section, 0)?.map(|id| id.cache_key());
            continue;
        }
        let (branch, name) = match (coverage::walk_of(section), coverage::family_of(section)) {
            (Some(branch), _) => (branch, None),
            (None, Some((name, branch))) => (branch, Some(name)),
            (None, None) => continue,
        };
        let previous = held.and_then(|held| held.branches.get(&branch));
        let gates = built.entry(branch).or_default();
        match name {
            None => {
                let previous = previous.map(|gates| &gates.walk);
                gates.walk = decode_gate(&snapshot, section, None, previous)?;
            }
            Some(name) => {
                let previous = previous.and_then(|gates| gates.named.get(name));
                let gate = decode_gate(&snapshot, section, Some((name, branch)), previous)?;
                gates.named.insert(name.to_owned(), gate);
            }
        }
    }
    Ok(Published {
        snapshot,
        versions_id,
        versions,
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

/// The gate whose coverage page is `section` and, for a family, whose publication is its root
/// on its branch, decoded unless `previous` holds the same ranges.
fn decode_gate(
    snapshot: &store::Snapshot,
    section: &str,
    root: Option<(&str, u64)>,
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
    let Some((family, branch)) = root else {
        return Ok(gate);
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

/// What each store folder this process bound or published to publishes now.
static CURRENT: Mutex<BTreeMap<PathBuf, Held>> = Mutex::new(BTreeMap::new());
/// Held while a publication is decoded and made current, so an earlier never replaces a later.
static DECODING: Mutex<()> = Mutex::new(());

fn held(folder: &Path) -> Option<Held> {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(folder)
        .cloned()
}

fn decoded_of(folder: &Path) -> Option<Arc<Published>> {
    match held(folder) {
        Some(Held::Decoded(published)) => Some(published),
        _ => None,
    }
}

fn make_current(folder: &Path, held: Held) {
    CURRENT
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(folder.to_owned(), held);
}

/// Decodes the publication `store`'s folder holds now, whichever key it was committed with, and
/// makes it current: binding a source of another key leaves that publication current for its
/// own key's queries.
pub fn refresh(store: &store::Store) {
    let _decoding = DECODING.lock().unwrap_or_else(PoisonError::into_inner);
    let folder = store.folder();
    let held = match decode(store, decoded_of(folder).as_deref()) {
        Ok(Some(published)) => Held::Decoded(Arc::new(published)),
        Ok(None) => Held::Nothing,
        // A Clear removed what it read: the Clear's own news makes the folder current.
        Err(error) if store::is_superseded(&error) => return,
        Err(_) => Held::Undecoded,
    };
    make_current(folder, held);
}

fn published(folder: &Path, publication: Option<io::Result<store::Snapshot>>) {
    let _decoding = DECODING.lock().unwrap_or_else(PoisonError::into_inner);
    // The committing writer holds the store until this returns: none of its files can go.
    let held = match publication {
        None => Held::Nothing,
        Some(Ok(snapshot)) => match build(snapshot, decoded_of(folder).as_deref()) {
            Ok(published) => Held::Decoded(Arc::new(published)),
            Err(_) => Held::Undecoded,
        },
        Some(Err(_)) => Held::Undecoded,
    };
    make_current(folder, held);
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

/// The gates `store` publishes to `key`, from memory alone: the snapshot this thread's query
/// pinned, else the folder's current publication when it is `key`'s, else a view publishing
/// nothing; None while the folder's publication is not decoded, as when it could not be.
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
    match held(folder)? {
        Held::Decoded(published) if published.key() == key => Some(published),
        Held::Decoded(_) | Held::Nothing => Some(Arc::new(Published::empty(folder, key))),
        Held::Undecoded => None,
    }
}
