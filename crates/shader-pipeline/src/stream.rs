
use super::{
    binding, gpu,
    library::Selection,
    vertices::{Declaration, Vertices},
};
use content_reader_layers::content::diagnostics::{self, Diagnostics};
use content_reader_gui::pdxshader::{Address, DepthStencilState, RasterizerState, program::{Filter, Sampler, Stage}};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

type TextureMap = BTreeMap<u32, Option<gpu::TextureBinding>>;

fn same_snapshot<T: PartialEq + ?Sized>(a: &Arc<T>, b: &Arc<T>) -> bool {
    Arc::ptr_eq(a, b) || **a == **b
}

pub enum WarmPass {
    Selected,
    Recorded(String),
}

#[derive(Clone)]
pub struct Binds {
    program: Option<(Selection, Layout)>,
    vertex_strides: Arc<BTreeMap<u32, u64>>,
    state: gpu::State,
    sampler_keys: Arc<BTreeMap<u32, [u32; 10]>>,
    depth_stencil: Option<DepthStencilState>,
    rasterizer: Option<RasterizerState>,
    constants: BTreeMap<u32, Vec<u8>>,
    known_constants: BTreeMap<u32, Vec<std::ops::Range<u32>>>,
    textures: TextureMap,
    texture_snapshot: Option<Arc<TextureMap>>,
}

pub struct Stream {
    program: Option<(Selection, Layout)>,
    vertex_strides: Arc<BTreeMap<u32, u64>>,
    state: gpu::State,
    sampler_keys: Arc<BTreeMap<u32, [u32; 10]>>,
    depth_stencil: Option<DepthStencilState>,
    rasterizer: Option<RasterizerState>,
    constants: BTreeMap<u32, Vec<u8>>,
    known_constants: BTreeMap<u32, Vec<std::ops::Range<u32>>>,
    textures: TextureMap,
    texture_snapshot: Option<Arc<TextureMap>>,
    null_textures: BTreeMap<super::TextureKind, gpu::TextureBinding>,
    last_pass: Option<ResolvedPass>,
    pub cache: gpu::Cache,
    pub refused: BTreeMap<String, u64>,
    builds: Option<Builds>,
    waits: u32,
}

type Target = (wgpu::TextureFormat, u32, wgpu::PrimitiveTopology);

struct Build {
    gui: bool,
    selection: Selection,
    state: gpu::State,
    target: Target,
    made: Option<(gpu::Cached, Diagnostics)>,
}

impl Build {
    fn is(&self, gui: bool, selection: &Selection, state: &gpu::State, target: Target) -> bool {
        if self.gui != gui
            || self.selection.effect.id != selection.effect.id
            || self.selection.index != selection.index
            || self.target != target
        {
            return false;
        }
        let compiled = selection
            .effect
            .compiled_now(selection.index)
            .and_then(Result::ok);
        gpu::same_packed(&self.state, state, compiled.as_deref())
    }
}

#[derive(Clone, Default)]
pub struct Builds(Arc<Mutex<Vec<Build>>>);

fn lock(builds: &Mutex<Vec<Build>>) -> std::sync::MutexGuard<'_, Vec<Build>> {
    builds.lock().unwrap_or_else(|error| error.into_inner())
}

impl Builds {
    fn take_or_start(
        &self,
        gui: bool,
        device: &wgpu::Device,
        selection: &Selection,
        state: &gpu::State,
        target: Target,
    ) -> Option<(gpu::Cached, Diagnostics)> {
        let mut builds = lock(&self.0);
        if let Some(at) = builds
            .iter()
            .position(|build| build.is(gui, selection, state, target))
        {
            return match builds[at].made {
                Some(_) => builds.swap_remove(at).made,
                None => None,
            };
        }
        builds.push(Build {
            gui,
            selection: selection.clone(),
            state: state.clone(),
            target,
            made: None,
        });
        drop(builds);
        let build = {
            let (shared, device, selection, state) = (
                Arc::clone(&self.0),
                device.clone(),
                selection.clone(),
                state.clone(),
            );
            let building = gpu::Building::start();
            move || {
                let _building = building;
                let (format, samples, topology) = target;
                let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut cache = gpu::Cache::default();
                    cache.gui = gui;
                    cache.build_packed(&device, &selection, &state, format, samples, topology)
                }))
                .unwrap_or_else(|_| {
                    (
                        Err("a GUI pipeline build panicked".into()),
                        Diagnostics::default(),
                    )
                });
                if let Some(build) = lock(&shared)
                    .iter_mut()
                    .find(|build| build.is(gui, &selection, &state, target))
                {
                    build.made = Some(made);
                }
            }
        };
        match std::thread::Builder::new()
            .name("GUI pipeline builds".into())
            .spawn(job_priority::carry(build))
        {
            Ok(_) => None,
            Err(error) => {
                lock(&self.0).retain(|build| !build.is(gui, selection, state, target));
                Some((
                    Err(format!(
                        "GUI pipeline builds could not start a thread: {error}"
                    )),
                    Diagnostics::default(),
                ))
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
enum Layout {
    Packed(Declaration),
    Native(Arc<[gpu::VertexLayout]>),
}

impl Layout {
    fn matches(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Native(a), Self::Native(b)) => same_snapshot(a, b),
            _ => self == other,
        }
    }
}

struct PassInputs {
    effect: uuid::Uuid,
    technique: usize,
    layout: Layout,
    strides: Arc<BTreeMap<u32, u64>>,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
    samplers: Arc<BTreeMap<u32, [u32; 10]>>,
    depth: Option<DepthStencilState>,
    raster: Option<[u32; 8]>,
    template: gpu::Pass,
    vertices: Option<Vec<gpu::VertexLayout>>,
}

fn raster_key(state: &RasterizerState) -> [u32; 8] {
    [
        state.fill_mode,
        state.cull_mode,
        u32::from(state.front_ccw),
        state.depth_bias as u32,
        state.depth_bias_clamp.to_bits(),
        state.slope_scaled_depth_bias.to_bits(),
        u32::from(state.scissor),
        u32::from(state.multisample),
    ]
}

impl PassInputs {
    fn new(stream: &Stream, template: &gpu::Pass, vertices: Option<&[gpu::VertexLayout]>) -> Self {
        let (selection, layout) = stream.program.as_ref().expect("selected pass");
        Self {
            effect: selection.effect.id,
            technique: selection.index,
            layout: layout.clone(),
            strides: stream.vertex_strides.clone(),
            blend: stream.state.blend,
            write_mask: stream.state.write_mask,
            samplers: stream.sampler_keys.clone(),
            depth: stream.depth_stencil.clone(),
            raster: stream.rasterizer.as_ref().map(raster_key),
            template: template.clone(),
            vertices: vertices.map(<[_]>::to_vec),
        }
    }

    fn matches(
        &self,
        stream: &Stream,
        template: &gpu::Pass,
        vertices: Option<&[gpu::VertexLayout]>,
    ) -> bool {
        let Some((selection, layout)) = &stream.program else {
            return false;
        };
        self.effect == selection.effect.id
            && self.technique == selection.index
            && self.layout.matches(layout)
            && same_snapshot(&self.strides, &stream.vertex_strides)
            && self.blend == stream.state.blend
            && self.write_mask == stream.state.write_mask
            && same_snapshot(&self.samplers, &stream.sampler_keys)
            && self.depth == stream.depth_stencil
            && self.raster == stream.rasterizer.as_ref().map(raster_key)
            && self.template == *template
            && self.vertices.as_deref() == vertices
    }
}

type PipelineResolution = (Arc<gpu::Pipeline>, Selection, Option<u32>, Option<bool>);

struct ResolvedPass {
    inputs: PassInputs,
    pipeline: Arc<gpu::Pipeline>,
    selection: Selection,
    stencil_reference: Option<u32>,
    scissor_enabled: Option<bool>,
    diagnostics: Diagnostics,
}

impl Default for Stream {
    fn default() -> Self {
        Self {
            program: None,
            vertex_strides: Arc::default(),
            state: gpu::State {
                samplers: BTreeMap::new(),
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            },
            sampler_keys: Arc::default(),
            depth_stencil: None,
            rasterizer: None,
            constants: BTreeMap::new(),
            known_constants: BTreeMap::new(),
            textures: BTreeMap::new(),
            texture_snapshot: None,
            null_textures: BTreeMap::new(),
            last_pass: None,
            cache: gpu::Cache::default(),
            refused: BTreeMap::new(),
            builds: None,
            waits: 0,
        }
    }
}

