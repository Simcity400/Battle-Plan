use super::{LayerPath, OVERLAY_READS, display, overlay_refusal_line};
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, replay_spool::Spooled)]
struct Notes {
    uncovered: BTreeMap<u32, BTreeSet<LayerPath>>,
    overlays: BTreeMap<String, BTreeSet<LayerPath>>,
    shaders: BTreeSet<String>,
    fallbacks: BTreeSet<String>,
    refused: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, replay_spool::Spooled)]
pub struct Diagnostics(Option<Arc<Notes>>);

static EMPTY: Notes = Notes {
    uncovered: BTreeMap::new(),
    overlays: BTreeMap::new(),
    shaders: BTreeSet::new(),
    fallbacks: BTreeSet::new(),
    refused: BTreeSet::new(),
};

impl Serialize for Diagnostics {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.as_deref().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Diagnostics {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Option::<Notes>::deserialize(deserializer).map(|notes| Self(notes.map(Arc::new)))
    }
}

impl Diagnostics {
    fn notes(&self) -> &Notes {
        self.0.as_deref().unwrap_or(&EMPTY)
    }
    pub fn is_empty(&self) -> bool {
        self.notes().uncovered.is_empty()
            && self.notes().overlays.is_empty()
            && self.notes().shaders.is_empty()
            && self.notes().fallbacks.is_empty()
            && self.notes().refused.is_empty()
    }

    pub fn has_uncovered(&self) -> bool {
        !self.notes().uncovered.is_empty()
    }

    pub fn heap_bytes(&self) -> usize {
        fn tree<K, V>(len: usize) -> usize {
            if len == 0 {
                return 0;
            }
            let nodes = (2 * len).div_ceil(11).max(1);
            let node = 2 * size_of::<usize>() + 11 * size_of::<(K, V)>();
            nodes * (node + node.div_ceil(6) + 2 * size_of::<usize>())
        }
        let Some(notes) = &self.0 else {
            return 0;
        };
        let mut bytes = size_of::<Notes>() + 2 * size_of::<usize>();
        bytes += tree::<u32, BTreeSet<LayerPath>>(notes.uncovered.len());
        bytes += tree::<String, BTreeSet<LayerPath>>(notes.overlays.len());
        bytes += notes.overlays.keys().map(String::capacity).sum::<usize>();
        for paths in notes.uncovered.values().chain(notes.overlays.values()) {
            bytes += tree::<LayerPath, ()>(paths.len());
            bytes += paths.iter().map(Vec::capacity).sum::<usize>();
        }
        for lines in [&notes.shaders, &notes.fallbacks, &notes.refused] {
            bytes += tree::<String, ()>(lines.len());
            bytes += lines.iter().map(String::capacity).sum::<usize>();
        }
        bytes
    }

    pub fn has_shader_warnings(&self) -> bool {
        !self.notes().shaders.is_empty()
    }

    pub fn shader_warnings(&self) -> Vec<String> {
        self.notes().shaders.iter().cloned().collect()
    }

    pub fn fallbacks(&self) -> &BTreeSet<String> {
        &self.notes().fallbacks
    }

    pub fn uncovered_paths(&self) -> impl Iterator<Item = &LayerPath> {
        self.notes()
            .uncovered
            .values()
            .flat_map(|paths| paths.iter())
    }

    pub fn merge(&mut self, other: &Self) {
        if other.is_empty()
            || self
                .0
                .as_ref()
                .zip(other.0.as_ref())
                .is_some_and(|(a, b)| Arc::ptr_eq(a, b))
        {
            return;
        }
        if self.is_empty() {
            self.0 = other.0.clone();
            return;
        }
        let own = Arc::make_mut(self.0.as_mut().expect("nonempty diagnostics"));
        for (policy, paths) in &other.notes().uncovered {
            own.uncovered
                .entry(*policy)
                .or_default()
                .extend(paths.iter().cloned());
        }
        for (name, paths) in &other.notes().overlays {
            own.overlays
                .entry(name.clone())
                .or_default()
                .extend(paths.iter().cloned());
        }
        own.shaders.extend(other.notes().shaders.iter().cloned());
        own.fallbacks
            .extend(other.notes().fallbacks.iter().cloned());
        own.refused.extend(other.notes().refused.iter().cloned());
    }

