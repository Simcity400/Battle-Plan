use shader_pipeline::gpu::stats::CountedPasses as _;
use crate::*;
mod admission;
mod builds;
mod bundles;
mod kept;
pub(super) mod walk;
mod plans;
mod plan_tail;
use crate::mesh_pose;
use viewer_scene_base::{mesh_shader, shader_scene};
use content_reader_gui::pdxshader::program::Stage;
use shader_pipeline::{
    binding,
    gpu::{CONSTANT_BINDINGS, TEXTURE_BINDINGS},
    library::{Library, Selection},
};
use viewer_gpu::gpu_contract::SCENE_FORMAT;

pub(super) type GeometryKey = (String, u32, usize, usize);

#[derive(Clone, PartialEq)]
pub(super) struct Part {
    pub geometry: Arc<mesh_shader::Geometry>,
    pub geometry_key: GeometryKey,
    pub material: mesh::Material,
    pub material_index: usize,
}

impl std::fmt::Debug for Part {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Part")
            .field("geometry", &self.geometry_key)
            .field("material", &self.material)
            .field("material_index", &self.material_index)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Descriptor {
    pub name: String,
    pub node: usize,
    pub distances: Vec<f32>,
    pub lods: Vec<Vec<Part>>,
}

fn lods<T>(records: impl IntoIterator<Item = (Option<i32>, T)>) -> io::Result<Vec<Vec<T>>> {
    let mut lods = Vec::new();
    let mut unnumbered = Vec::new();
    for (lod, record) in records {
        match lod.unwrap_or(-1) {
            -1 => unnumbered.push(record),
            lod if lod >= 0 => {
                let index = lod as usize;
                if lods.len() <= index {
                    lods.try_reserve(index + 1 - lods.len())
                        .map_err(io::Error::other)?;
                    lods.resize_with(index + 1, Vec::new);
                }
                lods[index].push(record);
            }
            lod => return Err(invalid(format!("native mesh invalid negative LOD {lod}"))),
        }
    }
    if !unnumbered.is_empty() {
        lods.push(unnumbered);
    }
    Ok(lods)
}

pub(super) fn descriptor(
    decoded: &mesh::Mesh,
    resolved: &mesh::ResolvedMesh<'_>,
    node: usize,
    name: &str,
    mut geometry: impl FnMut(GeometryKey, &mesh::SubMesh) -> io::Result<Arc<mesh_shader::Geometry>>,
    mut material_index: impl FnMut(&str, &mesh::SubMesh) -> io::Result<usize>,
) -> io::Result<Descriptor> {
    let mut records = Vec::new();
    for (shape_index, shape) in decoded.shapes.iter().enumerate() {
        for (sub_index, sub) in shape.meshes.iter().enumerate() {
            if !viewer_scene_base::mesh_effect::candidate(sub) {
                continue;
            }
            let geometry_key = (
                resolved.definition.file.clone(),
                resolved.definition.scale.to_bits(),
                shape_index,
                sub_index,
            );
            records.push((
                shape.lod,
                Part {
                    geometry: geometry(geometry_key.clone(), sub).map_err(|error| {
                        invalid(format!(
                            "{} shape {:?} record {}: {error}",
                            resolved.definition.file, shape.name, sub.child_index
                        ))
                    })?,
                    geometry_key,
                    material: resolved.definition.material(&shape.name, sub),
                    material_index: material_index(&shape.name, sub)?,
                },
            ));
        }
    }
    Ok(Descriptor {
        name: name.to_owned(),
        node,
        distances: decoded.lod_distances.iter().map(|v| v * v).collect(),
        lods: lods(records)?,
    })
}

impl Descriptor {

    fn lod(&self, position: [f32; 3], eye: [f32; 3]) -> io::Result<Option<usize>> {
        let Some(last) = self.lods.len().checked_sub(1) else {
            return Ok(None);
        };
        let [x, y, z] = std::array::from_fn(|i| position[i] - eye[i]);
        let distance = y * y + x * x + z * z;
        let index = self
            .distances
            .iter()
            .rposition(|v| *v <= distance)
            .unwrap_or(last);
        if index >= self.lods.len() {
            return Err(invalid(format!(
                "native mesh LOD {index} is outside {} descriptor levels",
                self.lods.len()
            )));
        }
        Ok(Some(index))
    }
}

#[derive(Clone, Copy)]
pub struct Inputs<'a> {
    pub samples: u32,
    pub frames: &'a [&'a [u8]],
    pub grid: &'a [u8],
    pub textures: [&'a wgpu::TextureView; 10],
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Queue {
    Regular,
    Alternate,
    Caster,
}

impl Queue {
    fn accepts(self, blended: bool, forced: bool) -> bool {
        match self {
            Self::Regular => !blended && !forced,
            Self::Alternate => blended || forced,
            Self::Caster => true,
        }
    }
}

pub struct ObjectConstants {
    bytes: [u8; 208],
    pixel: Option<[u8; 208]>,
    pub retained: viewer_scene_base::shader_scene::State,
}

impl Default for ObjectConstants {
    fn default() -> Self {
        Self {
            bytes: [0; 208],
            pixel: None,
            retained: Default::default(),
        }
    }
}

impl ObjectConstants {
    pub fn forget_pixel_snapshot(&mut self) {
        self.pixel = None;
    }

    pub fn begin(&mut self) {
        self.bytes.fill(0);
    }
}

struct Pending {
    part: Option<usize>,
    samplers: [Option<usize>; TEXTURE_BINDINGS],
    skin: Option<usize>,
    frame: Option<usize>,
    grid: bool,
}

impl Default for Pending {
    fn default() -> Self {
        Self {
            part: None,
            samplers: [None; TEXTURE_BINDINGS],
            skin: None,
            frame: None,
            grid: false,
        }
    }
}

impl Pending {
    fn record(&mut self, key: usize, resident: &Resident, textures: u32) {
        self.record_part(key, resident.binds(), textures);
    }

    fn record_part(&mut self, key: usize, (samplers, skinned): (u32, bool), textures: u32) {
        if self.part != Some(key) {
            let mut samplers = textures & samplers;
            while samplers != 0 {
                let slot = samplers.trailing_zeros() as usize;
                self.samplers[slot] = Some(key);
                samplers &= samplers - 1;
            }
            if skinned {
                self.skin = Some(key);
            }
        }
        self.part = Some(key);
    }

    fn apply_sources<'a>(
        &self,
        resident: impl Fn(usize) -> &'a Resident,
        textures: Textures,
        buffers: Constants,
        retained: &mut shader_scene::State,
    ) {
        let Some(part) = self.part else {
            return;
        };
        resident(part).retained.select(retained);
        let stage = |key: u32| {
            if key.is_multiple_of(2) {
                Stage::Vertex
            } else {
                Stage::Pixel
            }
        };
        let stream = &mut retained.stream;
        for (key, bytes) in (0..).zip(buffers) {
            let Some(bytes) = bytes else {
                continue;
            };
            if key / 2 == 0 {
                stream.bind_partial_constants(0, stage(key), Some(bytes), &[0..348, 352..408]);
            } else {
                stream.bind_constants(key / 2, stage(key), Some(bytes));
            }
        }
        for (key, texture) in (0..).zip(textures) {
            let Some(texture) = texture else {
                continue;
            };
            let sampler =
                self.samplers[key as usize].and_then(|part| resident(part).retained.sampler(key));
            stream.bind_texture(key / 2, stage(key), Some(texture.clone()), sampler);
        }
        let selected = resident(part);
        retained.bind_vertex(
            0,
            selected.geometry.native_stride,
            selected.geometry.cpu_vertices.clone(),
        );
        if let Some(skin) = self
            .skin
            .and_then(|part| resident(part).geometry.cpu_skin.clone())
        {
            retained.bind_vertex(1, 16, skin);
        }
    }
}

type Constants<'a> = [Option<&'a [u8]>; CONSTANT_BINDINGS];
type Textures<'a> = [Option<&'a wgpu::TextureView>; TEXTURE_BINDINGS];

struct Resident {
    pipeline: mesh_shader::Pass,
    required_constants: Vec<u32>,
    required_textures: Vec<u32>,
    constant_mask: u32,
    texture_mask: u32,
    retained: shader_scene::Program,
    geometry: SharedGeometry,
    caster_texture: bool,
    blended: bool,
    order: Option<mesh_shader::Order>,
    samplers: u32,
    program: Option<(ResourceStamp, plan_tail::Program)>,
}

enum PrepareError {
    Actor(io::Error),
    Shared(io::Error),
}

impl From<io::Error> for PrepareError {
    fn from(error: io::Error) -> Self {
        Self::Shared(error)
    }
}

struct PreparedPart {
    key: usize,
    bindings: Option<shader_pipeline::gpu::Bindings>,
    textures: u32,
    material: usize,
    count: u32,
}

impl Resident {
    fn binds(&self) -> (u32, bool) {
        (self.samplers, self.geometry.cpu_skin.is_some())
    }

