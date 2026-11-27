use crate::native::*;
use job_priority::SpawnCarried as _;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

pub(super) type PartKey = (usize, usize, usize, usize, usize);
type NodeKey = (usize, usize);
type Failure = (io::ErrorKind, String);

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
        instanced: u32,
    },
}

#[derive(Default)]
struct State {
    flags: HashMap<NodeKey, Option<Result<[bool; 2], Failure>>>,
    parts: HashMap<PartKey, Option<Result<Built, Failure>>>,
}

fn part_keys(
    (mesh, node): NodeKey,
    lods: &[Vec<Part>],
    forced: bool,
) -> impl Iterator<Item = (PartKey, usize, usize, bool)> + '_ {
    lods.iter().enumerate().flat_map(move |(lod, sources)| {
        (0..sources.len()).flat_map(move |part| {
            [false, true].map(|caster| {
                let variant = usize::from(caster) * 2 + usize::from(forced);
                ((mesh, node, lod, part, variant), lod, part, caster)
            })
        })
    })
}

fn fill<K: std::hash::Hash + Eq, T>(slots: &mut HashMap<K, Option<T>>, key: K, value: T) {
    if let Some(slot @ None) = slots.get_mut(&key) {
        *slot = Some(value);
    }
}

pub(super) struct Builds {
    libraries: Arc<Mutex<[Library; 2]>>,
    cache: Arc<Mutex<shader_pipeline::gpu::Cache>>,
    state: Arc<Mutex<State>>,
}

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
            Some(Some(Err(failure))) => return Some(Err(restore(failure.clone()))),
            Some(Some(Ok(_))) => {
                return state
                    .flags
                    .remove(&key)
                    .flatten()
                    .map(|r| r.map_err(restore));
            }
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

    #[expect(clippy::too_many_arguments, reason = "the queue, the part's key and library, and the pass's caster, sample and instancing modes")]
    pub fn ready(
        &self,
        queued: &mut Queued,
        key: PartKey,
        library: usize,
        part: &Part,
        (caster, forced): (bool, bool),
        (samples, instanced): (u32, u32),
        settled: bool,
    ) -> bool {
        let mut state = lock(&self.state);
        match state.parts.get(&key) {
            Some(Some(_)) => return true,
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
                (samples, instanced),
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
            instanced,
        });
        false
    }

    pub fn warm(
        &self,
        device: &wgpu::Device,
        (mesh, node): NodeKey,
        library: usize,
        lods: &[Vec<Part>],
        (forced, samples): (bool, u32),
        instanced: [u32; 2],
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
                    let built = build(
                        shared,
                        library,
                        source,
                        (caster, forced),
                        (samples, instanced[usize::from(caster)]),
                    )
                    .map_err(failure);
                    lock(&self.state).parts.entry(key).or_insert(Some(built));
                }
            }
        }
    }

    pub fn warm_behind(
        &self,
        device: &wgpu::Device,
        nodes: Vec<(NodeKey, usize, Vec<Vec<Part>>)>,
        (forced, samples): (bool, u32),
        instanced: [u32; 2],
    ) {
        if nodes.is_empty() {
            return;
        }
        let mut marked = Vec::new();
        {
            let mut state = lock(&self.state);
            for (key, _, lods) in &nodes {
                if !state.flags.contains_key(key) {
                    state.flags.insert(*key, None);
                    marked.push((*key, None));
                }
                for (part, ..) in part_keys(*key, lods, forced) {
                    if let std::collections::hash_map::Entry::Vacant(e) = state.parts.entry(part) {
                        e.insert(None);
                        marked.push((*key, Some(part)));
                    }
                }
            }
        }
        let shared = (
            Arc::clone(&self.libraries),
            Arc::clone(&self.cache),
            Arc::clone(&self.state),
            device.clone(),
        );
        let work = move || {
            viewer_runtime::worker_priority::enter(viewer_runtime::worker_priority::Class::Normal);
            let (libraries, cache, state, device) = shared;
            for (key, library, lods) in nodes {
                if lock(&state).flags.get(&key).is_some_and(Option::is_none) {
                    let parts: Vec<_> = lods.iter().flatten().cloned().collect();
                    let admitted = caught(|| flags(&libraries, library, &parts).map_err(failure));
                    fill(&mut lock(&state).flags, key, admitted);
                }
                for (part_key, lod, part, caster) in part_keys(key, &lods, forced) {
                    if !lock(&state).parts.get(&part_key).is_some_and(Option::is_none) {
                        continue;
                    }
                    let source = &lods[lod][part];
                    let built = caught(|| {
                        build(
                            (&libraries, &cache, &device),
                            library,
                            source,
                            (caster, forced),
                            (samples, instanced[usize::from(caster)]),
                        )
                        .map_err(failure)
                    });
                    fill(&mut lock(&state).parts, part_key, built);
                }
            }
        };
        if let Err(error) = std::thread::Builder::new()
            .name("model warm-up".into())
            .spawn_carried(work)
        {
            let refusal = (
                io::ErrorKind::Other,
                format!("model builds could not start a thread: {error}"),
            );
            let mut state = lock(&self.state);
            for (node, part) in marked {
                match part {
                    None => fill(&mut state.flags, node, Err(refusal.clone())),
                    Some(part) => fill(&mut state.parts, part, Err(refusal.clone())),
                }
            }
        }
    }

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
                viewer_runtime::worker_priority::enter(viewer_runtime::worker_priority::Class::Normal);
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
            instanced,
        } => {
            let built = caught(|| {
                build(shared, library, &part, (caster, forced), (samples, instanced))
                    .map_err(failure)
            });
            if let Some(slot @ None) = lock(state).parts.get_mut(&key) {
                *slot = Some(built);
            }
        }
    }
}

fn select_shared(
    libraries: &Mutex<[Library; 2]>,
    library: usize,
    part: &Part,
    caster: bool,
) -> io::Result<Selection> {
    let effect = material_effect(&mut lock(libraries)[library], part)?;
    let base = part.material.shader.as_deref().unwrap_or_default();
    let suffix = selection_suffix(part.geometry.format.skinned, caster);
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

fn select_alpha_shared(
    libraries: &Mutex<[Library; 2]>,
    library: usize,
    part: &Part,
) -> io::Result<Selection> {
    let base = select_shared(libraries, library, part, false)?;
    let effect = material_effect(&mut lock(libraries)[library], part)?;
    let name = part.material.shader.as_deref().unwrap_or_default();
    let skin = if !part.geometry.format.skinned {
        ""
    } else {
        "Skinned"
    };
    Ok(effect
        .select(&format!("{name}{skin}Alpha"))
        .map_err(invalid)?
        .unwrap_or(base))
}

fn blend(selection: &Selection) -> content_reader_gui::pdxshader::BlendState {
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
    (samples, instanced): (u32, u32),
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
            part.geometry.format,
            DEPTH_MAP_FORMAT,
            DEPTH_FORMAT,
            instanced,
        )?
    } else {
        mesh_shader::Pass::new(
            &mut cache,
            device,
            &selection,
            part.geometry.format,
            SCENE_FORMAT,
            DEPTH_FORMAT,
            samples,
            instanced,
        )?
    };
    Ok(Built { lit, shadow, pass })
}
