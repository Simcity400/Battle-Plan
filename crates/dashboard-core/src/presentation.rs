//! The presentation identity a source's widgets share: the visual mods they show over the
//! recording. The dashboard mounts it (dashboard-base's `presentation`) and keys every
//! catalogue, artwork cache and stored reply by it.

use installed_content_core::{
    ResolveOptions,
    content::{
        ResolvedContent,
        visual::{self, Choice, RecordedMod},
    },
};
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
};

/// What a source's widgets show over the recording: its campaign's resolved selection and
/// what that selection mounts now. Equal identities show the same artwork; a mod installed,
/// updated or removed since changes `revision`.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct Presentation {
    /// The selection as the campaign scope resolves it: each mod once, by id. The engine
    /// decides mount order, so another order of the same mods is the same identity.
    pub visual_mods: Vec<Choice>,
    /// [`visual::revision`] of the selection; empty for an empty selection.
    pub revision: String,
}

impl Presentation {
    /// The identity of `selected` for this recording's folders, read from the mods' metadata.
    pub fn current(
        options: &ResolveOptions,
        replay: &replay::container::Replay,
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

    /// Whether this identity shows no visual mod.
    pub fn is_empty(&self) -> bool {
        self.visual_mods.is_empty()
    }
}

/// The mods the recording was made with, as the selection is matched against them.
pub fn recorded_mods(replay: &replay::container::Replay) -> Vec<RecordedMod> {
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

/// A source's recorded content with one presentation mounted.
pub struct Presented {
    pub presentation: Presentation,
    /// The recorded layers, with the selection mounted as their presentation stack
    /// (`content.layers.presentation()`). Readers of recorded facts read `content.layers`
    /// itself; artwork reads the presentation through `ResolvedLayers::artwork` and its kin,
    /// which fall back to the recording by name. Its visual mod diagnostics are its own.
    pub content: Arc<ResolvedContent>,
    /// Every selected mod that is not shown, named with why, and why a selection that cannot be
    /// mounted is not: what a reply drawn under it must say. A selection that mounts in full
    /// says nothing, so its replies read as the recording's own do.
    pub refusals: Vec<String>,
}

impl Presented {
    /// `recorded` with `presentation` mounted. A selection whose mods cannot be read mounts
    /// nothing and says so; the recording's own artwork is then shown.
    /// Stops with `Interrupted` once `cancel` is set, mounting nothing.
    pub fn mount(
        recorded: &ResolvedContent,
        options: &ResolveOptions,
        replay: &replay::container::Replay,
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
            }),
            refusals,
        })
    }

    /// The standing lines every content reply of this presentation carries: the selected mods
    /// that are not shown and why ([`Self::refusals`]). Artwork a reply drew from the recording
    /// instead is its own line (`content::overlay_refusal_line`).
    pub fn append_to(&self, warnings: &mut Vec<String>) {
        for refusal in &self.refusals {
            if !warnings.contains(refusal) {
                warnings.push(refusal.clone());
            }
        }
    }
}

/// What one piece of work is cancelled by: a request's own flag, which `visual::apply` and every
/// wait for a mount watch, or one never set for work nobody cancels.
pub trait Cancel: Send + Sync {
    fn flag(&self) -> &AtomicBool;
}

impl Cancel for AtomicBool {
    fn flag(&self) -> &AtomicBool {
        self
    }
}

/// Work done ahead of requests under a binding is cancelled when the binding is retired.
impl Cancel for Binding {
    fn flag(&self) -> &AtomicBool {
        &self.retired
    }
}

/// Everything work on a source is bound to, as one immutable value: the configured folders, the
/// unverified-content choice and the cache folder (`options`, `allow_unverified`), and the visual
/// mods its widgets show (`presentation`, with `visual::revision` of what they mount). A source
/// holds its current one; a change of any part is a new binding swapped in whole, so no work
/// ever reads one binding's folders with another's mods. Its mount ([`Self::mount`]) is
/// computed once, by the first work that needs it, and lives as long as the binding does.
pub struct Binding {
    /// Distinct for every binding a process makes: work holding another is obsolete.
    pub generation: u64,
    pub options: ResolveOptions,
    pub allow_unverified: bool,
    pub presentation: Presentation,
    pub mount: MountCell,
    /// Set once the binding is no longer its source's (swapped out, unbound, the source
    /// dropped or the service closing): what cancels work done ahead of requests under it,
    /// such as a read-ahead computing its mount ([`Cancel`]).
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