    pub fn credit(&self) {
        if self.is_empty() {
            return;
        }
        SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            let from = scopes.iter().rposition(|scope| scope.isolated).unwrap_or(0);
            if from > 0
                && (!self.notes().uncovered.is_empty()
                    || !self.notes().overlays.is_empty()
                    || !self.notes().shaders.is_empty())
            {
                let mut reads = self.clone();
                let notes = Arc::make_mut(reads.0.as_mut().expect("diagnostics contain reads"));
                notes.fallbacks.clear();
                notes.refused.clear();
                for scope in &mut scopes[..from] {
                    scope.notes.merge(&reads);
                }
            }
            for scope in &mut scopes[from..] {
                scope.notes.merge(self);
            }
        });
        super::credit_overlay_observers(&self.notes().fallbacks);
    }

    pub(super) fn credit_to(&self, layers: &super::ResolvedLayers) {
        if self.is_empty() {
            return;
        }
        if let Some(paths) = layers
            .uncovered_reads
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_mut()
        {
            paths.extend(self.uncovered_paths().cloned());
        }
        {
            let mut own = layers
                .overlay_reads
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            for (name, paths) in &self.notes().overlays {
                own.reads
                    .entry(name.clone())
                    .or_default()
                    .extend(paths.iter().cloned());
            }
            own.refusals.extend(self.notes().fallbacks.iter().cloned());
            own.refused.extend(self.notes().refused.iter().cloned());
        }
        self.credit();
    }

    pub fn append_to(&self, warnings: &mut Vec<String>) {
        if self.is_empty() {
            return;
        }
        warnings.retain(|line| {
            !((line.starts_with("Content: ")
                && line.contains("file(s) read outside the recorded content coverage policy"))
                || line.starts_with(OVERLAY_READS))
        });
        let mut lines = self.shader_warnings();
        for (policy, paths) in &self.notes().uncovered {
            lines.push(format!(
                "Content: {} file(s) read outside the recorded content coverage policy {}; these bytes are not hash-verified: {}.",
                paths.len(), policy, paths.iter().map(|path| display(path)).collect::<Vec<_>>().join(", ")
            ));
        }
        let total: usize = self.notes().overlays.values().map(BTreeSet::len).sum();
        if total > 0 {
            lines.push(format!(
                "{OVERLAY_READS}: {total} file(s) shown from mods that are not part of the recording, not its recorded artwork: {}.",
                self.notes().overlays.iter().map(|(name, paths)| format!("{name} ({})", paths.len())).collect::<Vec<_>>().join(", ")
            ));
        }
        lines.extend(overlay_refusal_line(&self.notes().fallbacks));
        for line in lines {
            if !warnings.contains(&line) {
                warnings.push(line);
            }
        }
    }

    pub(super) fn uncovered(policy: u32, path: &[u8]) {
        if !is_active() {
            return;
        }
        Self(Some(Arc::new(Notes {
            uncovered: BTreeMap::from([(policy, BTreeSet::from([path.to_vec()]))]),
            ..Notes::default()
        })))
        .credit();
    }

    pub(super) fn overlay(name: &str, path: &[u8]) {
        if !is_active() {
            return;
        }
        Self(Some(Arc::new(Notes {
            overlays: BTreeMap::from([(name.to_owned(), BTreeSet::from([path.to_vec()]))]),
            ..Notes::default()
        })))
        .credit();
    }

    pub fn note(warning: String) {
        Self::shader(warning);
    }

    pub fn shader(warning: String) {
        if !is_active() {
            return;
        }
        Self(Some(Arc::new(Notes {
            shaders: BTreeSet::from([warning]),
            ..Notes::default()
        })))
        .credit();
    }

    pub(super) fn credit_fallbacks(warnings: &BTreeSet<String>) {
        Self(Some(Arc::new(Notes {
            fallbacks: warnings.clone(),
            ..Notes::default()
        })))
        .credit();
    }

    pub(super) fn refused(key: String) {
        if !is_active() {
            return;
        }
        Self(Some(Arc::new(Notes {
            refused: BTreeSet::from([key]),
            ..Notes::default()
        })))
        .credit();
    }
}

