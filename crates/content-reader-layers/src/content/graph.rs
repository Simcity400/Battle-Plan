use super::{ContentRead, ReadOutcome, diagnostics};
use sha2::{Digest as _, Sha256};
use std::{
    any::{Any, type_name},
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

pub type Digest = [u8; 32];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) struct Key {
    pub context: Digest,
    pub parser: &'static str,
    pub schema: u32,
    pub parameters: Vec<u8>,
}

impl Key {
    fn digest(&self) -> Digest {
        let mut hash = Sha256::new();
        hash.update(b"content-product/v1");
        hash.update(self.context);
        field(&mut hash, self.parser.as_bytes());
        hash.update(self.schema.to_le_bytes());
        field(&mut hash, &self.parameters);
        hash.finalize().into()
    }
}

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

#[derive(Clone)]
struct Failure(Arc<io::Error>);

impl Failure {
    fn capture(error: io::Error) -> Self {
        Self(Arc::new(error))
    }
    fn error(&self) -> io::Error {
        io::Error::new(self.0.kind(), Arc::clone(&self.0))
    }
}

pub(super) struct Product {
    metadata: Arc<Metadata>,
    value: Result<Arc<dyn Any + Send + Sync>, Failure>,
}

pub(super) struct Metadata {
    request: Digest,
    pub reads: Vec<ContentRead>,
    pub diagnostics: diagnostics::Diagnostics,
    content: Digest,
}

impl std::ops::Deref for Product {
    type Target = Metadata;
    fn deref(&self) -> &Metadata {
        &self.metadata
    }
}

enum Kept {
    Strong(Arc<Product>),
    Bytes(content_reader_image::WeakBytes),
    Weak(std::sync::Weak<dyn Any + Send + Sync>),
    Empty,
}

struct Entry {
    metadata: Arc<Metadata>,
    kept: Mutex<Kept>,
}

impl Entry {
    fn view(&self) -> Option<Arc<Product>> {
        match &*self.kept.lock().unwrap_or_else(|error| error.into_inner()) {
            Kept::Empty => None,
            Kept::Strong(product) => Some(Arc::clone(product)),
            Kept::Bytes(bytes) => bytes.upgrade().map(|bytes| {
                Arc::new(Product {
                    metadata: Arc::clone(&self.metadata),
                    value: Ok(Arc::new(bytes)),
                })
            }),
            Kept::Weak(value) => value.upgrade().map(|value| {
                Arc::new(Product {
                    metadata: Arc::clone(&self.metadata),
                    value: Ok(value),
                })
            }),
        }
    }

    fn release(&self) {
        let mut kept = self.kept.lock().unwrap_or_else(|error| error.into_inner());
        if let Kept::Strong(product) = &*kept {
            *kept = match &product.value {
                Ok(value) => match value.downcast_ref::<super::Bytes>() {
                    Some(bytes) => Kept::Bytes(bytes.downgrade()),
                    None => Kept::Weak(Arc::downgrade(value)),
                },
                Err(_) => Kept::Empty,
            };
        }
    }
}

impl Product {
    pub(super) fn successful(&self) -> bool {
        self.value.is_ok()
    }

    pub(super) fn request(&self) -> Digest {
        self.metadata.request
    }

    pub(super) fn identity(&self) -> (Digest, Digest) {
        (self.metadata.request, self.metadata.content)
    }
    pub(super) fn value<T: Send + Sync + 'static>(&self) -> io::Result<Arc<T>> {
        Arc::clone(self.value.as_ref().map_err(Failure::error)?)
            .downcast::<T>()
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "content product requested incompatible type {}",
                        type_name::<T>()
                    ),
                )
            })
    }
}

pub(super) struct Node {
    id: Digest,
    key: Key,
    result: Mutex<Option<Arc<Entry>>>,
    confirmed: OnceLock<Digest>,
    refresh: Mutex<()>,
    builds: AtomicU64,
    building: AtomicBool,
    requests: AtomicUsize,
    dependencies: Mutex<BTreeSet<Digest>>,
}