    fn check_bindings(&self, buffers: &Constants, textures: &Textures) -> Result<(), PrepareError> {
        for &slot in &self.required_constants {
            if buffers[slot as usize].is_none() {
                return Err(PrepareError::Actor(invalid(format!(
                    "native constant buffer {} has not been bound",
                    slot / 2
                ))));
            }
        }
        for &slot in &self.required_textures {
            if textures[slot as usize].is_none() {
                return Err(PrepareError::Actor(invalid(format!(
                    "native texture slot {} has not been bound",
                    slot / 2
                ))));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct SharedGeometry {
    stride: u64,
    native_stride: u64,
    cpu_vertices: Arc<[u8]>,
    cpu_skin: Option<Arc<[u8]>>,
    vertices: (usize, i32),
    indices: (usize, u32),
    skin_bones: usize,
}

type MeshSlots = Vec<Vec<Vec<[Option<usize>; 4]>>>;

pub(super) struct Renderer {
    builds: builds::Builds,
    residents: Vec<Resident>,
    slots: Vec<MeshSlots>,
    resource_ids: ResourceIdentities,
    geometry: BTreeMap<GeometryKey, SharedGeometry>,
    queue_slots: Vec<Vec<Option<[bool; 2]>>>,
    admission: admission::Admission,
    frame_handles: walk::FrameHandles,
    binding_storage: shader_pipeline::gpu::BindingSessionStorage,
    scratch: plans::Scratch,
    geometry_seen: Vec<bool>,
    prepared: Vec<PreparedPart>,
    candidates: Vec<(usize, shader_pipeline::gpu::BindingProgram)>,
    pools: BTreeMap<(u64, bool), Pool>,
    indices: Pool,
    draws: DrawList,
    casters: kept::Kept,
    lit: [Vec<kept::Kept>; 2],
    calls: (u64, [usize; 2]),
    lent: Option<(usize, usize)>,
}

impl Renderer {

    fn list(&mut self, id: ListId) -> &mut kept::Kept {
        list_of(&mut self.casters, &mut self.lit, id)
    }

    fn lit_call(&mut self, frame: u64, queue: usize) -> usize {
        if self.calls.0 != frame {
            self.calls = (frame, [0; 2]);
        }
        self.calls.1[queue] += 1;
        self.calls.1[queue] - 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum ListId {
    Casters,
    Lit(usize, usize),
}

fn list_of<'a>(
    casters: &'a mut kept::Kept,
    lit: &'a mut [Vec<kept::Kept>; 2],
    id: ListId,
) -> &'a mut kept::Kept {
    match id {
        ListId::Casters => casters,
        ListId::Lit(queue, at) => {
            let lists = &mut lit[queue];
            if lists.len() <= at {
                lists.resize_with(at + 1, || kept::Kept::new(kept::Kind::Lit));
            }
            &mut lists[at]
        }
    }
}

struct RecordNames {
    draws: &'static str,
    figures: &'static str,
    walks: &'static str,
    kept: &'static str,
    spliced: &'static str,
    splice_us: &'static str,
    splice_walk_us: &'static str,
    targets_us: &'static str,
    plan_us: &'static str,
    rewrite_us: &'static str,
    apply_us: &'static str,
    keep_test_us: &'static str,
    keep_rewrite_us: &'static str,
    entered: &'static str,
    left: &'static str,
    rewritten: &'static str,
    rows_changed: &'static str,
    row_bytes_changed: &'static str,
    live_rows: &'static str,
    reposed: &'static str,
    staged: &'static str,
    staged_bytes: &'static str,
    relevelled: &'static str,
    flipped: &'static str,
    waiting: &'static str,
    parts: &'static str,
    instanced_runs: &'static str,
    instanced_parts: &'static str,
    ordered: &'static str,
    bands: &'static str,
    judged: &'static str,
    lods: [&'static str; kept::LODS],
    whole: [&'static str; kept::WHOLES],
    inputs: [&'static str; kept::MADE_INPUTS],
    entered_by: [&'static str; kept::ENTERED],
}

macro_rules! prefixed {
    ($queue:literal, $kind:literal, [$($name:literal),* $(,)?]) => {
        [$(concat!($queue, $kind, $name)),*]
    };
}

macro_rules! why_names {
    ($queue:literal) => {
        (
            prefixed!($queue, "_whole_", [
                "first", "refused", "waiting", "not_followed", "most_walked", "no_buffer", "unplanned",
                "no_room", "unbound", "error",
            ]),
            prefixed!($queue, "_whole_input_", [
                "revision", "map", "offsets", "visible", "alpha", "shared", "reads", "samples", "textures",
                "globals", "chain", "cull",
            ]),
            prefixed!($queue, "_entered_", [
                "new", "other_view", "key_mesh", "key_epoch", "twice", "born", "waiting", "demoted",
                "rewrite",
            ]),
        )
    };
}

type WhyNames = ([&'static str; kept::WHOLES], [&'static str; kept::MADE_INPUTS], [&'static str; kept::ENTERED]);
const CASTER_WHY: WhyNames = why_names!("caster");
const LIT_WHY: WhyNames = why_names!("lit");

const CASTER_RECORD: RecordNames = RecordNames {
    draws: "caster_draws",
    figures: "caster_figures",
    walks: "caster_walks",
    kept: "caster_kept",
    spliced: "caster_spliced",
    splice_us: "caster_splice_us",
    splice_walk_us: "caster_splice_walk_us",
    targets_us: "caster_targets_us",
    plan_us: "caster_plan_us",
    rewrite_us: "caster_rewrite_us",
    apply_us: "caster_apply_us",
    keep_test_us: "caster_keep_test_us",
    keep_rewrite_us: "caster_keep_rewrite_us",
    entered: "caster_entered",
    left: "caster_left",
    rewritten: "caster_rewritten",
    rows_changed: "caster_rows_changed",
    row_bytes_changed: "caster_row_bytes_changed",
    live_rows: "caster_live_rows",
    reposed: "caster_reposed",
    staged: "caster_staged",
    staged_bytes: "caster_staged_bytes",
    relevelled: "caster_relevelled",
    flipped: "caster_groups_flipped",
    waiting: "caster_waiting",
    parts: "caster_parts",
    instanced_runs: "caster_instanced_runs",
    instanced_parts: "caster_instanced_parts",
    ordered: "caster_ordered",
    bands: "caster_bands",
    judged: "caster_judged",
    lods: ["caster_lod_0", "caster_lod_1", "caster_lod_2", "caster_lod_3", "caster_lod_4", "caster_lod_5_plus"],
    whole: CASTER_WHY.0,
    inputs: CASTER_WHY.1,
    entered_by: CASTER_WHY.2,
};

const LIT_RECORD: RecordNames = RecordNames {
    draws: "lit_draws",
    figures: "lit_figures",
    walks: "lit_walks",
    kept: "lit_kept",
    spliced: "lit_spliced",
    splice_us: "lit_splice_us",
    splice_walk_us: "lit_splice_walk_us",
    targets_us: "lit_targets_us",
    plan_us: "lit_plan_us",
    rewrite_us: "lit_rewrite_us",
    apply_us: "lit_apply_us",
    keep_test_us: "lit_keep_test_us",
    keep_rewrite_us: "lit_keep_rewrite_us",
    entered: "lit_entered",
    left: "lit_left",
    rewritten: "lit_rewritten",
    rows_changed: "lit_rows_changed",
    row_bytes_changed: "lit_row_bytes_changed",
    live_rows: "lit_live_rows",
    reposed: "lit_reposed",
    staged: "lit_staged",
    staged_bytes: "lit_staged_bytes",
    relevelled: "lit_relevelled",
    flipped: "lit_groups_flipped",
    waiting: "lit_waiting",
    parts: "lit_parts",
    instanced_runs: "lit_instanced_runs",
    instanced_parts: "lit_instanced_parts",
    ordered: "lit_ordered",
    bands: "lit_bands",
    judged: "lit_judged",
    lods: ["lit_lod_0", "lit_lod_1", "lit_lod_2", "lit_lod_3", "lit_lod_4", "lit_lod_5_plus"],
    whole: LIT_WHY.0,
    inputs: LIT_WHY.1,
    entered_by: LIT_WHY.2,
};

fn note_entered(label: &'static str, queue: Queue, entered: &[u64; kept::ENTERED]) {
    for (name, n) in record_names(queue).entered_by.iter().zip(entered) {
        if *n > 0 {
            viewer_runtime::view_record::add(label, name, *n);
        }
    }
}

#[expect(clippy::too_many_arguments, reason = "one line names the figure, its frame and why")]
fn note_culled(
    label: &str,
    mesh: &str,
    height: f32,
    instance: &ModelInstance,
    offset: f32,
    cull: Option<crate::GameCull>,
    why: crate::Culled,
    frame: u64,
) {
    static LIST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*LIST.get_or_init(|| std::env::var_os("BATTLEPLAN_CULL_LIST").or_else(|| std::env::var_os("HOI4_CULL_LIST")).is_some()) {
        return;
    }
    let Some(cull) = cull else {
        return;
    };
    let centre = [instance.position[0] + offset, instance.position[1] + height * instance.scale.abs() * 0.5, instance.position[2]];
    let distance = ((centre[0] - cull.eye[0]).powi(2) + (centre[1] - cull.eye[1]).powi(2) + (centre[2] - cull.eye[2]).powi(2)).sqrt();
    let why = match why {
        crate::Culled::Distance => "distance",
        crate::Culled::Planes => "planes",
    };
    eprintln!("culled by {why}: frame {frame} {label} {mesh} at {:.1},{:.1},{:.1}: {distance:.1}", centre[0], centre[1], centre[2]);
}

const ACTORS_POSED: &str = "actors_posed";

const WALK_POSE: usize = 0;
const WALK_USERDATA: usize = 1;
const WALK_PREPARE: usize = 2;
const WALK_CAPTURE: usize = 3;
const WALK_NODE: usize = 4;
const WALK_NODES: usize = 5;
const WALK_PARTS: usize = 6;
const WALK_STEPS: usize = 7;
const LIT_WALK_STEPS: [&str; WALK_STEPS] =
    ["lit_walk_pose_us", "lit_walk_userdata_us", "lit_walk_prepare_us", "lit_walk_capture_us", "lit_walk_node_us", "lit_walk_nodes", "lit_walk_parts"];
const CASTER_WALK_STEPS: [&str; WALK_STEPS] = [
    "caster_walk_pose_us",
    "caster_walk_userdata_us",
    "caster_walk_prepare_us",
    "caster_walk_capture_us",
    "caster_walk_node_us",
    "caster_walk_nodes",
    "caster_walk_parts",
];

fn part_programs<'r>(
    residents: &'r [Resident],
) -> impl Fn(usize) -> Option<kept::PartProgram<'r>> + 'r {
    move |key| {
        let resident = residents.get(key)?;
        let (_, program) = resident.program.as_ref()?;
        Some(kept::PartProgram {
            program: &**program,
            order: resident.order,
        })
    }
}

#[expect(clippy::too_many_arguments, reason = "one line names the part, its place and its order")]
fn note_part(
    key: usize,
    mesh: usize,
    node: usize,
    lod: usize,
    part: usize,
    blended: bool,
    selection: &Selection,
    source: &Part,
    order: Option<mesh_shader::Order>,
) {
    static LIST: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*LIST.get_or_init(|| std::env::var_os("BATTLEPLAN_ORDER_LIST").or_else(|| std::env::var_os("HOI4_ORDER_LIST")).is_some()) {
        return;
    }
    let technique = &selection.effect.source.techniques[selection.index].name;
    let shader = source.material.shader_file.as_deref().unwrap_or("");
    eprintln!(
        "order part: key {key} mesh {mesh} node {node} lod {lod} part {part} blended {blended}          technique {technique} shader {shader} diffuse {} order {order:?}",
        source.material.diffuse
    );
}

fn record_names(queue: Queue) -> &'static RecordNames {
    if queue == Queue::Caster { &CASTER_RECORD } else { &LIT_RECORD }
}

fn note_list(label: &'static str, queue: Queue, list: &mut kept::Kept) {
    let moved = list.take_moved();
    let wrote = list.take_wrote();
    if !viewer_runtime::view_record::on() {
        return;
    }
    use viewer_runtime::view_record::add;
    let names = record_names(queue);
    add(label, names.rows_changed, wrote.changed);
    add(label, names.row_bytes_changed, wrote.changed_bytes);
    add(label, names.live_rows, list.live().len() as u64);
    add(label, names.staged, wrote.staged);
    add(label, names.staged_bytes, wrote.staged_bytes);
    add(label, names.draws, list.draws.draws.iter().filter(|draw| draw.shown).count() as u64);
    add(label, names.figures, list.figures() as u64);
    add(label, names.entered, moved[0]);
    add(label, names.left, moved[1]);
    let waiting = list.waiting().map_or(0, |waiting| waiting.iter().filter(|waits| **waits).count());
    add(label, names.waiting, waiting as u64);
    let counts = list.part_counts();
    let blended = queue == Queue::Alternate;
    let drawn = list.draws.draws.len();
    if !list.order_free() {
        add(label, names.ordered, 1);
    }
    add(label, names.bands, list.bands() as u64);
    add(label, names.parts, counts.parts as u64);
    add(label, names.instanced_runs, if blended { drawn } else { counts.runs } as u64);
    add(label, names.instanced_parts, if blended { drawn } else { counts.in_views } as u64);
    for (name, nodes) in names.lods.iter().zip(list.lods()) {
        add(label, name, nodes);
    }
}

fn judge_list(label: &'static str, queue: Queue, list: &mut kept::Kept, residents: &[Resident]) {
    if !viewer_scene_base::actor_check::due(list.checks()) {
        return;
    }
    let what = if queue == Queue::Caster { "caster draws" } else { "lit draws" };
    list.judge(what, part_programs(residents));
    list.judge_staged(if queue == Queue::Caster { "caster staged" } else { "lit staged" });
    viewer_runtime::view_record::add(label, record_names(queue).judged, 1);
}

struct Walk<'a> {
    id: ListId,
    queue: Queue,
    shared: Vec<&'a [u8]>,
    reads: kept::Reads,
    incoming: Option<([u8; 208], Option<[u8; 208]>)>,
    userdata: Option<plan_tail::Caller<'a>>,
}

impl Walk<'_> {
    fn chain(&self) -> (usize, &[u8]) {
        match &self.incoming {
            Some((bytes, _)) if self.reads.own_pixel => (96, &bytes[96..]),
            Some((bytes, _)) => (64, &bytes[64..]),
            None => (0, &[]),
        }
    }

    fn made(
        &self,
        revision: u64,
        frame: &ModelFrame<'_>,
        samples: u32,
        textures: u64,
        globals: u64,
        cull: Option<crate::GameCull>,
    ) -> kept::Made {
        let (_, chain) = self.chain();
        kept::Made::new(
            revision,
            frame,
            &self.shared,
            self.reads,
            samples,
            textures,
            globals,
            chain,
            cull,
        )
    }
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct ResourceStamp {
    local: u64,
    globals: u64,
}

#[derive(Default)]
struct ResourceIdentities {
    local: Vec<Option<wgpu::TextureView>>,
    globals: Option<[wgpu::TextureView; 10]>,
    stamp: ResourceStamp,
    replaced: u64,
}

impl ResourceIdentities {
    fn observe(
        &mut self,
        local: &[Option<std::sync::Arc<Bound>>],
        globals: Option<[&wgpu::TextureView; 10]>,
    ) -> ResourceStamp {
        let kept = |old: &Option<wgpu::TextureView>, new: &Option<std::sync::Arc<Bound>>| match (old, new) {
            (None, _) => true,
            (Some(old), Some(new)) => *old == new.view,
            (Some(_), None) => false,
        };
        let unchanged = self.local.len() == local.len()
            && self
                .local
                .iter()
                .zip(local)
                .all(|(old, new)| old.is_some() == new.is_some() && kept(old, new));
        if !unchanged {
            if self.local.len() > local.len()
                || !self.local.iter().zip(local).all(|(old, new)| kept(old, new))
            {
                self.replaced += 1;
            }
            self.local = local
                .iter()
                .map(|texture| texture.as_ref().map(|texture| texture.view.clone()))
                .collect();
            self.stamp.local += 1;
        }
        if let Some(globals) = globals
            && self
                .globals
                .as_ref()
                .is_none_or(|last| !last.iter().eq(globals))
        {
            self.globals = Some(globals.map(Clone::clone));
            self.stamp.globals += 1;
        }
        ResourceStamp {
            local: self.stamp.local,
            globals: globals.map_or(0, |_| self.stamp.globals),
        }
    }

    fn replaced(&self) -> u64 {
        self.replaced
    }
}

type VertexKey = ((u64, bool), usize);

type BundleTarget = (wgpu::TextureFormat, u32);

fn lit_target(samples: u32) -> BundleTarget {
    (SCENE_FORMAT, samples)
}

const CASTER_TARGET: BundleTarget = (DEPTH_MAP_FORMAT, 1);

#[derive(Default)]
pub struct DrawList {
    draws: Vec<RecordedDraw>,
    vertices: BTreeMap<VertexKey, [Option<wgpu::Buffer>; 2]>,
    indices: BTreeMap<usize, wgpu::Buffer>,
    runs: Vec<bundles::Run<(wgpu::RenderBundle, BundleTarget, [u64; 4])>>,
    unplanned: bool,
}

fn draw_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

struct RecordedDraw {
    id: u64,
    shown: bool,
    bindings: shader_pipeline::gpu::Bindings,
    reference: u32,
    vertices: VertexKey,
    indices: usize,
    first: u32,
    count: u32,
    base: i32,
    instances: u32,
}

impl RecordedDraw {
    fn geometry(&self, view: usize) -> kept::DrawGeometry {
        kept::DrawGeometry {
            view,
            reference: self.reference,
            vertices: self.vertices,
            indices: self.indices,
            first: self.first,
            count: self.count,
            base: self.base,
        }
    }
}

impl DrawList {
    pub(crate) fn record(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.record_slice(pass, &self.draws);
    }

    fn record_slice(&self, pass: &mut wgpu::RenderPass<'_>, draws: &[RecordedDraw]) {
        if draws.is_empty() {
            return;
        }
        let mut previous: Option<&RecordedDraw> = None;
        let mut vertices = None;
        let mut indices = None;
        shader_pipeline::gpu::stats::state();
        pass.set_blend_constant(wgpu::Color::WHITE);
        for draw in draws.iter().filter(|draw| draw.shown) {
            if previous.is_none_or(|last| last.reference != draw.reference) {
                pass.set_stencil_reference(draw.reference);
            }
            if vertices != Some(draw.vertices) {
                for (slot, stream) in self.vertices[&draw.vertices].iter().enumerate() {
                    if let Some(stream) = stream {
                        shader_pipeline::gpu::stats::state();
                        pass.set_vertex_buffer(slot as u32, stream.slice(..));
                    }
                }
                vertices = Some(draw.vertices);
            }
            if indices != Some(draw.indices) {
                shader_pipeline::gpu::stats::state();
                pass.set_index_buffer(
                    self.indices[&draw.indices].slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                indices = Some(draw.indices);
            }
            draw.bindings
                .bind_after(pass, previous.map(|last| &last.bindings));
            shader_pipeline::gpu::stats::draw();
            pass.draw_indexed(draw.first..draw.first + draw.count, draw.base, 0..draw.instances);
            previous = Some(draw);
        }
    }

    fn encode(lists: &[&Self], targets: &Targets, encoder: &mut wgpu::CommandEncoder) {
        if lists.iter().all(|list| list.draws.is_empty()) {
            return;
        }
        let clock = viewer_runtime::profile_writer::profiling().then(std::time::Instant::now);
        {
            let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("native models"),
                color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
                depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
                ..Default::default()
            });
            for list in lists {
                list.record_range(&mut pass, 0..list.draws.len(), lit_target(targets.samples()));
            }
        }
        if let Some(clock) = clock {
            viewer_runtime::profiling::record_lap("native_encode_caller", clock.elapsed());
        }
    }

    fn clear(&mut self) {
        self.draws.clear();
        self.vertices.clear();
        self.indices.clear();
        self.runs.clear();
        self.unplanned = false;
    }

    fn record_range(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        range: std::ops::Range<usize>,
        target: BundleTarget,
    ) {
        if self.runs.is_empty() || range.is_empty() {
            self.record_slice(pass, &self.draws[range]);
            return;
        }
        use shader_pipeline::gpu::stats::{Count, add};
        let mut at = range.start;
        let mut blend = false;
        let first = self.runs.partition_point(|run| run.end() <= range.start);
        for run in &self.runs[first..] {
            if run.start >= range.end {
                break;
            }
            let (start, end) = (run.start.max(range.start), run.end().min(range.end));
            if at < start {
                self.record_slice(pass, &self.draws[at..start]);
            }
            match &run.bundle {
                Some((bundle, recorded, counts))
                    if *recorded == target && start == run.start && end == run.end() =>
                {
                    if !std::mem::replace(&mut blend, true) {
                        add(Count::States, 1);
                        pass.set_blend_constant(wgpu::Color::WHITE);
                    }
                    pass.set_stencil_reference(run.reference);
                    pass.execute_bundles(std::iter::once(bundle));
                    for (count, n) in [Count::Draws, Count::Pipelines, Count::BindGroups, Count::States]
                        .into_iter()
                        .zip(counts)
                    {
                        add(count, *n);
                    }
                }
                _ => self.record_slice(pass, &self.draws[start..end]),
            }
            at = end;
        }
        if at < range.end {
            self.record_slice(pass, &self.draws[at..range.end]);
        }
    }

    fn bundle(&mut self, device: &wgpu::Device, target: BundleTarget) {
        if std::mem::take(&mut self.unplanned) {
            let draws: Vec<(u64, u32)> =
                self.draws.iter().map(|draw| (draw.id, draw.reference)).collect();
            self.runs = bundles::plan(std::mem::take(&mut self.runs), &draws);
        }
        let Self {
            draws,
            vertices,
            indices,
            runs,
            ..
        } = self;
        for run in runs.iter_mut() {
            if run.bundle.as_ref().is_some_and(|(_, recorded, _)| *recorded == target) {
                continue;
            }
            run.still += 1;
            if run.still < bundles::STILL {
                continue;
            }
            let (bundle, counts) =
                record_bundle(device, target, &draws[run.start..run.end()], vertices, indices);
            run.bundle = Some((bundle, target, counts));
        }
    }
}

impl DrawList {
}

fn record_bundle(
    device: &wgpu::Device,
    (color, samples): BundleTarget,
    draws: &[RecordedDraw],
    vertices: &BTreeMap<VertexKey, [Option<wgpu::Buffer>; 2]>,
    indices: &BTreeMap<usize, wgpu::Buffer>,
) -> (wgpu::RenderBundle, [u64; 4]) {
    let drawn = || draws.iter().filter(|draw| draw.shown);
    let read_only = drawn().fold(shader_pipeline::gpu::ReadOnly::ALL, |all, draw| all.and(draw.bindings.read_only()));
    let mut bundle = device.create_render_bundle_encoder(&wgpu::RenderBundleEncoderDescriptor {
        label: Some("native model draws"),
        color_formats: &[Some(color)],
        depth_stencil: Some(read_only.bundle(DEPTH_FORMAT)),
        sample_count: samples,
        multiview: None,
    });
    let mut counts = [0; 4];
    let mut previous: Option<&RecordedDraw> = None;
    let (mut bound_vertices, mut bound_indices) = (None, None);
    for draw in drawn() {
        if bound_vertices != Some(draw.vertices) {
            for (slot, stream) in vertices[&draw.vertices].iter().enumerate() {
                if let Some(stream) = stream {
                    counts[3] += 1;
                    bundle.set_vertex_buffer(slot as u32, stream.slice(..));
                }
            }
            bound_vertices = Some(draw.vertices);
        }
        if bound_indices != Some(draw.indices) {
            counts[3] += 1;
            bundle.set_index_buffer(indices[&draw.indices].slice(..), wgpu::IndexFormat::Uint32);
            bound_indices = Some(draw.indices);
        }
        let [pipelines, groups] = draw
            .bindings
            .bundle_after(&mut bundle, previous.map(|last| &last.bindings));
        counts[1] += pipelines;
        counts[2] += groups;
        counts[0] += 1;
        bundle.draw_indexed(draw.first..draw.first + draw.count, draw.base, 0..draw.instances);
        previous = Some(draw);
    }
    let bundle = bundle.finish(&wgpu::RenderBundleDescriptor {
        label: Some("native model draws"),
    });
    (bundle, counts)
}

#[derive(Default)]
struct Pool {
    chunks: Vec<PoolChunk>,
}

struct PoolChunk {
    streams: [Option<wgpu::Buffer>; 2],
    capacity: u64,
    used: u64,
}

const POOL_RECORDS: u64 = 1 << 16;

impl Pool {
    fn place(
        &mut self,
        arena: &shader_pipeline::gpu::Arena,
        usage: wgpu::BufferUsages,
        strides: [u64; 2],
        streams: [Option<&[u8]>; 2],
        count: u64,
    ) -> io::Result<(usize, u64)> {
        let device = arena.device();
        let fits = |chunk: &PoolChunk| chunk.used + count <= chunk.capacity;
        if self.chunks.last().is_none_or(|chunk| !fits(chunk)) {
            let capacity = POOL_RECORDS.max(count);
            let mut chunk = PoolChunk {
                streams: [None, None],
                capacity,
                used: 0,
            };
            for (stream, (stride, bytes)) in
                chunk.streams.iter_mut().zip(strides.iter().zip(streams))
            {
                if bytes.is_none() {
                    continue;
                }
                let size = (capacity * stride).next_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT);
                if size > device.limits().max_buffer_size {
                    return Err(invalid(format!(
                        "native mesh pool: {size}-byte buffer exceeds GPU limit {}",
                        device.limits().max_buffer_size
                    )));
                }
                *stream = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("native mesh pool"),
                    size,
                    usage: usage | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            self.chunks.push(chunk);
        }
        let index = self.chunks.len() - 1;
        let chunk = &mut self.chunks[index];
        let first = chunk.used;
        for ((stream, stride), bytes) in chunk.streams.iter().zip(strides).zip(streams) {
            if let (Some(buffer), Some(bytes)) = (stream, bytes) {
                arena.write(buffer, first * stride, bytes);
            }
        }
        chunk.used += count;
        Ok((index, first))
    }
}

fn part_program(
    pipeline: &mesh_shader::Pass,
    geometry: &mesh_shader::Geometry,
) -> io::Result<(Arc<shader_pipeline::Compiled>, shader_scene::Program)> {
    let selection = &pipeline.selection;
    let compiled = selection
        .effect
        .compiled(selection.index)
        .map_err(invalid)?;
    let mut layouts = mesh_shader::vertex_layouts(
        &compiled.vertex.inputs,
        geometry.uv1,
        !geometry.skin.is_empty(),
    )?;
    layouts[0].stride = geometry.stride;
    let effect = &selection.effect.source;
    let technique = &effect.techniques[selection.index];
    let mut state = shader_pipeline::gpu::State::authored(effect, technique).map_err(invalid)?;
    state
        .samplers
        .retain(|key, _| (0..3).any(|slot| *key == binding(slot, Stage::Pixel)));
    state.samplers.extend(mesh_shader::map_samplers());
    let retained = shader_scene::Program::new(
        selection,
        &layouts,
        &state,
        &effect.rasterizer(technique),
        &effect.depth_stencil(technique),
    );
    Ok((compiled, retained))
}

fn material_effect(
    library: &mut Library,
    part: &Part,
) -> io::Result<Arc<shader_pipeline::library::Effect>> {
    let defines = if part.geometry.uv1 {
        vec!["PDX_MESH_UV1".to_owned()]
    } else {
        Vec::new()
    };
    let file = part.material.shader_file.as_deref();
    library
        .load(file.unwrap_or("gfx/FX/pdxmesh.lua").as_bytes(), &defines)
        .map_err(invalid)
}

fn selection_suffix(skinned: bool, caster: bool) -> &'static str {
    match (skinned, caster) {
        (false, false) => "",
        (true, false) => "Skinned",
        (false, true) => "Shadow",
        (true, true) => "SkinnedShadow",
    }
}

fn material_flags(
    lit: &content_reader_gui::pdxshader::BlendState,
    shadow: &content_reader_gui::pdxshader::BlendState,
    diffuse: bool,
) -> (bool, bool) {
    let mut textured = diffuse && (lit.enable || lit.alpha_test);
    let mut alternate = lit.enable;
    if !textured && (shadow.enable || shadow.alpha_test) {
        alternate = true;
        textured = diffuse;
    }
    (textured, alternate)
}

fn prepare_span(label: &str, queue: Queue) -> &'static str {
    let caster = queue == Queue::Caster;
    match label {
        "unit models" if caster => "ncp/units",
        "unit models" => "nlp/units",
        "map buildings" if caster => "ncp/buildings",
        "map buildings" => "nlp/buildings",
        "city sprawl" if caster => "ncp/sprawl",
        "city sprawl" => "nlp/sprawl",
        "map frame" if caster => "ncp/map_frame",
        "map frame" => "nlp/map_frame",
        "flying aircraft" if caster => "ncp/aircraft",
        "flying aircraft" => "nlp/aircraft",
        "supply trains" if caster => "ncp/trains",
        "supply trains" => "nlp/trains",
        "railway bridges" if caster => "ncp/bridges",
        "railway bridges" => "nlp/bridges",
        _ if caster => "ncp/other",
        _ => "nlp/other",
    }
}

fn matrix(transform: &mesh::Transform) -> [[f32; 4]; 4] {
    let [a, b, c] = transform.basis;
    let t = transform.translation;
    [
        [a[0], a[1], a[2], 0.],
        [b[0], b[1], b[2], 0.],
        [c[0], c[1], c[2], 0.],
        [t[0], t[1], t[2], 1.],
    ]
}

impl ModelPass {
    pub fn settle_builds(&self, settled: bool) {
        self.settled.set(settled);
    }

    pub fn reset_drawn(&self) {
        self.drawn.set(0);
    }

    #[expect(clippy::too_many_arguments, reason = "the arena, encoder and targets, plus the frame, inputs, queue and constants of one draw")]
    pub fn draw_native(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        encoder: &mut wgpu::CommandEncoder,
        targets: &Targets,
        frame: ModelFrame<'_>,
        inputs: Inputs<'_>,
        queue: Queue,
        object: &mut ObjectConstants,
    ) -> io::Result<()> {
        if self.field.native_order.is_empty() {
            return Ok(());
        }
        let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(self.label),
            color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
            depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        self.record_native(arena, &mut pass, frame, inputs, queue, object)
    }

    pub(crate) fn record_native(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        pass: &mut wgpu::RenderPass<'_>,
        frame: ModelFrame<'_>,
        inputs: Inputs<'_>,
        queue: Queue,
        object: &mut ObjectConstants,
    ) -> io::Result<()> {
        let draws = self.prepare_native(arena, frame, inputs, queue, object)?;
        {
            let wall_profile = viewer_runtime::profile_writer::profiling();
            let clock = wall_profile.then(std::time::Instant::now);
            draws.record(pass);
            let caller = clock.map_or(std::time::Duration::ZERO, |clock| clock.elapsed());

            if wall_profile {
                viewer_runtime::profiling::record_lap("native_encode_caller", caller);
                viewer_runtime::profiling::record_lap(
                    "native_turns_finish_wait",
                    std::time::Duration::ZERO,
                );
            }
        }
        self.recycle_native(draws);
        Ok(())
    }

    pub(crate) fn record_casters(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        pass: &mut wgpu::RenderPass<'_>,
    ) {
        let mut renderer = self.native.borrow_mut();
        if let Some(native) = renderer.as_mut()
            && !self.field.native_order.is_empty()
        {
            let draws = &mut native.casters.draws;
            draws.bundle(arena.device(), CASTER_TARGET);
            draws.record_range(pass, 0..draws.draws.len(), CASTER_TARGET);
        }
    }

    pub(crate) fn prepare_casters(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frame: ModelFrame<'_>,
        inputs: Inputs<'_>,
    ) -> io::Result<bool> {
        let _wall = viewer_runtime::profiling::span("native_caster_prepare");
        let _pass = viewer_runtime::profiling::span(prepare_span(self.label, Queue::Caster));
        if self.field.native_order.is_empty() {
            return Ok(!self.cast.replace(false));
        }
        let walk = Walk {
            id: ListId::Casters,
            queue: Queue::Caster,
            shared: inputs.frames.to_vec(),
            reads: kept::Reads {
                views: frame.offsets.len(),
                ..Default::default()
            },
            incoming: None,
            userdata: None,
        };
        let cast = self.cast.replace(false);
        self.prepare_list(arena, frame, inputs, &walk, None)?;
        self.cast.set(true);
        Ok(cast && self.native.borrow().as_ref().is_some_and(|native| native.casters.repeated()))
    }

    pub fn prepare_kept(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frame: ModelFrame<'_>,
        inputs: Inputs<'_>,
        queue: Queue,
        object: &mut ObjectConstants,
    ) -> io::Result<DrawList> {
        let Some(caller) = self.kept_lit(queue) else {
            if let Some(native) = self.native.borrow_mut().as_mut() {
                native.lent = None;
            }
            let draws = self.prepare_native(arena, frame, inputs, queue, object)?;
            viewer_runtime::view_record::add(self.label, LIT_RECORD.draws, draws.draws.len() as u64);
            return Ok(draws);
        };
        let ordinal = {
            let mut renderer = self.native.borrow_mut();
            if renderer.is_none() {
                *renderer = Some(self.new_renderer([frame.camera.map.width, frame.camera.map.height])?);
            }
            let native = renderer.as_mut().expect("made above");
            native.lent = None;
            native.lit_call(arena.frame(), usize::from(queue == Queue::Alternate))
        };
        let _wall = viewer_runtime::profiling::span("native_lit_prepare");
        let _pass = viewer_runtime::profiling::span(prepare_span(self.label, queue));
        let own = !matches!(caller, plan_tail::Caller::Unchanged);
        let pixel = if own { None } else { object.pixel };
        let mut shared = inputs.frames.to_vec();
        shared.push(inputs.grid);
        if let Some(pixel) = &pixel {
            shared.push(pixel);
        }
        let queue_at = usize::from(queue == Queue::Alternate);
        let walk = Walk {
            id: ListId::Lit(queue_at, ordinal),
            queue,
            shared,
            reads: kept::Reads {
                views: frame.offsets.len(),
                grid: true,
                pixel: pixel.is_some(),
                own_pixel: own,
            },
            incoming: Some((object.bytes, object.pixel)),
            userdata: own.then_some(caller),
        };
        self.prepare_list(arena, frame, inputs, &walk, Some(object))?;
        let mut renderer = self.native.borrow_mut();
        let native = renderer.as_mut().expect("made by the list");
        native.lent = Some((queue_at, ordinal));
        Ok(std::mem::take(&mut native.list(walk.id).draws))
    }

    fn instanced(&self, caster: bool) -> u32 {
        let caller = plan_tail::Caller::select(self.label);
        if !caster && (self.direct || matches!(caller, plan_tail::Caller::Unavailable(_))) {
            return 0;
        }
        let own = !caster && !matches!(caller, plan_tail::Caller::Unchanged);
        (1 << binding(1, Stage::Vertex))
            | (1 << binding(2, Stage::Vertex))
            | if own { 1 << binding(1, Stage::Pixel) } else { 0 }
    }

    fn instanced_masks(&self) -> [u32; 2] {
        [self.instanced(false), self.instanced(true)]
    }

    fn kept_lit(&self, queue: Queue) -> Option<plan_tail::Caller<'static>> {
        let caller = plan_tail::Caller::select(self.label);
        (queue != Queue::Caster
            && !matches!(caller, plan_tail::Caller::Unavailable(_))
            && !self.field.native_order.is_empty())
        .then_some(caller)
    }

    pub fn recycle_kept(&self, draws: DrawList) {
        let mut renderer = self.native.borrow_mut();
        if let Some(native) = renderer.as_mut()
            && let Some((queue, at)) = native.lent.take()
        {
            native.list(ListId::Lit(queue, at)).draws = draws;
            return;
        }
        drop(renderer);
        self.recycle_native(draws);
    }

    fn prepare_list(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frame: ModelFrame<'_>,
        inputs: Inputs<'_>,
        walk: &Walk<'_>,
        mut object: Option<&mut ObjectConstants>,
    ) -> io::Result<()> {
        let judged = viewer_scene_base::actor_check::every().is_some();
        if judged {
            self.judge_view(&frame, walk.queue);
        }
        if self.keep_list(arena, &frame, &inputs, walk, object.as_deref_mut())? {
            if let Some(native) = self.native.borrow_mut().as_mut() {
                viewer_runtime::view_record::add(self.label, record_names(walk.queue).kept, 1);
                let Renderer { casters, lit, residents, admission, queue_slots, .. } = native;
                let list = list_of(casters, lit, walk.id);
                list.chosen_by(self.drawn_rule(&frame));
                note_list(self.label, walk.queue, list);
                judge_list(self.label, walk.queue, list, residents);
                if judged {
                    self.judge_drawn(&frame, walk.queue, list, admission, queue_slots);
                }
            }
            return Ok(());
        }
        let mut recording = kept::Recording::default();
        let refused = self.refused.get();
        let mut alone = ObjectConstants::default();
        let draws = self.walk_native(
            arena,
            frame,
            inputs,
            walk.queue,
            object.as_deref_mut().unwrap_or(&mut alone),
            Some(&mut recording),
        )?;
        let mut renderer = self.native.borrow_mut();
        let native = renderer.as_mut().expect("made by the walk");
        let (textures, globals) = self.resources(native, walk, &inputs);
        let made = (self.refused.get() == refused).then(|| {
            walk.made(self.revision, &frame, inputs.samples, textures, globals, self.cull.get())
        });
        let Renderer {
            casters,
            lit,
            residents,
            admission,
            queue_slots,
            ..
        } = native;
        let list = list_of(casters, lit, walk.id);
        list.finish(arena, recording, draws, &walk.shared, walk.reads, part_programs(residents), made)?;
        list.levelled_at(frame.camera.eye());
        list.chosen_by(self.drawn_rule(&frame));
        list.unmade_for(if self.refused.get() != refused { kept::Whole::Refused } else { kept::Whole::Waiting });
        list.read(arena.frame());
        if walk.queue != Queue::Caster {
            list.replay = Some(list.replay(|key| residents[key].binds()));
        }
        viewer_runtime::view_record::add(self.label, record_names(walk.queue).walks, 1);
        note_list(self.label, walk.queue, list);
        judge_list(self.label, walk.queue, list, residents);
        if judged {
            self.judge_drawn(&frame, walk.queue, list, admission, queue_slots);
        }
        Ok(())
    }

    fn resources(&self, native: &mut Renderer, walk: &Walk<'_>, inputs: &Inputs<'_>) -> (u64, u64) {
        let globals = (walk.queue != Queue::Caster).then_some(inputs.textures);
        let stamp = native.resource_ids.observe(&self.textures, globals);
        (native.resource_ids.replaced(), stamp.globals)
    }

    fn keep_list(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frame: &ModelFrame<'_>,
        inputs: &Inputs<'_>,
        walk: &Walk<'_>,
        object: Option<&mut ObjectConstants>,
    ) -> io::Result<bool> {
        let began = viewer_runtime::view_record::on().then(std::time::Instant::now);
        let change = self.frame_change(arena.frame());
        let mut renderer = self.native.borrow_mut();
        let Some(native) = renderer.as_mut() else {
            return Ok(false);
        };
        let (textures, globals) = self.resources(native, walk, inputs);
        let made = walk
            .made(self.revision, frame, inputs.samples, textures, globals, self.cull.get());
        let names = record_names(walk.queue);
        let list = native.list(walk.id);
        if let Some(stale) = list.stale(&made, change.since) {
            let why = match stale {
                kept::Stale::Whole(why) => names.whole[why as usize],
                kept::Stale::Input(at) => names.inputs[at],
            };
            viewer_runtime::view_record::add(self.label, why, 1);
            list.forget();
            return Ok(false);
        }
        let groups = self.field.groups.len();
        let flips_at = |group: usize| {
            self.field.groups.get(group).is_none_or(|bounds| {
                frame.offsets.iter().enumerate().any(|(view, offset)| {
                    visible(bounds, *offset, frame.visible) != list.visited(view, group, groups)
                })
            })
        };
        let flips: Vec<usize> = change.changed.groups.iter().copied().filter(|&group| flips_at(group)).collect();
        let redrawn = list.drawn_by() != Some(self.drawn_rule(frame)) || !change.changed.groups.is_empty();
        let crossed = if redrawn {
            self.cull_crossings(frame, list, None)
        } else if self.seen.get().is_some() && !change.changed.instances.is_empty() {
            self.cull_crossings(frame, list, Some(&change.changed.instances[..]))
        } else {
            0
        };
        viewer_runtime::view_record::add(self.label, "cull_crossed_still", crossed);
        let flipped = !flips.is_empty() || crossed > 0;
        let waiting = list.waiting();
        let mut demoted = waiting.unwrap_or_else(|| vec![false; list.figures()]);
        let relevelled = self.relevel(list, frame, &mut demoted);
        viewer_runtime::view_record::add(self.label, names.relevelled, relevelled);
        viewer_runtime::view_record::add(self.label, record_names(walk.queue).flipped, flips.len() as u64);
        if change.change != crate::Change::Whole
            && change.born.is_empty()
            && !flipped
            && !demoted.contains(&true)
        {
            let mut units: Vec<usize> = change
                .changed
                .instances
                .iter()
                .flat_map(|&instance| list.units_of(instance))
                .collect();
            if self.pose_epoch != list.posed() {
                units.extend_from_slice(list.live());
                viewer_runtime::view_record::add(self.label, names.reposed, 1);
            }
            units.sort_unstable();
            units.dedup();
            let tested = began.map(|_| std::time::Instant::now());
            let failed = self.rewrite(arena, native, walk, frame, &units);
            if failed.is_empty() {
                self.kept(arena, native, walk, inputs, object);
                if let (Some(began), Some(tested)) = (began, tested) {
                    use viewer_runtime::view_record::add;
                    add(self.label, names.keep_test_us, (tested - began).as_micros() as u64);
                    add(self.label, names.keep_rewrite_us, tested.elapsed().as_micros() as u64);
                }
                return Ok(true);
            }
            let list = native.list(walk.id);
            demoted = vec![false; list.figures()];
            for unit in failed {
                demoted[list.figure_of(unit)] = true;
            }
        }
        drop(renderer);
        let spliced = self.splice_list(arena, frame, inputs, walk, &change, &demoted);
        let mut renderer = self.native.borrow_mut();
        let native = renderer.as_mut().expect("kept above");
        if !matches!(spliced, Ok(true)) {
            let list = native.list(walk.id);
            let why = match spliced {
                Ok(_) => list.dropped().unwrap_or(kept::Whole::Error),
                Err(_) => kept::Whole::Error,
            };
            viewer_runtime::view_record::add(self.label, names.whole[why as usize], 1);
            list.forget_writes();
            list.forget();
            return spliced;
        }
        self.kept(arena, native, walk, inputs, object);
        Ok(true)
    }

    fn judge_view(&self, frame: &ModelFrame<'_>, queue: Queue) {
        let what = if queue == Queue::Caster { "caster view" } else { "lit view" };
        let mut judged = 0;
        for (view, offset) in frame.offsets.iter().enumerate() {
            for (at, group) in self.field.groups.iter().enumerate() {
                judged += 1;
                if visible(group, *offset, frame.seen) && !visible(group, *offset, frame.visible) {
                    viewer_scene_base::actor_check::differs(what, &(view, at), &"a group in view the gathering left out");
                }
            }
        }
        viewer_scene_base::actor_check::checked(what, judged);
    }

    fn relevel(&self, list: &mut kept::Kept, frame: &ModelFrame<'_>, demoted: &mut [bool]) -> u64 {
        let eye = frame.camera.eye();
        if list.lods_for() == Some(eye) {
            return 0;
        }
        let mut changed = 0;
        for at in 0..list.units().len() {
            let unit = list.unit(at);
            let level = self
                .field
                .groups
                .get(unit.group)
                .and_then(|group| self.meshes.get(group.mesh))
                .and_then(|mesh| mesh.native.get(unit.descriptor))
                .map(|descriptor| descriptor.lod(unit.at, eye));
            if level.map(|level| level.ok()) != Some(Some(unit.lod)) {
                demoted[list.figure_of(at)] = true;
                changed += 1;
            }
        }
        list.levelled_at(eye);
        changed
    }

    fn drawn_rule(&self, frame: &ModelFrame<'_>) -> kept::Drawn {
        (self.seen.get(), frame.seen.map(|ground| ground.map(f32::to_bits)))
    }

    fn judge_drawn(
        &self,
        frame: &ModelFrame<'_>,
        queue: Queue,
        list: &kept::Kept,
        admission: &mut admission::Admission,
        queue_slots: &[Vec<Option<[bool; 2]>>],
    ) {
        let what = if queue == Queue::Caster { "caster drawn" } else { "lit drawn" };
        let route = |position: usize| self.field.native_order.get(position).copied();
        let extras = list
            .places()
            .filter(|&(view, position)| {
                route(position).is_none_or(|(group, index)| {
                    frame.offsets.get(view).is_none_or(|offset| !self.camera_draws(frame, group, index, *offset))
                })
            })
            .count();
        let mut missing = 0;
        let mut judged = list.figures();
        for (view, offset) in frame.offsets.iter().enumerate() {
            for (position, (group, index, row)) in self.native_walk.routes(&self.field).enumerate() {
                if !visible(&self.field.groups[group], *offset, frame.seen)
                    || self.passed_over(admission, queue_slots, queue, group, index, row)
                    || !self.camera_draws(frame, group, index, *offset)
                {
                    continue;
                }
                judged += 1;
                if !list.holds(view, position) {
                    missing += 1;
                }
            }
        }
        if extras > 0 {
            viewer_scene_base::actor_check::differs(what, &extras, &"figures held that the frame's camera does not draw");
        }
        if missing > 0 {
            viewer_scene_base::actor_check::differs(what, &missing, &"figures the frame's camera draws that the list left out");
        }
        viewer_scene_base::actor_check::checked(what, judged as u64);
    }

    fn passed_over(
        &self,
        admission: &mut admission::Admission,
        queue_slots: &[Vec<Option<[bool; 2]>>],
        queue: Queue,
        group: usize,
        index: usize,
        row: Option<&walk::Row>,
    ) -> bool {
        let mesh = self.field.groups[group].mesh;
        let forced = self.label == "supply trains";
        let nodes = self.meshes[mesh].native.iter().map(|descriptor| descriptor.node);
        if admission
            .of(mesh, nodes, queue_slots.get(mesh).map_or(&[][..], Vec::as_slice))
            .is_some_and(|admits| admission::passes_over(queue, forced, admits))
        {
            return true;
        }
        let (seed, identity) = self.field.instances[index].animation_key();
        row.map_or_else(|| self.hidden.contains(&(mesh, seed, identity)), |row| row.hidden)
    }

    fn camera_draws(&self, frame: &ModelFrame<'_>, group: usize, index: usize, offset: f32) -> bool {
        let bounds = &self.field.groups[group];
        visible(bounds, offset, frame.seen)
            && self.game_culled(bounds.mesh, &self.field.instances[index], offset, &frame.camera).is_none()
    }

    fn cull_crossings(&self, frame: &ModelFrame<'_>, list: &kept::Kept, changed: Option<&[usize]>) -> u64 {
        let groups = self.field.groups.len();
        let crossed = |view: usize, position: usize| -> bool {
            let (Some(offset), Some(&(group, index))) = (frame.offsets.get(view), self.field.native_order.get(position)) else {
                return false;
            };
            let drawn = self.camera_draws(frame, group, index, *offset);
            if list.holds(view, position) {
                !drawn
            } else {
                drawn && list.was_culled(view, kept::figure_key(self.field.groups[group].mesh, &self.field.instances[index]))
            }
        };
        let count = match changed {
            Some(changed) => self
                .native_walk
                .routes(&self.field)
                .enumerate()
                .filter(|(_, (_, index, _))| changed.binary_search(index).is_ok())
                .flat_map(|(position, (group, _, _))| {
                    (0..frame.offsets.len())
                        .filter(move |&view| list.visited(view, group, groups))
                        .map(move |view| (view, position))
                })
                .filter(|&(view, position)| crossed(view, position))
                .count(),
            None => list
                .gathered()
                .iter()
                .flat_map(|&at| {
                    let (view, group) = (at / groups.max(1), at % groups.max(1));
                    self.native_walk.by_group.get(group).into_iter().flatten().map(move |&position| (view, position))
                })
                .filter(|&(view, position)| crossed(view, position))
                .count(),
        };
        count as u64
    }

    fn kept(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        native: &mut Renderer,
        walk: &Walk<'_>,
        inputs: &Inputs<'_>,
        object: Option<&mut ObjectConstants>,
    ) {
        let list = native.list(walk.id);
        list.keep(arena, &walk.shared);
        list.read(arena.frame());
        list.posed_for(self.pose_epoch);
        if let Some(object) = object {
            self.replay_into(native, walk, inputs, object, true);
        }
    }

    fn replay_into(
        &self,
        native: &mut Renderer,
        walk: &Walk<'_>,
        inputs: &Inputs<'_>,
        object: &mut ObjectConstants,
        count: bool,
    ) {
        let Renderer {
            casters,
            lit,
            residents,
            ..
        } = native;
        let list = list_of(casters, lit, walk.id);
        let Some(replay) = list.replay.as_ref() else {
            return;
        };
        let pixel = walk.incoming.and_then(|(_, pixel)| pixel);
        if replay.sources.state.part.is_some() {
            let buffers = replay.sources.resolve_with(|source| match source {
                plans::Source::Frame(view) => inputs.frames[view],
                plans::Source::Grid => inputs.grid,
                plans::Source::Object(kept::SHARED_PIXEL) => {
                    pixel.as_ref().map_or(&[][..], |pixel| &pixel[..])
                }
                plans::Source::Object(unit) => list.vertex_bytes(unit).unwrap_or(&[]),
                plans::Source::Skin(unit, _) => list.skin_bytes(unit).unwrap_or(&[]),
            });
            let mut textures: Textures = [None; TEXTURE_BINDINGS];
            for (i, texture) in inputs.textures.iter().enumerate() {
                textures[binding(i as u32 + 4, Stage::Pixel) as usize] = Some(*texture);
            }
            if let Some(material) = replay.material {
                let material = &self.materials[material];
                for (slot, index) in [material.diffuse, material.specular, material.normal]
                    .into_iter()
                    .enumerate()
                {
                    textures[binding(slot as u32, Stage::Pixel) as usize] =
                        self.textures[index].as_ref().map(|texture| &texture.view);
                }
            }
            replay.sources.state.apply_sources(
                |key| &residents[key],
                textures,
                buffers,
                &mut object.retained,
            );
        }
        if let Some(last) = replay.last.and_then(|last| list.vertex_bytes(last))
            && last.len() == object.bytes.len()
        {
            object.bytes.copy_from_slice(last);
            if walk.reads.own_pixel {
                object.pixel = Some(object.bytes);
            }
        }
        if count {
            self.drawn.set(self.drawn.get() + replay.drawn);
        }
    }

    fn splice_list(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frame: &ModelFrame<'_>,
        inputs: &Inputs<'_>,
        walk: &Walk<'_>,
        change: &crate::FrameChange,
        demoted: &[bool],
    ) -> io::Result<bool> {
        let began = std::time::Instant::now();
        let layout = &self.native_walk;
        let groups = self.field.groups.len();
        let mut visited = vec![false; frame.offsets.len() * groups];
        for (view, offset) in frame.offsets.iter().enumerate() {
            for (group, bounds) in self.field.groups.iter().enumerate() {
                visited[view * groups + group] = visible(bounds, *offset, frame.visible);
            }
        }
        let mut renderer = self.native.borrow_mut();
        let native = renderer.as_mut().expect("kept by the caller");
        let mut targets = Vec::new();
        let mut culled = Vec::new();
        for view in 0..frame.offsets.len() {
            for (position, (group, index, row)) in layout.routes(&self.field).enumerate() {
                if !visited[view * groups + group] {
                    continue;
                }
                let mesh = self.field.groups[group].mesh;
                if self.passed_over(&mut native.admission, &native.queue_slots, walk.queue, group, index, row) {
                    continue;
                }
                let instance = self.field.instances[index];
                let (seed, identity) = instance.animation_key();
                if !self.camera_draws(frame, group, index, frame.offsets[view]) {
                    culled.push((view, kept::figure_key(mesh, &instance)));
                    continue;
                }
                let player = match row.filter(|_| layout.installed) {
                    Some(row) => layout.players[row.player],
                    None => (self.drawn_slot(mesh), seed, identity),
                };
                targets.push(kept::Target {
                    view,
                    position,
                    group,
                    index,
                    row: row.copied(),
                    key: kept::figure_key(mesh, &instance),
                    born: change.born.binary_search(&player).is_ok(),
                });
            }
        }
        let targets_ended = std::time::Instant::now();
        let list = native.list(walk.id);
        let mut entered = viewer_runtime::view_record::on().then_some([0u64; kept::ENTERED]);
        let mut next = list.plan_counting(&targets, demoted, entered.as_mut());
        if next.iter().filter(|figure| figure.is_none()).count() * 2 > targets.len() {
            list.drop_for(kept::Whole::MostWalked);
            if let Some(entered) = &entered {
                note_entered(self.label, walk.queue, entered);
            }
            return Ok(false);
        }
        let mut units = Vec::new();
        for (target, figure) in targets.iter().zip(&next) {
            if let Some(figure) = *figure {
                list.follow(figure, target);
                if list.figure_live(figure)
                    || change.changed.instances.binary_search(&target.index).is_ok()
                {
                    units.extend(list.units_of_figure(figure));
                }
            }
        }
        let plan_ended = std::time::Instant::now();
        let failed = self.rewrite(arena, native, walk, frame, &units);
        let rewrite_ended = std::time::Instant::now();
        if !failed.is_empty() {
            let list = native.list(walk.id);
            let mut demote = vec![false; list.figures()];
            for unit in failed {
                demote[list.figure_of(unit)] = true;
            }
            for figure in &mut next {
                if figure.is_some_and(|figure| demote[figure]) {
                    *figure = None;
                    if let Some(entered) = &mut entered {
                        entered[kept::Entered::Rewrite as usize] += 1;
                    }
                }
            }
        }
        let only: Vec<(usize, usize)> = targets
            .iter()
            .zip(&next)
            .filter(|(_, figure)| figure.is_none())
            .map(|(target, _)| (target.view, target.position))
            .collect();
        drop(renderer);
        let refused = self.refused.get();
        let any = !only.is_empty();
        let mut recording = kept::Recording::of(only);
        let mut alone = ObjectConstants::default();
        if let Some((bytes, pixel)) = walk.incoming {
            (alone.bytes, alone.pixel) = (bytes, pixel);
        }
        let walk_began = std::time::Instant::now();
        let mut walked = if any {
            Some(self.walk_native(
                arena,
                *frame,
                *inputs,
                walk.queue,
                &mut alone,
                Some(&mut recording),
            )?)
        } else {
            None
        };
        let walk_ended = std::time::Instant::now();
        let mut renderer = self.native.borrow_mut();
        let native = renderer.as_mut().expect("kept by the caller");
        let Renderer {
            casters,
            lit,
            residents,
            ..
        } = native;
        let list = list_of(casters, lit, walk.id);
        if let Some(draws) = &mut walked {
            list.draws.vertices.append(&mut draws.vertices);
            list.draws.indices.append(&mut draws.indices);
        }
        let alignment = kept::block_alignment(arena);
        let spliced =
            list.splice(arena, &next, &recording, visited, alignment, part_programs(residents))?;
        if spliced {
            list.set_culled(culled);
        }
        if spliced {
            if self.refused.get() != refused {
                list.drop_for(kept::Whole::Refused);
            } else if recording.incomplete() {
                list.drop_for(kept::Whole::Waiting);
            }
            if walk.queue != Queue::Caster {
                list.replay = Some(list.replay(|key| residents[key].binds()));
            }
        }
        drop(renderer);
        if let Some(draws) = walked {
            self.recycle_native(draws);
        }
        if let Some(entered) = &entered {
            note_entered(self.label, walk.queue, entered);
        }
        if spliced {
            use viewer_runtime::view_record::add;
            let names = record_names(walk.queue);
            let own = (walk_began - began) + walk_ended.elapsed();
            add(self.label, names.spliced, 1);
            add(self.label, names.splice_us, own.as_micros() as u64);
            add(self.label, names.splice_walk_us, (walk_ended - walk_began).as_micros() as u64);
            add(self.label, names.targets_us, (targets_ended - began).as_micros() as u64);
            add(self.label, names.plan_us, (plan_ended - targets_ended).as_micros() as u64);
            add(self.label, names.rewrite_us, (rewrite_ended - plan_ended).as_micros() as u64);
            add(self.label, names.apply_us, walk_ended.elapsed().as_micros() as u64);
        }
        Ok(spliced)
    }

    fn rewrite(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        native: &mut Renderer,
        walk: &Walk<'_>,
        frame: &ModelFrame<'_>,
        units: &[usize],
    ) -> Vec<usize> {
        let mut failed = Vec::new();
        if units.is_empty() {
            return failed;
        }
        crate::count(crate::Count::CasterRewritten, units.len() as u64);
        viewer_runtime::view_record::add(self.label, record_names(walk.queue).rewritten, units.len() as u64);
        let mut posed = 0u64;
        let layout = &self.native_walk;
        let Renderer {
            casters,
            lit,
            frame_handles,
            ..
        } = native;
        let list = list_of(casters, lit, walk.id);
        let dense_handles = layout.installed && frame_handles.begin(layout.players.len());
        let mut poses = self.poses.borrow_mut();
        if poses.0 != arena.frame() {
            *poses = (arena.frame(), Vec::new());
        }
        poses.1.resize_with(self.field.instances.len(), || None);
        let eye = frame.camera.eye();
        for &at in units {
            let unit = list.unit(at);
            let (Some(group), Some(&instance)) = (
                self.field.groups.get(unit.group),
                self.field.instances.get(unit.index),
            ) else {
                failed.push(at);
                continue;
            };
            let Some(descriptor) = self
                .meshes
                .get(group.mesh)
                .and_then(|mesh| mesh.native.get(unit.descriptor))
            else {
                failed.push(at);
                continue;
            };
            let (Ok(instance_world), Some(rig)) =
                (instance.native_transform(), self.rigs.get(group.mesh))
            else {
                failed.push(at);
                continue;
            };
            let actors = poses.1[unit.index]
                .get_or_insert_with(|| {
                    posed += 1;
                    let frozen = !matches!(
                        instance.animation,
                        entity_animation::STOPPED | entity_animation::DEFAULT_STATE
                    );
                    if dense_handles && let Some(row) = unit.row {
                        frame_handles.resolve(row.player, row.mesh, frozen, || {
                            self.actors(
                                group.mesh,
                                layout.players[row.player],
                                rig,
                                self.players.get(&layout.players[row.player]),
                                frozen,
                            )
                        })
                    } else {
                        self.actors(
                            group.mesh,
                            self.player_key(group.mesh, &instance),
                            rig,
                            self.player(group.mesh, &instance),
                            frozen,
                        )
                    }
                })
                .clone();
            let Some(actor) = actors.get(descriptor.node) else {
                failed.push(at);
                continue;
            };
            let Ok(actor_world) = actor.world.as_ref() else {
                failed.push(at);
                continue;
            };
            let world = mesh_pose::multiply(matrix(&instance_world), *actor_world);
            let offset = frame.offsets[unit.view];
            let position = [world[3][0] + offset, world[3][1], world[3][2]];
            if descriptor.lod(position, eye).ok() != Some(unit.lod) {
                failed.push(at);
                continue;
            }
            if unit.lod.is_none() || unit.vertex.is_none() {
                continue;
            }
            let userdata = match walk.userdata {
                None => None,
                Some(caller) => match rig.nodes[0]
                    .native
                    .as_ref()
                    .map(|root| caller.evaluate(root, instance, frame.alpha))
                {
                    Some(Ok(userdata)) => userdata,
                    _ => {
                        failed.push(at);
                        continue;
                    }
                },
            };
            let bones = match (&unit.skin, &actor.pose) {
                (None, _) => None,
                (Some(_), Ok(pose)) if pose.bones.len() >= unit.bones => {
                    Some(bytemuck::cast_slice(&pose.bones))
                }
                _ => {
                    failed.push(at);
                    continue;
                }
            };
            if !list.write_unit(
                at,
                kept::LiveBytes {
                    world: &world,
                    userdata,
                    bones,
                },
            ) {
                failed.push(at);
            }
        }
        viewer_runtime::view_record::add(self.label, ACTORS_POSED, posed);
        failed
    }

    pub(crate) fn recycle_native(&self, mut draws: DrawList) {
        draws.clear();
        if let Some(native) = self.native.borrow_mut().as_mut() {
            native.draws = draws;
        }
    }

    pub fn encode_native(
        lists: &[(&Self, DrawList)],
        targets: &Targets,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        DrawList::encode(
            &lists.iter().map(|(_, list)| list).collect::<Vec<_>>(),
            targets,
            encoder,
        );
    }

    fn new_renderer(&self, map: [u32; 2]) -> io::Result<Renderer> {
        let layers = self
            .layers
            .as_ref()
            .ok_or_else(|| invalid("mesh content stack unavailable"))?;
        let globals = viewer_scene_base::shader_sources::globals(map, self.decode);
        let recorded = layers
            .without_overlays()
            .map_or_else(|| layers.clone(), Arc::new);
        Ok(Renderer {
            builds: builds::Builds::new([
                Library::new(layers.clone()).with_globals(globals.clone()),
                Library::new(recorded).with_globals(globals),
            ]),
            residents: Vec::new(),
            slots: Vec::new(),
            resource_ids: ResourceIdentities::default(),
            geometry: BTreeMap::new(),
            queue_slots: Vec::new(),
            admission: Default::default(),
            frame_handles: Default::default(),
            binding_storage: Default::default(),
            scratch: Default::default(),
            geometry_seen: Vec::new(),
            prepared: Vec::new(),
            candidates: Vec::new(),
            pools: BTreeMap::new(),
            indices: Pool::default(),
            draws: DrawList::default(),
            casters: kept::Kept::new(kept::Kind::Casters),
            lit: Default::default(),
            calls: (u64::MAX, [0; 2]),
            lent: None,
        })
    }

    pub fn warm_pipelines(&self, device: &wgpu::Device, map: [u32; 2], samples: u32) {
        let mut renderer = self.native.borrow_mut();
        if renderer.is_none() {
            match self.new_renderer(map) {
                Ok(created) => *renderer = Some(created),
                Err(_) => return,
            }
        }
        let native = renderer.as_mut().expect("initialized above");
        let forced = self.label == "supply trains";
        for (index, mesh) in self.meshes.iter().enumerate() {
            let library = self.shader_library(index);
            for descriptor in &mesh.native {
                native.builds.warm(
                    device,
                    (index, descriptor.node),
                    library,
                    &descriptor.lods,
                    (forced, samples),
                    self.instanced_masks(),
                );
            }
        }
        self.warmed.set(self.meshes.len());
    }

    pub fn warm_added(&self, device: &wgpu::Device, map: [u32; 2], samples: u32) -> io::Result<()> {
        let _span = viewer_runtime::profiling::span("model_warm_added");
        let from = self.warmed.get();
        if from >= self.meshes.len() {
            return Ok(());
        }
        let mut renderer = self.native.borrow_mut();
        if renderer.is_none() {
            *renderer = Some(self.new_renderer(map)?);
        }
        let builds = &renderer.as_ref().expect("initialized above").builds;
        let forced = self.label == "supply trains";
        let added: Vec<_> = (from..self.meshes.len())
            .flat_map(|index| {
                let library = self.shader_library(index);
                self.meshes[index].native.iter().map(move |descriptor| {
                    ((index, descriptor.node), library, descriptor.lods.clone())
                })
            })
            .collect();
        crate::count(crate::Count::WarmedNodes, added.len() as u64);
        builds.warm_behind(device, added, (forced, samples), self.instanced_masks());
        self.warmed.set(self.meshes.len());
        Ok(())
    }

    pub(crate) fn prepare_native(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frame: ModelFrame<'_>,
        inputs: Inputs<'_>,
        queue: Queue,
        object: &mut ObjectConstants,
    ) -> io::Result<DrawList> {
        let _wall = viewer_runtime::profiling::span(if queue == Queue::Caster {
            "native_caster_prepare"
        } else {
            "native_lit_prepare"
        });
        let _pass = viewer_runtime::profiling::span(prepare_span(self.label, queue));
        self.walk_native(arena, frame, inputs, queue, object, None)
    }

    fn walk_native(
        &self,
        arena: &shader_pipeline::gpu::Arena,
        frame: ModelFrame<'_>,
        inputs: Inputs<'_>,
        queue: Queue,
        object: &mut ObjectConstants,
        mut recording: Option<&mut kept::Recording>,
    ) -> io::Result<DrawList> {
        if self.field.native_order.is_empty() {
            return Ok(DrawList::default());
        }
        let caster = queue == Queue::Caster;
        let partial = recording.as_ref().is_some_and(|recording| recording.partial());
        let caller = plan_tail::Caller::select(self.label);
        let forced = self.label == "supply trains";
        let instanced = self.instanced(caster);
        let mut renderer = self.native.borrow_mut();
        if renderer.is_none() {
            *renderer = Some(self.new_renderer([frame.camera.map.width, frame.camera.map.height])?);
        }
        let native = renderer.as_mut().expect("initialized above");
        if recording.is_some() {
        }
        let stamp = native
            .resource_ids
            .observe(&self.textures, (!caster).then_some(inputs.textures));
        native.slots.resize_with(self.meshes.len(), Vec::new);
        native.queue_slots.resize_with(self.meshes.len(), Vec::new);
        let layout = &self.native_walk;
        let mut frame_handles = std::mem::take(&mut native.frame_handles);
        let dense_handles = layout.installed && frame_handles.begin(layout.players.len());

        let mut commands = std::mem::take(&mut native.draws);
        let device = arena.device();
        let settled = self.settled.get();
        let mut queued = native.builds.queue(device);
        let mut poses = self.poses.borrow_mut();
        if poses.0 != arena.frame() {
            *poses = (arena.frame(), Vec::new());
        }
        poses.1.resize_with(self.field.instances.len(), || None);
        let eye = frame.camera.eye();
        let mut posed = 0u64;
        let mut drawn = 0;
        let timed = viewer_runtime::view_record::on();
        let spent: [std::cell::Cell<u64>; WALK_STEPS] = Default::default();
        let clock = || timed.then(std::time::Instant::now);
        let spend = |step: usize, since: Option<std::time::Instant>| {
            if let Some(since) = since {
                spent[step].set(spent[step].get() + since.elapsed().as_nanos() as u64);
            }
        };
        let mut last_material = None;
        let mut pending_sources = plans::PendingSources::default();
        let mut scratch = std::mem::take(&mut native.scratch);
        scratch.begin(object);
        let mut binding_session = shader_pipeline::gpu::BindingSession::with_storage(
            arena, std::mem::take(&mut native.binding_storage));
        native.geometry_seen.clear();
        native.geometry_seen.resize(native.residents.len(), false);
        let mut prepared = std::mem::take(&mut native.prepared);
        let mut candidates = std::mem::take(&mut native.candidates);
        let mut textures: Textures = [None; TEXTURE_BINDINGS];
        let mut texture_mask = 0;
        if !caster {
            for (i, texture) in inputs.textures.iter().enumerate() {
                let slot = binding(i as u32 + 4, Stage::Pixel) as usize;
                textures[slot] = Some(*texture);
                texture_mask |= 1 << slot;
            }
        }
        for (view, offset) in frame.offsets.iter().enumerate() {
            for (position, (group_index, index, row)) in layout.routes(&self.field).enumerate() {
                if recording.as_deref_mut().is_some_and(|recording| !recording.takes(view, position)) {
                    continue;
                }
                let group = &self.field.groups[group_index];
                let library = row.map_or_else(|| self.shader_library(group.mesh), |row| row.library);
                if !visible(group, *offset, frame.visible) {
                    continue;
                }
                if let Some(recording) = recording.as_deref_mut() {
                    let groups = self.field.groups.len();
                    recording.visited.resize(frame.offsets.len() * groups, false);
                    recording.visited[view * groups + group_index] = true;
                }
                let mesh = &self.meshes[group.mesh];
                if native
                    .admission
                    .of(
                        group.mesh,
                        mesh.native.iter().map(|descriptor| descriptor.node),
                        &native.queue_slots[group.mesh],
                    )
                    .is_some_and(|admits| admission::passes_over(queue, forced, admits))
                {
                    continue;
                }
                {
                    let instance = self.field.instances[index];
                    let hidden = row.map_or_else(|| {
                        let (seed, identity) = instance.animation_key();
                        self.hidden.contains(&(group.mesh, seed, identity))
                    }, |row| row.hidden);
                    if hidden { continue; }
                    let every = recording.as_deref().is_some_and(|recording| recording.every_lod);
                    let ground = (!visible(group, *offset, frame.seen)).then_some(crate::Culled::Planes);
                    if !every && let Some(why) = ground.or_else(|| self.game_culled(group.mesh, &instance, *offset, &frame.camera)) {
                        let name = match (why, caster) {
                            (crate::Culled::Distance, false) => "lit_culled_distance",
                            (crate::Culled::Distance, true) => "caster_culled_distance",
                            (crate::Culled::Planes, false) => "lit_culled_planes",
                            (crate::Culled::Planes, true) => "caster_culled_planes",
                        };
                        viewer_runtime::view_record::add(self.label, name, 1);
                        if let Some(recording) = recording.as_deref_mut() {
                            recording.culled.push((view, kept::figure_key(group.mesh, &instance)));
                        }
                        if !caster {
                            let moved = self.seen.get().map_or(0.0, |cull| cull.x - frame.camera.center[0]);
                            note_culled(self.label, &mesh.name, mesh.height, &instance, *offset + moved, self.seen.get(), why, arena.frame());
                        }
                        continue;
                    }
                    if !every && self.seen.get().is_some() {
                        viewer_runtime::view_record::add(self.label, if caster { "caster_cull_passed" } else { "lit_cull_passed" }, 1);
                    }
                    let live = recording.is_some()
                        && if let Some(row) = row.filter(|_| layout.installed) {
                            self.players.contains_key(&layout.players[row.player])
                        } else {
                            self.player(group.mesh, &instance).is_some()
                        };
                    if let Some(recording) = recording.as_deref_mut() {
                        let key = kept::figure_key(group.mesh, &instance);
                        recording.begin(view, position, key, live);
                    }

                    let instance_world = match instance.native_transform() {
                        Ok(world) => world,
                        Err(why) => {

                            self.refuse_actor(arena, None, index, usize::MAX, &mesh.name, why);
                            continue;
                        }
                    };

                    let Some(rig) = self.rigs.get(group.mesh) else {
                        self.refuse_actor(
                            arena,
                            Some(group.mesh),
                            index,
                            usize::MAX,
                            &mesh.name,
                            "native actor rig unavailable",
                        );
                        continue;
                    };
                    let since = clock();
                    let actors = poses.1[index].get_or_insert_with(|| {
                        posed += 1;
                        let frozen = !matches!(
                            instance.animation,
                            entity_animation::STOPPED | entity_animation::DEFAULT_STATE
                        );
                        if dense_handles {
                            let row = row.expect("installed actor route");
                            frame_handles.resolve(row.player, row.mesh, frozen, || {
                                self.actors(group.mesh, layout.players[row.player], rig, self.players.get(&layout.players[row.player]), frozen)
                            })
                        } else {
                            self.actors(group.mesh, self.player_key(group.mesh, &instance), rig, self.player(group.mesh, &instance), frozen)
                        }
                    });
                    spend(WALK_POSE, since);

                    for (descriptor_at, descriptor) in mesh.native.iter().enumerate() {
                        let source_before = scratch.checkpoint();
                        candidates.clear();

                        let mut submit = || -> Result<(), PrepareError> {
                            let actor = &actors[descriptor.node];
                            let world = mesh_pose::multiply(
                                matrix(&instance_world),
                                *actor
                                    .world
                                    .as_ref()
                                    .map_err(|why| PrepareError::Actor(invalid(why.clone())))?,
                            );
                            let position = [world[3][0] + offset, world[3][1], world[3][2]];

                            let chosen = descriptor.lod(position, eye).map_err(PrepareError::Actor)?;
                            let every = recording.as_deref().is_some_and(|recording| recording.every_lod);
                            let Some(lod) = chosen.or(every.then_some(0)) else {
                                if let Some(recording) = recording.as_deref_mut() {
                                    recording.units.push(kept::Unit {
                                        live,
                                        view,
                                        group: group_index,
                                        index,
                                        row: row.copied(),
                                        descriptor: descriptor_at,
                                        lod: None,
                                        at: position,
                                        vertex: None,
                                        skin: None,
                                        bones: 0,
                                        mask: 0,
                                        draws: 0,
                                        parts: 0,
                                    });
                                }
                                return Ok(());
                            };
                            let lods = if every { 0..descriptor.lods.len() } else { lod..lod + 1 };
                            for lod in lods {

                            for source in &descriptor.lods[lod] {
                                if let Some(why) =
                                    self.material_refusals.get(&source.material_index)
                                {
                                    return Err(PrepareError::Actor(invalid(why.clone())));
                                }
                                if !self.ready[source.material_index] {
                                    return Err(io::Error::other(format!(
                                        "{} material is not resident (slot {})",
                                        self.label, source.material_index
                                    )).into());
                                }
                            }

                            let skin_pose = descriptor.lods[lod]
                                .iter()
                                .any(|source| !source.geometry.skin.is_empty())
                                .then(|| {
                                    actor.pose.as_ref().map_err(|why| {
                                        PrepareError::Actor(invalid(format!("skin: {why}")))
                                    })
                                })
                                .transpose()?;

                            let queues = &mut native.queue_slots[group.mesh];
                            if queues.len() <= descriptor.node {
                                queues.resize(descriptor.node + 1, None);
                            }
                            if queues[descriptor.node].is_none() {
                                let Some(admitted) = native.builds.flags(
                                    &mut queued,
                                    (group.mesh, descriptor.node),
                                    library,
                                    || descriptor.lods.iter().flatten().cloned().collect(),
                                    settled,
                                ) else {
                                    arena.request_frame();
                                    if let Some(recording) = recording.as_deref_mut() {
                                        recording.wait();
                                    }
                                    return Ok(());
                                };
                                native.queue_slots[group.mesh][descriptor.node] = Some(admitted?);
                            }
                            let queues = &mut native.queue_slots[group.mesh];
                            let flags = queues[descriptor.node].unwrap();
                            if !match queue {
                                Queue::Regular => flags[0] && !forced,
                                Queue::Alternate => flags[1] || forced,
                                Queue::Caster => flags[0],
                            } {
                                return Ok(());
                            }

                            let root = rig.nodes[0].native.as_ref().ok_or_else(|| {
                                PrepareError::Actor(invalid("native root actor source unavailable"))
                            })?;
                            let since = clock();
                            let userdata = if caster {
                                None
                            } else {
                                caller.evaluate(root, instance, frame.alpha).map_err(PrepareError::Actor)?
                            };
                            spend(WALK_USERDATA, since);

                            scratch.update(&world, userdata);
                            let (vertex, pixel) = (scratch.vertex(), scratch.pixel());

                            let since = clock();
                            prepared.clear();

                            let nodes = &mut native.slots[group.mesh];
                            nodes.resize_with(rig.nodes.len(), Vec::new);
                            if nodes[descriptor.node].is_empty() {
                                nodes[descriptor.node] = descriptor
                                    .lods
                                    .iter()
                                    .map(|parts| vec![[None; 4]; parts.len()])
                                    .collect();
                            }

                            let variants = usize::from(caster) * 2 + usize::from(forced);
                            let mut built = true;
                            for (part, source) in descriptor.lods[lod].iter().enumerate() {
                                if native.slots[group.mesh][descriptor.node][lod][part][variants]
                                    .is_none()
                                {
                                    built &= native.builds.ready(
                                        &mut queued,
                                        (group.mesh, descriptor.node, lod, part, variants),
                                        library,
                                        source,
                                        (caster, forced),
                                        (inputs.samples, instanced),
                                        settled,
                                    );
                                }
                            }
                            if !built {
                                arena.request_frame();
                                if let Some(recording) = recording.as_deref_mut() {
                                    recording.wait();
                                }
                                if every {
                                    continue;
                                }
                                return Ok(());
                            }
                            let mut submitted_mask = 0;
                            for (part, source) in descriptor.lods[lod].iter().enumerate() {
                                let key = match native.slots[group.mesh][descriptor.node][lod][part]
                                    [variants]
                                {
                                    Some(key) => key,
                                    None => {
                                        let _first = viewer_runtime::profiling::span(
                                            "native_first_part",
                                        );
                                        crate::count(crate::Count::FirstParts, 1);
                                        let builds::Built {
                                            lit,
                                            shadow,
                                            pass: pipeline,
                                        } = native.builds.take(&(
                                            group.mesh,
                                            descriptor.node,
                                            lod,
                                            part,
                                            variants,
                                        ))?;
                                        let source_blend = |s: &Selection| {
                                            s.effect
                                                .source
                                                .blend(&s.effect.source.techniques[s.index])
                                        };
                                        let (caster_texture, blended) = material_flags(
                                            &source_blend(&lit),
                                            &source_blend(&shadow),
                                            !source.material.diffuse.is_empty(),
                                        );
                                        let order = pipeline.order();
                                        note_part(
                                            native.residents.len(),
                                            group.mesh,
                                            descriptor.node,
                                            lod,
                                            part,
                                            blended,
                                            &lit,
                                            source,
                                            order,
                                        );
                                        let (compiled, retained) =
                                            part_program(&pipeline, &source.geometry)?;
                                        let geometry = match native
                                            .geometry
                                            .get(&source.geometry_key)
                                        {
                                            Some(geometry) => geometry.clone(),
                                            None => {
                                                let stride =
                                                    if source.geometry.uv1 { 56 } else { 48 };
                                                let skin: Option<&[u8]> =
                                                    (!source.geometry.skin.is_empty()).then(|| {
                                                        bytemuck::cast_slice(&source.geometry.skin)
                                                    });
                                                let packed = match native
                                                    .builds
                                                    .packed(&source.geometry_key)
                                                {
                                                    Some(packed) => packed,
                                                    None => {
                                                        crate::count(crate::Count::PackedHere, 1);
                                                        Arc::new(
                                                            source
                                                                .geometry
                                                                .packed(source.geometry.uv1)
                                                                .map_err(PrepareError::Actor)?,
                                                        )
                                                    }
                                                };
                                                crate::count(
                                                    crate::Count::FirstGeometryBytes,
                                                    (packed.len() * 4
                                                        + source.geometry.skin.len() * 16
                                                        + source.geometry.indices.len() * 4)
                                                        as u64,
                                                );
                                                let vertices = native
                                                    .pools
                                                    .entry((stride, skin.is_some()))
                                                    .or_default()
                                                    .place(
                                                        arena,
                                                        wgpu::BufferUsages::VERTEX,
                                                        [stride, 16],
                                                        [Some(bytemuck::cast_slice(&packed)), skin],
                                                        packed.len() as u64 * 4 / stride,
                                                    )?;
                                                let indices = native.indices.place(
                                                    arena,
                                                    wgpu::BufferUsages::INDEX,
                                                    [4, 0],
                                                    [
                                                        Some(bytemuck::cast_slice(
                                                            &source.geometry.indices,
                                                        )),
                                                        None,
                                                    ],
                                                    source.geometry.indices.len() as u64,
                                                )?;
                                                let geometry = SharedGeometry {
                                                    stride,
                                                    native_stride: source.geometry.stride,
                                                    cpu_vertices: bytemuck::cast_slice(
                                                        &source.geometry.vertices,
                                                    )
                                                    .into(),
                                                    cpu_skin: skin.map(Into::into),
                                                    skin_bones: source
                                                        .geometry
                                                        .skin
                                                        .iter()
                                                        .flat_map(|v| v.indices)
                                                        .map(|i| usize::from(i) + 1)
                                                        .max()
                                                        .unwrap_or(0),
                                                    vertices: (
                                                        vertices.0,
                                                        i32::try_from(vertices.1)
                                                            .map_err(io::Error::other)?,
                                                    ),
                                                    indices: (
                                                        indices.0,
                                                        u32::try_from(indices.1)
                                                            .map_err(io::Error::other)?,
                                                    ),
                                                };
                                                native.geometry.insert(
                                                    source.geometry_key.clone(),
                                                    geometry.clone(),
                                                );
                                                geometry
                                            }
                                        };
                                        let samplers = (0..TEXTURE_BINDINGS as u32)
                                            .filter(|slot| retained.sampler(*slot).is_some())
                                            .fold(0, |mask, slot| mask | (1 << slot));
                                        let key = native.residents.len();
                                        native.residents.push(Resident {
                                            retained,
                                            geometry,
                                            pipeline,
                                            required_constants: compiled
                                                .vertex
                                                .buffers
                                                .iter()
                                                .chain(&compiled.pixel.buffers)
                                                .map(|buffer| buffer.binding)
                                                .collect(),
                                            required_textures: compiled
                                                .vertex
                                                .textures
                                                .iter()
                                                .chain(&compiled.pixel.textures)
                                                .map(|(slot, _)| *slot)
                                                .collect(),
                                            constant_mask: compiled.vertex.buffers.iter().chain(&compiled.pixel.buffers)
                                                .fold(0, |mask, buffer| mask | (1 << buffer.binding)),
                                            texture_mask: compiled.vertex.textures.iter().chain(&compiled.pixel.textures)
                                                .fold(0, |mask, (slot, _)| mask | (1 << slot)),
                                            caster_texture,
                                            blended,
                                            order,
                                            samplers,
                                            program: None,
                                        });
                                        native.slots[group.mesh][descriptor.node][lod][part]
                                            [variants] = Some(key);
                                        key
                                    }
                                };

                                let gpu = &native.residents[key];

                                let material = &self.materials[source.material_index];
                                for (slot, index) in
                                    [material.diffuse, material.specular, material.normal]
                                        .into_iter()
                                        .enumerate()
                                {
                                    let slot = binding(slot as u32, Stage::Pixel) as usize;
                                    let bound = &mut textures[slot];
                                    if caster
                                        && (slot != binding(0, Stage::Pixel) as usize
                                            || !gpu.caster_texture)
                                    {
                                        *bound = None;
                                        texture_mask &= !(1 << slot);
                                        continue;
                                    }
                                    let texture =
                                        self.textures[index].as_ref().ok_or_else(|| {
                                            PrepareError::Actor(invalid(
                                                "native mesh material is not resident",
                                            ))
                                        })?;
                                    *bound = Some(&texture.view);
                                    texture_mask |= 1 << slot;
                                }

                                let skin = if gpu.geometry.cpu_skin.is_some() {
                                    let pose = skin_pose.expect("skinned geometry checked above");
                                    if gpu.geometry.skin_bones > pose.bones.len() {
                                        return Err(PrepareError::Actor(invalid(format!(
                                            "skin: vertex reads outside {} initialized bones",
                                            pose.bones.len()
                                        ))));
                                    }
                                    Some(bytemuck::cast_slice(&pose.bones))
                                } else { None };
                                let supplies = plan_tail::Supplies::new(
                                    inputs.frames[view], vertex, pixel, skin,
                                    (!caster).then_some(inputs.grid));
                                if supplies.present() & gpu.constant_mask != gpu.constant_mask
                                    || texture_mask & gpu.texture_mask != gpu.texture_mask
                                {
                                    gpu.check_bindings(&supplies.constants(), &textures)?;
                                }

                                let bindings = if recording.is_some() {
                                    match &gpu.program {
                                        Some((installed, program)) if *installed == stamp => {
                                            let supplied = program.constant_reads().iter().all(
                                                |spec| supplies.get(spec.binding).is_some(),
                                            );
                                            if !supplied {
                                                let supply = |binding| supplies.get(binding);
                                                program.bind_with(arena, supply).map_err(invalid)?;
                                            }
                                        }
                                        _ => {
                                            let program = gpu
                                                .pipeline
                                                .pipeline
                                                .rows_program(
                                                    arena,
                                                    |binding| textures[binding as usize],
                                                    None,
                                                )
                                                .map_err(invalid)?;
                                            candidates.push((key, program));
                                        }
                                    }
                                    None
                                } else { Some(match &gpu.program {
                                    Some((installed, program)) if *installed == stamp => {
                                        match program.recipe() {
                                            Some(recipe) => binding_session.bind_ordered(key, program, recipe.ordered(supplies)),
                                            None => binding_session.bind_with(key, program, |binding| supplies.get(binding)),
                                        }.map_err(invalid)?
                                    }
                                    _ => {
                                        let (bindings, program) = gpu
                                            .pipeline
                                            .pipeline
                                            .bind_program_with(
                                                arena,
                                                |binding| supplies.get(binding),
                                                |binding| textures[binding as usize],
                                            )
                                            .map_err(invalid)?;
                                        candidates.push((key, program));
                                        bindings
                                    }
                                }) };

                                if queue.accepts(gpu.blended, forced) {
                                    submitted_mask |= supplies.present();
                                    prepared.push(PreparedPart {
                                        key,
                                        bindings,
                                        textures: texture_mask,
                                        material: source.material_index,
                                        count: source.geometry.indices.len() as u32,
                                    });
                                }
                            }

                            spend(WALK_PREPARE, since);
                            spent[WALK_PARTS].set(spent[WALK_PARTS].get() + prepared.len() as u64);
                            let since = clock();
                            for (key, program) in candidates.drain(..) {
                                native.residents[key].program = Some((stamp, plan_tail::Program::new(program)));
                            }
                            if !caster && !prepared.is_empty() {
                                pending_sources.constants_mask(submitted_mask, view, &scratch, index, descriptor.node);
                            }
                            let blocks = match recording.as_deref_mut() {
                                Some(recording) if !prepared.is_empty() || !caster => {
                                    let mut bones = 0;
                                    for part in &prepared {
                                        let resident = &native.residents[part.key];
                                        if resident.geometry.cpu_skin.is_some() {
                                            bones = bones.max(resident.geometry.skin_bones);
                                        }
                                    }
                                    let vertex = recording.place(scratch.vertex());
                                    let skin = skin_pose.filter(|_| bones != 0).map(|pose| {
                                        let palette: &[u8] = bytemuck::cast_slice(&pose.bones);
                                        recording.place(palette)
                                    });
                                    recording.units.push(kept::Unit {
                                        live,
                                        view,
                                        group: group_index,
                                        index,
                                        row: row.copied(),
                                        descriptor: descriptor_at,
                                        lod: Some(lod),
                                        at: position,
                                        vertex: Some(vertex),
                                        skin,
                                        bones,
                                        mask: submitted_mask,
                                        draws: 0,
                                        parts: prepared.len(),
                                    });
                                    Some((vertex, skin))
                                }
                                Some(recording) => {
                                    recording.units.push(kept::Unit {
                                        live,
                                        view,
                                        group: group_index,
                                        index,
                                        row: row.copied(),
                                        descriptor: descriptor_at,
                                        lod: Some(lod),
                                        at: position,
                                        vertex: None,
                                        skin: None,
                                        bones: 0,
                                        mask: 0,
                                        draws: 0,
                                        parts: 0,
                                    });
                                    None
                                }
                                None => None,
                            };
                            for PreparedPart {
                                key,
                                bindings,
                                textures,
                                material,
                                count,
                            } in prepared.drain(..)
                            {
                                let gpu = &native.residents[key];
                                if !caster {
                                    pending_sources.state.record(key, gpu, textures);
                                }
                                let pool = (gpu.geometry.stride, gpu.geometry.cpu_skin.is_some());
                                let vertices = (pool, gpu.geometry.vertices.0);
                                native.geometry_seen.resize(native.residents.len(), false);
                                let first_geometry = !native.geometry_seen[key];
                                native.geometry_seen[key] = true;
                                if first_geometry {
                                commands.vertices.entry(vertices).or_insert_with(|| {
                                    native.pools[&pool].chunks[vertices.1].streams.clone()
                                });
                                commands
                                    .indices
                                    .entry(gpu.geometry.indices.0)
                                    .or_insert_with(|| {
                                        native.indices.chunks[gpu.geometry.indices.0].streams[0]
                                            .as_ref()
                                            .expect("index stream")
                                            .clone()
                                    });
                                }
                                match (bindings, recording.as_deref_mut(), blocks) {
                                    (Some(bindings), _, _) => commands.draws.push(RecordedDraw {
                                        id: 0,
                                        shown: true,
                                        bindings,
                                        reference: gpu.pipeline.reference(),
                                        vertices,
                                        indices: gpu.geometry.indices.0,
                                        first: gpu.geometry.indices.1,
                                        count,
                                        base: gpu.geometry.vertices.1,
                                        instances: 1,
                                    }),
                                    (None, Some(recording), Some((vertex, skin))) => {
                                        recording.pending.push(kept::Pending {
                                            key,
                                            view,
                                            vertex,
                                            skin: skin.filter(|_| gpu.geometry.cpu_skin.is_some()),
                                            reference: gpu.pipeline.reference(),
                                            vertices,
                                            indices: gpu.geometry.indices.0,
                                            first: gpu.geometry.indices.1,
                                            count,
                                            base: gpu.geometry.vertices.1,
                                            textures,
                                            material,
                                        });
                                    }
                                    _ => unreachable!("a recorded part has its node's blocks"),
                                }
                                last_material = Some(material);
                                drawn += 1;
                            }
                            spend(WALK_CAPTURE, since);
                            }
                            Ok(())
                        };
                        let since = clock();
                        let submitted = submit();
                        spend(WALK_NODE, since);
                        spent[WALK_NODES].set(spent[WALK_NODES].get() + 1);
                        if let Err(why) = submitted {
                            let why = match why {
                                PrepareError::Actor(why) => why,
                                PrepareError::Shared(why) => {
                                    scratch.sync(object);
                                    return Err(why);
                                }
                            };
                            scratch.restore(source_before);
                            let name = if descriptor.name == mesh.name {
                                mesh.name.clone()
                            } else {
                                format!("{}/{}", mesh.name, descriptor.name)
                            };
                            self.refuse_actor(
                                arena,
                                Some(group.mesh),
                                index,
                                descriptor.node,
                                &name,
                                why,
                            );
                        }
                    }
                }
            }
        }
        if !caster && !partial {
            if let Some(material) = last_material {
                let material = &self.materials[material];
                for (slot, index) in [material.diffuse, material.specular, material.normal]
                    .into_iter()
                    .enumerate()
                {
                    textures[binding(slot as u32, Stage::Pixel) as usize] =
                        self.textures[index].as_ref().map(|texture| &texture.view);
                }
            }
            pending_sources.apply(native, textures, inputs, &scratch, &poses.1, &mut object.retained);
            self.drawn.set(self.drawn.get() + drawn);
        }
        scratch.sync(object);
        native.scratch = scratch;
        native.binding_storage = binding_session.into_storage();
        native.frame_handles = frame_handles;
        prepared.clear();
        candidates.clear();
        native.prepared = prepared;
        native.candidates = candidates;
        viewer_runtime::view_record::add(self.label, ACTORS_POSED, posed);
        if timed {
            let names = if caster { &CASTER_WALK_STEPS } else { &LIT_WALK_STEPS };
            for (step, name) in names.iter().enumerate() {
                let value = spent[step].get();
                let value = if step >= WALK_NODES { value } else { value / 1000 };
                viewer_runtime::view_record::add(self.label, name, value);
            }
        }
        Ok(commands)
    }
}

#[cfg(test)]
mod tests {

    use crate::native::*;

    #[test]
    #[ignore = "requires a GPU"]
    fn the_models_own_pass_keeps_blend_depth_stencil_and_dynamic_bindings() {
        use shader_pipeline::{
            Buffer, Compiled, ScalarKind, Shader, TextureKind, VertexInput, gpu,
        };
        use viewer_gpu::gpu_contract::{buffer, read_texture};
        content_reader_script::test_game::enter();
        let (_serial, device, queue) = viewer_gpu::test_support::gpu::device();
        let program = content_reader_gui::pdxshader::program::read_with(b"slices.shader", &|path| {
            Some(if path == b"slices.shader" {
                b"Effect Draw { }".to_vec()
            } else {
                Vec::new()
            })
        })
        .unwrap();
        let shader = |wgsl: &str| Shader {
            wgsl: wgsl.into(),
            independent_draws: false,
            buffers: Vec::new(),
            textures: Vec::new(),
            samplers: Vec::new(),
            sampling_pairs: Vec::new(),
            inputs: Vec::new(),
            clock: Default::default(),
        };
        let mut vertex = shader(
            r#"
            struct Block { colour: vec4<f32>, shift: vec4<f32> }
            @group(0) @binding(0) var<uniform> block: Block;
            struct Out { @builtin(position) position: vec4<f32>, @location(0) colour: vec4<f32> }
            @vertex fn main(@location(0) p: vec3<f32>) -> Out {
                var out: Out;
                out.position = vec4<f32>(p + block.shift.xyz, 1.0);
                out.colour = block.colour;
                return out;
            }
        "#,
        );
        vertex.buffers.push(Buffer {
            binding: 0,
            size: 32,
            reads: std::iter::once(0..32).collect(),
        });
        vertex.inputs.push(VertexInput {
            kind: ScalarKind::Float,
            location: 0,
            semantic: "POSITION".into(),
            components: 3,
        });
        let mut pixel = shader(
            r#"
            @group(1) @binding(1) var image: texture_2d<f32>;
            @fragment fn main(@location(0) colour: vec4<f32>) -> @location(0) vec4<f32> {
                return colour * textureLoad(image, vec2<i32>(0), 0);
            }
        "#,
        );
        pixel.textures.push((1, TextureKind::D2));
        let compiled = Compiled { vertex, pixel };
        let views: Vec<_> = [[255, 255, 255, 255], [128, 255, 128, 255]]
            .into_iter()
            .map(|texel| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("slice texture"),
                    size: wgpu::Extent3d {
                        width: 1,
                        height: 1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                queue.write_texture(
                    texture.as_image_copy(),
                    &texel,
                    wgpu::TexelCopyBufferLayout::default(),
                    texture.size(),
                );
                texture.create_view(&Default::default())
            })
            .collect();
        let vertices: Vec<_> = [0.0_f32, 0.125]
            .into_iter()
            .map(|shift| {
                let positions = [
                    [0.; 3],
                    [-0.9 + shift, -0.8, 0.],
                    [0.8, -0.7 + shift, 0.],
                    [-0.1, 0.9, 0.],
                ];
                buffer(
                    &device,
                    "slice vertices",
                    wgpu::BufferUsages::VERTEX,
                    bytemuck::cast_slice(&positions),
                )
                .unwrap()
            })
            .collect();
        let indices: Vec<_> = [[0_u32, 0, 1, 2], [0_u32, 2, 1, 0]]
            .into_iter()
            .map(|data| {
                buffer(
                    &device,
                    "slice indices",
                    wgpu::BufferUsages::INDEX,
                    bytemuck::cast_slice(&data),
                )
                .unwrap()
            })
            .collect();
        for samples in [1, 4] {
            let arena = gpu::Arena::new(&device);
            let state = gpu::State::authored(&program, &program.techniques[0]).unwrap();
            let pipelines: Vec<_> = (0..3)
                .map(|mode| {
                    let mut pass = gpu::Pass::packed(&compiled, &state, SCENE_FORMAT, samples);
                    pass.primitive.cull_mode = None;
                    let face = wgpu::StencilFaceState {
                        compare: if mode == 0 {
                            wgpu::CompareFunction::Always
                        } else {
                            wgpu::CompareFunction::Equal
                        },
                        pass_op: if mode == 0 {
                            wgpu::StencilOperation::Replace
                        } else {
                            wgpu::StencilOperation::Keep
                        },
                        ..Default::default()
                    };
                    pass.depth_stencil = Some(wgpu::DepthStencilState {
                        format: DEPTH_FORMAT,
                        depth_write_enabled: Some(mode != 1),
                        depth_compare: Some(wgpu::CompareFunction::LessEqual),
                        stencil: wgpu::StencilState {
                            front: face,
                            back: face,
                            read_mask: 0xff,
                            write_mask: 0xff,
                        },
                        bias: Default::default(),
                    });
                    if mode == 1 {
                        pass.targets[0].as_mut().unwrap().blend =
                            Some(wgpu::BlendState::ALPHA_BLENDING);
                    }
                    gpu::Pipeline::for_pass(
                        &device,
                        &compiled,
                        &program,
                        &program.techniques[0],
                        &state,
                        &pass,
                    )
                    .unwrap()
                })
                .collect();
            let mut lists = [
                DrawList::default(),
                DrawList::default(),
                DrawList::default(),
            ];
            for list in &mut lists {
                for (index, vertex) in vertices.iter().enumerate() {
                    list.vertices
                        .insert(((12, false), index), [Some(vertex.clone()), None]);
                    list.indices.insert(index, indices[index].clone());
                }
            }
            for index in 0..19 {
                let mode = if index == 0 {
                    0
                } else if index < 17 {
                    1
                } else {
                    2
                };
                let block = [
                    index as f32 / 32.,
                    0.5,
                    0.75,
                    0.25,
                    (index % 3) as f32 / 16.,
                    0.,
                    if index == 17 {
                        0.9
                    } else if index == 18 {
                        0.25
                    } else {
                        0.5
                    },
                    0.,
                ];
                let bindings = pipelines[mode]
                    .bind_with(
                        &arena,
                        |_| Some(bytemuck::cast_slice(&block)),
                        |_| Some(&views[index % views.len()]),
                    )
                    .unwrap();
                lists[usize::from(index >= 11) * 2]
                    .draws
                    .push(RecordedDraw {
                        id: draw_id(),
                        shown: index != 8 && index != 5,
                        bindings,
                        reference: if index == 18 { 2 } else { 1 },
                        vertices: ((12, false), index % 2),
                        indices: index % 2,
                        first: 1,
                        count: 3,
                        base: 1,
                        instances: 1,
                    });
            }
            arena.flush(&queue);
            if samples == 1 {
                for list in &mut lists {
                    list.unplanned = true;
                    for _ in 0..bundles::STILL {
                        list.bundle(&device, lit_target(samples));
                    }
                    assert!(list.runs.iter().all(|run| run.bundle.is_some()));
                }
                assert!(lists.iter().any(|list| !list.runs.is_empty()));
            }
            let render = |lists: &[DrawList], encode: bool, empty: bool, bundled: u8| {
                let targets = Targets::new(&device, [16, 16], [16, 16], samples, viewer_gpu::gpu_contract::SeaBed::Half);
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
                        color_attachments: &[Some(
                            targets.scene_attachment(wgpu::LoadOp::Clear(wgpu::Color::BLACK)),
                        )],
                        depth_stencil_attachment: Some(
                            targets.depth_attachment(wgpu::LoadOp::Clear(1.)),
                        ),
                        ..Default::default()
                    });
                    if !encode && !empty {
                        for list in lists {
                            match bundled {
                                0 => list.record(&mut pass),
                                1 => list.record_range(&mut pass, 0..list.draws.len(), lit_target(samples)),
                                _ => {
                                    pass.set_blend_constant(wgpu::Color::WHITE);
                                    for at in 0..list.draws.len() {
                                        let draw = &list.draws[at];
                                        if !draw.shown {
                                            continue;
                                        }
                                        let (bundle, _) = record_bundle(
                                            &device,
                                            lit_target(samples),
                                            &list.draws[at..at + 1],
                                            &list.vertices,
                                            &list.indices,
                                        );
                                        pass.set_stencil_reference(draw.reference);
                                        pass.execute_bundles(std::iter::once(&bundle));
                                    }
                                }
                            }
                        }
                    }
                }
                if encode {
                    let lists = if empty {
                        Vec::new()
                    } else {
                        lists.iter().collect()
                    };
                    DrawList::encode(&lists, &targets, &mut encoder);
                }
                queue.submit([encoder.finish()]);
                let mut resolve = device.create_command_encoder(&Default::default());
                targets.resolve_scene(&mut resolve);
                queue.submit([resolve.finish()]);
                read_texture(&device, &queue, targets.scene_texture(), 8).unwrap()
            };
            for empty in [false, true] {
                let reference = render(&lists, false, empty, 0);
                assert_eq!(
                    reference
                        .chunks_exact(8)
                        .any(|pixel| pixel[..6].iter().any(|byte| *byte != 0)),
                    !empty
                );
                assert_eq!(render(&lists, false, empty, 2), reference, "one draw a bundle {samples}x MSAA empty={empty}");
                assert_eq!(render(&lists, false, empty, 1), reference, "bundled directly {samples}x MSAA empty={empty}");
                assert_eq!(
                    render(&lists, true, empty, 0),
                    reference,
                    "the models' own pass {samples}x MSAA empty={empty}"
                );
            }
        }
    }