#[derive(Clone)]
struct Scope {
    notes: Collected,
    isolated: bool,
}

#[derive(Clone)]
enum Collected {
    Local(Diagnostics),
    Shared(Arc<Mutex<Diagnostics>>),
}

impl Collected {
    fn snapshot(&self) -> Diagnostics {
        match self {
            Self::Local(notes) => notes.clone(),
            Self::Shared(notes) => notes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone(),
        }
    }

    fn merge(&mut self, other: &Diagnostics) {
        match self {
            Self::Local(notes) => notes.merge(other),
            Self::Shared(notes) => notes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .merge(other),
        }
    }

    fn share(&mut self) {
        if let Self::Local(notes) = self {
            *self = Self::Shared(Arc::new(Mutex::new(std::mem::take(notes))));
        }
    }
}

thread_local! {
    static SCOPES: RefCell<Vec<Scope>> = const { RefCell::new(Vec::new()) };
}

pub fn current() -> Diagnostics {
    SCOPES.with(|scopes| {
        scopes
            .borrow()
            .last()
            .map(|scope| scope.notes.snapshot())
            .unwrap_or_default()
    })
}

pub fn is_active() -> bool {
    SCOPES.with(|scopes| !scopes.borrow().is_empty())
}

pub fn scope<T>(own: &mut Diagnostics, isolated: bool, run: impl FnOnce() -> T) -> T {
    struct Exit<'a>(&'a mut Diagnostics);
    impl Drop for Exit<'_> {
        fn drop(&mut self) {
            if let Some(scope) = SCOPES.with(|scopes| scopes.borrow_mut().pop()) {
                *self.0 = scope.notes.snapshot();
            }
        }
    }
    SCOPES.with(|scopes| {
        scopes.borrow_mut().push(Scope {
            notes: Collected::Local(own.clone()),
            isolated,
        })
    });
    let _exit = Exit(own);
    let run = || {
        current().credit();
        run()
    };
    if isolated {
        super::observe_overlay_refusals_isolated(run).0
    } else {
        run()
    }
}

pub fn capture<T>(run: impl FnOnce() -> T) -> (T, Diagnostics) {
    let mut own = Diagnostics::default();
    let value = scope(&mut own, true, run);
    (value, own)
}

pub fn observe<T>(run: impl FnOnce() -> T) -> (T, Diagnostics) {
    let mut own = Diagnostics::default();
    let value = scope(&mut own, false, run);
    own.credit();
    (value, own)
}

#[derive(Clone)]
pub struct Context(Vec<Scope>);

impl Context {
    pub fn capture() -> Self {
        Self(SCOPES.with(|scopes| {
            let mut scopes = scopes.borrow_mut();
            for scope in scopes.iter_mut() {
                scope.notes.share();
            }
            scopes.clone()
        }))
    }

    pub fn enter<T>(&self, run: impl FnOnce() -> T) -> T {
        struct Exit(Vec<Scope>);
        impl Drop for Exit {
            fn drop(&mut self) {
                SCOPES.with(|scopes| *scopes.borrow_mut() = std::mem::take(&mut self.0));
            }
        }
        let _exit = Exit(SCOPES.with(|scopes| scopes.replace(self.0.clone())));
        run()
    }
}

#[derive(Debug)]
pub struct Observed<T> {
    value: T,
    diagnostics: Diagnostics,
}

impl<T> Observed<T> {
    pub fn new(initialize: impl FnOnce() -> T) -> Self {
        let (value, diagnostics) = capture(initialize);
        Self { value, diagnostics }
    }

    pub fn get(&self) -> &T {
        self.diagnostics.credit();
        &self.value
    }

    pub fn peek(&self) -> &T {
        &self.value
    }

    pub fn get_mut(&mut self) -> &mut T {
        self.diagnostics.credit();
        &mut self.value
    }

    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> Observed<U> {
        Observed {
            value: map(self.value),
            diagnostics: self.diagnostics,
        }
    }
}