impl Node {
    pub(super) fn request(&self) -> Digest {
        self.id
    }

    fn content(&self) -> Option<Digest> {
        self.result
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(|entry| entry.metadata.content)
    }

    fn ready(&self, retry_errors: bool) -> Option<Arc<Product>> {
        self.result
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .and_then(|entry| entry.view())
            .filter(|product| !retry_errors || product.successful())
    }
}

#[derive(Default)]
struct WatchState {
    pending: BTreeSet<(std::path::PathBuf, bool)>,
    arming: bool,
    ready: bool,
}

pub(super) struct Graph {
    state: Arc<Mutex<State>>,
    pub witnesses: Arc<super::generation::Witnesses>,
    pub(super) changes: Arc<super::changes::Changes>,
    watches: Arc<Mutex<WatchState>>,
    arming_lock: Arc<Mutex<()>>,
    watches_ready: Arc<AtomicBool>,
    pub(super) inventories: Arc<Mutex<Vec<std::sync::Weak<super::generation::Witnesses>>>>,
    loading: Arc<AtomicBool>,
    scoped: Arc<AtomicBool>,
    pub stale: Arc<AtomicBool>,
    pub snapshots: Arc<Mutex<BTreeMap<std::path::PathBuf, super::generation::Snapshot>>>,
    users: Arc<Mutex<usize>>,
}

impl Default for Graph {
    fn default() -> Self {
        super::generation::current().map_or_else(Self::fresh, |current| Self {
            state: Arc::clone(&current.state),
            witnesses: Arc::clone(&current.witnesses),
            changes: Arc::clone(&current.changes),
            watches: Arc::clone(&current.watches),
            arming_lock: Arc::clone(&current.arming_lock),
            watches_ready: Arc::clone(&current.watches_ready),
            inventories: Arc::clone(&current.inventories),
            loading: Arc::clone(&current.loading),
            scoped: Arc::clone(&current.scoped),
            stale: Arc::clone(&current.stale),
            snapshots: Arc::clone(&current.snapshots),
            users: Arc::clone(&current.users),
        })
    }
}

#[derive(Default)]
struct State {
    nodes: BTreeMap<Key, Arc<Node>>,
    by_id: BTreeMap<Digest, Arc<Node>>,
}

impl std::fmt::Debug for Graph {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Graph").finish_non_exhaustive()
    }
}

#[derive(serde::Serialize)]
pub struct ProductCount {
    pub parser: &'static str,
    pub schema: u32,
    pub request: Digest,
    pub content: Option<Digest>,
    pub builds: u64,
    pub dependencies: Vec<Digest>,
}

#[derive(serde::Serialize)]
pub struct Counts {
    pub generation_validations: u64,
    pub change_watch_failed: bool,
    pub active_loading_leases: usize,
    pub retained_source_bytes: u64,
    pub total_reads: u64,
    pub bytes_read: u64,
    pub repeated_resolved_byte_reads: u64,
    pub repeated_bytes: u64,
    pub products: Vec<ProductCount>,
}

impl Graph {
    pub(super) fn fresh() -> Self {
        Self {
            state: Arc::default(),
            witnesses: Arc::default(),
            changes: Arc::default(),
            watches: Arc::default(),
            arming_lock: Arc::default(),
            watches_ready: Arc::new(AtomicBool::new(false)),
            inventories: Arc::default(),
            loading: Arc::new(AtomicBool::new(false)),
            scoped: Arc::new(AtomicBool::new(false)),
            stale: Arc::new(AtomicBool::new(false)),
            snapshots: Arc::default(),
            users: Arc::default(),
        }
    }

    pub(super) fn is_scoped(&self) -> bool {
        self.scoped.load(Ordering::Acquire)
    }