    #[test]
    fn a_part_draws_with_the_technique_its_pass_fell_back_to() {
        use viewer_scene_base::mesh_shader::vertex_layouts;
        let (pass, asked, recorded, geometry) = viewer_scene_base::mesh_shader::test_support::fallen_back_pass();
        assert!(!Arc::ptr_eq(&asked.effect, &recorded));
        let inputs = |selection: &shader_pipeline::library::Selection| {
            let compiled = selection.effect.compiled(selection.index).unwrap();
            compiled.vertex.inputs.clone()
        };
        assert!(vertex_layouts(&inputs(&asked), false, false).is_err());
        assert!(Arc::ptr_eq(&pass.selection.effect, &recorded));
        let layouts = vertex_layouts(&inputs(&pass.selection), false, false).unwrap();
        assert_eq!(layouts[0].attributes.len(), 2);
        let (compiled, retained) = part_program(&pass, &geometry).unwrap();
        assert!(Arc::ptr_eq(&retained.selection().effect, &recorded));
        assert_eq!(compiled.vertex.inputs[1].semantic, "TEXCOORD0");
        let layouts = retained.layouts();
        assert_eq!(layouts[0].attributes.len(), 2);
        assert_eq!(layouts[0].stride, geometry.stride);
        assert!(retained.state().blend.is_some());
    }