#[derive(Debug)]
pub struct Once<T>(OnceLock<Observed<T>>);

impl<T> Default for Once<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Once<T> {
    pub const fn new() -> Self {
        Self(OnceLock::new())
    }

    pub fn peek(&self) -> Option<&T> {
        self.0.get().map(|entry| &entry.value)
    }

    pub fn get(&self) -> Option<&T> {
        self.0.get().map(|entry| {
            entry.diagnostics.credit();
            &entry.value
        })
    }

    pub fn get_or_init(&self, initialize: impl FnOnce() -> T) -> &T {
        let entry = self.0.get_or_init(|| {
            let (value, diagnostics) = capture(initialize);
            Observed { value, diagnostics }
        });
        entry.diagnostics.credit();
        &entry.value
    }

    pub fn set(&self, value: T) -> Result<(), T> {
        self.0
            .set(Observed {
                value,
                diagnostics: Diagnostics::default(),
            })
            .map_err(|entry| entry.value)
    }
}

impl<T> From<T> for Once<T> {
    fn from(value: T) -> Self {
        Self(OnceLock::from(Observed {
            value,
            diagnostics: Diagnostics::default(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hits_replay_their_reads_and_shader_notes_without_other_entries() {
        let root =
            std::env::temp_dir().join(format!("content-diagnostic-cache-{}", std::process::id()));
        std::fs::create_dir_all(root.join("outside_policy")).unwrap();
        std::fs::write(root.join("outside_policy/a"), b"a").unwrap();
        std::fs::write(root.join("outside_policy/b"), b"b").unwrap();
        let layers = crate::ResolvedLayers::from_folders(&[(&root, &[])]).unwrap();
        let global = layers.report_uncovered_reads();
        let a = Once::new();
        let b = Once::new();
        let read = |entry: &Once<Vec<u8>>, path, warning: &str| {
            observe(|| {
                entry
                    .get_or_init(|| {
                        std::thread::scope(|scope| {
                            crate::parallel::join(crate::parallel::spawn(scope, || {
                                layers.note_shader_warning(warning.to_owned());
                                layers.read_uncovered(path).unwrap().into_vec()
                            }))
                        })
                    })
                    .clone()
            })
            .1
        };
        let first = read(&a, "outside_policy/a", "shader a");
        let mut lines = Vec::new();
        first.append_to(&mut lines);
        assert_eq!(lines, ["shader a".to_owned(), global.warning().unwrap()]);
        let second = read(&b, "outside_policy/b", "shader b");
        assert_ne!(first, second);
        assert_eq!(read(&a, "outside_policy/a", "never resolved again"), first);
        assert_eq!(read(&b, "outside_policy/b", "never resolved again"), second);
        let saved = serde_json::to_vec(&first).unwrap();
        let restored: Diagnostics = serde_json::from_slice(&saved).unwrap();
        assert_eq!(restored, first);
        assert!(!is_active());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn discarded_choices_keep_actual_reads_but_publish_only_accepted_fallbacks() {
        let (_, outer) = observe(|| {
            let (_, candidate) = capture(|| {
                Diagnostics::uncovered(2, b"outside_policy/candidate");
                Diagnostics::credit_fallbacks(&BTreeSet::from(["candidate fallback".to_owned()]));
            });
            assert!(current().has_uncovered());
            assert!(current().fallbacks().is_empty());
            candidate.credit();
            assert!(current().fallbacks().contains("candidate fallback"));
        });
        assert!(outer.has_uncovered());
        assert!(outer.fallbacks().contains("candidate fallback"));
    }

    #[test]
    fn unwinding_keeps_owned_notes_and_restores_the_previous_scope() {
        let mut own = Diagnostics::default();
        let failed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scope(&mut own, false, || {
                Diagnostics::shader("before panic".to_owned());
                panic!("synthetic failure");
            })
        }));
        assert!(failed.is_err());
        assert_eq!(own.shader_warnings(), ["before panic"]);
        assert!(current().is_empty());
        assert!(!is_active());
    }
}