    pub(super) fn watch_root(&self, root: &std::path::Path, subtree: bool) {
        if !self.watches_ready.load(Ordering::Acquire) {
            let mut watches = self
                .watches
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !watches.ready {
                watches.pending.insert((root.to_path_buf(), subtree));
                return;
            }
        }
        self.changes.watch_root(root, subtree);
    }

    pub(super) fn has_unwatched_roots(&self) -> bool {
        if self.watches_ready.load(Ordering::Acquire) {
            return false;
        }
        let watches = self
            .watches
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        !watches.ready && (watches.arming || !watches.pending.is_empty())
    }

    pub(super) fn acquire(&self) {
        {
            let mut users = self.users.lock().unwrap_or_else(|error| error.into_inner());
            if *users == 0 {
                self.snapshots
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clear();
                self.prune();
            }
            *users += 1;
            self.scoped.store(true, Ordering::Release);
            self.loading.store(true, Ordering::Release);
        }
        if !self.watches_ready.load(Ordering::Acquire) {
            self.arm_pending_watches_with(|root, subtree| self.changes.watch_root(root, subtree));
        }
    }

    pub(super) fn arm_pending_watches_with(
        &self,
        mut register: impl FnMut(&std::path::Path, bool),
    ) {
        let _arm = self
            .arming_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut armed_any = false;
        struct ResetArming<'a>(&'a Mutex<WatchState>);
        impl Drop for ResetArming<'_> {
            fn drop(&mut self) {
                self.0
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .arming = false;
            }
        }
        let _reset = ResetArming(&self.watches);
        loop {
            let batch = {
                let mut watches = self
                    .watches
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if watches.ready {
                    return;
                }
                if watches.pending.is_empty() {
                    if armed_any {
                        self.changes.changed();
                    }
                    watches.arming = false;
                    watches.ready = true;
                    self.watches_ready.store(true, Ordering::Release);
                    return;
                }
                watches.arming = true;
                armed_any = true;
                std::mem::take(&mut watches.pending)
            };
            for (root, subtree) in batch {
                register(&root, subtree);
            }
        }
    }

    pub(super) fn inventory(&self, inventory: &Arc<super::generation::Witnesses>) {
        let mut inventories = self.inventories.lock().unwrap_or_else(|error| error.into_inner());
        inventories.retain(|inventory| inventory.strong_count() != 0);
        if !inventories.iter().filter_map(std::sync::Weak::upgrade).any(|known| Arc::ptr_eq(&known, inventory)) {
            inventories.push(Arc::downgrade(inventory));
            self.changes.changed();
        }
    }

    pub(super) fn release(&self) {
        let mut users = self.users.lock().unwrap_or_else(|error| error.into_inner());
        *users -= 1;
        if *users == 0 {
            self.finish();
        }
    }

    fn prune(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut keep = BTreeSet::new();
        let mut pending: Vec<_> = state
            .nodes
            .values()
            .filter(|node| {
                node.requests.load(Ordering::Acquire) != 0 || node.result
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .as_ref()
                    .is_none_or(|entry| entry.view().is_some())
            })
            .map(|node| node.id)
            .collect();
        while let Some(id) = pending.pop() {
            if keep.insert(id)
                && let Some(node) = state.by_id.get(&id)
            {
                pending.extend(
                    node.dependencies
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .iter()
                        .copied(),
                );
            }
        }
        state.nodes.retain(|_, node| keep.contains(&node.id));
        state.by_id.retain(|id, _| keep.contains(id));
    }

    fn finish(&self) {
        if !self.loading.swap(false, Ordering::AcqRel) {
            return;
        }
        self.snapshots
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
        let entries: Vec<_> = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .nodes
            .values()
            .filter_map(|node| {
                node.result
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone()
            })
            .collect();
        for entry in entries {
            entry.release();
        }
    }

    pub(super) fn release_sources(&self) {
        self.release_parsers(&["source bytes", "resolved bytes"]);
    }

    pub(super) fn release_parsers(&self, parsers: &[&str]) {
        let entries: Vec<_> = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .nodes
            .values()
            .filter(|node| parsers.contains(&node.key.parser))
            .filter_map(|node| {
                node.result
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone()
            })
            .collect();
        for entry in entries {
            entry.release();
        }
    }

    pub(super) fn release_source(&self, request: Digest) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let Some(node) = state.by_id.get(&request) else {
            return;
        };
        let dependencies = node.dependencies.lock().unwrap_or_else(|error| error.into_inner());
        for node in std::iter::once(node).chain(dependencies.iter().filter_map(|id| state.by_id.get(id))) {
            if matches!(node.key.parser, "resolved bytes" | "source bytes")
                && let Some(entry) = node.result.lock().unwrap_or_else(|error| error.into_inner()).as_ref()
            {
                entry.release();
            }
        }
    }

    pub(super) fn depends(&self, parent: &Arc<Node>, product: &Product) -> io::Result<()> {
        self.depends_on(parent, product.request)
    }

    pub(super) fn depends_on(&self, parent: &Arc<Node>, request: Digest) -> io::Result<()> {
        let child = self
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .by_id
            .get(&request)
            .cloned();
        if let Some(child) = child {
            self.dependency(parent, &child)?;
        }
        Ok(())
    }
    pub(super) fn product<T: Send + Sync + 'static>(
        &self,
        key: Key,
        parent: Option<&Arc<Node>>,
        build: impl FnOnce(&Arc<Node>) -> (io::Result<T>, Vec<ContentRead>, diagnostics::Diagnostics),
    ) -> io::Result<(Arc<Product>, bool)> {
        self.product_with_retry(key, parent, true, build)
    }

    pub(super) fn product_with_retry<T: Send + Sync + 'static>(
        &self,
        key: Key,
        parent: Option<&Arc<Node>>,
        retry_errors: bool,
        build: impl FnOnce(&Arc<Node>) -> (io::Result<T>, Vec<ContentRead>, diagnostics::Diagnostics),
    ) -> io::Result<(Arc<Product>, bool)> {
        if !self.scoped.load(Ordering::Acquire) {
            let node = Arc::new(Node {
                id: key.digest(),
                key,
                result: Mutex::new(None),
                confirmed: OnceLock::new(),
                refresh: Mutex::new(()),
                builds: AtomicU64::new(0),
                building: AtomicBool::new(false),
                requests: AtomicUsize::new(0),
                dependencies: Mutex::new(BTreeSet::new()),
            });
            let entry = self.construct(&node, build);
            return Ok((entry.view().expect("new product holds its value"), true));
        }
        let node = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let node = Arc::clone(state.nodes.entry(key.clone()).or_insert_with(|| {
                Arc::new(Node {
                    id: key.digest(),
                    key,
                    result: Mutex::new(None),
                    confirmed: OnceLock::new(),
                    refresh: Mutex::new(()),
                    builds: AtomicU64::new(0),
                    building: AtomicBool::new(false),
                    requests: AtomicUsize::new(0),
                    dependencies: Mutex::new(BTreeSet::new()),
                })
            }));
            state
                .by_id
                .entry(node.request())
                .or_insert_with(|| Arc::clone(&node));
            node.requests.fetch_add(1, Ordering::Relaxed);
            node
        };
        struct Request<'a>(&'a AtomicUsize);
        impl Drop for Request<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::Release);
            }
        }
        let _request = Request(&node.requests);
        if let Some(parent) = parent {
            self.dependency(parent, &node)?;
        }
        let (product, built) = match node.ready(retry_errors) {
            Some(product) => (product, false),
            None => {
                let _refresh = node
                    .refresh
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                match node.ready(retry_errors) {
                    Some(product) => (product, false),
                    None => {
                        let replacement = self.construct(&node, build);
                        let product = replacement.view().expect("new product holds its value");
                        if product.successful()
                            && *node.confirmed.get_or_init(|| product.content) != product.content
                        {
                            self.stale.store(true, Ordering::Release);
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!(
                                    "content product changed within its generation: {} (schema {})",
                                    node.key.parser, node.key.schema
                                ),
                            ));
                        }
                        *node
                            .result
                            .lock()
                            .unwrap_or_else(|error| error.into_inner()) = Some(replacement);
                        (product, true)
                    }
                }
            }
        };
        if !self.loading.load(Ordering::Acquire)
            && let Some(entry) = node
                .result
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
        {
            entry.release();
        }
        Ok((product, built))
    }

    pub(super) fn product_shared_if<T: Send + Sync + 'static>(
        &self,
        key: Key,
        parent: Option<&Arc<Node>>,
        build: impl FnOnce(&Arc<Node>) -> (io::Result<T>, Vec<ContentRead>, diagnostics::Diagnostics),
        shareable: impl FnOnce(&T, &[ContentRead]) -> bool,
    ) -> io::Result<(Arc<Product>, bool)> {
        let node = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let node = Arc::clone(state.nodes.entry(key.clone()).or_insert_with(|| {
                Arc::new(Node {
                    id: key.digest(),
                    key,
                    result: Mutex::new(None),
                    confirmed: OnceLock::new(),
                    refresh: Mutex::new(()),
                    builds: AtomicU64::new(0),
                    building: AtomicBool::new(false),
                    requests: AtomicUsize::new(0),
                    dependencies: Mutex::new(BTreeSet::new()),
                })
            }));
            state
                .by_id
                .entry(node.request())
                .or_insert_with(|| Arc::clone(&node));
            node.requests.fetch_add(1, Ordering::Relaxed);
            node
        };
        struct Request<'a>(&'a AtomicUsize);
        impl Drop for Request<'_> {
            fn drop(&mut self) {
                self.0.fetch_sub(1, Ordering::Release);
            }
        }
        let _request = Request(&node.requests);
        if let Some(parent) = parent {
            self.dependency(parent, &node)?;
        }
        let (product, built) = match node.ready(true) {
            Some(product) => (product, false),
            None => {
                let _refresh = node
                    .refresh
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                match node.ready(true) {
                    Some(product) => (product, false),
                    None => {
                        let replacement = self.construct(&node, build);
                        let product = replacement.view().expect("new product holds its value");
                        let share = product
                            .value::<T>()
                            .is_ok_and(|value| shareable(&value, &product.reads));
                        if share {
                            if self.is_scoped()
                                && *node.confirmed.get_or_init(|| product.content)
                                    != product.content
                            {
                                self.stale.store(true, Ordering::Release);
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    format!(
                                        "content product changed within its generation: {} (schema {})",
                                        node.key.parser, node.key.schema
                                    ),
                                ));
                            }
                            *node
                                .result
                                .lock()
                                .unwrap_or_else(|error| error.into_inner()) = Some(replacement);
                        }
                        (product, true)
                    }
                }
            }
        };
        if self.is_scoped()
            && !self.loading.load(Ordering::Acquire)
            && let Some(entry) = node
                .result
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
        {
            entry.release();
        }
        Ok((product, built))
    }

    fn construct<T: Send + Sync + 'static>(
        &self,
        node: &Arc<Node>,
        build: impl FnOnce(&Arc<Node>) -> (io::Result<T>, Vec<ContentRead>, diagnostics::Diagnostics),
    ) -> Arc<Entry> {
        struct Building<'a>(&'a AtomicBool);
        impl Drop for Building<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        node.building.store(true, Ordering::Release);
        let _building = Building(&node.building);
        node.builds.fetch_add(1, Ordering::Relaxed);
        let (value, reads, diagnostics) = build(node);
        let mut hash = Sha256::new();
        hash.update(content_digest(&node.key, &reads));
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        for dependency in node
            .dependencies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
        {
            if let Some(content) = state.by_id.get(dependency).and_then(|node| node.content()) {
                hash.update(content);
            }
        }
        let metadata = Arc::new(Metadata {
            request: node.request(),
            reads,
            diagnostics,
            content: hash.finalize().into(),
        });
        let product = Arc::new(Product {
            metadata: Arc::clone(&metadata),
            value: value
                .map(|value| Arc::new(value) as Arc<dyn Any + Send + Sync>)
                .map_err(Failure::capture),
        });
        Arc::new(Entry {
            metadata,
            kept: Mutex::new(Kept::Strong(product)),
        })
    }

    fn dependency(&self, parent: &Arc<Node>, child: &Arc<Node>) -> io::Result<()> {
        let from = parent.request();
        let to = child.request();
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let refreshing = parent.builds.load(Ordering::Relaxed) > 1;
        let mut pending = vec![to];
        let mut visited = BTreeSet::new();
        while let Some(id) = pending.pop() {
            if id == from {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "content dependency cycle: {} needs {}",
                        parent.key.parser, child.key.parser,
                    ),
                ));
            }
            if visited.insert(id)
                && let Some(node) = state.by_id.get(&id)
                && (refreshing
                    || node.building.load(Ordering::Acquire)
                    || node
                        .result
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .is_none())
            {
                pending.extend(
                    node.dependencies
                        .lock()
                        .unwrap_or_else(|error| error.into_inner())
                        .iter()
                        .copied(),
                );
            }
        }
        parent
            .dependencies
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(to);
        Ok(())
    }

    pub(super) fn counts(&self) -> Vec<ProductCount> {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .nodes
            .values()
            .map(|node| ProductCount {
                parser: node.key.parser,
                schema: node.key.schema,
                request: node.request(),
                content: node.content(),
                builds: node.builds.load(Ordering::Relaxed),
                dependencies: node
                    .dependencies
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .iter()
                    .copied()
                    .collect(),
            })
            .collect()
    }

    pub(super) fn snapshot(&self) -> Counts {
        let active_loading_leases = *self.users.lock().unwrap_or_else(|error| error.into_inner());
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut seen = BTreeMap::<(Digest, Vec<u8>, Digest), (u64, u64)>::new();
        let (mut total_reads, mut bytes_read, mut retained_source_bytes) = (0, 0, 0);
        for node in state
            .nodes
            .values()
            .filter(|node| node.key.parser == "resolved bytes" || node.key.parser == "source bytes")
        {
            if node.key.parser == "resolved bytes"
                && node
                    .dependencies
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .iter()
                    .any(|id| {
                        state
                            .by_id
                            .get(id)
                            .is_some_and(|source| source.key.parser == "source bytes")
                    })
            {
                continue;
            }
            let count = node.builds.load(Ordering::Relaxed);
            if let Some(entry) = node
                .result
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_ref()
            {
                if let Kept::Strong(product) =
                    &*entry.kept.lock().unwrap_or_else(|error| error.into_inner())
                    && let Ok(value) = &product.value
                    && let Some(bytes) = value.downcast_ref::<super::Bytes>()
                {
                    retained_source_bytes += bytes.len() as u64;
                }
                for read in &entry.metadata.reads {
                    if let ReadOutcome::Bytes { length, digest } = &read.outcome {
                        total_reads += count;
                        bytes_read += length * count;
                        let entry = seen
                            .entry((node.key.context, read.path.clone(), *digest))
                            .or_default();
                        entry.0 += count;
                        entry.1 = *length;
                    }
                }
            }
        }
        let repeated_resolved_byte_reads = seen
            .values()
            .map(|(count, _)| count.saturating_sub(1))
            .sum();
        let repeated_bytes = seen
            .values()
            .map(|(count, length)| count.saturating_sub(1) * length)
            .sum();
        drop(state);
        Counts {
            generation_validations: self.changes.validations.load(Ordering::Relaxed),
            change_watch_failed: self.changes.failed(),
            active_loading_leases,
            retained_source_bytes,
            total_reads,
            bytes_read,
            repeated_resolved_byte_reads,
            repeated_bytes,
            products: self.counts(),
        }
    }
}

