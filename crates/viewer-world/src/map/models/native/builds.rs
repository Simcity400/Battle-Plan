//! Shader compiles and pipeline builds a model part needs, made off the window thread.
//!
//! A frame that finds a part unbuilt queues its build and draws without the actor; a later frame
//! installs the finished pass. A settled scene (a capture) builds in place instead.
use super::*;
use job_priority::SpawnCarried as _;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// Mesh, node, LOD, part and caster/forced variant.
pub(super) type PartKey = (usize, usize, usize, usize, usize);
type NodeKey = (usize, usize);
type Failure = (io::ErrorKind, String);

/// What a part's resident is made from: the lit and shadow selections that decide its material
/// flags, and the pass built for its draw selection.
pub(super) struct Built {
    pub lit: Selection,
    pub shadow: Selection,
    pub pass: mesh_shader::Pass,
}

enum Request {
    Flags {
        key: NodeKey,
        library: usize,
        parts: Vec<Part>,
    },
    Part {
        key: PartKey,
        library: usize,
        part: Part,
        caster: bool,
        forced: bool,
        samples: u32,
    },
}

#[derive(Default)]
struct State {
    /// `None` while a build holds the request.
    flags: HashMap<NodeKey, Option<Result<[bool; 2], Failure>>>,
    parts: HashMap<PartKey, Option<Result<Built, Failure>>>,
}

/// The renderer's effect libraries and pipeline cache, shared with its background builds.
pub(super) struct Builds {
    libraries: Arc<Mutex<[Library; 2]>>,
    cache: Arc<Mutex<shader_pipeline::gpu::Cache>>,
    state: Arc<Mutex<State>>,
}

/// One prepare's queued builds, started together when it ends, whichever way it ends.
pub(super) struct Queued {
    requests: Vec<Request>,
    libraries: Arc<Mutex<[Library; 2]>>,
    cache: Arc<Mutex<shader_pipeline::gpu::Cache>>,
    state: Arc<Mutex<State>>,
    device: wgpu::Device,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

fn failure(error: io::Error) -> Failure {
    (error.kind(), error.to_string())
}

fn restore((kind, message): Failure) -> io::Error {
    io::Error::new(kind, message)
}

impl Builds {
    pub fn new(libraries: [Library; 2]) -> Self {
        Self {
            libraries: Arc::new(Mutex::new(libraries)),
            cache: Default::default(),
            state: Default::default(),
        }
    }

    pub fn queue(&self, device: &wgpu::Device) -> Queued {
        Queued {
            requests: Vec::new(),
            libraries: Arc::clone(&self.libraries),
            cache: Arc::clone(&self.cache),
            state: Arc::clone(&self.state),
            device: device.clone(),
        }
    }

    /// A node's queue flags: built, or `None` while a build is under way (started here unless
    /// `settled`, which builds them in place).
    pub fn flags(
        &self,
        queued: &mut Queued,
        key: NodeKey,
        library: usize,
        parts: impl FnOnce() -> Vec<Part>,
        settled: bool,
    ) -> Option<io::Result<[bool; 2]>> {
        let mut state = lock(&self.state);
        match state.flags.get(&key) {
            // A refusal stays, so every later draw meets it rather than building it again.
            Some(Some(Err(failure))) => return Some(Err(restore(failure.clone()))),
            Some(Some(Ok(_))) => {
                return state
                    .flags
                    .remove(&key)
                    .flatten()
                    .map(|r| r.map_err(restore));
            }
            // A capture builds what a worker still holds, rather than draw without it.
            Some(None) if !settled => return None,
            _ => {}
        }
        if settled {
            drop(state);
            return Some(flags(&self.libraries, library, &parts()));
        }
        state.flags.insert(key, None);
        queued.requests.push(Request::Flags {
            key,
            library,
            parts: parts(),
        });
        None
    }

    /// Whether `key`'s part is built (or failed), queueing its build when it is not.
    #[allow(clippy::too_many_arguments)]
    pub fn ready(
        &self,
        queued: &mut Queued,
        key: PartKey,
        library: usize,
        part: &Part,
        (caster, forced): (bool, bool),
        samples: u32,
        settled: bool,
    ) -> bool {
        let mut state = lock(&self.state);
        match state.parts.get(&key) {
            Some(Some(_)) => return true,
            // A capture builds what a worker still holds, rather than draw without it.
            Some(None) if !settled => return false,
            _ => {}
        }
        if settled {
            drop(state);
            let built = build(
                (&self.libraries, &self.cache, &queued.device),
                library,
                part,
                (caster, forced),
                samples,
            );
            lock(&self.state)
                .parts
                .insert(key, Some(built.map_err(failure)));
            return true;
        }
        state.parts.insert(key, None);
        queued.requests.push(Request::Part {
            key,
            library,
            part: part.clone(),
            caster,
            forced,
            samples,
        });
        false
    }

