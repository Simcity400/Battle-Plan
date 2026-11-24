
use content_reader_layers::{ResolveOptions, content::{ResolvedContent, visual::{self, Choice, RecordedMod}}};
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct Presentation {
    pub visual_mods: Vec<Choice>,
    pub revision: String,
}

impl Presentation {
    pub fn current(
        options: &ResolveOptions,
        replay: &replay::campaign::Replay,
        selected: &[Choice],
        cancel: &AtomicBool,
    ) -> io::Result<Self> {
        let mut visual_mods = selected.to_vec();
        visual_mods.sort_by(|a, b| a.id.cmp(&b.id));
        visual_mods.dedup_by(|a, b| a.id == b.id);
        let revision = visual::revision(options, &visual_mods, &recorded_mods(replay), cancel)?;
        Ok(Self {
            visual_mods,
            revision,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.visual_mods.is_empty()
    }
}

pub fn recorded_mods(replay: &replay::campaign::Replay) -> Vec<RecordedMod> {
    replay
        .identity
        .mods
        .iter()
        .map(|recorded| RecordedMod {
            id: recorded.id.clone(),
            name: recorded.display_name.clone(),
        })
        .collect()
}

pub struct Presented {
    pub presentation: Presentation,
    pub content: Arc<ResolvedContent>,
    pub refusals: Vec<String>,
}

impl Presented {
    pub fn mount(
        recorded: &ResolvedContent,
        options: &ResolveOptions,
        replay: &replay::campaign::Replay,
        presentation: &Presentation,
        cancel: &AtomicBool,
    ) -> io::Result<Self> {
        let mut layers = recorded.layers.with_own_overlay_diagnostics();
        let mut refusals = Vec::new();
        if !presentation.is_empty() {
            match visual::apply(
                &mut layers,
                options,
                &presentation.visual_mods,
                &recorded_mods(replay),
                cancel,
            ) {
                Ok(plan) => refusals.extend(plan.warnings),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => return Err(error),
                Err(error) => refusals.push(format!("Visual mods are not shown: {error}.")),
            }
        }
        Ok(Self {
            presentation: presentation.clone(),
            content: Arc::new(ResolvedContent {
                layers,
                stats: recorded.stats.clone(),
                changed: recorded.changed.clone(),
                notes: recorded.notes.clone(),
            }),
            refusals,
        })
    }

    pub fn append_to(&self, warnings: &mut Vec<String>) {
        for refusal in &self.refusals {
            if !warnings.contains(refusal) {
                warnings.push(refusal.clone());
            }
        }
    }
}

pub trait Cancel: Send + Sync {
    fn flag(&self) -> &AtomicBool;
}

impl Cancel for AtomicBool {
    fn flag(&self) -> &AtomicBool {
        self
    }
}

impl Cancel for Binding {
    fn flag(&self) -> &AtomicBool {
        &self.retired
    }
}

pub struct Binding {
    pub generation: u64,
    pub options: ResolveOptions,
    pub allow_unverified: bool,
    pub presentation: Presentation,
    pub mount: MountCell,
    retired: AtomicBool,
}

impl Binding {
    pub fn new(
        options: ResolveOptions,
        allow_unverified: bool,
        presentation: Presentation,
    ) -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            generation: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            options,
            allow_unverified,
            presentation,
            mount: MountCell::default(),
            retired: AtomicBool::new(false),
        }
    }

    pub fn retire(&self) {
        self.retired
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub fn binds(
        &self,
        options: &ResolveOptions,
        allow_unverified: bool,
        presentation: &Presentation,
    ) -> bool {
        self.options.game_dir == options.game_dir
            && self.options.user_dir == options.user_dir
            && self.options.cache_dir == options.cache_dir
            && self.allow_unverified == allow_unverified
            && self.presentation == *presentation
    }
}

#[derive(Default)]
pub struct MountCell {
    state: std::sync::Mutex<Mount>,
    settled: std::sync::Condvar,
}

#[derive(Default)]
enum Mount {
    #[default]
    Empty,
    Computing,
    Done(Arc<Presented>),
}

impl MountCell {
    pub fn get(&self) -> Option<Arc<Presented>> {
        match &*self.state.lock().unwrap_or_else(|error| error.into_inner()) {
            Mount::Done(presented) => Some(Arc::clone(presented)),
            _ => None,
        }
    }

    pub fn get_or_mount(
        &self,
        cancel: &AtomicBool,
        mount: impl FnOnce() -> io::Result<Arc<Presented>>,
    ) -> io::Result<Arc<Presented>> {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        loop {
            match &*state {
                Mount::Done(presented) => return Ok(Arc::clone(presented)),
                Mount::Empty => break,
                Mount::Computing => {
                    if cancel.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(io::Error::new(
                            io::ErrorKind::Interrupted,
                            "cancelled while the visual mods were mounted",
                        ));
                    }
                    state = self
                        .settled
                        .wait_timeout(state, std::time::Duration::from_millis(50))
                        .unwrap_or_else(|error| error.into_inner())
                        .0;
                }
            }
        }
        *state = Mount::Computing;
        drop(state);
        struct Settle<'a>(&'a MountCell, Option<Arc<Presented>>);
        impl Drop for Settle<'_> {
            fn drop(&mut self) {
                *self
                    .0
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()) = match self.1.take() {
                    Some(presented) => Mount::Done(presented),
                    None => Mount::Empty,
                };
                self.0.settled.notify_all();
            }
        }
        let mut settle = Settle(self, None);
        let presented = mount()?;
        settle.1 = Some(Arc::clone(&presented));
        Ok(presented)
    }
}