fn content_digest(key: &Key, reads: &[ContentRead]) -> Digest {
    let mut inputs = BTreeSet::new();
    for read in reads {
        let mut hash = Sha256::new();
        field(&mut hash, &read.path);
        hash.update(
            read.layer
                .map(|layer| layer as u64)
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        match &read.outcome {
            ReadOutcome::Bytes { length, digest } => {
                hash.update([0]);
                hash.update(length.to_le_bytes());
                hash.update(digest);
            }
            ReadOutcome::Probe(present) => {
                hash.update([1, u8::from(*present)]);
            }
            ReadOutcome::Error {
                kind,
                message,
                payload,
            } => {
                hash.update([2]);
                field(&mut hash, kind.as_bytes());
                field(&mut hash, message.as_bytes());
                if let Some(payload) = payload {
                    hash.update(payload.length.to_le_bytes());
                    hash.update(payload.digest);
                }
            }
        }
        inputs.insert(<Digest>::from(hash.finalize()));
    }
    let mut hash = Sha256::new();
    hash.update(key.digest());
    for input in inputs {
        hash.update(input);
    }
    hash.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_load_keeps_an_unleased_rebuild_registered() {
        let graph = Graph::fresh();
        let key = Key {
            context: [1; 32], parser: "pruning regression", schema: 1,
            parameters: Vec::new(),
        };
        graph.acquire();
        drop(graph.product(key.clone(), None, |_| {
            (Ok(7_u32), Vec::new(), diagnostics::Diagnostics::default())
        }).unwrap());
        graph.release();
        let (product, built) = graph.product(key, None, |node| {
            graph.acquire();
            assert!(graph.state.lock().unwrap().by_id.contains_key(&node.id));
            graph.release();
            (Ok(7_u32), Vec::new(), diagnostics::Diagnostics::default())
        }).unwrap();
        assert!(built);
        assert_eq!(*product.value::<u32>().unwrap(), 7);
    }

    #[test]
    fn release_parsers_downgrades_only_matching_entries_and_keeps_confirmation() {
        let graph = Graph::fresh();
        let released_key = Key {
            context: [2; 32],
            parser: "binary asset grammar",
            schema: 1,
            parameters: Vec::new(),
        };
        let kept_key = Key {
            context: [2; 32],
            parser: "retained product",
            schema: 1,
            parameters: Vec::new(),
        };
        graph.acquire();
        let (released, built) = graph
            .product(released_key.clone(), None, |_| {
                (Ok(42_u32), Vec::new(), diagnostics::Diagnostics::default())
            })
            .unwrap();
        assert!(built);
        let (kept, built) = graph
            .product(kept_key.clone(), None, |_| {
                (Ok(99_u32), Vec::new(), diagnostics::Diagnostics::default())
            })
            .unwrap();
        assert!(built);
        let held = released.value::<u32>().unwrap();
        let kept_weak = Arc::downgrade(&kept.value::<u32>().unwrap());
        let released_weak = Arc::downgrade(&held);
        drop(released);
        drop(kept);

        graph.release_parsers(&["binary asset grammar"]);
        assert!(released_weak.upgrade().is_some());
        drop(held);
        assert!(released_weak.upgrade().is_none());
        assert!(kept_weak.upgrade().is_some());

        let (kept_again, rebuilt_kept) = graph
            .product(kept_key, None, |_| {
                (Ok(0_u32), Vec::new(), diagnostics::Diagnostics::default())
            })
            .unwrap();
        assert!(!rebuilt_kept);
        assert_eq!(*kept_again.value::<u32>().unwrap(), 99);

        let (rebuilt, was_rebuilt) = graph
            .product(released_key, None, |_| {
                (Ok(42_u32), Vec::new(), diagnostics::Diagnostics::default())
            })
            .unwrap();
        assert!(was_rebuilt);
        assert_eq!(*rebuilt.value::<u32>().unwrap(), 42);
        graph.release();
    }
}