    /// Makes a node's queue flags and each of its parts' lit (alpha when `forced`) and caster
    /// builds, under the keys a frame looks up, so a frame first drawing it finds them made. A
    /// refusal is kept for the frame that draws it to report.
    pub fn warm(
        &self,
        device: &wgpu::Device,
        (mesh, node): NodeKey,
        library: usize,
        lods: &[Vec<Part>],
        (forced, samples): (bool, u32),
    ) {
        let shared = (&*self.libraries, &*self.cache, device);
        if !lock(&self.state).flags.contains_key(&(mesh, node)) {
            let parts: Vec<_> = lods.iter().flatten().cloned().collect();
            let admitted = flags(&self.libraries, library, &parts).map_err(failure);
            lock(&self.state)
                .flags
                .entry((mesh, node))
                .or_insert(Some(admitted));
        }
        for (lod, sources) in lods.iter().enumerate() {
            for (part, source) in sources.iter().enumerate() {
                for caster in [false, true] {
                    let key = (
                        mesh,
                        node,
                        lod,
                        part,
                        usize::from(caster) * 2 + usize::from(forced),
                    );
                    if lock(&self.state).parts.contains_key(&key) {
                        continue;
                    }
                    let built =
                        build(shared, library, source, (caster, forced), samples).map_err(failure);
                    lock(&self.state).parts.entry(key).or_insert(Some(built));
                }
            }
        }
    }

    /// Refuses, by `reason`, a node's flags and every part variant [`Self::warm`] left unmade.
    pub fn refuse(&self, (mesh, node): NodeKey, lods: &[Vec<Part>], forced: bool, reason: &str) {
        let refusal = || (io::ErrorKind::Other, reason.to_string());
        let mut state = lock(&self.state);
        state
            .flags
            .entry((mesh, node))
            .or_insert_with(|| Some(Err(refusal())));
        for (lod, sources) in lods.iter().enumerate() {
            for part in 0..sources.len() {
                for caster in [false, true] {
                    let key = (
                        mesh,
                        node,
                        lod,
                        part,
                        usize::from(caster) * 2 + usize::from(forced),
                    );
                    state
                        .parts
                        .entry(key)
                        .or_insert_with(|| Some(Err(refusal())));
                }
            }
        }
    }

    /// The finished build of a part [`Self::ready`] reported built. A refusal stays, so every
    /// later draw of the part meets it rather than building it again.
    pub fn take(&self, key: &PartKey) -> io::Result<Built> {
        let mut state = lock(&self.state);
        if let Some(Some(Err(failure))) = state.parts.get(key) {
            return Err(restore(failure.clone()));
        }
        state
            .parts
            .remove(key)
            .flatten()
            .ok_or_else(|| invalid("native part build is not finished"))?
            .map_err(restore)
    }
}

impl Drop for Queued {
    fn drop(&mut self) {
        if self.requests.is_empty() {
            return;
        }
        let threads = self.requests.len().min(4);
        let requests = Arc::new(Mutex::new(VecDeque::from(std::mem::take(
            &mut self.requests,
        ))));
        let mut started = 0;
        let mut refused = None;
        for _ in 0..threads {
            let requests = Arc::clone(&requests);
            let shared = (
                Arc::clone(&self.libraries),
                Arc::clone(&self.cache),
                Arc::clone(&self.state),
                self.device.clone(),
            );
            let work = move || {
                let (libraries, cache, state, device) = shared;
                while let Some(request) = lock(&requests).pop_front() {
                    run(request, (&libraries, &cache, &device), &state);
                }
            };
            match std::thread::Builder::new()
                .name("model builds".into())
                .spawn_carried(work)
            {
                Ok(_) => started += 1,
                Err(error) => refused = Some(error),
            }
        }
        if started > 0 {
            return;
        }
        // With no thread to build on, its parts are refused by name, never built here.
        let reason = format!(
            "model builds could not start a thread: {}",
            refused.map_or_else(String::new, |error| error.to_string())
        );
        let mut state = lock(&self.state);
        for request in lock(&requests).drain(..) {
            let refusal = (io::ErrorKind::Other, reason.clone());
            match request {
                Request::Flags { key, .. } => {
                    state.flags.insert(key, Some(Err(refusal)));
                }
                Request::Part { key, .. } => {
                    state.parts.insert(key, Some(Err(refusal)));
                }
            }
        }
    }
}

type Shared<'a> = (
    &'a Mutex<[Library; 2]>,
    &'a Mutex<shader_pipeline::gpu::Cache>,
    &'a wgpu::Device,
);