pub struct Submission {
    pub pipeline: Arc<gpu::Pipeline>,
    pub draw: gpu::Draw,
}

pub struct PassSubmission {
    pub pipeline: Arc<gpu::Pipeline>,
    pub bindings: gpu::Bindings,
    pub stencil_reference: Option<u32>,
    pub scissor_enabled: Option<bool>,
}

pub struct NativePrepared<'a> {
    pub layouts: &'a [gpu::VertexLayout],
    pub vertices: &'a [gpu::Slice],
}

struct NativeBindings {
    textures: Arc<TextureMap>,
    program: gpu::BindingProgram,
    used: u64,
}

impl NativeBindings {
    fn matches(&self, stream: &Stream) -> bool {
        stream
            .texture_snapshot
            .as_ref()
            .is_some_and(|snapshot| Arc::ptr_eq(snapshot, &self.textures))
            || *self.textures == stream.textures
    }
}

#[derive(Default)]
pub struct NativePassCache {
    resolved: Option<ResolvedPass>,
    bindings: Vec<NativeBindings>,
    frame: u64,
    pending: Option<NativeBindings>,
}

impl NativePassCache {
    #[cfg(any(test, feature = "test-support"))]
    pub fn binding_textures(&self) -> impl Iterator<Item = &Arc<TextureMap>> {
        self.bindings.iter().map(|entry| &entry.textures)
    }
}

pub struct NativeSubmission {
    pub pass: PassSubmission,
    pub vertices: Vec<gpu::Slice>,
}