    #[test]
    fn shadow_alpha_state_can_admit_an_alternate_material() {
        use content_reader_gui::pdxshader::BlendState;
        let opaque = BlendState::default();
        let cutout = BlendState {
            enable: false,
            alpha_test: true,
            ..opaque
        };
        let opaque = BlendState {
            enable: false,
            alpha_test: false,
            ..opaque
        };
        assert_eq!(material_flags(&cutout, &cutout, true), (true, false));
        assert_eq!(material_flags(&opaque, &cutout, true), (true, true));
        assert_eq!(material_flags(&cutout, &cutout, false), (false, true));
        assert_eq!(material_flags(&opaque, &opaque, true), (false, false));
    }

    #[test]
    fn object_upload_retains_pixel_snapshot_and_cpu_userdata_independently() {
        let mut object = ObjectConstants::default();
        let mut scratch = plans::Scratch::default();
        scratch.begin(&object);
        scratch.update(&[[2.; 4]; 4], None);
        assert!(scratch.pixel().is_none());
        scratch.update(&[[3.; 4]; 4], Some([5.; 8]));
        scratch.update(&[[7.; 4]; 4], None);
        let (vertex, pixel) = (scratch.vertex(), scratch.pixel());
        let pixel = pixel.unwrap();
        assert_eq!(&pixel[..64], bytemuck::bytes_of(&[[3_f32; 4]; 4]));
        assert_eq!(&pixel[64..96], bytemuck::bytes_of(&[5_f32; 8]));
        let retained = pixel.to_vec();
        assert_eq!(&vertex[..64], bytemuck::bytes_of(&[[7_f32; 4]; 4]));
        assert_eq!(&vertex[64..96], bytemuck::bytes_of(&[5_f32; 8]));
        scratch.sync(&mut object);
        object.begin();
        scratch.begin(&object);
        scratch.update(&[[11.; 4]; 4], None);
        let (vertex, pixel) = (scratch.vertex(), scratch.pixel());
        assert_eq!(pixel.unwrap(), retained);
        assert_eq!(&vertex[..64], bytemuck::bytes_of(&[[11_f32; 4]; 4]));
        assert!(vertex[64..].iter().all(|byte| *byte == 0));
        scratch.update(&[[13.; 4]; 4], Some([17.; 8]));
        let pixel = scratch.pixel().unwrap();
        assert_eq!(&pixel[..64], bytemuck::bytes_of(&[[13_f32; 4]; 4]));
        assert_eq!(&pixel[64..96], bytemuck::bytes_of(&[17_f32; 8]));
    }