/// A build that panicked is refused by name, so its part never waits for it.
fn caught<T>(build: impl FnOnce() -> Result<T, Failure>) -> Result<T, Failure> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(build))
        .unwrap_or_else(|_| Err((io::ErrorKind::Other, "a model build panicked".to_string())))
}

fn run(request: Request, shared: Shared<'_>, state: &Mutex<State>) {
    match request {
        Request::Flags {
            key,
            library,
            parts,
        } => {
            let built = caught(|| flags(shared.0, library, &parts).map_err(failure));
            if let Some(slot @ None) = lock(state).flags.get_mut(&key) {
                *slot = Some(built);
            }
        }
        Request::Part {
            key,
            library,
            part,
            caster,
            forced,
            samples,
        } => {
            let built = caught(|| {
                build(shared, library, &part, (caster, forced), samples).map_err(failure)
            });
            if let Some(slot @ None) = lock(state).parts.get_mut(&key) {
                *slot = Some(built);
            }
        }
    }
}

/// The part's technique for the lit or shadow pass, its material's when the effect declares it,
/// else the plain colour fallback. The library is locked only to load effects, never to compile.
fn select_shared(
    libraries: &Mutex<[Library; 2]>,
    library: usize,
    part: &Part,
    caster: bool,
) -> io::Result<Selection> {
    let effect = material_effect(&mut lock(libraries)[library], part)?;
    let base = part.material.shader.as_deref().unwrap_or_default();
    let suffix = selection_suffix(!part.geometry.skin.is_empty(), caster);
    if !base.is_empty()
        && let Some(selection) = effect.select(&format!("{base}{suffix}")).map_err(invalid)?
    {
        return Ok(selection);
    }
    let fallback = lock(libraries)[library]
        .load(b"gfx/FX/pdxmesh.lua", &[])
        .map_err(invalid)?;
    fallback
        .select(&format!("PdxMeshColor{suffix}"))
        .map_err(invalid)?
        .ok_or_else(|| invalid("native PdxMeshColor fallback is unavailable"))
}

/// The game appends Alpha to the skin suffix; absent Alpha keeps the base variant (not
/// PdxMeshColor). Draws use this handle as the alpha variant.
fn select_alpha_shared(
    libraries: &Mutex<[Library; 2]>,
    library: usize,
    part: &Part,
) -> io::Result<Selection> {
    let base = select_shared(libraries, library, part, false)?;
    let effect = material_effect(&mut lock(libraries)[library], part)?;
    let name = part.material.shader.as_deref().unwrap_or_default();
    let skin = if part.geometry.skin.is_empty() {
        ""
    } else {
        "Skinned"
    };
    Ok(effect
        .select(&format!("{name}{skin}Alpha"))
        .map_err(invalid)?
        .unwrap_or(base))
}

fn blend(selection: &Selection) -> installed_content_core::pdxshader::BlendState {
    selection
        .effect
        .source
        .blend(&selection.effect.source.techniques[selection.index])
}

fn flags(libraries: &Mutex<[Library; 2]>, library: usize, parts: &[Part]) -> io::Result<[bool; 2]> {
    let mut admitted = [false; 2];
    for part in parts {
        let lit = select_shared(libraries, library, part, false)?;
        let shadow = select_shared(libraries, library, part, true)?;
        let (_, alternate) = material_flags(
            &blend(&lit),
            &blend(&shadow),
            !part.material.diffuse.is_empty(),
        );
        admitted[usize::from(alternate)] = true;
    }
    Ok(admitted)
}

fn build(
    (libraries, cache, device): Shared<'_>,
    library: usize,
    part: &Part,
    (caster, forced): (bool, bool),
    samples: u32,
) -> io::Result<Built> {
    let selection = if forced && !caster {
        select_alpha_shared(libraries, library, part)?
    } else {
        select_shared(libraries, library, part, caster)?
    };
    let lit = select_shared(libraries, library, part, false)?;
    let shadow = select_shared(libraries, library, part, true)?;
    let mut cache = lock(cache);
    let pass = if caster {
        mesh_shader::Pass::caster(
            &mut cache,
            device,
            &selection,
            &part.geometry,
            DEPTH_MAP_FORMAT,
            DEPTH_FORMAT,
        )?
    } else {
        mesh_shader::Pass::new(
            &mut cache,
            device,
            &selection,
            &part.geometry,
            SCENE_FORMAT,
            DEPTH_FORMAT,
            samples,
        )?
    };
    Ok(Built { lit, shadow, pass })
}