impl Stream {
    pub fn gui() -> Self {
        let mut stream = Self::default();
        stream.cache.gui = true;
        stream
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn same_bindings(&self, other: &Self) -> bool {
        let program = match (&self.program, &other.program) {
            (None, None) => true,
            (Some((a, al)), Some((b, bl))) => {
                a.effect.id == b.effect.id && a.index == b.index && al == bl
            }
            _ => false,
        };
        program && self.same_state(other) && self.textures == other.textures
    }

    #[cfg(any(test, feature = "test-support"))]
    fn same_state(&self, other: &Self) -> bool {
        self.vertex_strides == other.vertex_strides
            && self.state.blend == other.state.blend
            && self.state.write_mask == other.state.write_mask
            && self
                .state
                .samplers
                .iter()
                .map(|(k, s)| (*k, gpu::sampler_key(s)))
                .eq(other
                    .state
                    .samplers
                    .iter()
                    .map(|(k, s)| (*k, gpu::sampler_key(s))))
            && self.depth_stencil == other.depth_stencil
            && self.rasterizer.as_ref().map(raster_key) == other.rasterizer.as_ref().map(raster_key)
            && self.constants == other.constants
            && self.known_constants == other.known_constants
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn same_bindings_mapped_for_tests(
        &self,
        other: &Self,
        mut same_texture: impl FnMut(&wgpu::TextureView, &wgpu::TextureView) -> bool,
    ) -> bool {
        let program = match (&self.program, &other.program) {
            (None, None) => true,
            (Some((a, al)), Some((b, bl))) => {
                a.effect.source == b.effect.source
                    && a.effect.defines == b.effect.defines
                    && a.index == b.index
                    && al == bl
            }
            _ => false,
        };
        program
            && self.same_state(other)
            && self.textures.len() == other.textures.len()
            && self
                .textures
                .iter()
                .zip(&other.textures)
                .all(|((a_key, a), (b_key, b))| {
                    a_key == b_key
                        && match (a, b) {
                            (Some(a), Some(b)) => a.format == b.format && same_texture(a, b),
                            (None, None) => true,
                            _ => false,
                        }
                })
    }

    pub fn binds(&self) -> Binds {
        Binds {
            program: self.program.clone(),
            vertex_strides: self.vertex_strides.clone(),
            state: self.state.clone(),
            sampler_keys: self.sampler_keys.clone(),
            depth_stencil: self.depth_stencil.clone(),
            rasterizer: self.rasterizer.clone(),
            constants: self.constants.clone(),
            known_constants: self.known_constants.clone(),
            textures: self.textures.clone(),
            texture_snapshot: self.texture_snapshot.clone(),
        }
    }

    pub fn restore(&mut self, binds: &Binds) {
        self.program.clone_from(&binds.program);
        self.vertex_strides.clone_from(&binds.vertex_strides);
        self.state.clone_from(&binds.state);
        self.sampler_keys.clone_from(&binds.sampler_keys);
        self.depth_stencil.clone_from(&binds.depth_stencil);
        self.rasterizer.clone_from(&binds.rasterizer);
        self.constants.clone_from(&binds.constants);
        self.known_constants.clone_from(&binds.known_constants);
        self.textures.clone_from(&binds.textures);
        self.texture_snapshot.clone_from(&binds.texture_snapshot);
    }

    pub fn inherit_bindings(&mut self, caller: &Self) {
        self.program = caller.program.clone();
        self.vertex_strides = caller.vertex_strides.clone();
        self.state = caller.state.clone();
        self.sampler_keys = caller.sampler_keys.clone();
        self.depth_stencil = caller.depth_stencil.clone();
        self.rasterizer = caller.rasterizer.clone();
        self.constants = caller.constants.clone();
        self.known_constants = caller.known_constants.clone();
        self.textures = caller.textures.clone();
        self.texture_snapshot = caller.texture_snapshot.clone();
    }

    pub fn bind_blend(&mut self, blend: Option<wgpu::BlendState>, write_mask: wgpu::ColorWrites) {
        self.state.blend = blend;
        self.state.write_mask = write_mask;
    }

    pub fn blend(&self) -> (Option<wgpu::BlendState>, wgpu::ColorWrites) {
        (self.state.blend, self.state.write_mask)
    }

    pub fn accumulate_alpha(&mut self) {
        if let Some(blend) = &mut self.state.blend {
            blend.alpha = wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            };
        }
        self.state.write_mask = wgpu::ColorWrites::ALL;
    }

    pub fn bind_program(&mut self, selection: Option<Selection>, vertices: Vec<gpu::VertexLayout>) {
        if let Some(selection) = selection {
            self.program = Some((selection, Layout::Native(vertices.into())));
        }
    }

    pub fn select_program(&mut self, selection: &Selection, vertices: &[gpu::VertexLayout]) {
        if self.program.as_ref().is_some_and(|(bound, layout)| {
            bound.effect.id == selection.effect.id
                && bound.index == selection.index
                && matches!(layout, Layout::Native(bound) if bound.as_ref() == vertices)
        }) {
            return;
        }
        self.bind_program(Some(selection.clone()), vertices.to_vec());
    }

    pub fn bind_vertex_stride(&mut self, slot: u32, stride: u64) {
        if self.vertex_strides.get(&slot) != Some(&stride) {
            Arc::make_mut(&mut self.vertex_strides).insert(slot, stride);
        }
    }

    pub fn vertex_layouts(&self) -> Result<Vec<gpu::VertexLayout>, String> {
        let (_, layout) = self.program.as_ref().ok_or(
            "null technique needs the preceding native program, which has not been supplied",
        )?;
        let Layout::Native(vertices) = layout else {
            return Err("packed vertex decoding requires prepare".into());
        };
        let mut vertices = vertices.to_vec();
        for (slot, vertex) in vertices.iter_mut().enumerate() {
            if let Some(stride) = self.vertex_strides.get(&(slot as u32)) {
                vertex.stride = *stride;
            }
        }
        Ok(vertices)
    }

    pub fn bind_depth_stencil(&mut self, state: DepthStencilState) {
        self.depth_stencil = Some(state);
    }

    pub fn bind_rasterizer(&mut self, state: RasterizerState) {
        self.rasterizer = Some(state);
    }

    pub fn begin_frame(&mut self) {
        *self = Self {
            cache: std::mem::take(&mut self.cache),
            null_textures: std::mem::take(&mut self.null_textures),
            builds: self.builds.take(),
            ..Default::default()
        };
    }

    pub fn warm_pass(
        &mut self,
        device: &wgpu::Device,
        template: &gpu::Pass,
        vertices: Vec<gpu::VertexLayout>,
    ) -> Result<WarmPass, String> {
        let selection = &self
            .program
            .as_ref()
            .ok_or("no selected warm-up program")?
            .0;
        let (mut pass, _, _) = self.pass_state(template);
        pass.vertices = vertices;
        match self.cache.get_pass(device, selection, &self.state, &pass) {
            Ok(_) => Ok(WarmPass::Selected),
            Err(why) => {
                let recorded = selection.recorded().ok_or_else(|| why.clone())?;
                self.cache
                    .get_pass(device, &recorded, &self.state, &pass)
                    .map(|_| WarmPass::Recorded(why))
            }
        }
    }

    pub fn invalidate_bindings(&mut self) {
        self.program = None;
        self.vertex_strides = Arc::default();
        self.depth_stencil = None;
        self.rasterizer = None;
        self.state.samplers.clear();
        self.sampler_keys = Arc::default();
        self.constants.clear();
        self.known_constants.clear();
        self.textures.clear();
        self.texture_snapshot = None;
    }

    pub fn bind_effect(
        &mut self,
        selection: Option<Selection>,
        declaration: Declaration,
    ) -> Result<(), String> {
        let Some(selection) = selection else {
            self.bind_blend(None, wgpu::ColorWrites::ALL);
            return Ok(());
        };
        let blend = |selection: &Selection| {
            gpu::blend(
                &selection.effect.source,
                &selection.effect.source.techniques[selection.index],
            )
        };
        let (selection, blend) = match blend(&selection) {
            Ok(state) => (selection, Ok(state)),
            Err(error) => match selection.fall_back(&error) {
                Some(recorded) => {
                    let state = blend(&recorded);
                    (recorded, state)
                }
                None => (selection, Err(error)),
            },
        };
        self.program = Some((selection, Layout::Packed(declaration)));
        let (blend, mask) = blend?;
        self.bind_blend(blend, mask);
        Ok(())
    }

    pub fn forget_program(&mut self) {
        self.program = None;
    }

    pub fn bind_constants(&mut self, slot: u32, stage: Stage, bytes: Option<&[u8]>) {
        if let Some(bytes) = bytes {
            self.write_constants(binding(slot, stage), bytes);
            self.known_constants.remove(&binding(slot, stage));
        }
    }

    fn write_constants(&mut self, key: u32, bytes: &[u8]) {
        let retained = self.constants.entry(key).or_default();
        retained.clear();
        retained.extend_from_slice(bytes);
    }

    pub fn bind_partial_constants(
        &mut self,
        slot: u32,
        stage: Stage,
        bytes: Option<&[u8]>,
        known: &[std::ops::Range<u32>],
    ) {
        if let Some(bytes) = bytes {
            self.write_constants(binding(slot, stage), bytes);
            let retained = self
                .known_constants
                .entry(binding(slot, stage))
                .or_default();
            retained.clear();
            retained.extend_from_slice(known);
        }
    }

    fn check_constants(&self, pipeline: &gpu::Pipeline) -> Result<(), String> {
        for (binding, known) in &self.known_constants {
            let physical = u32::try_from(self.constants[binding].len())
                .map_err(|_| "native constant allocation exceeds u32")?;
            pipeline.check_initialized(*binding, physical, known)?;
        }
        Ok(())
    }

    fn set_sampler(&mut self, key: u32, sampler: Sampler) {
        let bits = gpu::sampler_key(&sampler);
        if self.sampler_keys.get(&key) != Some(&bits) {
            Arc::make_mut(&mut self.sampler_keys).insert(key, bits);
        }
        self.state.samplers.insert(key, sampler);
    }

    pub fn bind_texture(
        &mut self,
        slot: u32,
        stage: Stage,
        texture: Option<wgpu::TextureView>,
        sampler: Option<&Sampler>,
    ) {
        self.bind_formatted_texture(slot, stage, texture.map(Into::into), sampler);
    }

    pub fn bind_formatted_texture(
        &mut self,
        slot: u32,
        stage: Stage,
        texture: Option<gpu::TextureBinding>,
        sampler: Option<&Sampler>,
    ) {
        let slot = sampler.map_or(slot, |sampler| sampler.index);
        let key = binding(slot, stage);
        if self.textures.get(&key) != Some(&texture) {
            self.textures.insert(key, texture);
            self.texture_snapshot = None;
        }
        if let Some(sampler) = sampler {
            if self.state.samplers.get(&key) != Some(sampler) {
                self.set_sampler(key, sampler.clone());
            }
        } else if stage == Stage::Vertex {
            self.set_sampler(
                key,
                Sampler {
                    name: "D3D11 null sampler".into(),
                    index: slot,
                    cube: false,
                    mag: Filter::Linear,
                    min: Filter::Linear,
                    mip: Filter::Linear,
                    u: Address::Clamp,
                    v: Address::Clamp,
                    w: Address::Clamp,
                    lod_bias: 0.,
                    min_lod: i32::MIN,
                    max_lod: i32::MAX,
                },
            );
        }
    }

    pub fn forget_texture(&mut self, slot: u32, stage: Stage, sampler: Option<&Sampler>) {
        self.bind_texture(slot, stage, None, sampler);
        let slot = sampler.map_or(slot, |sampler| sampler.index);
        if self.textures.remove(&binding(slot, stage)).is_some() {
            self.texture_snapshot = None;
        }
    }

    pub fn defer_builds(&mut self, builds: Option<Builds>) {
        self.builds = builds;
    }

    pub fn take_waiting(&mut self) -> bool {
        std::mem::take(&mut self.waits) != 0
    }

    fn packed(
        &mut self,
        device: &wgpu::Device,
        selection: &Selection,
        (format, samples, topology): Target,
    ) -> Option<gpu::Cached> {
        if let Some(found) =
            self.cache
                .find_packed(selection, &self.state, (format, samples, topology))
        {
            return Some(found);
        }
        if let Some(builds) = &self.builds
            && !self
                .cache
                .ready(device, selection, &self.state, format, samples, topology)
        {
            let made = builds.take_or_start(
                self.cache.gui,
                device,
                selection,
                &self.state,
                (format, samples, topology),
            )?;
            self.cache
                .adopt_packed(selection, &self.state, (format, samples, topology), made);
        }
        Some(
            self.cache
                .get(device, selection, &self.state, format, samples, topology),
        )
    }

    pub fn warm(
        &mut self,
        device: &wgpu::Device,
        selection: &Selection,
        (default_blend, premultiply): (bool, bool),
        (format, samples, topology): Target,
    ) {
        let blend = |selection: &Selection| {
            let source = &selection.effect.source;
            gpu::blend(source, &source.techniques[selection.index])
        };
        let (selection, (blend, mask)) = match blend(selection) {
            Ok(state) => (selection.clone(), state),
            Err(_) => match selection.recorded() {
                Some(recorded) => match blend(&recorded) {
                    Ok(state) => (recorded, state),
                    Err(_) => return,
                },
                None => return,
            },
        };
        let source = &selection.effect.source;
        let kept = self.blend();
        self.invalidate_bindings();
        if default_blend {
            self.bind_blend(None, wgpu::ColorWrites::ALL);
        } else {
            self.bind_blend(blend, mask);
        }
        if premultiply {
            self.accumulate_alpha();
        }
        for sampler in source.common.samplers.iter().chain(&source.pixel.samplers) {
            self.bind_texture(sampler.index, Stage::Pixel, None, Some(sampler));
        }
        for sampler in &source.vertex.samplers {
            self.bind_texture(sampler.index, Stage::Vertex, None, Some(sampler));
        }
        if self
            .cache
            .get(device, &selection, &self.state, format, samples, topology)
            .is_err()
            && let Some(recorded) = selection.recorded()
        {
            drop(
                self.cache
                    .get(device, &recorded, &self.state, format, samples, topology),
            );
        }
        self.invalidate_bindings();
        self.bind_blend(kept.0, kept.1);
    }

    fn wait(&mut self) -> String {
        self.waits += 1;
        "its pipeline is being built".into()
    }

    pub fn prepare(
        &mut self,
        arena: &gpu::Arena,
        label: &str,
        bytes: &[u8],
        stride: usize,
        count: usize,
        topology: wgpu::PrimitiveTopology,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Submission, String> {
        let waits = self.waits;
        let result = self
            .prepare_inner(arena, bytes, stride, count, topology, format, samples)
            .map_err(|reason| format!("native draw {label:?}: {reason}"));
        if let Err(reason) = &result
            && self.waits == waits
        {
            self.refuse(arena, label, reason);
        }
        result
    }

    fn prepare_inner(
        &mut self,
        arena: &gpu::Arena,
        bytes: &[u8],
        stride: usize,
        count: usize,
        topology: wgpu::PrimitiveTopology,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Submission, String> {
        match self.prepare_packet_inner(arena, bytes, stride, count, topology, format, samples)? {
            gpu::batch::Packet::Native { pipeline, draw } => Ok(Submission { pipeline, draw }),
            gpu::batch::Packet::Batched(_) => {
                Err("GUI batch packet requires its sprite pass".into())
            }
        }
    }

    pub fn prepare_packet(
        &mut self,
        arena: &gpu::Arena,
        label: &str,
        bytes: &[u8],
        stride: usize,
        count: usize,
        topology: wgpu::PrimitiveTopology,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<gpu::batch::Packet, String> {
        let waits = self.waits;
        let result = self
            .prepare_packet_inner(arena, bytes, stride, count, topology, format, samples)
            .map_err(|reason| format!("native draw {label:?}: {reason}"));
        if let Err(reason) = &result
            && self.waits == waits
        {
            self.refuse(arena, label, reason);
        }
        result
    }

    fn prepare_packet_inner(
        &mut self,
        arena: &gpu::Arena,
        bytes: &[u8],
        stride: usize,
        count: usize,
        topology: wgpu::PrimitiveTopology,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<gpu::batch::Packet, String> {
        let (mut selection, declaration) = self.program.clone().ok_or(
            "null technique needs the preceding native program, which has not been supplied",
        )?;
        let Layout::Packed(declaration) = declaration else {
            return Err("native vertex buffers require prepare_pass".into());
        };
        let device = arena.device();
        let target = (format, samples, topology);
        let pipeline = match self.packed(device, &selection, target) {
            None => return Err(self.wait()),
            Some(Ok(pipeline)) => pipeline,
            Some(Err(error)) => {
                let recorded = selection.fall_back(&error).ok_or(error)?;
                let Some(pipeline) = self.packed(device, &recorded, target) else {
                    return Err(self.wait());
                };
                selection = recorded;
                pipeline?
            }
        };
        let compiled = selection.effect.compiled(selection.index)?;
        let kinds = || {
            compiled
                .vertex
                .textures
                .iter()
                .chain(&compiled.pixel.textures)
        };
        self.null_textures(arena.device(), &selection, kinds())?;
        self.check_constants(&pipeline)?;
        let bound = |binding| match self.textures.get(&binding)? {
            Some(view) => Some(pipeline.sampled_texture(binding, view)),
            None => {
                let (_, kind) = kinds().find(|(key, _)| *key == binding)?;
                self.null_textures.get(kind).map(|bound| &bound.view)
            }
        };
        if self.builds.is_some() && pipeline.start_variant(device, bound) {
            return Err(self.wait());
        }
        let draw = pipeline
            .packet(
                arena,
                |binding| self.constants.get(&binding).map(Vec::as_slice),
                |binding| match self.textures.get(&binding)? {
                    Some(view) => Some(view),
                    None => {
                        let (_, kind) = kinds().find(|(key, _)| *key == binding)?;
                        self.null_textures.get(kind)
                    }
                },
                &Vertices {
                    declaration,
                    bytes,
                    stride,
                    count,
                },
            )
            .map_err(|reason| format!("effect {:?}: {reason}", selection.effect.source.effect))?;
        Ok(draw)
    }

    pub fn prepare_pass(
        &mut self,
        arena: &gpu::Arena,
        label: &str,
        template: &gpu::Pass,
    ) -> Result<PassSubmission, String> {
        let result = self
            .prepare_pass_inner(arena, template, None, None)
            .map(|(pass, _)| pass)
            .map_err(|reason| format!("native draw {label:?}: {reason}"));
        if let Err(reason) = &result {
            self.refuse(arena, label, reason);
        }
        result
    }

    pub fn prepare_packed_pass(
        &mut self,
        arena: &gpu::Arena,
        label: &str,
        template: &gpu::Pass,
        bytes: &[u8],
        stride: usize,
        count: usize,
    ) -> Result<NativeSubmission, String> {
        let result = (|| {
            let Some((_, Layout::Packed(declaration))) = &self.program else {
                return Err(match self.program {
                    None => "null technique needs the preceding native program, which has not been supplied",
                    Some(_) => "native vertex buffers require prepare_native_pass",
                }
                .into());
            };
            let declaration = *declaration;
            let (pass, drawn) = self.prepare_pass_inner(arena, template, None, None)?;
            let compiled = drawn.effect.compiled(drawn.index)?;
            let packed = Vertices {
                declaration,
                bytes,
                stride,
                count,
            }
            .pack(&compiled.vertex.inputs)?;
            let vertices = vec![arena.vertices(bytemuck::cast_slice(&packed))?];
            Ok(NativeSubmission { pass, vertices })
        })()
        .map_err(|reason: String| format!("native draw {label:?}: {reason}"));
        if let Err(reason) = &result {
            self.refuse(arena, label, reason);
        }
        result
    }

    pub fn prepare_native_pass(
        &mut self,
        arena: &gpu::Arena,
        label: impl std::fmt::Display,
        template: &gpu::Pass,
        buffers: &BTreeMap<u32, Option<super::vertices::NativeBuffer<'_>>>,
        vertices: u32,
        instances: u32,
    ) -> Result<NativeSubmission, String> {
        self.prepare_native_pass_with(
            arena, label, template, buffers, vertices, instances, None, None,
        )
    }

    pub fn prepare_native_pass_with(
        &mut self,
        arena: &gpu::Arena,
        label: impl std::fmt::Display,
        template: &gpu::Pass,
        buffers: &BTreeMap<u32, Option<super::vertices::NativeBuffer<'_>>>,
        vertices: u32,
        instances: u32,
        prepared: Option<NativePrepared<'_>>,
        previous: Option<&PassSubmission>,
    ) -> Result<NativeSubmission, String> {
        self.prepare_native_pass_with_cache(
            arena, label, template, buffers, vertices, instances, prepared, previous, None,
        )
    }

    pub fn prepare_native_pass_with_cache(
        &mut self,
        arena: &gpu::Arena,
        label: impl std::fmt::Display,
        template: &gpu::Pass,
        buffers: &BTreeMap<u32, Option<super::vertices::NativeBuffer<'_>>>,
        vertices: u32,
        instances: u32,
        prepared: Option<NativePrepared<'_>>,
        previous: Option<&PassSubmission>,
        mut cache: Option<&mut NativePassCache>,
    ) -> Result<NativeSubmission, String> {
        let result: Result<NativeSubmission, String> = (|| {
            let packed = if prepared.is_none() {
                Some(super::vertices::pack_native(
                    &self.vertex_layouts()?,
                    buffers,
                    vertices,
                    instances,
                )?)
            } else {
                None
            };
            let layouts = prepared
                .as_ref()
                .map(|p| p.layouts)
                .or_else(|| packed.as_ref().map(|p| p.layouts.as_slice()));
            let (pass, _) = if let Some(cache) = cache.as_deref_mut() {
                self.prepare_cached_pass_inner(arena, template, layouts, previous, cache)?
            } else {
                self.prepare_pass_inner(arena, template, layouts, previous)?
            };
            let vertices = if let Some(prepared) = prepared {
                prepared.vertices.to_vec()
            } else {
                packed
                    .as_ref()
                    .expect("packed native vertices")
                    .bytes
                    .iter()
                    .map(|bytes| arena.vertices(bytes))
                    .collect::<Result<_, _>>()?
            };
            Ok(NativeSubmission { pass, vertices })
        })();
        if let Some(cache) = cache {
            if let Some(pending) = cache.pending.take()
                && result.is_ok()
            {
                cache.bindings.push(pending);
            }
        }
        match result {
            Ok(submission) => Ok(submission),
            Err(reason) => {
                let label = label.to_string();
                let reason = format!("native draw {label:?}: {reason}");
                self.refuse(arena, &label, &reason);
                Err(reason)
            }
        }
    }

    fn refuse(&mut self, arena: &gpu::Arena, label: &str, reason: &str) {
        *self.refused.entry(reason.to_owned()).or_default() += 1;
        arena.refuse(&match &self.program {
            Some((selection, _)) => format!(
                "native draw {label:?}: effect {:?} technique {:?}",
                selection.effect.source.effect,
                selection.effect.source.techniques[selection.index].name
            ),
            None => format!("native draw {label:?}: no program"),
        });
    }

    fn prepare_pass_inner(
        &mut self,
        arena: &gpu::Arena,
        template: &gpu::Pass,
        vertices: Option<&[gpu::VertexLayout]>,
        previous: Option<&PassSubmission>,
    ) -> Result<(PassSubmission, Selection), String> {
        let resolved = self.resolve_pass(arena, template, vertices)?;
        self.bind_resolved_pass(arena, resolved, previous, None)
    }

    fn prepare_cached_pass_inner(
        &mut self,
        arena: &gpu::Arena,
        template: &gpu::Pass,
        vertices: Option<&[gpu::VertexLayout]>,
        previous: Option<&PassSubmission>,
        cache: &mut NativePassCache,
    ) -> Result<(PassSubmission, Selection), String> {
        let resolved = if let Some(cached) = &cache.resolved
            && cached.inputs.matches(self, template, vertices)
        {
            cached.diagnostics.credit();
            (
                cached.pipeline.clone(),
                cached.selection.clone(),
                cached.stencil_reference,
                cached.scissor_enabled,
            )
        } else {
            cache.resolved = None;
            cache.bindings.clear();
            let (resolved, diagnostics) =
                diagnostics::capture(|| self.resolve_pass(arena, template, vertices));
            diagnostics.credit();
            let resolved = resolved?;
            let (pipeline, selection, stencil_reference, scissor_enabled) = &resolved;
            if self.program.as_ref().is_some_and(|(requested, _)| {
                requested.effect.id == selection.effect.id && requested.index == selection.index
            }) {
                cache.resolved = Some(ResolvedPass {
                    inputs: PassInputs::new(self, template, vertices),
                    pipeline: pipeline.clone(),
                    selection: selection.clone(),
                    stencil_reference: *stencil_reference,
                    scissor_enabled: *scissor_enabled,
                    diagnostics,
                });
            }
            resolved
        };
        let successful = cache.resolved.is_some();
        self.bind_resolved_pass(arena, resolved, previous, successful.then_some(cache))
    }

    fn bind_resolved_pass(
        &mut self,
        arena: &gpu::Arena,
        resolved: PipelineResolution,
        previous: Option<&PassSubmission>,
        cache: Option<&mut NativePassCache>,
    ) -> Result<(PassSubmission, Selection), String> {
        let (pipeline, selection, stencil_reference, scissor_enabled) = resolved;
        if let Some(previous) =
            previous.filter(|previous| Arc::ptr_eq(&previous.pipeline, &pipeline))
        {
            return Ok((
                PassSubmission {
                    pipeline,
                    bindings: previous.bindings.clone(),
                    stencil_reference,
                    scissor_enabled,
                },
                selection,
            ));
        }
        let compiled = selection.effect.compiled(selection.index)?;
        let kinds = || {
            compiled
                .vertex
                .textures
                .iter()
                .chain(&compiled.pixel.textures)
        };
        self.null_textures(arena.device(), &selection, kinds())?;
        self.check_constants(&pipeline)?;
        let constants = |binding| self.constants.get(&binding).map(Vec::as_slice);
        let textures = |binding| match self.textures.get(&binding)? {
            Some(view) => Some(pipeline.sampled_texture(binding, view)),
            None => {
                let (_, kind) = kinds().find(|(key, _)| *key == binding)?;
                self.null_textures.get(kind).map(|bound| &bound.view)
            }
        };
        let bindings = if let Some(cache) = cache {
            let frame = arena.frame();
            if cache.frame != frame {
                cache.frame = frame;
                cache.bindings.retain(|entry| entry.used + 1 >= frame);
            }
            if let Some(entry) = cache
                .bindings
                .iter_mut()
                .rev()
                .find(|entry| entry.matches(self))
            {
                entry.used = frame;
                self.texture_snapshot = Some(entry.textures.clone());
                entry.program.bind_with(arena, constants)
            } else {
                pipeline
                    .bind_program_with(arena, constants, textures)
                    .map(|(bindings, program)| {
                        let snapshot = self
                            .texture_snapshot
                            .get_or_insert_with(|| Arc::new(self.textures.clone()))
                            .clone();
                        cache.pending = Some(NativeBindings {
                            textures: snapshot,
                            program,
                            used: frame,
                        });
                        bindings
                    })
            }
        } else {
            pipeline.bind_with(arena, constants, textures)
        }
        .map_err(|reason| format!("effect {:?}: {reason}", selection.effect.source.effect))?;
        Ok((
            PassSubmission {
                pipeline,
                bindings,
                stencil_reference,
                scissor_enabled,
            },
            selection,
        ))
    }

    fn resolve_pass(
        &mut self,
        arena: &gpu::Arena,
        template: &gpu::Pass,
        vertices: Option<&[gpu::VertexLayout]>,
    ) -> Result<PipelineResolution, String> {
        let (result, diagnostics) =
            diagnostics::capture(|| self.resolve_pass_uncached(arena, template, vertices));
        diagnostics.credit();
        let same = self
            .last_pass
            .as_ref()
            .is_some_and(|cached| cached.inputs.matches(self, template, vertices));
        if same {
            self.last_pass.as_mut().expect("matched above").diagnostics = diagnostics;
        }
        result
    }

    fn resolve_pass_uncached(
        &mut self,
        arena: &gpu::Arena,
        template: &gpu::Pass,
        vertices: Option<&[gpu::VertexLayout]>,
    ) -> Result<PipelineResolution, String> {
        if let Some(cached) = &self.last_pass
            && cached.inputs.matches(self, template, vertices)
        {
            cached.diagnostics.credit();
            return Ok((
                cached.pipeline.clone(),
                cached.selection.clone(),
                cached.stencil_reference,
                cached.scissor_enabled,
            ));
        }
        let (selection, layout) = self.program.as_ref().ok_or(
            "null technique needs the preceding native program, which has not been supplied",
        )?;
        let packed = matches!(layout, Layout::Packed(_));
        let mut selection = selection.clone();
        let (mut pass, stencil_reference, scissor_enabled) = self.pass_state(template);
        let layouts = |stream: &Self, selection: &Selection| match &vertices {
            Some(vertices) => Ok(vertices.to_vec()),
            None if packed => {
                let compiled = selection.effect.compiled(selection.index)?;
                let format = template
                    .targets
                    .iter()
                    .flatten()
                    .next()
                    .ok_or("packed native pass has no colour attachment")?
                    .format;
                Ok(
                    gpu::Pass::packed(&compiled, &stream.state, format, template.multisample.count)
                        .vertices,
                )
            }
            None => stream.vertex_layouts(),
        };
        pass.vertices = layouts(self, &selection)?;
        let pipeline = match self
            .cache
            .get_pass(arena.device(), &selection, &self.state, &pass)
        {
            Ok(pipeline) => pipeline,
            Err(error) => {
                let recorded = selection.fall_back(&error).ok_or(error)?;
                pass.vertices = layouts(self, &recorded)?;
                let pipeline =
                    self.cache
                        .get_pass(arena.device(), &recorded, &self.state, &pass)?;
                selection = recorded;
                pipeline
            }
        };
        if self.program.as_ref().is_some_and(|(requested, _)| {
            requested.effect.id == selection.effect.id && requested.index == selection.index
        }) {
            self.last_pass = Some(ResolvedPass {
                inputs: PassInputs::new(self, template, vertices),
                pipeline: pipeline.clone(),
                selection: selection.clone(),
                stencil_reference,
                scissor_enabled,
                diagnostics: Diagnostics::default(),
            });
        }
        Ok((pipeline, selection, stencil_reference, scissor_enabled))
    }

    fn pass_state(&self, template: &gpu::Pass) -> (gpu::Pass, Option<u32>, Option<bool>) {
        let mut pass = template.clone();
        for target in pass.targets.iter_mut().flatten() {
            target.blend = self.state.blend;
            target.write_mask = self.state.write_mask;
        }
        let mut scissor = None;
        if let Some(raster) = &self.rasterizer {
            let (mut primitive, enabled) = gpu::primitive(raster, pass.primitive.topology);
            primitive.strip_index_format = pass.primitive.strip_index_format;
            pass.primitive = primitive;
            scissor = Some(enabled);
        }
        let mut reference = None;
        if let Some(depth) = &mut pass.depth_stencil {
            if let Some(raster) = &self.rasterizer {
                depth.bias =
                    gpu::depth_stencil(&DepthStencilState::default(), raster, depth.format)
                        .0
                        .bias;
            }
            if let Some(state) = &self.depth_stencil {
                let bias = depth.bias;
                let (mut lowered, value) =
                    gpu::depth_stencil(state, &RasterizerState::default(), depth.format);
                lowered.bias = bias;
                *depth = lowered;
                reference = Some(value);
            }
        }
        (pass, reference, scissor)
    }

    fn null_textures<'a>(
        &mut self,
        device: &wgpu::Device,
        selection: &Selection,
        kinds: impl Iterator<Item = &'a (u32, super::TextureKind)>,
    ) -> Result<(), String> {
        for &(key, cube) in kinds {
            match self.textures.get(&key) {
                Some(Some(_)) => {}
                Some(None) => {
                    self.null_textures
                        .entry(cube)
                        .or_insert_with(|| gpu::null_texture(device, cube).into());
                }
                None => {
                    return Err(format!(
                        "effect {:?}: native texture binding {key} has not been supplied",
                        selection.effect.source.effect
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn template() -> gpu::Pass {
        gpu::Pass {
            vertices: Vec::new(),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                strip_index_format: Some(wgpu::IndexFormat::Uint16),
                ..Default::default()
            },
            depth_stencil: Some(
                gpu::depth_stencil(
                    &DepthStencilState::default(),
                    &RasterizerState::default(),
                    wgpu::TextureFormat::Depth24PlusStencil8,
                )
                .0,
            ),
            multisample: wgpu::MultisampleState {
                count: 4,
                ..Default::default()
            },
            targets: vec![Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba16Float,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            instanced: 0,
            stream: None,
            vertex_color_tint: None,
        }
    }

    #[test]
    fn resolved_pass_key_covers_pipeline_inputs_but_not_draw_resources() {
        crate::tests::initialize_runtime_tables();
        let source = content_reader_gui::pdxshader::program::read_with(b"key.shader", &|path| {
            Some(if path == b"key.shader" {
                b"Effect Draw { }".to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let selection = Selection {
            effect: super::super::library::Effect::from_compiled(
                Arc::new(source),
                Vec::new(),
                vec![],
            ),
            index: 0,
        };
        let layouts = vec![gpu::VertexLayout {
            stride: 16,
            step: wgpu::VertexStepMode::Vertex,
            attributes: vec![wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x4,
                offset: 0,
                shader_location: 0,
            }],
        }];
        let mut stream = Stream::default();
        stream.select_program(&selection, &layouts);
        let Layout::Native(bound) = &stream.program.as_ref().unwrap().1 else {
            unreachable!()
        };
        let allocation = bound.as_ptr();
        stream.select_program(&selection, &layouts);
        let Layout::Native(bound) = &stream.program.as_ref().unwrap().1 else {
            unreachable!()
        };
        assert_eq!(
            bound.as_ptr(),
            allocation,
            "unchanged selection retains its declaration allocation"
        );
        stream.bind_vertex_stride(0, 16);
        stream.bind_depth_stencil(Default::default());
        stream.bind_rasterizer(Default::default());
        stream.bind_texture(0, Stage::Vertex, None, None);
        let template = template();
        let key = PassInputs::new(&stream, &template, Some(&layouts));
        assert!(key.matches(&stream, &template, Some(&layouts)));
        let changes: &[fn(&mut Stream)] = &[
            |s| s.program.as_mut().unwrap().0.index += 1,
            |s| {
                let selection = &mut s.program.as_mut().unwrap().0;
                selection.effect = super::super::library::Effect::from_compiled(
                    selection.effect.source.clone(),
                    Vec::new(),
                    vec![],
                );
            },
            |s| s.program.as_mut().unwrap().1 = Layout::Packed(Declaration::Sprite),
            |s| {
                let Layout::Native(v) = &mut s.program.as_mut().unwrap().1 else {
                    unreachable!()
                };
                Arc::make_mut(v)[0].attributes[0].offset = 4;
            },
            |s| s.bind_vertex_stride(0, 20),
            |s| {
                s.bind_blend(
                    Some(wgpu::BlendState::ALPHA_BLENDING),
                    wgpu::ColorWrites::ALL,
                )
            },
            |s| s.bind_blend(None, wgpu::ColorWrites::RED),
            |s| s.depth_stencil.as_mut().unwrap().stencil_ref = 3,
            |s| s.rasterizer.as_mut().unwrap().scissor = true,
            |s| s.rasterizer.as_mut().unwrap().slope_scaled_depth_bias = -0.,
            |s| {
                let key = binding(0, Stage::Vertex);
                let mut sampler = s.state.samplers[&key].clone();
                sampler.lod_bias = -0.;
                s.set_sampler(key, sampler);
            },
            |s| {
                let key = binding(0, Stage::Vertex);
                let mut sampler = s.state.samplers[&key].clone();
                sampler.max_lod = 3;
                s.set_sampler(key, sampler);
            },
            |s| s.invalidate_bindings(),
        ];
        for (index, change) in changes.iter().enumerate() {
            let mut changed = Stream::default();
            changed.inherit_bindings(&stream);
            change(&mut changed);
            assert!(
                !key.matches(&changed, &template, Some(&layouts)),
                "changed pipeline input {index}"
            );
        }
        let mut equal = Stream::default();
        equal.inherit_bindings(&stream);
        equal.bind_vertex_stride(0, 20);
        equal.bind_vertex_stride(0, 16);
        let slot = binding(0, Stage::Vertex);
        let sampler = equal.state.samplers[&slot].clone();
        let mut changed_sampler = sampler.clone();
        changed_sampler.lod_bias = -0.;
        equal.set_sampler(slot, changed_sampler);
        equal.set_sampler(slot, sampler);
        equal.bind_program(Some(selection.clone()), layouts.clone());
        assert!(!Arc::ptr_eq(&equal.vertex_strides, &key.strides));
        assert!(!Arc::ptr_eq(&equal.sampler_keys, &key.samplers));
        assert!(
            key.matches(&equal, &template, Some(&layouts)),
            "equal independently owned keys still match"
        );
        let mut changed = template.clone();
        changed.multisample.count = 1;
        assert!(!key.matches(&stream, &changed, Some(&layouts)));
        changed = template.clone();
        changed.primitive.topology = wgpu::PrimitiveTopology::TriangleList;
        assert!(!key.matches(&stream, &changed, Some(&layouts)));
        changed = template.clone();
        changed.targets[0].as_mut().unwrap().format = wgpu::TextureFormat::Rgba8Unorm;
        assert!(!key.matches(&stream, &changed, Some(&layouts)));
        assert!(!key.matches(&stream, &template, None));
        let mut repacked = layouts.clone();
        repacked[0].stride = 20;
        assert!(!key.matches(&stream, &template, Some(&repacked)));
        stream.bind_constants(0, Stage::Vertex, Some(&[9; 16]));
        stream.bind_partial_constants(
            0,
            Stage::Pixel,
            Some(&[7; 16]),
            std::slice::from_ref(&(0..8)),
        );
        stream.bind_texture(0, Stage::Pixel, None, None);
        assert!(
            key.matches(&stream, &template, Some(&layouts)),
            "resources are checked afresh, not cached with a pipeline"
        );
    }

    #[test]
    fn immutable_input_snapshots_preserve_noops_bits_and_scoped_mutations() {
        for frame in 0..256 {
            let mut owner = Stream::default();
            owner.bind_vertex_stride(0, 16 + frame);
            owner.bind_texture(0, Stage::Vertex, None, None);
            let slot = binding(0, Stage::Vertex);
            let textures = Arc::new(owner.textures.clone());
            owner.texture_snapshot = Some(textures.clone());
            let strides = owner.vertex_strides.clone();
            let samplers = owner.sampler_keys.clone();
            let mut scoped = Stream::default();
            scoped.inherit_bindings(&owner);
            scoped.bind_vertex_stride(0, 16 + frame);
            scoped.bind_texture(0, Stage::Vertex, None, None);
            assert!(Arc::ptr_eq(&scoped.vertex_strides, &strides));
            assert!(Arc::ptr_eq(&scoped.sampler_keys, &samplers));
            assert!(Arc::ptr_eq(
                scoped.texture_snapshot.as_ref().unwrap(),
                &textures
            ));

            let mut sampler = scoped.state.samplers[&slot].clone();
            sampler.lod_bias = -0.;
            scoped.bind_texture(0, Stage::Vertex, None, Some(&sampler));
            assert_eq!(
                scoped.state.samplers[&slot].lod_bias.to_bits(),
                0,
                "equal sampler command retains positive zero"
            );
            assert!(Arc::ptr_eq(&scoped.sampler_keys, &samplers));
            sampler.lod_bias = f32::from_bits(0x7fc0_0000 | frame as u32);
            scoped.bind_texture(0, Stage::Vertex, None, Some(&sampler));
            let nan_keys = scoped.sampler_keys.clone();
            scoped.bind_texture(0, Stage::Vertex, None, Some(&sampler));
            assert!(
                Arc::ptr_eq(&scoped.sampler_keys, &nan_keys),
                "same NaN bits do not copy a key"
            );
            assert_eq!(
                scoped.sampler_keys[&slot],
                gpu::sampler_key(&scoped.state.samplers[&slot])
            );
            assert_eq!(owner.sampler_keys, samplers);
            assert_eq!(owner.state.samplers[&slot].lod_bias.to_bits(), 0);

            scoped.bind_vertex_stride(0, 17 + frame);
            assert_eq!(owner.vertex_strides[&0], 16 + frame);
            assert_eq!(scoped.vertex_strides[&0], 17 + frame);
            scoped.bind_texture(1, Stage::Pixel, None, None);
            assert!(scoped.texture_snapshot.is_none());
            assert_eq!(*textures, owner.textures);
            assert!(!textures.contains_key(&binding(1, Stage::Pixel)));
            scoped.forget_texture(0, Stage::Vertex, None);
            assert!(!scoped.textures.contains_key(&slot));
            assert_eq!(
                textures.get(&slot),
                Some(&None),
                "missing is not explicit null"
            );
            scoped.invalidate_bindings();
            assert!(scoped.vertex_strides.is_empty());
            assert!(scoped.sampler_keys.is_empty());
            assert!(scoped.texture_snapshot.is_none());
            assert_eq!(owner.vertex_strides, strides);
            assert_eq!(*textures, owner.textures);
        }
    }

    #[test]
    fn independent_handles_survive_program_misses_and_scoped_passes() {
        crate::tests::initialize_runtime_tables();
        let program = content_reader_gui::pdxshader::program::read_with(b"states.shader", &|path| {
            Some(if path == b"states.shader" {
                b"Effect Draw { }".to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let selection = Selection {
            effect: super::super::library::Effect::from_compiled(
                Arc::new(program),
                Vec::new(),
                vec![Err("not compiled in a state test".into())],
            ),
            index: 0,
        };
        let layout = vec![gpu::VertexLayout {
            stride: 48,
            step: wgpu::VertexStepMode::Instance,
            attributes: vec![wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint8x4,
                offset: 28,
                shader_location: 3,
            }],
        }];
        let mut stream = Stream::default();
        stream.bind_program(Some(selection), layout.clone());
        stream.bind_depth_stencil(DepthStencilState {
            depth_write_mask: 0,
            stencil_enable: true,
            stencil_ref: 37,
            ..Default::default()
        });
        stream.bind_rasterizer(RasterizerState {
            front_ccw: true,
            scissor: true,
            depth_bias: -7,
            slope_scaled_depth_bias: 2.,
            ..Default::default()
        });
        stream.bind_blend(
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::ColorWrites::RED,
        );
        stream.bind_constants(1, Stage::Vertex, Some(&[1, 2, 3, 4]));
        stream.bind_constants(1, Stage::Pixel, Some(&[5, 6, 7, 8]));
        stream.bind_program(None, Vec::new());
        stream.bind_constants(1, Stage::Pixel, None);
        let (_, Layout::Native(retained)) = stream.program.as_ref().unwrap() else {
            panic!("native layout lost")
        };
        assert_eq!(retained.as_ref(), layout);
        let template = template();
        let (pass, reference, scissor) = stream.pass_state(&template);
        assert_eq!((reference, scissor), (Some(37), Some(true)));
        assert_eq!(
            pass.primitive.strip_index_format,
            Some(wgpu::IndexFormat::Uint16)
        );
        assert_eq!(pass.primitive.front_face, wgpu::FrontFace::Ccw);
        assert_eq!(pass.multisample.count, 4);
        let depth = pass.depth_stencil.unwrap();
        assert_eq!(depth.depth_write_enabled, Some(false));
        assert_eq!(depth.bias.constant, -7);
        assert_eq!(depth.bias.slope_scale, 2.);
        let target = pass.targets[0].as_ref().unwrap();
        assert_eq!(target.format, wgpu::TextureFormat::Rgba16Float);
        assert_eq!(target.write_mask, wgpu::ColorWrites::RED);
        assert_eq!(target.blend, Some(wgpu::BlendState::ALPHA_BLENDING));
        let mut scoped = Stream::default();
        stream.bind_vertex_stride(0, 64);
        scoped.inherit_bindings(&stream);
        scoped.bind_vertex_stride(0, 80);
        assert_eq!(stream.vertex_layouts().unwrap()[0].stride, 64);
        assert_eq!(scoped.vertex_layouts().unwrap()[0].stride, 80);
        assert_eq!(scoped.pass_state(&template), stream.pass_state(&template));
        scoped.bind_constants(1, Stage::Pixel, Some(&[9; 4]));
        scoped.bind_rasterizer(RasterizerState::default());
        assert_eq!(stream.constants[&binding(1, Stage::Pixel)], [5, 6, 7, 8]);
        assert_eq!(scoped.constants[&binding(1, Stage::Vertex)], [1, 2, 3, 4]);
        assert_eq!(stream.rasterizer.as_ref().unwrap().depth_bias, -7);
        assert_eq!(scoped.rasterizer.as_ref().unwrap().depth_bias, 0);
    }

    #[test]
    fn packed_world_text_retains_raster_and_distinct_stage_allocations() {
        crate::tests::initialize_runtime_tables();
        let source = content_reader_gui::pdxshader::program::read_with(b"text.shader", &|path| {
            Some(if path == b"text.shader" {
                b"Effect Text3D { }".to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let selection = Selection {
            effect: super::super::library::Effect::from_compiled(
                Arc::new(source),
                Vec::new(),
                vec![],
            ),
            index: 0,
        };
        let mut stream = Stream::default();
        stream.bind_constants(0, Stage::Pixel, Some(&[17; 408]));
        stream.bind_rasterizer(RasterizerState {
            front_ccw: true,
            depth_bias: -3,
            scissor: true,
            ..Default::default()
        });
        stream.bind_depth_stencil(DepthStencilState {
            depth_enable: false,
            depth_write_mask: 0,
            ..Default::default()
        });
        stream
            .bind_effect(Some(selection), Declaration::Text)
            .unwrap();
        stream.bind_constants(0, Stage::Vertex, Some(&[29; 80]));
        let (pass, _, scissor) = stream.pass_state(&template());
        assert_eq!(stream.constants[&binding(0, Stage::Vertex)], [29; 80]);
        assert_eq!(stream.constants[&binding(0, Stage::Pixel)], [17; 408]);
        assert_eq!(pass.primitive.front_face, wgpu::FrontFace::Ccw);
        assert_eq!(scissor, Some(true));
        let depth = pass.depth_stencil.unwrap();
        assert_eq!(depth.depth_write_enabled, Some(false));
        assert_eq!(depth.depth_compare, Some(wgpu::CompareFunction::Always));
        assert_eq!(depth.bias.constant, -3);
        assert_eq!(pass.multisample.count, 4);
        stream.bind_effect(None, Declaration::Sprite).unwrap();
        assert!(matches!(
            stream.program,
            Some((_, Layout::Packed(Declaration::Text)))
        ));
    }

    #[test]
    fn a_premultiplied_layer_accumulates_alpha_over_the_bound_colour_terms() {
        let mut stream = Stream::default();
        stream.bind_blend(
            Some(wgpu::BlendState::ALPHA_BLENDING),
            wgpu::ColorWrites::COLOR,
        );
        stream.accumulate_alpha();
        let blend = stream.state.blend.unwrap();
        assert_eq!(blend.color, wgpu::BlendState::ALPHA_BLENDING.color);
        assert_eq!(
            blend.alpha,
            wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            }
        );
        assert_eq!(stream.state.write_mask, wgpu::ColorWrites::ALL);
        stream.bind_blend(
            Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::REPLACE,
            }),
            wgpu::ColorWrites::RED,
        );
        stream.accumulate_alpha();
        let blend = stream.state.blend.unwrap();
        assert_eq!(blend.color.dst_factor, wgpu::BlendFactor::One);
        assert_eq!(blend.alpha.dst_factor, wgpu::BlendFactor::OneMinusSrcAlpha);
        stream.bind_blend(None, wgpu::ColorWrites::RED);
        stream.accumulate_alpha();
        assert_eq!(stream.state.blend, None);
        assert_eq!(stream.state.write_mask, wgpu::ColorWrites::ALL);
    }

    #[test]
    fn a_replacement_blend_that_cannot_be_built_binds_the_recorded_program() {
        use super::super::library::{Effect, Recorded};
        use std::sync::Mutex;
        crate::tests::initialize_runtime_tables();
        let mut replacement =
            content_reader_gui::pdxshader::program::read_with(b"mod.shader", &|path| {
                Some(match path {
                b"mod.shader" => {
                    b"VertexShader = { MainCode V [[ float4 main() : SV_POSITION { return 1; } ]] }
PixelShader = { MainCode P [[ float4 main() : SV_TARGET { return 1; } ]] }
BlendState BlendState { BlendEnable = yes SourceBlend = src_alpha DestBlend = inv_src_alpha }
Effect Up { VertexShader = V PixelShader = P }
"
                    .to_vec()
                }
                _ => Vec::new(),
            })
            })
            .unwrap();
        replacement.blends[0].value.source = 99;
        let cache = Arc::new(crate::cache::Cache::new(
            None,
            Arc::new(crate::cache::tests::Count::default()),
        ));
        let recorded = Arc::new(crate::cache::tests::source("static const float4 C = 1;"));
        let base = Effect::with_globals(recorded, vec![], vec![], cache.clone(), None);
        let notes = Arc::new(Mutex::new(Vec::new()));
        let sink = notes.clone();
        let shown = Effect::with_globals(
            Arc::new(replacement),
            vec![],
            vec![],
            cache,
            Some(Recorded {
                effect: base.clone(),
                note: Arc::new(move |note| sink.lock().unwrap().push(note)),
            }),
        );
        let selection = shown.select("Up").unwrap().unwrap();
        let mut stream = Stream::default();
        stream
            .bind_effect(Some(selection), Declaration::Sprite)
            .unwrap();
        let (bound, _) = stream.program.as_ref().unwrap();
        assert!(Arc::ptr_eq(&bound.effect, &base));
        assert_eq!(base.source.techniques[bound.index].name, "Up");
        let notes = notes.lock().unwrap();
        assert_eq!(notes.len(), 1);
        assert!(
            notes[0].contains("unknown native blend factor 99"),
            "{notes:?}"
        );
    }

    #[test]
    fn refused_draws_are_recorded_by_stable_names() {
        use super::super::library::Effect;
        crate::tests::initialize_runtime_tables();
        let _serial = crate::tests::GPU_TESTS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (device, _queue) = crate::tests::device();
        let refused = || {
            let cache = Arc::new(crate::cache::Cache::new(
                None,
                Arc::new(crate::cache::tests::Count::default()),
            ));
            let source = Arc::new(crate::cache::tests::source("static const float4 C = 1;"));
            let effect = Effect::with_globals(source, vec![], vec![], cache, None);
            let arena = gpu::Arena::new(&device);
            let mut stream = Stream::default();
            stream
                .bind_effect(effect.select("Up").unwrap(), Declaration::Sprite)
                .unwrap();
            let Err(reason) = stream.prepare_pass(&arena, "sprite", &template()) else {
                panic!("a pipeline was built from the counting compiler's shaders");
            };
            assert_eq!(stream.refused.keys().collect::<Vec<_>>(), [&reason]);
            assert_eq!(
                arena.refusals(),
                [format!(
                    "native draw \"sprite\": effect {:?} technique \"Up\"",
                    effect.source.effect
                )]
                .into()
            );
            arena.refusals()
        };
        assert_eq!(refused(), refused());
    }

    fn falling_back() -> (
        std::sync::MutexGuard<'static, ()>,
        gpu::Arena,
        Arc<super::super::library::Effect>,
    ) {
        use super::super::library::{Effect, Recorded};
        use crate::{Compiled, ScalarKind, Shader, VertexInput};
        use content_reader_gui::pdxshader::program::{Program, Technique};
        struct Layouts([u8; 32]);
        impl crate::cache::Compile for Layouts {
            fn compile(
                &self,
                source: &Program,
                _: &Technique,
                _: &[String],
            ) -> Result<Compiled, String> {
                let input = |location, semantic: &str, components| VertexInput {
                    kind: ScalarKind::Float,
                    location,
                    semantic: semantic.into(),
                    components,
                };
                let shader = |wgsl: &str, inputs| Shader {
                    wgsl: wgsl.into(),
                    independent_draws: false,
                    buffers: vec![],
                    textures: vec![],
                    samplers: vec![],
                    sampling_pairs: vec![],
                    inputs,
                    clock: Default::default(),
                };
                if source.identity == self.0 {
                    return Ok(Compiled {
                        vertex: shader("fixture", vec![input(0, "POSITION", 3)]),
                        pixel: shader("fixture", vec![]),
                    });
                }
                Ok(Compiled {
                    vertex: shader(
                        "@vertex fn main(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>) -> @builtin(position) vec4<f32> { return vec4<f32>(p.xy + uv, p.z, 1.0); }",
                        vec![input(0, "POSITION", 3), input(1, "TEXCOORD0", 2)],
                    ),
                    pixel: shader(
                        "@fragment fn main() -> @location(0) vec4<f32> { return vec4<f32>(1.0); }",
                        vec![],
                    ),
                })
            }
        }
        crate::tests::initialize_runtime_tables();
        let serial = crate::tests::GPU_TESTS
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let (device, _queue) = crate::tests::device();
        let arena = gpu::Arena::new(&device);
        let recorded = Arc::new(crate::cache::tests::source("static const float4 C = 1;"));
        let replacement = Arc::new(crate::cache::tests::source("static const float4 C = 2;"));
        let cache = Arc::new(crate::cache::Cache::new(
            None,
            Arc::new(Layouts(replacement.identity)),
        ));
        let base = Effect::with_globals(recorded, vec![], vec![], cache.clone(), None);
        let shown = Effect::with_globals(
            replacement,
            vec![],
            vec![],
            cache,
            Some(Recorded {
                effect: base,
                note: Arc::new(|_| {}),
            }),
        );
        (serial, arena, shown)
    }

    #[test]
    fn packed_geometry_follows_the_program_a_replacement_falls_back_to() {
        let (_serial, arena, shown) = falling_back();
        let mut stream = Stream::default();
        stream
            .bind_effect(shown.select("Up").unwrap(), Declaration::Sprite)
            .unwrap();
        let submission =
            match stream.prepare_packed_pass(&arena, "text", &template(), &[0; 60], 20, 3) {
                Ok(submission) => submission,
                Err(reason) => panic!("{reason}"),
            };
        assert_eq!(submission.vertices[0].size, 3 * 5 * 4);
    }
}