    #[test]
    fn caller_basis_survives_roll_and_nonorthogonal_interpolation() {
        let basis = [[0.5, 0.25, 0.], [-0.75, 1., 0.125], [0., -0.5, 0.875]];
        let mut instance = ModelInstance {
            native_flags: 4,
            native_basis: basis,
            position: [11., 12., 13.],
            rotation: f32::NAN,
            pitch: f32::NAN,
            roll: 0.4,
            scale: 2.,
            ..Default::default()
        };
        let world = instance.native_transform().unwrap();
        assert_eq!(world.translation, [11., 12., 13.]);
        assert_eq!(
            world.basis,
            [[1., 0.5, 0.], [-1.5, 2., 0.25], [0., -1., 1.75]]
        );
        instance.native_flags = 0;
        assert!(instance.native_transform().is_err());
    }

    #[test]
    fn descriptor_lods_keep_source_order_and_append_unnumbered_records() {
        assert_eq!(
            lods([
                (Some(2), 'a'),
                (None, 'b'),
                (Some(0), 'c'),
                (Some(-1), 'd'),
                (Some(2), 'e')
            ])
            .unwrap(),
            vec![vec!['c'], vec![], vec!['a', 'e'], vec!['b', 'd']]
        );
        assert_eq!(
            lods([(None, 'a'), (None, 'b')]).unwrap(),
            vec![vec!['a', 'b']]
        );
        assert!(lods::<char>([]).unwrap().is_empty());
        assert!(
            lods([(Some(-2), 'a')])
                .unwrap_err()
                .to_string()
                .contains("negative LOD -2")
        );
    }