    /// Marks the binding as no longer its source's, cancelling work done ahead under it.
    pub fn retire(&self) {
        self.retired
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether this binds these folders, choice and presentation, so work under either reads the
    /// same content and artwork.
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

/// A binding's mount, computed once outside every source lock. Work that needs it while another
/// computes it waits on the cell, watching its own cancellation; a computation that is
/// cancelled or fails leaves the cell empty for the next to try.
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
    /// The mount, when it has been computed.
    pub fn get(&self) -> Option<Arc<Presented>> {
        match &*self.state.lock().unwrap_or_else(|error| error.into_inner()) {
            Mount::Done(presented) => Some(Arc::clone(presented)),
            _ => None,
        }
    }

    /// The mount, computing it with `mount` when no one has, or waiting for whoever is;
    /// `Interrupted` once `cancel` is set while waiting.
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
        // Settles the cell, and wakes its waiters, however the computation ends: done with the
        // mount, or empty for the next to try.
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

/// What work on a source was admitted under (`ContentOptions::presentation`): its source's
/// binding as it was admitted ([`Binding`]), whether it is content work (whose replies name what
/// the binding's mount cannot show), and what cancels it. Every clone of one admission, on any
/// thread, holds the same binding, which lives as long as anything holds it.
#[derive(Clone)]
pub struct Admitted {
    binding: Arc<Binding>,
    content_work: bool,
    /// Work that reads the recording's own artwork whatever its source shows (preparation):
    /// its identity is no visual mod, and it never mounts its binding's.
    recorded: bool,
    cancel: Arc<dyn Cancel>,
    /// The binding's mount as the work's entry resolved it, before any of its code ran
    /// ([`Admitted::set_mount`]); `None` until then, and for work that needs none.
    mount: Option<Result<Arc<Presented>, (io::ErrorKind, String)>>,
}

/// The identity of recorded-only work: no visual mod.
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
    /// Content work under a binding of its own showing `identity` and no folders: a test's, or
    /// work outside the service, never cancelled.
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

    /// Work admitted under `binding`, cancelled by `cancel`.
    pub fn admitted(binding: Arc<Binding>, content_work: bool, cancel: Arc<dyn Cancel>) -> Self {
        Self {
            binding,
            content_work,
            recorded: false,
            cancel,
            mount: None,
        }
    }

    /// Recorded-only work under `binding`'s folders: it reads the recording's own artwork, never
    /// the binding's visual mods, and is never cancelled.
    pub fn recorded(binding: Arc<Binding>) -> Self {
        Self {
            binding,
            content_work: false,
            recorded: true,
            cancel: Arc::new(AtomicBool::new(false)),
            mount: None,
        }
    }

    /// Whether the work reads the recording's own artwork whatever its source shows.
    pub fn is_recorded(&self) -> bool {
        self.recorded
    }

    /// The visual mods the work's artwork shows: its binding's, or none for recorded-only work.
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

    /// Whether the work reads content, so its replies name what its mount cannot show.
    pub fn content_work(&self) -> bool {
        self.content_work
    }

    /// What cancels the work.
    pub fn cancel(&self) -> &AtomicBool {
        self.cancel.flag()
    }

    /// The binding's mount, when it has been computed; recorded-only work holds none.
    pub fn held(&self) -> Option<Arc<Presented>> {
        (!self.recorded).then(|| self.binding.mount.get()).flatten()
    }

    /// Whether the work draws its binding's visual mods, so its entry must resolve their mount
    /// before any of its code runs: not recorded-only work, nor work showing no visual mod.
    pub fn shows_mods(&self) -> bool {
        !self.identity().is_empty()
    }

    /// Keeps the mount the work's entry resolved, so its code finds it resolved under any lock.
    pub fn set_mount(&mut self, mount: &io::Result<Arc<Presented>>) {
        self.mount = Some(match mount {
            Ok(presented) => Ok(Arc::clone(presented)),
            Err(error) => Err((error.kind(), error.to_string())),
        });
    }

    /// The mount the work's entry resolved, if it did.
    pub fn mount(&self) -> Option<io::Result<Arc<Presented>>> {
        self.mount.as_ref().map(|mount| match mount {
            Ok(presented) => Ok(Arc::clone(presented)),
            Err((kind, message)) => Err(io::Error::new(*kind, message.clone())),
        })
    }
}

/// A configuration and the generation every change of it bumps, for writers that install what
/// they derive from it only while it is still the configuration they read ([`rebind`]).
/// Closing it is the last change: nothing is installed afterwards.
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

    /// The configuration and its generation, read together; `None` once closed.
    pub fn snapshot(&self) -> Option<(T, u64)> {
        let state = self.lock();
        (!state.closed).then(|| (state.value.clone(), state.generation))
    }

    /// Changes the configuration (or only marks it changed, for configuration kept elsewhere,
    /// such as saved map settings) and bumps its generation; returns what `change` returns.
    pub fn update<R>(&self, change: impl FnOnce(&mut T) -> R) -> R {
        let mut state = self.lock();
        let result = change(&mut state.value);
        state.generation += 1;
        result
    }

    /// Closes the configuration for good (shutdown) and bumps its generation, so a derivation
    /// still in flight installs nothing, and every later [`rebind`] stops.
    pub fn close(&self) {
        let mut state = self.lock();
        state.closed = true;
        state.generation += 1;
    }

    /// Runs `install` under the configuration's lock if it is still open at `generation`,
    /// returning whether it ran.
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

/// Derives what a configuration gives (`compute`, outside every lock) and installs it
/// (`install`, under the configuration's lock) only while the configuration is still the one it
/// was derived from; a configuration changed meanwhile is read again and the derivation redone,
/// so the last change always wins and nothing derived from an older one stays installed.
/// Retries happen only because the configuration really changed. Stops with `Interrupted` once
/// `cancel` is set or the configuration is closed, installing nothing.
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

/// The identity the work shows ([`Admitted::identity`]): recorded-only work shows no visual
/// mod whatever its binding selects.
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