#[derive(Clone)]
pub struct Admitted {
    binding: Arc<Binding>,
    content_work: bool,
    recorded: bool,
    cancel: Arc<dyn Cancel>,
    mount: Option<Result<Arc<Presented>, (io::ErrorKind, String)>>,
}

static RECORDED: Presentation = Presentation {
    visual_mods: Vec::new(),
    revision: String::new(),
};

impl Default for Admitted {
    fn default() -> Self {
        Self::new(Presentation::default())
    }
}

impl Admitted {
    pub fn new(identity: Presentation) -> Self {
        Self::admitted(
            Arc::new(Binding::new(
                ResolveOptions {
                    game_dir: Default::default(),
                    user_dir: Default::default(),
                    cache_dir: Default::default(),
                },
                false,
                identity,
            )),
            true,
            Arc::new(AtomicBool::new(false)),
        )
    }

    pub fn admitted(binding: Arc<Binding>, content_work: bool, cancel: Arc<dyn Cancel>) -> Self {
        Self {
            binding,
            content_work,
            recorded: false,
            cancel,
            mount: None,
        }
    }

    pub fn recorded(binding: Arc<Binding>) -> Self {
        Self {
            binding,
            content_work: false,
            recorded: true,
            cancel: Arc::new(AtomicBool::new(false)),
            mount: None,
        }
    }

    pub fn is_recorded(&self) -> bool {
        self.recorded
    }

    pub fn identity(&self) -> &Presentation {
        if self.recorded {
            &RECORDED
        } else {
            &self.binding.presentation
        }
    }

    pub fn binding(&self) -> &Arc<Binding> {
        &self.binding
    }

    pub fn content_work(&self) -> bool {
        self.content_work
    }

    pub fn cancel(&self) -> &AtomicBool {
        self.cancel.flag()
    }

    pub fn held(&self) -> Option<Arc<Presented>> {
        (!self.recorded).then(|| self.binding.mount.get()).flatten()
    }

    pub fn shows_mods(&self) -> bool {
        !self.identity().is_empty()
    }

    pub fn set_mount(&mut self, mount: &io::Result<Arc<Presented>>) {
        self.mount = Some(match mount {
            Ok(presented) => Ok(Arc::clone(presented)),
            Err(error) => Err((error.kind(), error.to_string())),
        });
    }

    pub fn mount(&self) -> Option<io::Result<Arc<Presented>>> {
        self.mount.as_ref().map(|mount| match mount {
            Ok(presented) => Ok(Arc::clone(presented)),
            Err((kind, message)) => Err(io::Error::new(*kind, message.clone())),
        })
    }
}

pub struct Config<T> {
    state: std::sync::Mutex<ConfigState<T>>,
}

struct ConfigState<T> {
    value: T,
    generation: u64,
    closed: bool,
}

impl<T: Clone> Config<T> {
    pub fn new(value: T) -> Self {
        Self {
            state: std::sync::Mutex::new(ConfigState {
                value,
                generation: 0,
                closed: false,
            }),
        }
    }

    pub fn get(&self) -> T {
        self.lock().value.clone()
    }

    pub fn snapshot(&self) -> Option<(T, u64)> {
        let state = self.lock();
        (!state.closed).then(|| (state.value.clone(), state.generation))
    }

    pub fn update<R>(&self, change: impl FnOnce(&mut T) -> R) -> R {
        let mut state = self.lock();
        let result = change(&mut state.value);
        state.generation += 1;
        result
    }

    pub fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.generation += 1;
    }

    pub fn install_if(&self, generation: u64, install: impl FnOnce(&T)) -> bool {
        let state = self.lock();
        if state.closed || state.generation != generation {
            return false;
        }
        install(&state.value);
        true
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ConfigState<T>> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }
}

pub fn rebind<T: Clone, X>(
    config: &Config<T>,
    cancel: &AtomicBool,
    compute: impl Fn(&T) -> io::Result<X>,
    install: impl Fn(&T, X),
) -> io::Result<()> {
    loop {
        if cancel.load(std::sync::atomic::Ordering::Acquire) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let Some((value, generation)) = config.snapshot() else {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "closed"));
        };
        let derived = compute(&value)?;
        let mut derived = Some(derived);
        if config.install_if(generation, |value| {
            install(value, derived.take().expect("derived once"))
        }) {
            return Ok(());
        }
    }
}

impl std::ops::Deref for Admitted {
    type Target = Presentation;
    fn deref(&self) -> &Presentation {
        self.identity()
    }
}

impl PartialEq<Presentation> for Admitted {
    fn eq(&self, other: &Presentation) -> bool {
        self.identity() == other
    }
}

impl std::fmt::Debug for Admitted {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.identity().fmt(formatter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_mount_is_still_a_refusal_when_read_back() {
        let mut admitted = Admitted::default();
        admitted.set_mount(&Err(crate::refused("Synthetic visual mods are refused")));
        let Some(Err(error)) = admitted.clone().mount() else {
            panic!("the failed mount is kept");
        };
        assert_eq!(crate::refusal(&error), Some("Synthetic visual mods are refused"));
        assert_eq!(error.to_string(), "Synthetic visual mods are refused");
    }

    #[test]
    fn an_update_that_panics_still_retires_older_snapshots() {
        let config = Config::new(0);
        let (_, generation) = config.snapshot().unwrap();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            config.update(|value| {
                *value = 1;
                panic!("synthetic panic part way through an update");
            })
        }));
        assert!(panicked.is_err());
        assert!(!config.install_if(generation, |_| {}), "a result derived before the update is stale");
    }
}