    #[test]
    fn lod_uses_actor_distance_squared_and_native_last_level_fallback() {
        let mut descriptor = Descriptor {
            name: "synthetic_lods".into(),
            node: 0,
            distances: vec![4., 100.],
            lods: vec![vec![], vec![], vec![]],
        };
        assert_eq!(descriptor.lod([1., 0., 0.], [0.; 3]).unwrap(), Some(2));
        assert_eq!(descriptor.lod([2., 0., 0.], [0.; 3]).unwrap(), Some(0));
        assert_eq!(descriptor.lod([6., 8., 0.], [0.; 3]).unwrap(), Some(1));
        descriptor.distances = vec![0., 100., 25.];
        assert_eq!(descriptor.lod([6., 0., 0.], [0.; 3]).unwrap(), Some(2));
        descriptor.distances.clear();
        assert_eq!(descriptor.lod([0.; 3], [0.; 3]).unwrap(), Some(2));
        descriptor.distances = vec![0.; 4];
        assert!(
            descriptor
                .lod([0.; 3], [0.; 3])
                .unwrap_err()
                .to_string()
                .contains("LOD 3")
        );
        descriptor.lods.clear();
        assert_eq!(descriptor.lod([0.; 3], [0.; 3]).unwrap(), None);
    }

    #[test]
    fn native_userdata_is_caller_specific_and_land_motion_is_independent_of_clip() {
        let source = mesh_pose::Source {
            pdxmesh: true,
            entity_locators: Vec::new(),
            binary_locators: Vec::new(),
            skeleton: Vec::new(),
            mesh_scale: 0.5,
            entity_scale: 1.,
            frozen: None,
            game_data: Some(Ok(mesh::EntityGameData {
                atlas_size: [8, 3],
                texture_anim_speed: 0.75,
            })),
        };
        let mut instance = ModelInstance {
            native_flags: 1,
            ..Default::default()
        };
        assert_eq!(
            plan_tail::Caller::select("city sprawl").evaluate(&source, instance, 1.).unwrap(),
            None
        );
        assert_eq!(
            plan_tail::Caller::select("unit models").evaluate(&source, instance, 1.).unwrap(),
            Some([3., 0., 8., 3., 0., 0., 0., 0.])
        );
        instance.native_flags = 3;
        assert_eq!(
            plan_tail::Caller::select("unit models").evaluate(&source, instance, 1.)
                .unwrap()
                .unwrap()[4],
            0.75
        );
        assert_eq!(
            plan_tail::Caller::select("map buildings").evaluate(&source, instance, 1.).unwrap(),
            Some([0., 0., 1., 1., 1., 0., 0., 0.])
        );
        instance.train_clip = [2., 3., 5., 7.];
        assert_eq!(
            plan_tail::Caller::select("supply trains").evaluate(&source, instance, 0.25).unwrap(),
            Some([1., 1., 1., 0.25, 2., 3., 5., 7.])
        );
    }

    #[test]
    fn actor_matrix_keeps_affine_w_and_separate_import_scale() {
        let actor = mesh::Transform {
            basis: [[0., 0., 2.], [0., 2., 0.], [-2., 0., 0.]],
            translation: [7., 11., 13.],
        };
        let packed = matrix(&actor);
        let imported = [0.5, 1., 1.5, 1.];
        let transformed: [f32; 4] = std::array::from_fn(|row| {
            (0..4)
                .map(|column| packed[column][row] * imported[column])
                .sum()
        });
        assert_eq!(transformed, [4., 13., 14., 1.]);
        assert_eq!(actor.transform_point([0.5, 1., 1.5]), [4., 13., 14.]);
        assert_eq!(packed[3], [7., 11., 13., 1.]);
    }
}

#[cfg(test)]
mod dump_tests;
