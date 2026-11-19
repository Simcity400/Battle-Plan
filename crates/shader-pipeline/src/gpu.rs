//! GPU resources for compiled game techniques. Inputs are native register bytes and vertex
//! semantics, so changing an effect's uniform names never changes what the engine uploads.

use super::{Buffer, Compiled};
use installed_content_core::content::diagnostics::{self, Diagnostics};
use installed_content_core::pdxshader::{
    Address, DepthStencilState, RasterizerState,
    program::{Filter, Program, Sampler, Stage, Technique},
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Condvar, Mutex, OnceLock},
};
use wgpu;

#[path = "gpu/arena.rs"]
mod arena;
pub use arena::{Arena, Slice};
pub mod batch;
mod batch_position;
mod batch_shader;
mod batch_views;

#[path = "gpu/stats.rs"]
pub mod stats;

#[cfg(any(test, feature = "test-support"))]
#[path = "gpu/uploads.rs"]
mod uploads;
#[cfg(any(test, feature = "test-support"))]
pub use uploads::FrozenBufferWrites;

/// Bytes a differential write compares and writes as one block. Smaller finds more unchanged
/// bytes; larger issues fewer writes. A GUI draw's constant block is 256 bytes or less.
const BLOCK: usize = 256;
/// Unchanged blocks one write crosses rather than splitting in two.
const BRIDGE: usize = 4;

/// The spans of `fresh` a buffer holding `held` does not hold, offset by `start`, compared in
/// [`BLOCK`] units and bridging runs of up to [`BRIDGE`] unchanged blocks so one changed window
/// is one write. `fresh` and `held` are the same length.
///
/// This is how a pass writes only what changed: a frame that stages what the last one staged
/// writes nothing, and one in which a window moved writes that window's constants alone.
pub fn unheld(start: usize, fresh: &[u8], held: &[u8]) -> Vec<std::ops::Range<usize>> {
    let mut spans: Vec<std::ops::Range<usize>> = Vec::new();
    let mut at = 0;
    while at < fresh.len() {
        let end = (at + BLOCK).min(fresh.len());
        if fresh[at..end] != held[at..end] {
            match spans.last_mut() {
                Some(span) if at - span.end <= BRIDGE * BLOCK => span.end = end,
                _ => spans.push(at..end),
            }
        }
        at = end;
    }
    for span in &mut spans {
        span.start += start;
        span.end += start;
    }
    spans
}

#[cfg(test)]
mod unheld_tests {
    #[test]
    fn unheld_blocks_bridge_short_unchanged_runs_and_split_long_ones() {
        let held = vec![0_u8; super::BLOCK * 12];
        assert!(super::unheld(0, &held, &held).is_empty());
        let mut fresh = held.clone();
        fresh[super::BLOCK] = 1;
        assert_eq!(
            super::unheld(0, &fresh, &held),
            vec![super::BLOCK..super::BLOCK * 2]
        );
        // One write bridges up to BRIDGE unchanged blocks; a longer run is two writes.
        fresh[super::BLOCK * 6] = 1;
        assert_eq!(
            super::unheld(0, &fresh, &held),
            vec![super::BLOCK..super::BLOCK * 7]
        );
        let mut apart = held.clone();
        apart[0] = 1;
        apart[super::BLOCK * 11] = 1;
        assert_eq!(
            super::unheld(8, &apart, &held),
            vec![8..super::BLOCK + 8, super::BLOCK * 11 + 8..super::BLOCK * 12 + 8]
        );
        // A tail shorter than a block is compared and written whole.
        let short = vec![0_u8; 3];
        assert!(super::unheld(0, &short, &short).is_empty());
        assert_eq!(super::unheld(4, &[1, 0, 0], &short), vec![4..7]);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureBinding {
    pub view: wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub array: Option<(wgpu::TextureView, u32)>,
}

impl From<wgpu::TextureView> for TextureBinding {
    fn from(view: wgpu::TextureView) -> Self {
        Self {
            format: view.texture().format(),
            view,
            array: None,
        }
    }
}

impl std::ops::Deref for TextureBinding {
    type Target = wgpu::TextureView;
    fn deref(&self) -> &Self::Target {
        &self.view
    }
}

/// One user's pipeline: a device's shared [`Core`] and the bind groups this user's own arena made.
pub struct Pipeline {
    core: Arc<Core>,
    topology: wgpu::PrimitiveTopology,
    /// Stamped with this user's arena frames, so users of one core never prune each other's.
    groups: Arc<Mutex<Groups>>,
    /// Shader-filtered variants by native binding mask, made for this user; `None` while one
    /// builds elsewhere ([`Pipeline::start_variant`]).
    variants: Arc<Mutex<HashMap<u32, Option<Cached>>>>,
}

/// What a pipeline is on its device: the compiled program, its layouts and samplers. Every user
/// on the device with the same key draws with one core ([`Cache`]).
pub struct Core {
    _source: Arc<Source>,
    batch: Option<Arc<batch::Program>>,
    /// Each constant binding the draw reads one row of per instance, ascending, and its row
    /// stride; empty when the pass draws no instance rows.
    rows: Vec<(u32, u32)>,
    /// The vertex stream the draw reads from storage per instance, no longer a vertex buffer.
    stream: Option<usize>,
    pub pipeline: wgpu::RenderPipeline,
    label: String,
    null_cube: Option<wgpu::TextureView>,
    /// Constant and texture layouts; the samplers are fixed per pipeline.
    layouts: [wgpu::BindGroupLayout; 2],
    samplers: wgpu::BindGroup,
    /// Ascending binding order, the order of the constant group's dynamic offsets.
    buffers: Vec<Buffer>,
    textures: Vec<(u32, super::TextureKind)>,
    /// Texture bindings with a filtering image/sampler pair.
    filtered: Vec<u32>,
    fallback: Option<Fallback>,
    packed_inputs: [super::vertices::Packing; 4],
    /// What the pipeline leaves unwritten of its pass's depth-stencil attachment, as a render
    /// bundle drawing with it declares it.
    read_only: ReadOnly,
}

/// What a pipeline leaves unwritten of its pass's depth-stencil attachment, by wgpu's own rule
/// ([`wgpu::DepthStencilState::is_depth_read_only`], [`wgpu::DepthStencilState::is_stencil_read_only`]).
/// A render bundle declares read-only what none of its pipelines writes, so it executes in every
/// pass its draws could be drawn in directly: a pass reading an aspect only takes a bundle that
/// reads it only, and a pipeline writing it is refused there either way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadOnly {
    pub depth: bool,
    pub stencil: bool,
}

impl ReadOnly {
    /// Nothing written.
    pub const ALL: Self = Self { depth: true, stencil: true };

    /// What a pipeline made for `pass` leaves unwritten.
    pub fn of(pass: &Pass) -> Self {
        pass.depth_stencil.as_ref().map_or(Self::ALL, |state| Self {
            depth: state.is_depth_read_only(),
            stencil: state.is_stencil_read_only(pass.primitive.cull_mode),
        })
    }

    /// Unwritten by both.
    pub fn and(self, other: Self) -> Self {
        Self { depth: self.depth && other.depth, stencil: self.stencil && other.stencil }
    }

    /// A bundle's depth-stencil attachment of `format`, declared so.
    pub fn bundle(self, format: wgpu::TextureFormat) -> wgpu::RenderBundleDepthStencil {
        wgpu::RenderBundleDepthStencil { format, depth_read_only: self.depth, stencil_read_only: self.stencil }
    }
}

impl std::ops::Deref for Pipeline {
    type Target = Core;
    fn deref(&self) -> &Core {
        &self.core
    }
}

impl Pipeline {
    /// A new user of `core`, with bind groups of its own.
    fn over(core: Arc<Core>, topology: wgpu::PrimitiveTopology) -> Self {
        Self {
            core,
            topology,
            groups: Arc::default(),
            variants: Arc::default(),
        }
    }
}

/// Float formats are known only when views are bound; keep variants by native binding mask.
struct Fallback {
    gui: bool,
    compiled: Compiled,
    effect: String,
    technique: Technique,
    state: State,
    pass: Pass,
}

/// Bind groups by resource identity: constants per arena buffer, textures per view list.
/// An entry unused for a whole frame is dropped, releasing the resources it holds.
#[derive(Default)]
struct Groups {
    frame: u64,
    constants: Vec<(Vec<wgpu::Buffer>, wgpu::BindGroup, GroupStamp)>,
    batch_constants: Vec<(Vec<wgpu::Buffer>, wgpu::BindGroup, GroupStamp)>,
    /// By the hash of the views, so a hit clones none of them.
    textures: HashMap<u64, Vec<(Vec<wgpu::TextureView>, wgpu::BindGroup, GroupStamp)>>,
    /// The last texture entry used: consecutive draws usually share a material.
    last: Option<(u64, usize)>,
    clock: Arc<GroupClock>,
}

impl Groups {
    fn advance(&mut self, frame: u64) {
        if self.frame != frame {
            self.clock.invalidate();
            self.frame = frame;
            let live = |used: &GroupStamp| used.get() + 1 >= frame;
            self.constants.retain(|(_, _, used)| live(used));
            self.batch_constants.retain(|(_, _, used)| live(used));
            for entries in self.textures.values_mut() {
                entries.retain(|(_, _, used)| live(used));
            }
            self.textures.retain(|_, entries| !entries.is_empty());
            self.last = None;
        }
    }
}

pub struct Draw {
    bindings: Bindings,
    vertices: Slice,
    count: u32,
}

/// Owned vertex layouts allow any pass ABI, including separate and instanced buffers.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VertexLayout {
    pub stride: u64,
    pub step: wgpu::VertexStepMode,
    pub attributes: Vec<wgpu::VertexAttribute>,
}

/// Complete immutable pass state, part of the device cache key. Dynamic viewport/scissor,
/// stencil reference and blend constants remain the caller's render-pass commands.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Pass {
    pub vertices: Vec<VertexLayout>,
    pub primitive: wgpu::PrimitiveState,
    pub depth_stencil: Option<wgpu::DepthStencilState>,
    pub multisample: wgpu::MultisampleState,
    pub targets: Vec<Option<wgpu::ColorTargetState>>,
    /// The constant bindings whose block the draw reads one of per instance, by
    /// `binding(slot, stage)`. Zero draws each instance's blocks as its own draw; a program that
    /// cannot take them per instance is refused by name and keeps the plain pipeline.
    pub instanced: u32,
    /// The vertex stream, the last one, the draw reads one slab of per instance, read from
    /// storage instead of fetched. It is no longer a vertex buffer the caller binds.
    pub stream: Option<usize>,
}

impl Pass {
    /// Tightly packed 32-bit input helper. Map callers can provide their native layouts directly.
    pub fn packed(
        compiled: &Compiled,
        state: &State,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Self {
        let mut stride = 0;
        let attributes = compiled
            .vertex
            .inputs
            .iter()
            .map(|input| {
                use super::ScalarKind;
                let format = match (input.kind, input.components) {
                    (ScalarKind::Float, 1) => wgpu::VertexFormat::Float32,
                    (ScalarKind::Float, 2) => wgpu::VertexFormat::Float32x2,
                    (ScalarKind::Float, 3) => wgpu::VertexFormat::Float32x3,
                    (ScalarKind::Float, _) => wgpu::VertexFormat::Float32x4,
                    (ScalarKind::Sint, 1) => wgpu::VertexFormat::Sint32,
                    (ScalarKind::Sint, 2) => wgpu::VertexFormat::Sint32x2,
                    (ScalarKind::Sint, 3) => wgpu::VertexFormat::Sint32x3,
                    (ScalarKind::Sint, _) => wgpu::VertexFormat::Sint32x4,
                    (ScalarKind::Uint, 1) => wgpu::VertexFormat::Uint32,
                    (ScalarKind::Uint, 2) => wgpu::VertexFormat::Uint32x2,
                    (ScalarKind::Uint, 3) => wgpu::VertexFormat::Uint32x3,
                    (ScalarKind::Uint, _) => wgpu::VertexFormat::Uint32x4,
                };
                let attribute = wgpu::VertexAttribute {
                    format,
                    offset: stride,
                    shader_location: input.location,
                };
                stride += u64::from(input.components) * 4;
                attribute
            })
            .collect();
        Self {
            vertices: vec![VertexLayout {
                stride,
                step: wgpu::VertexStepMode::Vertex,
                attributes,
            }],
            primitive: Default::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                ..Default::default()
            },
            targets: vec![Some(wgpu::ColorTargetState {
                format,
                blend: state.blend,
                write_mask: state.write_mask,
            })],
            instanced: 0,
            stream: None,
        }
    }
}

/// The bytes of a draw's rows block ([`BindingProgram::rows_block`]), which it selects with a
/// dynamic offset and which must stand at the device's uniform offset alignment.
pub const ROWS_BYTES: usize = batch_shader::ROWS_BYTES;

/// Every `binding(slot, stage)` of a constant buffer or texture is below these: D3D11's
/// registers over both stages, which compilation enforces.
pub const CONSTANT_BINDINGS: usize = 2 * super::BUFFER_SLOTS as usize;
pub const TEXTURE_BINDINGS: usize = 2 * super::SAMPLER_SLOTS as usize;
const BUFFERS: usize = CONSTANT_BINDINGS + 1;
const TEXTURES: usize = TEXTURE_BINDINGS;

#[derive(Clone)]
pub struct Bindings {
    resources: ResourceHandle,
    offsets: [u32; BUFFERS],
    count: usize,
    #[cfg(any(test, feature = "test-support"))]
    proof: Vec<(u32, Vec<u8>)>,
}
impl Bindings {
    /// Logical staged bytes (including physical zero tails) and offsets, for exactness proofs.
    #[cfg(any(test, feature = "test-support"))]
    pub fn preparation_proof(&self) -> (&[(u32, Vec<u8>)], &[u32]) {
        (&self.proof, &self.offsets[..self.count])
    }

    /// Same pipeline and texture/sampler groups, independent of uploaded constants.
    #[cfg(any(test, feature = "test-support"))]
    pub fn same_resources(&self, other: &Self) -> bool {
        let (a, b) = (self.resources.get(), other.resources.get());
        a.pipeline == b.pipeline && a.groups[1..] == b.groups[1..]
    }

    pub fn bind(&self, pass: &mut wgpu::RenderPass<'_>) {
        self.bind_after(pass, None);
    }

    /// Bind after `previous` of the same pipeline in the same pass, setting only the
    /// groups that differ. Each set merges all of a group's resources into the pass.
    pub fn bind_after(&self, pass: &mut wgpu::RenderPass<'_>, previous: Option<&Self>) {
        let resources = self.resources.get();
        let before = previous.map(|previous| previous.resources.get());
        // The bound views may select a shader-filtered variant of the caller's pipeline.
        if before.is_none_or(|previous| previous.pipeline != resources.pipeline) {
            crate::gpu::stats::pipeline();
            pass.set_pipeline(&resources.pipeline);
        }
        let offsets = &self.offsets[..self.count];
        if previous.is_none_or(|previous| {
            previous.resources.get().groups[0] != resources.groups[0]
                || previous.offsets[..previous.count] != *offsets
        }) {
            crate::gpu::stats::bind();
            pass.set_bind_group(0, &resources.groups[0], offsets);
        }
        for index in 1..3 {
            if before.is_none_or(|previous| previous.groups[index] != resources.groups[index]) {
                crate::gpu::stats::bind();
                pass.set_bind_group(index as u32, &resources.groups[index], &[]);
            }
        }
    }

    /// What the pipeline these bindings set leaves unwritten of its pass's depth and stencil.
    pub fn read_only(&self) -> ReadOnly {
        self.resources.get().read_only
    }

    /// [`Self::bind_after`] into a render bundle, counting nothing (a bundle's commands are
    /// counted where it is executed); returns the pipeline and the bind group sets it made.
    pub fn bundle_after<'a>(
        &'a self,
        bundle: &mut wgpu::RenderBundleEncoder<'a>,
        previous: Option<&'a Self>,
    ) -> [u64; 2] {
        let resources = self.resources.get();
        let before = previous.map(|previous| previous.resources.get());
        let mut made = [0; 2];
        if before.is_none_or(|previous| previous.pipeline != resources.pipeline) {
            made[0] += 1;
            bundle.set_pipeline(&resources.pipeline);
        }
        let offsets = &self.offsets[..self.count];
        if previous.is_none_or(|previous| {
            previous.resources.get().groups[0] != resources.groups[0]
                || previous.offsets[..previous.count] != *offsets
        }) {
            made[1] += 1;
            bundle.set_bind_group(0, &resources.groups[0], offsets);
        }
        for index in 1..3 {
            if before.is_none_or(|previous| previous.groups[index] != resources.groups[index]) {
                made[1] += 1;
                bundle.set_bind_group(index as u32, &resources.groups[index], &[]);
            }
        }
        made
    }
}

#[cfg(any(test, feature = "test-support"))]
fn preparation_bytes(specs: &[Buffer], blocks: &[(&[u8], u64)]) -> Vec<(u32, Vec<u8>)> {
    specs
        .iter()
        .zip(blocks)
        .map(|(spec, (bytes, size))| {
            let mut bytes = bytes.to_vec();
            bytes.resize(bytes.len().max(*size as usize), 0);
            (spec.binding, bytes)
        })
        .collect()
}

#[cfg(test)]
#[path = "gpu/program_tests.rs"]
mod program_tests;

#[path = "gpu/session.rs"]
mod session;
pub use session::{BindingSession, BindingSessionStorage};
use session::{GroupClock, GroupResources, GroupStamp, ResolvedGroups, ResourceHandle};

/// Immutable resources of an actually successful bind, including a shader-filtered variant.
/// This is not native retained state: callers still perform their ordered checks, fallbacks
/// and binds. Only identical resource identities may reuse it; every constant is restaged.
#[derive(Clone)]
pub struct BindingProgram {
    pipeline: wgpu::RenderPipeline,
    read_only: ReadOnly,
    samplers: wgpu::BindGroup,
    textures: wgpu::BindGroup,
    views: Vec<wgpu::TextureView>,
    texture_slots: Vec<u32>,
    hash: u64,
    buffers: Vec<Buffer>,
    /// Each constant binding the draw reads one row of per instance, and its row stride.
    rows: Vec<(u32, u32)>,
    /// The instanced vertex stream's words and each instance's first vertex in them.
    stream: Option<[wgpu::Buffer; 2]>,
    layouts: [wgpu::BindGroupLayout; 2],
    groups: Arc<Mutex<Groups>>,
    label: String,
}

impl BindingProgram {
    /// The effective pipeline, not necessarily the pipeline originally requested.
    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }

    /// Read-only fixture access: prove the exact successful program and recover
    /// the actual constant buffer bound by this draw, within its own owner.
    #[cfg(any(test, feature = "test-support"))]
    pub fn constant_buffer_for_tests(&self, bindings: &Bindings) -> Option<wgpu::Buffer> {
        let actual = bindings.resources.get();
        if actual.pipeline != self.pipeline || actual.groups[2] != self.samplers {
            return None;
        }
        let groups = self
            .groups
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // A captured texture group may have retired. Prove the actual live group's
        // complete view list, not equality with the capture's obsolete group handle.
        if !groups
            .textures
            .values()
            .flatten()
            .any(|(views, group, _)| *group == actual.groups[1] && *views == self.views)
        {
            return None;
        }
        groups
            .constants
            .iter()
            .chain(&groups.batch_constants)
            .find(|(_, group, _)| *group == actual.groups[0])
            .and_then(|(buffers, _, _)| buffers.first().cloned())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn texture_views_for_tests(&self) -> impl Iterator<Item = (u32, &wgpu::TextureView)> {
        self.texture_slots.iter().copied().zip(&self.views)
    }

    /// Reflected physical sizes and conservative reads in dynamic-offset order.
    pub fn constant_reads(&self) -> &[Buffer] {
        &self.buffers
    }

    /// Each constant binding the draw reads one row of per instance, ascending, and its row
    /// stride; empty when the pass reads none per instance.
    pub fn instanced_rows(&self) -> &[(u32, u32)] {
        &self.rows
    }

    /// The row stride of `binding`, when the draw reads one row of it per instance.
    fn row_stride(&self, binding: u32) -> Option<u32> {
        self.rows
            .iter()
            .find(|(row, _)| *row == binding)
            .map(|(_, stride)| *stride)
    }

    /// Stage all constants through the ordinary frame arena, with its unchanged allocation,
    /// deduplication and failure order. Resource groups keep the pipeline's normal retirement.
    pub fn bind_with<'a>(
        &self,
        arena: &Arena,
        constants: impl Fn(u32) -> Option<&'a [u8]>,
    ) -> Result<Bindings, String> {
        self.bind(arena, constants)
            .map_err(|reason| format!("{}: {reason}", self.label))
    }

    fn bind<'a>(
        &self,
        arena: &Arena,
        constants: impl Fn(u32) -> Option<&'a [u8]>,
    ) -> Result<Bindings, String> {
        let count = self.buffers.len();
        let mut blocks = [(&[][..], 0); BUFFERS];
        for (block, spec) in blocks.iter_mut().zip(&self.buffers) {
            let bytes = constants(spec.binding).ok_or_else(|| {
                format!(
                    "native constant buffer {} has not been bound",
                    spec.binding / 2
                )
            })?;
            *block = (bytes, u64::from(spec.size));
        }
        let mut offsets = [0; BUFFERS];
        let source = arena.constants(&blocks[..count], &mut offsets[..count])?;
        let resolved = self.resolve_groups(arena, source, false)?;
        Ok(Bindings {
            resources: ResourceHandle::Inline(resolved.resources),
            offsets,
            count,
            #[cfg(any(test, feature = "test-support"))]
            proof: preparation_bytes(&self.buffers, &blocks[..count]),
        })
    }

    /// Bindings of constants a caller placed in a buffer it keeps and writes itself, as the arena
    /// places them: each read's bytes at its offset, zero-filled to the read's declared size.
    /// `blocks` gives each read's bytes and offset in [`Self::constant_reads`] order.
    pub fn bind_buffer(
        &self,
        arena: &Arena,
        buffer: &wgpu::Buffer,
        blocks: &[(&[u8], u32)],
    ) -> Result<Bindings, String> {
        if blocks.len() != self.buffers.len() {
            return Err(format!(
                "{}: {} constant blocks for {} reads",
                self.label,
                blocks.len(),
                self.buffers.len()
            ));
        }
        let mut offsets = [0; BUFFERS];
        for (slot, (_, offset)) in offsets.iter_mut().zip(blocks) {
            *slot = *offset;
        }
        let resolved = self
            .resolve_groups(arena, buffer.clone(), false)
            .map_err(|reason| format!("{}: {reason}", self.label))?;
        #[cfg(any(test, feature = "test-support"))]
        let sized: Vec<_> = blocks
            .iter()
            .zip(&self.buffers)
            .map(|((bytes, _), spec)| (*bytes, u64::from(spec.size)))
            .collect();
        Ok(Bindings {
            resources: ResourceHandle::Inline(resolved.resources),
            offsets,
            count: self.buffers.len(),
            #[cfg(any(test, feature = "test-support"))]
            proof: preparation_bytes(&self.buffers, &sized),
        })
    }

    /// The bytes of a draw's rows block: the base row of each binding it reads per instance, and
    /// of its instanced stream's bases. `rows` gives each in [`Self::instanced_rows`] order, the
    /// stream's last when the pass reads one.
    pub fn rows_block(&self, rows: &[u32]) -> Result<[u8; batch::ROWS_BYTES], String> {
        let wanted = self.rows.len() + usize::from(self.stream.is_some());
        if rows.len() != wanted {
            return Err(format!(
                "{}: {} base rows for {wanted} reads per instance",
                self.label,
                rows.len()
            ));
        }
        let mut block = [0u8; batch::ROWS_BYTES];
        let places = self
            .rows
            .iter()
            .map(|(binding, _)| *binding as usize)
            .chain(self.stream.iter().map(|_| batch::STREAM_ROW));
        for (at, row) in places.zip(rows) {
            block[at * 4..at * 4 + 4].copy_from_slice(&row.to_le_bytes());
        }
        Ok(block)
    }

    /// Bindings of a draw whose per-instance rows stand in `buffer`: `blocks` gives each read of
    /// [`Self::constant_reads`] its bytes and offset, as [`Self::bind_buffer`] takes them, and
    /// `rows` the offset of the draw's own rows block there. A read taken per instance keeps its
    /// bytes for the proof; its offset is the whole array's and is never bound dynamically.
    pub fn bind_rows(
        &self,
        arena: &Arena,
        buffer: &wgpu::Buffer,
        blocks: &[(&[u8], u32)],
        rows: u32,
    ) -> Result<Bindings, String> {
        if blocks.len() != self.buffers.len() {
            return Err(format!(
                "{}: {} constant blocks for {} reads",
                self.label,
                blocks.len(),
                self.buffers.len()
            ));
        }
        let mut offsets = [0; BUFFERS];
        let mut count = 0;
        for ((_, offset), spec) in blocks.iter().zip(&self.buffers) {
            if self.row_stride(spec.binding).is_none() {
                offsets[count] = *offset;
                count += 1;
            }
        }
        offsets[count] = rows;
        count += 1;
        let resolved = self
            .resolve_groups(arena, buffer.clone(), false)
            .map_err(|reason| format!("{}: {reason}", self.label))?;
        #[cfg(any(test, feature = "test-support"))]
        let sized: Vec<_> = blocks
            .iter()
            .zip(&self.buffers)
            .map(|((bytes, _), spec)| (*bytes, u64::from(spec.size)))
            .collect();
        Ok(Bindings {
            resources: ResourceHandle::Inline(resolved.resources),
            offsets,
            count,
            #[cfg(any(test, feature = "test-support"))]
            proof: preparation_bytes(&self.buffers, &sized),
        })
    }

    fn resolve_groups(
        &self,
        arena: &Arena,
        source: wgpu::Buffer,
        capture_use: bool,
    ) -> Result<ResolvedGroups, String> {
        let frame = arena.frame();
        let device = arena.device();
        let mut groups = self.groups.lock().unwrap_or_else(|e| e.into_inner());
        let groups = &mut *groups;
        groups.advance(frame);
        let held: Vec<wgpu::Buffer> = std::iter::once(source.clone())
            .chain(self.stream.iter().flatten().cloned())
            .collect();
        let rows = !self.rows.is_empty() || self.stream.is_some();
        let cached = if rows {
            &mut groups.batch_constants
        } else {
            &mut groups.constants
        };
        let (constants, constants_stamp) =
            match cached.iter_mut().find(|(buffers, _, _)| *buffers == held) {
                Some((_, group, used)) => {
                    used.set(frame);
                    (group.clone(), capture_use.then(|| used.clone()))
                }
                None => {
                    let group = scoped(device, || {
                        crate::gpu::stats::create_bind_group(&device, &wgpu::BindGroupDescriptor {
                            label: Some("native constants"),
                            layout: &self.layouts[0],
                            entries: &self
                                .buffers
                                .iter()
                                .map(|spec| wgpu::BindGroupEntry {
                                    binding: spec.binding,
                                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                        buffer: &source,
                                        offset: 0,
                                        // A row's array is bound whole; the instance index picks
                                        // the row from the base the draw's rows block gives.
                                        size: self
                                            .row_stride(spec.binding)
                                            .is_none()
                                            .then(|| {
                                                std::num::NonZeroU64::new(u64::from(spec.size))
                                            })
                                            .flatten(),
                                    }),
                                })
                                .chain(rows.then(|| wgpu::BindGroupEntry {
                                    binding: batch::ROWS_BINDING,
                                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                        buffer: &source,
                                        offset: 0,
                                        size: std::num::NonZeroU64::new(batch::ROWS_BYTES as u64),
                                    }),
                                }))
                                .chain(
                                    self.stream
                                        .iter()
                                        .flatten()
                                        .zip(batch::STREAM_BINDINGS)
                                        .map(|(buffer, binding)| wgpu::BindGroupEntry {
                                            binding,
                                            resource: buffer.as_entire_binding(),
                                        }),
                                )
                                .collect::<Vec<_>>(),
                        })
                    })?;
                    let used = GroupStamp::new(frame);
                    cached.push((held, group.clone(), used.clone()));
                    (group, capture_use.then_some(used))
                }
            };
        let entries = groups.textures.entry(self.hash).or_default();
        let found = entries
            .iter()
            .position(|(_, group, _)| *group == self.textures)
            .or_else(|| {
                entries
                    .iter()
                    .position(|(views, _, _)| *views == self.views)
            });
        let (index, textures, textures_stamp) = match found {
            Some(index) => {
                let (_, group, used) = &mut entries[index];
                used.set(frame);
                (index, group.clone(), capture_use.then(|| used.clone()))
            }
            None => {
                let group = scoped(device, || {
                    crate::gpu::stats::create_bind_group(&device, &wgpu::BindGroupDescriptor {
                        label: Some("native textures"),
                        layout: &self.layouts[1],
                        entries: &self
                            .views
                            .iter()
                            .zip(&self.texture_slots)
                            .map(|(view, binding)| wgpu::BindGroupEntry {
                                binding: *binding,
                                resource: wgpu::BindingResource::TextureView(view),
                            })
                            .collect::<Vec<_>>(),
                    })
                })?;
                let used = GroupStamp::new(frame);
                entries.push((self.views.clone(), group.clone(), used.clone()));
                (entries.len() - 1, group, capture_use.then_some(used))
            }
        };
        groups.last = Some((self.hash, index));
        groups
            .clock
            .hint
            .store(true, std::sync::atomic::Ordering::Release);

        Ok(ResolvedGroups {
            resources: GroupResources {
                pipeline: self.pipeline.clone(),
                read_only: self.read_only,
                groups: [constants, textures, self.samplers.clone()],
            },
            constants: constants_stamp,
            textures: textures_stamp,
            clock: capture_use.then(|| Arc::clone(&groups.clock)),
            revision: groups.clock.revision(),
        })
    }
}

/// Resolved native state after applying this draw's binds. Program selection, blend selection
/// and each sampler bind are independent, especially for the null-fallback technique.
#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub samplers: BTreeMap<u32, Sampler>,
    pub blend: Option<wgpu::BlendState>,
    pub write_mask: wgpu::ColorWrites,
}

impl State {
    pub fn authored(program: &Program, technique: &Technique) -> Result<Self, String> {
        let (blend, write_mask) = blend(program, technique)?;
        let mut samplers = BTreeMap::new();
        for sampler in program
            .common
            .samplers
            .iter()
            .chain(&program.pixel.samplers)
        {
            samplers.insert(super::binding(sampler.index, Stage::Pixel), sampler.clone());
        }
        for sampler in &program.vertex.samplers {
            samplers.insert(
                super::binding(sampler.index, Stage::Vertex),
                sampler.clone(),
            );
        }
        Ok(Self {
            samplers,
            blend,
            write_mask,
        })
    }
}

#[derive(Clone)]
struct PipelineKey {
    gui: bool,
    effect: uuid::Uuid,
    technique: usize,
    samplers: Vec<(u32, [u32; 10])>,
    blend: Option<wgpu::BlendState>,
    write_mask: wgpu::ColorWrites,
    pass: Pass,
}

pub(crate) fn sampler_key(sampler: &Sampler) -> [u32; 10] {
    [
        sampler.mag as u32,
        sampler.min as u32,
        sampler.mip as u32,
        sampler.u as u32,
        sampler.v as u32,
        sampler.w as u32,
        sampler.lod_bias.to_bits(),
        sampler.min_lod as u32,
        sampler.max_lod as u32,
        u32::from(sampler.cube),
    ]
}

/// The bound samplers a pipeline is built from: those its program reads, or every one when the
/// program is not compiled. The others cannot change the pipeline.
fn read_samplers<'a>(
    state: &'a State,
    reads: Option<&'a Compiled>,
) -> impl Iterator<Item = (u32, [u32; 10])> + 'a {
    state
        .samplers
        .iter()
        .filter(move |(binding, _)| {
            reads.is_none_or(|compiled| {
                compiled.vertex.samplers.contains(binding)
                    || compiled.pixel.samplers.contains(binding)
            })
        })
        .map(|(binding, sampler)| (*binding, sampler_key(sampler)))
}

impl PipelineKey {
    fn same(&self, other: &Self) -> bool {
        self.gui == other.gui
            && self.effect == other.effect
            && self.technique == other.technique
            && self.blend == other.blend
            && self.write_mask == other.write_mask
            && self.pass == other.pass
            && self.samplers == other.samplers
    }

    /// Hash of the key a lookup would build, from borrowed parts.
    fn hash(
        gui: bool,
        effect: uuid::Uuid,
        technique: usize,
        state: &State,
        pass: &Pass,
        reads: Option<&Compiled>,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        (gui, effect, technique, state.blend, state.write_mask, pass).hash(&mut hasher);
        for sampler in read_samplers(state, reads) {
            sampler.hash(&mut hasher);
        }
        hasher.finish()
    }

    fn matches(
        &self,
        gui: bool,
        effect: uuid::Uuid,
        technique: usize,
        state: &State,
        pass: &Pass,
        reads: Option<&Compiled>,
    ) -> bool {
        self.gui == gui
            && self.effect == effect
            && self.technique == technique
            && self.blend == state.blend
            && self.write_mask == state.write_mask
            && self.pass == *pass
            && self
                .samplers
                .iter()
                .copied()
                .eq(read_samplers(state, reads))
    }
}

pub type Cached = Result<Arc<Pipeline>, String>;

/// GUI pipeline and texture-variant builds under way on their threads. Nothing waits on them:
/// a frame draws what is ready and the next one picks up what has been built since.
static BUILDING: Mutex<usize> = Mutex::new(0);

/// Counts one build under way until dropped.
pub(crate) struct Building(());

impl Building {
    pub(crate) fn start() -> Self {
        *BUILDING.lock().unwrap_or_else(|error| error.into_inner()) += 1;
        Self(())
    }
}

/// How many GUI pipeline and texture-variant builds are under way.
pub fn builds_under_way() -> usize {
    *BUILDING.lock().unwrap_or_else(|error| error.into_inner())
}

impl Drop for Building {
    fn drop(&mut self) {
        *BUILDING.lock().unwrap_or_else(|error| error.into_inner()) -= 1;
    }
}

/// Compare only sampler slots the compiled program reads.
pub(crate) fn same_packed(cached: &State, state: &State, reads: Option<&Compiled>) -> bool {
    cached.blend == state.blend
        && cached.write_mask == state.write_mask
        && read_samplers(cached, reads).eq(read_samplers(state, reads))
}

type PackedKey = (
    uuid::Uuid,
    usize,
    wgpu::TextureFormat,
    u32,
    wgpu::PrimitiveTopology,
);

/// Every device's pipeline cores by [`PipelineKey`], held weakly: a cache that misses takes a core
/// another user on its device already holds, so N map views compile each pipeline once, and the
/// last user lets it go. Failures are not shared; each cache keeps its own. A core is made
/// outside the lock, so a lookup never waits on another thread's build; only a user asking for
/// the core under way waits for it.
struct DeviceCores(Mutex<Vec<DeviceEntry>>, Condvar);

struct DeviceEntry {
    device: wgpu::Device,
    cores: HashMap<u64, Vec<(PipelineKey, std::sync::Weak<Core>, Diagnostics)>>,
    /// Cores being made, by hash.
    building: Vec<(u64, PipelineKey)>,
}

static DEVICE_CORES: DeviceCores = DeviceCores(Mutex::new(Vec::new()), Condvar::new());

/// Why [`DeviceCores::get_or_create`] handed over no core.
enum NotMade {
    /// Making it failed, by name; the caller keeps the failure.
    Failed(String),
    /// Another thread is making it and the caller does not wait: the thread that draws the
    /// frames draws without it this frame, and keeps nothing.
    Building,
}

struct Source {
    compiled: Compiled,
    samplers: Vec<(u32, [u32; 10])>,
    vertices: Vec<VertexLayout>,
    topology: wgpu::PrimitiveTopology,
    strip_safe: bool,
    unfilterable: u32,
    gui: bool,
    instanced: u32,
    stream: Option<usize>,
    prepared: OnceLock<Result<Modules, String>>,
    null_cube: Arc<Mutex<Option<wgpu::TextureView>>>,
}

struct Modules {
    batch: Option<Arc<batch::Program>>,
    instanced: Option<Arc<batch_shader::Instanced>>,
    vertex: wgpu::ShaderModule,
    pixel: wgpu::ShaderModule,
}

struct DeviceSources {
    device: wgpu::Device,
    sources: Vec<std::sync::Weak<Source>>,
    null_cube: std::sync::Weak<Mutex<Option<wgpu::TextureView>>>,
}

static SOURCES: Mutex<Vec<DeviceSources>> = Mutex::new(Vec::new());

impl Source {
    fn get(
        device: &wgpu::Device,
        compiled: &Compiled,
        state: &State,
        pass: &Pass,
        unfilterable: u32,
        gui: bool,
    ) -> Arc<Self> {
        let samplers: Vec<_> = read_samplers(state, Some(compiled)).collect();
        let mut retired = Retired(Vec::new());
        let mut devices = SOURCES.lock().unwrap_or_else(|error| error.into_inner());
        retired.0.extend(devices.extract_if(.., |entry| {
            entry.sources.retain(|source| source.strong_count() != 0);
            entry.sources.is_empty()
        }));
        let entry = match devices.iter().position(|entry| entry.device == *device) {
            Some(index) => &mut devices[index],
            None => {
                devices.push(DeviceSources { device: device.clone(), sources: Vec::new(), null_cube: Default::default() });
                devices.last_mut().unwrap()
            }
        };
        let sources = &mut entry.sources;
        if let Some(source) = sources
            .iter()
            .filter_map(std::sync::Weak::upgrade)
            .find(|source| {
                source.gui == gui
                    && source.unfilterable == unfilterable
                    && source.instanced == pass.instanced
                    && source.stream == pass.stream
                    && (source.topology == pass.primitive.topology
                        || (source.strip_safe
                            && matches!(
                                source.topology,
                                wgpu::PrimitiveTopology::TriangleList
                                    | wgpu::PrimitiveTopology::TriangleStrip
                            )
                            && matches!(
                                pass.primitive.topology,
                                wgpu::PrimitiveTopology::TriangleList
                                    | wgpu::PrimitiveTopology::TriangleStrip
                            )))
                    && source.vertices == pass.vertices
                    && source.samplers == samplers
                    && source.compiled == *compiled
            })
        {
            return source;
        }
        let null_cube = entry.null_cube.upgrade().unwrap_or_else(|| {
            let shared = Arc::new(Mutex::new(None));
            entry.null_cube = Arc::downgrade(&shared);
            shared
        });
        let source = Arc::new(Self {
            compiled: compiled.clone(),
            samplers,
            vertices: pass.vertices.clone(),
            topology: pass.primitive.topology,
            strip_safe: gui && batch_shader::strip_safe(&compiled.vertex.wgsl),
            unfilterable,
            gui,
            instanced: pass.instanced,
            stream: pass.stream,
            prepared: OnceLock::new(),
            null_cube,
        });
        sources.push(Arc::downgrade(&source));
        source
    }

    fn prepare(
        &self,
        device: &wgpu::Device,
        state: &State,
        pass: &Pass,
        label: &str,
    ) -> Result<&Modules, String> {
        self.prepared
            .get_or_init(|| {
                scoped(device, || {
                    let mut translated = self.compiled.clone();
                    translated.vertex.wgsl = super::sampling::lower(
                        &self.compiled.vertex.wgsl,
                        &state.samplers,
                        &self.compiled.vertex.textures,
                        self.unfilterable,
                    )?;
                    translated.pixel.wgsl = super::sampling::lower(
                        &self.compiled.pixel.wgsl,
                        &state.samplers,
                        &self.compiled.pixel.textures,
                        self.unfilterable,
                    )?;
                    let batch = if self.gui {
                        batch::Program::translate(device, &translated, pass, label)?.map(Arc::new)
                    } else {
                        None
                    };
                    let instanced = ((self.instanced != 0 || pass.stream.is_some())
                        && batch.is_none())
                    .then(|| batch::instanced(device, &translated, pass, label))
                    .flatten();
                    let module = |source: &str| {
                        device.create_shader_module(wgpu::ShaderModuleDescriptor {
                            label: Some(label),
                            source: wgpu::ShaderSource::Wgsl(source.into()),
                        })
                    };
                    Ok(Modules {
                        vertex: module(match (&batch, &instanced) {
                            (Some(batch), _) => &batch.shader.vertex,
                            (None, Some(instanced)) => &instanced.vertex,
                            (None, None) => &translated.vertex.wgsl,
                        }),
                        pixel: module(match (&batch, &instanced) {
                            (Some(batch), _) => &batch.shader.pixel,
                            (None, Some(instanced)) => instanced
                                .pixel
                                .as_deref()
                                .unwrap_or(&translated.pixel.wgsl),
                            (None, None) => &translated.pixel.wgsl,
                        }),
                        batch,
                        instanced,
                    })
                })
                .and_then(|result| result)
            })
            .as_ref()
            .map_err(Clone::clone)
    }
}

/// Takes a core under way off its device's list when its making ends, however it ends.
struct Making<'a> {
    cores: &'a DeviceCores,
    device: &'a wgpu::Device,
    hash: u64,
    key: PipelineKey,
}

impl Drop for Making<'_> {
    fn drop(&mut self) {
        let mut devices = self.cores.lock();
        if let Some(entry) = devices
            .iter_mut()
            .find(|entry| entry.device == *self.device)
            && let Some(at) = entry
                .building
                .iter()
                .position(|(hash, key)| *hash == self.hash && key.same(&self.key))
        {
            entry.building.swap_remove(at);
        }
        self.cores.1.notify_all();
    }
}

impl DeviceCores {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<DeviceEntry>> {
        self.0.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// The live core on `device` that `matches`, taken while it is still held, without making
    /// one.
    fn held(
        &self,
        device: &wgpu::Device,
        hash: u64,
        matches: impl Fn(&PipelineKey) -> bool,
    ) -> Option<(Arc<Core>, Diagnostics)> {
        let devices = self.lock();
        devices
            .iter()
            .find(|entry| entry.device == *device)
            .and_then(|entry| entry.cores.get(&hash))
            .and_then(|cores| {
                cores.iter().find_map(|(cached, core, diagnostics)| {
                    matches(cached)
                        .then(|| core.upgrade())
                        .flatten()
                        .map(|core| (core, diagnostics.clone()))
                })
            })
    }

    /// The live core on `device` that `matches`, else the one `create` makes, recorded under
    /// `key`. Users asking at once for one under way wait for it, so it is made once; a user
    /// that does not `wait` is told it is being built and draws without it.
    fn get_or_create(
        &self,
        device: &wgpu::Device,
        hash: u64,
        matches: impl Fn(&PipelineKey) -> bool,
        key: impl FnOnce() -> PipelineKey,
        create: impl FnOnce() -> (Result<Arc<Core>, String>, Diagnostics),
        wait: bool,
    ) -> (Result<Arc<Core>, NotMade>, Diagnostics) {
        let mut retired = Retired(Vec::new());
        let mut devices = self.lock();
        loop {
            retired.0.extend(devices.extract_if(.., |entry| {
                entry.cores.retain(|_, cores| {
                    cores.retain(|(_, core, _)| core.strong_count() != 0);
                    !cores.is_empty()
                });
                entry.cores.is_empty() && entry.building.is_empty() && entry.device != *device
            }));
            let at = match devices.iter().position(|entry| entry.device == *device) {
                Some(at) => at,
                None => {
                    devices.push(DeviceEntry {
                        device: device.clone(),
                        cores: HashMap::new(),
                        building: Vec::new(),
                    });
                    devices.len() - 1
                }
            };
            let entry = &devices[at];
            if let Some((core, diagnostics)) = entry.cores.get(&hash).and_then(|bucket| {
                bucket.iter().find_map(|(cached, core, diagnostics)| {
                    matches(cached)
                        .then(|| core.upgrade())
                        .flatten()
                        .map(|core| (core, diagnostics.clone()))
                })
            }) {
                return (Ok(core), diagnostics);
            }
            if !entry
                .building
                .iter()
                .any(|(building, key)| *building == hash && matches(key))
            {
                break;
            }
            if !wait {
                return (Err(NotMade::Building), Diagnostics::default());
            }
            devices = self
                .1
                .wait(devices)
                .unwrap_or_else(|error| error.into_inner());
        }
        let key = key();
        let entry = devices
            .iter_mut()
            .find(|entry| entry.device == *device)
            .expect("kept above");
        entry.building.push((hash, key.clone()));
        drop(devices);
        let making = Making {
            cores: self,
            device,
            hash,
            key,
        };
        let (result, diagnostics) = create();
        if let Ok(core) = &result {
            let mut devices = self.lock();
            if let Some(entry) = devices.iter_mut().find(|entry| entry.device == *device) {
                entry.cores.entry(hash).or_default().push((
                    making.key.clone(),
                    Arc::downgrade(core),
                    diagnostics.clone(),
                ));
            }
        }
        drop(making);
        (result.map_err(NotMade::Failed), diagnostics)
    }
}

/// Device-scoped pipelines, including validation failures, keyed by effect, technique, sampler
/// state and the complete pass. One cache belongs to one device. A hit clones no key. A miss
/// shares the device's core when any user holds one ([`DeviceCores`]), with bind groups its own.
#[derive(Default)]
pub struct Cache {
    pub(crate) gui: bool,
    sources: Vec<Arc<Source>>,
    /// Packed passes derive their complete vertex layout only on a state-cache miss.
    packed: HashMap<PackedKey, Vec<(State, Cached, Diagnostics)>>,
    entries: HashMap<u64, Vec<(PipelineKey, Cached, Diagnostics)>>,
    pub refused: BTreeMap<String, u64>,
    pub created: usize,
}

impl Cache {
    /// Release cache-held bindings after encoding; submitted commands keep their
    /// own handles. This lets the owner bound its retained texture uploads.
    pub fn release_bind_groups(&self) {
        for result in self
            .entries
            .values()
            .flatten()
            .map(|(_, result, _)| result)
            .chain(self.packed.values().flatten().map(|(_, result, _)| result))
        {
            if let Ok(pipeline) = result {
                pipeline.release_bind_groups();
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_texture_bind_groups(&self) -> usize {
        self.entries
            .values()
            .flatten()
            .filter_map(|(_, result, _)| result.as_ref().ok())
            .map(|pipeline| {
                pipeline
                    .groups
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .textures
                    .values()
                    .map(Vec::len)
                    .sum::<usize>()
            })
            .sum()
    }

    /// The packed pipeline of `selection` drawing the caller's `topology`; each topology
    /// is its own pipeline.
    pub fn get(
        &mut self,
        device: &wgpu::Device,
        selection: &super::library::Selection,
        state: &State,
        format: wgpu::TextureFormat,
        samples: u32,
        topology: wgpu::PrimitiveTopology,
    ) -> Result<Arc<Pipeline>, String> {
        let key = (
            selection.effect.id,
            selection.index,
            format,
            samples,
            topology,
        );
        if let Some(found) = self.find_packed(selection, state, (format, samples, topology)) {
            return found;
        }
        let (result, diagnostics) =
            self.build_packed(device, selection, state, format, samples, topology);
        diagnostics.credit();
        self.packed
            .entry(key)
            .or_default()
            .push((state.clone(), result.clone(), diagnostics));
        result
    }

    /// Find and credit a held packed pipeline without building it.
    pub fn find_packed(
        &mut self,
        selection: &super::library::Selection,
        state: &State,
        (format, samples, topology): (wgpu::TextureFormat, u32, wgpu::PrimitiveTopology),
    ) -> Option<Cached> {
        let key = (
            selection.effect.id,
            selection.index,
            format,
            samples,
            topology,
        );
        let reads = selection
            .effect
            .compiled_now(selection.index)
            .and_then(Result::ok);
        let (_, result, diagnostics) = self
            .packed
            .get(&key)?
            .iter()
            .find(|(cached, _, _)| same_packed(cached, state, reads.as_deref()))?;
        diagnostics.credit();
        if let Err(reason) = result {
            *self.refused.entry(reason.clone()).or_default() += 1;
        }
        Some(result.clone())
    }

    /// What [`Self::get`] makes on a miss, with what making it observed, kept by no cache.
    pub fn build_packed(
        &mut self,
        device: &wgpu::Device,
        selection: &super::library::Selection,
        state: &State,
        format: wgpu::TextureFormat,
        samples: u32,
        topology: wgpu::PrimitiveTopology,
    ) -> (Cached, Diagnostics) {
        diagnostics::capture(|| {
            let compiled = selection.effect.compiled(selection.index)?;
            let mut pass = Pass::packed(&compiled, state, format, samples);
            pass.primitive.topology = topology;
            self.get_pass(device, selection, state, &pass)
        })
    }

    /// Keeps a [`Self::build_packed`] result made elsewhere, as [`Self::get`] keeps its own.
    pub fn adopt_packed(
        &mut self,
        selection: &super::library::Selection,
        state: &State,
        (format, samples, topology): (wgpu::TextureFormat, u32, wgpu::PrimitiveTopology),
        (result, diagnostics): (Cached, Diagnostics),
    ) {
        let key = (
            selection.effect.id,
            selection.index,
            format,
            samples,
            topology,
        );
        let reads = selection
            .effect
            .compiled_now(selection.index)
            .and_then(Result::ok);
        let entries = self.packed.entry(key).or_default();
        if !entries
            .iter()
            .any(|(cached, _, _)| same_packed(cached, state, reads.as_deref()))
        {
            entries.push((state.clone(), result, diagnostics));
        }
    }

    /// Whether [`Self::get`] finds this draw's pipeline without compiling a program or building a
    /// pipeline: made here, or held on its device, which this cache then holds too.
    pub fn ready(
        &mut self,
        device: &wgpu::Device,
        selection: &super::library::Selection,
        state: &State,
        format: wgpu::TextureFormat,
        samples: u32,
        topology: wgpu::PrimitiveTopology,
    ) -> bool {
        let key = (
            selection.effect.id,
            selection.index,
            format,
            samples,
            topology,
        );
        let compiled = selection.effect.compiled_now(selection.index);
        let reads = compiled.as_ref().and_then(|result| result.as_deref().ok());
        if self.packed.get(&key).is_some_and(|entries| {
            entries
                .iter()
                .any(|(cached, _, _)| same_packed(cached, state, reads))
        }) {
            return true;
        }
        let compiled = match compiled {
            None => return false,
            // A failed compile is found, not built.
            Some(Err(_)) => return true,
            Some(Ok(compiled)) => compiled,
        };
        let mut pass = Pass::packed(&compiled, state, format, samples);
        pass.primitive.topology = topology;
        if self.gui
            && device
                .features()
                .contains(wgpu::Features::FLOAT32_FILTERABLE)
            && self.sources.iter().any(|source| {
                source.topology == topology
                    && source.vertices == pass.vertices
                    && source.compiled == *compiled
                    && source
                        .samplers
                        .iter()
                        .copied()
                        .eq(read_samplers(state, Some(&compiled)))
                    && source.prepared.get().is_some_and(|modules| {
                        modules
                            .as_ref()
                            .is_ok_and(|modules| modules.batch.is_some())
                    })
            })
        {
            pass.primitive.topology = wgpu::PrimitiveTopology::TriangleList;
            pass.primitive.strip_index_format = None;
        }
        let (effect, index, reads) = (selection.effect.id, selection.index, Some(&*compiled));
        let hash = PipelineKey::hash(self.gui, effect, index, state, &pass, reads);
        let matches =
            |cached: &PipelineKey| cached.matches(self.gui, effect, index, state, &pass, reads);
        if self
            .entries
            .get(&hash)
            .is_some_and(|bucket| bucket.iter().any(|(cached, _, _)| matches(cached)))
        {
            return true;
        }
        let Some((core, diagnostics)) = DEVICE_CORES.held(device, hash, matches) else {
            return false;
        };
        let key = PipelineKey {
            gui: self.gui,
            effect,
            technique: index,
            samplers: read_samplers(state, reads).collect(),
            blend: state.blend,
            write_mask: state.write_mask,
            pass,
        };
        self.entries.entry(hash).or_default().push((
            key,
            Ok(Arc::new(Pipeline::over(core, topology))),
            diagnostics,
        ));
        true
    }

    pub fn get_pass(
        &mut self,
        device: &wgpu::Device,
        selection: &super::library::Selection,
        state: &State,
        pass: &Pass,
    ) -> Result<Arc<Pipeline>, String> {
        let effect = &selection.effect;
        let topology = pass.primitive.topology;
        let source = if self.gui {
            let compiled = effect.compiled(selection.index)?;
            let source = Source::get(device, &compiled, state, pass, 0, true);
            if !self.sources.iter().any(|held| Arc::ptr_eq(held, &source)) {
                self.sources.push(Arc::clone(&source));
            }
            if let Err(reason) = source.prepare(
                device,
                state,
                pass,
                &format!(
                    "{}:{}",
                    effect.source.effect, effect.source.techniques[selection.index].name
                ),
            ) {
                let reason = format!(
                    "effect {:?}, technique {:?}: {reason}",
                    effect.source.effect, effect.source.techniques[selection.index].name
                );
                *self.refused.entry(reason.clone()).or_default() += 1;
                return Err(reason);
            }
            Some(source)
        } else {
            None
        };
        let mut canonical;
        let pass = if source.as_ref().is_some_and(|source| {
            source
                .prepared
                .get()
                .unwrap()
                .as_ref()
                .is_ok_and(|modules| modules.batch.is_some())
        }) && device
            .features()
            .contains(wgpu::Features::FLOAT32_FILTERABLE)
        {
            canonical = pass.clone();
            canonical.primitive.topology = wgpu::PrimitiveTopology::TriangleList;
            canonical.primitive.strip_index_format = None;
            &canonical
        } else {
            pass
        };
        let reads = effect.compiled_now(selection.index).and_then(Result::ok);
        let reads = reads.as_deref();
        let hash = PipelineKey::hash(self.gui, effect.id, selection.index, state, pass, reads);
        let bucket = self.entries.entry(hash).or_default();
        let result = match bucket.iter().find(|(key, _, _)| {
            key.matches(self.gui, effect.id, selection.index, state, pass, reads)
        }) {
            Some((_, result, diagnostics)) => {
                diagnostics.credit();
                result.clone()
            }
            None => {
                let key = || PipelineKey {
                    gui: self.gui,
                    effect: effect.id,
                    technique: selection.index,
                    samplers: read_samplers(state, reads).collect(),
                    blend: state.blend,
                    write_mask: state.write_mask,
                    pass: pass.clone(),
                };
                let created = &mut self.created;
                let (made, diagnostics) = DEVICE_CORES.get_or_create(
                    device,
                    hash,
                    |cached| {
                        cached.matches(self.gui, effect.id, selection.index, state, pass, reads)
                    },
                    key,
                    || {
                        diagnostics::capture(|| {
                            let compiled = effect.compiled(selection.index)?;
                            let core = Core::for_pass(
                                self.gui,
                                device,
                                &compiled,
                                &effect.source,
                                &effect.source.techniques[selection.index],
                                state,
                                pass,
                                source.clone(),
                            )?;
                            *created += 1;
                            Ok(Arc::new(core))
                        })
                    },
                    // The thread that draws the frames never waits for another thread's build.
                    !crate::gpu::stats::on_window_thread(),
                );
                diagnostics.credit();
                let result = match made {
                    Ok(core) => Ok(Arc::new(Pipeline::over(core, topology))),
                    // Kept by no cache: the core is on its way and a later frame finds it.
                    Err(NotMade::Building) => {
                        let reason = format!(
                            "effect {:?}, technique {:?}: its pipeline is being built on another thread",
                            effect.source.effect, effect.source.techniques[selection.index].name
                        );
                        *self.refused.entry(reason.clone()).or_default() += 1;
                        return Err(reason);
                    }
                    Err(NotMade::Failed(reason)) => Err(reason),
                };
                bucket.push((key(), result.clone(), diagnostics));
                result
            }
        };
        if let Err(reason) = &result {
            *self.refused.entry(reason.clone()).or_default() += 1;
        }
        result.map(|pipeline| {
            if pipeline.topology == topology {
                pipeline
            } else {
                Arc::new(Pipeline::over(Arc::clone(&pipeline.core), topology))
            }
        })
    }
}

impl Pipeline {
    /// The filtered bindings whose bound view cannot be filtered on `device`.
    fn unfilterable<'a>(
        &self,
        device: &wgpu::Device,
        textures: &impl Fn(u32) -> Option<&'a wgpu::TextureView>,
    ) -> u32 {
        self.filtered.iter().fold(0, |mask, binding| {
            let manual = textures(*binding).is_some_and(|view| {
                view.texture()
                    .format()
                    .sample_type(None, Some(device.features()))
                    == Some(wgpu::TextureSampleType::Float { filterable: false })
            });
            mask | if manual { 1 << binding } else { 0 }
        })
    }

    fn make_variant(&self, device: &wgpu::Device, mask: u32) -> Cached {
        let fallback = self
            .fallback
            .as_ref()
            .expect("variants exist for fallbacks");
        scoped(device, || {
            Core::create(
                device,
                &fallback.compiled,
                &fallback.effect,
                &fallback.technique,
                &fallback.state,
                &fallback.pass,
                mask,
                fallback.gui,
                None,
            )
        })
        .and_then(|result| result)
        .map(|core| Arc::new(Pipeline::over(Arc::new(core), self.topology)))
        .map_err(|reason| {
            format!(
                "effect {:?}, technique {:?}: {reason}",
                fallback.effect, fallback.technique.name
            )
        })
    }

    /// Whether binding these textures draws with a variant this pipeline has not made; if so its
    /// build starts on a thread of its own, unless one is under way.
    pub fn start_variant<'a>(
        self: &Arc<Self>,
        device: &wgpu::Device,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView>,
    ) -> bool {
        if self.fallback.is_none() {
            return false;
        }
        let mask = self.unfilterable(device, &textures);
        if mask == 0 {
            return false;
        }
        let mut variants = self
            .variants
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match variants.get(&mask) {
            Some(Some(_)) => return false,
            Some(None) => return true,
            None => {}
        }
        variants.insert(mask, None);
        drop(variants);
        let (pipeline, device, building) = (Arc::clone(self), device.clone(), Building::start());
        let build = move || {
            let _building = building;
            // A build that panicked is refused by name, so its draws never wait for it.
            let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                pipeline.make_variant(&device, mask)
            }))
            .unwrap_or_else(|_| Err("a texture variant build panicked".into()));
            pipeline
                .variants
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(mask, Some(made));
        };
        if let Err(error) = std::thread::Builder::new()
            .name("pipeline variant builds".into())
            .spawn(job_priority::carry(build))
        {
            // With no thread to build on, the draw is refused by name, never built here.
            self.variants
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .insert(
                    mask,
                    Some(Err(format!(
                        "texture variant builds could not start a thread: {error}"
                    ))),
                );
            return false;
        }
        true
    }

    fn variant(&self, device: &wgpu::Device, mask: u32) -> Result<Arc<Pipeline>, String> {
        let made = self
            .variants
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&mask)
            .cloned()
            .flatten();
        match made {
            Some(variant) => variant,
            None => {
                let variant = self.make_variant(device, mask);
                self.variants
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .insert(mask, Some(variant.clone()));
                variant
            }
        }
    }

    fn release_bind_groups(&self) {
        {
            let mut groups = self.groups.lock().unwrap_or_else(|e| e.into_inner());
            groups.clock.invalidate();
            let clock = Arc::clone(&groups.clock);
            *groups = Groups {
                clock,
                ..Default::default()
            };
        }
        for pipeline in self
            .variants
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .filter_map(|result| result.as_ref()?.as_ref().ok())
        {
            pipeline.release_bind_groups();
        }
    }

    pub fn new(
        device: &wgpu::Device,
        compiled: &Compiled,
        program: &Program,
        technique: &Technique,
        format: wgpu::TextureFormat,
        samples: u32,
    ) -> Result<Self, String> {
        Self::with_state(
            device,
            compiled,
            program,
            technique,
            format,
            samples,
            &State::authored(program, technique)?,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_state(
        device: &wgpu::Device,
        compiled: &Compiled,
        program: &Program,
        technique: &Technique,
        format: wgpu::TextureFormat,
        samples: u32,
        state: &State,
    ) -> Result<Self, String> {
        Self::for_pass(
            device,
            compiled,
            program,
            technique,
            state,
            &Pass::packed(compiled, state, format, samples),
        )
    }

    pub fn for_pass(
        device: &wgpu::Device,
        compiled: &Compiled,
        program: &Program,
        technique: &Technique,
        state: &State,
        pass: &Pass,
    ) -> Result<Self, String> {
        Core::for_pass(
            false, device, compiled, program, technique, state, pass, None,
        )
        .map(|core| Self::over(Arc::new(core), pass.primitive.topology))
    }
}

impl Core {
    fn for_pass(
        gui: bool,
        device: &wgpu::Device,
        compiled: &Compiled,
        program: &Program,
        technique: &Technique,
        state: &State,
        pass: &Pass,
        source: Option<Arc<Source>>,
    ) -> Result<Self, String> {
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let result = Self::create(
            device,
            compiled,
            &program.effect,
            technique,
            state,
            pass,
            0,
            gui,
            source,
        );
        let error = pollster::block_on(validation.pop()).or(pollster::block_on(memory.pop()));
        match error {
            Some(error) => Err(error.to_string()),
            None => result,
        }
        .map_err(|reason| {
            format!(
                "effect {:?}, technique {:?}: {reason}",
                program.effect, technique.name
            )
        })
        .map(|mut pipeline| {
            if !pipeline.filtered.is_empty()
                && !device
                    .features()
                    .contains(wgpu::Features::FLOAT32_FILTERABLE)
            {
                pipeline.fallback = Some(Fallback {
                    gui,
                    compiled: compiled.clone(),
                    effect: program.effect.clone(),
                    technique: technique.clone(),
                    state: state.clone(),
                    pass: pass.clone(),
                });
            }
            pipeline
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn create(
        device: &wgpu::Device,
        compiled: &Compiled,
        effect: &str,
        technique: &Technique,
        state: &State,
        pass: &Pass,
        unfilterable: u32,
        gui: bool,
        source: Option<Arc<Source>>,
    ) -> Result<Self, String> {
        stats::build();
        let source =
            source.unwrap_or_else(|| Source::get(device, compiled, state, pass, unfilterable, gui));
        let modules =
            source.prepare(device, state, pass, &format!("{effect}:{}", technique.name))?;
        let batch = modules.batch.clone();
        let rows: Vec<(u32, u32)> = modules
            .instanced
            .as_ref()
            .map(|instanced| instanced.rows.clone())
            .unwrap_or_default();
        let stream = modules
            .instanced
            .as_ref()
            .and_then(|instanced| instanced.stream.as_ref())
            .map(|stream| stream.slot);
        let row_of = |binding: u32| {
            rows.iter()
                .find(|(row, _)| *row == binding)
                .map(|(_, stride)| *stride)
        };
        let mut buffers = compiled.vertex.buffers.clone();
        buffers.extend_from_slice(&compiled.pixel.buffers);
        buffers.sort_by_key(|buffer| buffer.binding);
        let dynamic = device
            .limits()
            .max_dynamic_uniform_buffers_per_pipeline_layout;
        // A row's array is bound whole; the rows block the draw selects takes one dynamic offset.
        let offsets = buffers.len() - rows.len() + usize::from(!rows.is_empty());
        if offsets > dynamic as usize {
            return Err(format!(
                "{offsets} constant buffers exceed this device's {dynamic} dynamically offset buffers"
            ));
        }
        let mut textures = compiled.vertex.textures.clone();
        textures.extend_from_slice(&compiled.pixel.textures);
        let mut samplers = Vec::new();
        for binding in compiled
            .vertex
            .samplers
            .iter()
            .chain(&compiled.pixel.samplers)
        {
            let fact = state
                .samplers
                .get(binding)
                .ok_or_else(|| format!("sampler binding {binding} has no native bound state"))?;
            let address = |mode| match mode {
                Address::Wrap => Ok(wgpu::AddressMode::Repeat),
                Address::Mirror => Ok(wgpu::AddressMode::MirrorRepeat),
                Address::Clamp | Address::MirrorOnce => Ok(wgpu::AddressMode::ClampToEdge),
                Address::Border
                    if device
                        .features()
                        .contains(wgpu::Features::ADDRESS_MODE_CLAMP_TO_BORDER) =>
                {
                    Ok(wgpu::AddressMode::ClampToBorder)
                }
                Address::Border => {
                    Err("adapter has no clamp-to-border sampler support".to_string())
                }
            };
            let anisotropic = [fact.mag, fact.min, fact.mip].contains(&Filter::Anisotropic);
            let filter = |filter| {
                if anisotropic || matches!(filter, Filter::Linear) {
                    wgpu::FilterMode::Linear
                } else {
                    wgpu::FilterMode::Nearest
                }
            };
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some(&fact.name),
                address_mode_u: address(fact.u)?,
                address_mode_v: address(fact.v)?,
                address_mode_w: address(fact.w)?,
                mag_filter: filter(fact.mag),
                min_filter: filter(fact.min),
                mipmap_filter: if anisotropic || fact.mip == Filter::Linear {
                    wgpu::MipmapFilterMode::Linear
                } else {
                    wgpu::MipmapFilterMode::Nearest
                },
                anisotropy_clamp: if anisotropic { 16 } else { 1 }, // Authored MaxAnisotropy is ignored; anisotropic filtering uses 16.
                lod_min_clamp: fact.min_lod.max(0) as f32,
                lod_max_clamp: fact.max_lod.max(0) as f32,
                border_color: Some(wgpu::SamplerBorderColor::TransparentBlack),
                ..Default::default()
            });
            samplers.push((*binding, sampler));
        }
        let visibility = |binding: u32| {
            if binding.is_multiple_of(2) {
                wgpu::ShaderStages::VERTEX
            } else {
                wgpu::ShaderStages::FRAGMENT
            }
        };
        let filtering = |binding: &u32| {
            state.samplers.get(binding).is_some_and(|sampler| {
                [sampler.mag, sampler.min, sampler.mip]
                    .iter()
                    .any(|filter| matches!(filter, Filter::Linear | Filter::Anisotropic))
            })
        };
        let filterable = |binding: &u32| {
            compiled
                .vertex
                .sampling_pairs
                .iter()
                .chain(&compiled.pixel.sampling_pairs)
                .any(|(image, sampler)| image == binding && filtering(sampler))
        };
        let [constants, textures_layout, samplers_layout] = [
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("native constants"),
                entries: &buffers
                    .iter()
                    .map(|b| wgpu::BindGroupLayoutEntry {
                        binding: b.binding,
                        visibility: visibility(b.binding),
                        ty: wgpu::BindingType::Buffer {
                            ty: if batch.is_some() || row_of(b.binding).is_some() {
                                wgpu::BufferBindingType::Storage { read_only: true }
                            } else {
                                wgpu::BufferBindingType::Uniform
                            },
                            has_dynamic_offset: batch.is_none() && row_of(b.binding).is_none(),
                            min_binding_size: std::num::NonZeroU64::new(u64::from(
                                row_of(b.binding).unwrap_or(b.size),
                            )),
                        },
                        count: None,
                    })
                    .chain(batch.iter().map(|_| batch::metadata_layout()))
                    .chain(
                        (!rows.is_empty() || stream.is_some()).then(batch::rows_layout),
                    )
                    .chain(stream.iter().flat_map(|_| batch::stream_layouts()))
                    .collect::<Vec<_>>(),
            }),
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("native textures"),
                entries: &textures
                    .iter()
                    .map(|(binding, cube)| wgpu::BindGroupLayoutEntry {
                        binding: *binding,
                        visibility: visibility(*binding),
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float {
                                filterable: filterable(binding)
                                    && unfilterable & (1 << binding) == 0,
                            },
                            view_dimension: if batch.is_some() {
                                wgpu::TextureViewDimension::D2Array
                            } else {
                                cube.view()
                            },
                            multisampled: false,
                        },
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            }),
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("native samplers"),
                entries: &samplers
                    .iter()
                    .map(|(binding, _)| wgpu::BindGroupLayoutEntry {
                        binding: *binding,
                        visibility: visibility(*binding),
                        ty: wgpu::BindingType::Sampler(if filtering(binding) {
                            wgpu::SamplerBindingType::Filtering
                        } else {
                            wgpu::SamplerBindingType::NonFiltering
                        }),
                        count: None,
                    })
                    .collect::<Vec<_>>(),
            }),
        ];
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(effect),
            bind_group_layouts: &[
                Some(&constants),
                Some(&textures_layout),
                Some(&samplers_layout),
            ],
            immediate_size: 0,
        });
        let samplers = crate::gpu::stats::create_bind_group(
            &device,
            &wgpu::BindGroupDescriptor {
                label: Some("native samplers"),
                layout: &samplers_layout,
                entries: &samplers
                    .iter()
                    .map(|(binding, sampler)| wgpu::BindGroupEntry {
                        binding: *binding,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    })
                    .collect::<Vec<_>>(),
            },
        );
        let mut vertices = pass.vertices.clone();
        let mut primitive = pass.primitive;
        if let Some(slot) = stream {
            vertices.truncate(slot);
        }
        if let Some(batch) = &batch {
            let stride = vertices[0].stride;
            vertices[0].attributes.push(wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Uint32,
                offset: stride,
                shader_location: batch.shader.input,
            });
            vertices[0].stride += 4;
            primitive.topology = wgpu::PrimitiveTopology::TriangleList;
            primitive.strip_index_format = None;
        }
        let vertex_layouts: Vec<_> = vertices
            .iter()
            .map(|layout| {
                Some(wgpu::VertexBufferLayout {
                    array_stride: layout.stride,
                    step_mode: layout.step,
                    attributes: &layout.attributes,
                })
            })
            .collect();
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(&format!("{effect}:{}", technique.name)),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &modules.vertex,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                buffers: &vertex_layouts,
            },
            primitive,
            depth_stencil: pass.depth_stencil.clone(),
            multisample: pass.multisample,
            fragment: Some(wgpu::FragmentState {
                module: &modules.pixel,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                targets: &pass.targets,
            }),
            multiview_mask: None,
            cache: None,
        });
        let null_cube = if textures.iter().any(|(_, kind)| *kind == super::TextureKind::Cube) {
            let mut held = source.null_cube.lock().unwrap_or_else(|error| error.into_inner());
            if held.is_none() {
                *held = Some(scoped(device, || null_texture(device, super::TextureKind::Cube))?);
            }
            held.clone()
        } else {
            None
        };
        Ok(Self {
            _source: source,
            batch,
            rows,
            stream,
            pipeline,
            label: format!("effect {effect:?}, technique {:?}", technique.name),
            null_cube,
            layouts: [constants, textures_layout],
            samplers,
            buffers,
            filtered: textures
                .iter()
                .map(|(binding, _)| *binding)
                .filter(filterable)
                .collect(),
            textures,
            packed_inputs: super::vertices::Declaration::ALL.map(|declaration| {
                super::vertices::Packing::new(declaration, &compiled.vertex.inputs)
            }),
            fallback: None,
            read_only: ReadOnly::of(pass),
        })
    }
}

impl Pipeline {
    /// Resources are keyed by `binding(slot, stage)`: native vertex/pixel slots are independent.
    /// Slots without an upload must be supplied by the caller's retained bind state.
    pub fn prepare(
        &self,
        arena: &Arena,
        constants: &BTreeMap<u32, impl AsRef<[u8]>>,
        textures: &BTreeMap<u32, wgpu::TextureView>,
        vertices: &super::vertices::Vertices<'_>,
    ) -> Result<Draw, String> {
        self.prepare_with(
            arena,
            |binding| constants.get(&binding).map(AsRef::as_ref),
            |binding| textures.get(&binding),
            vertices,
        )
    }

    /// Packed geometry with borrowed resource lookups, without building per-draw maps.
    pub(crate) fn prepare_with<'a>(
        &'a self,
        arena: &Arena,
        constants: impl Fn(u32) -> Option<&'a [u8]>,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView>,
        vertices: &super::vertices::Vertices<'_>,
    ) -> Result<Draw, String> {
        let bindings = self.bind_with(arena, constants, textures)?;
        let packed =
            arena.packed_vertices(&self.packed_inputs[vertices.declaration as usize], vertices)?;
        Ok(Draw {
            bindings,
            vertices: packed,
            count: vertices.count as u32,
        })
    }

    /// Bind arbitrary native register bytes and texture views. Caller supplies vertex/index
    /// buffers and draw commands; no GUI geometry, buffer names or matrix ABI is imposed.
    /// The bytes are staged in `arena`; the bindings are valid until its next reset.
    pub fn bindings(
        &self,
        arena: &Arena,
        constants: &BTreeMap<u32, impl AsRef<[u8]>>,
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) -> Result<Bindings, String> {
        self.bind_with(
            arena,
            |binding| constants.get(&binding).map(AsRef::as_ref),
            |binding| textures.get(&binding),
        )
    }

    /// Bindings over constant blocks the caller keeps in a buffer of its own: made once and
    /// valid while that buffer and these views live, so a pass whose constants and views change
    /// only when its targets or its settings do records its draw with them on every frame,
    /// staging nothing and looking nothing up. `blocks` gives, by `binding(slot, stage)`, where
    /// that binding's block stands in `buffer` and the bytes the caller wrote there; the buffer
    /// must hold the binding's whole physical size at that offset, zero past those bytes (the
    /// native programs read past a block as zero), and the offset must stand at the device's
    /// uniform offset alignment.
    pub fn kept_bindings<'a>(
        &'a self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        blocks: impl Fn(u32) -> Option<(u32, &'a [u8])> + Copy,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView> + Copy,
    ) -> Result<Bindings, String> {
        self.kept(device, buffer, blocks, |key| {
            let view = textures(key)?;
            Some(self.sampled_view(key, view, false))
        })
        .map_err(|reason| format!("{}: {reason}", self.label))
    }

    fn kept<'a>(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        blocks: impl Fn(u32) -> Option<(u32, &'a [u8])> + Copy,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView> + Copy,
    ) -> Result<Bindings, String> {
        if self.fallback.is_some() {
            let mask = self.unfilterable(device, &textures);
            if mask != 0 {
                return self.variant(device, mask)?.kept(device, buffer, blocks, textures);
            }
        }
        if !self.rows.is_empty() || self.stream.is_some() {
            return Err("this pass reads its rows per instance and binds its own buffer".into());
        }
        let alignment = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let mut offsets = [0; BUFFERS];
        #[cfg(any(test, feature = "test-support"))]
        let mut held: [(&[u8], u64); BUFFERS] = [(&[], 0); BUFFERS];
        for (index, buffer_spec) in self.buffers.iter().enumerate() {
            let name = buffer_spec.binding / 2;
            let (offset, bytes) = blocks(buffer_spec.binding)
                .ok_or_else(|| format!("native constant buffer {name} has not been bound"))?;
            let size = u64::from(buffer_spec.size).max(bytes.len() as u64);
            if u64::from(offset) % alignment != 0 {
                return Err(format!(
                    "native constant buffer {name} stands at {offset}, not at this device's {alignment}-byte offset alignment"
                ));
            }
            if u64::from(offset) + size > buffer.size() {
                return Err(format!(
                    "native constant buffer {name}'s {size} bytes at {offset} are past the {} bytes kept for them",
                    buffer.size()
                ));
            }
            offsets[index] = offset;
            #[cfg(any(test, feature = "test-support"))]
            {
                held[index] = (bytes, u64::from(buffer_spec.size));
            }
        }
        let count = self.buffers.len();
        let constants = scoped(device, || {
            crate::gpu::stats::create_bind_group(&device, &wgpu::BindGroupDescriptor {
                label: Some("native kept constants"),
                layout: &self.layouts[0],
                entries: &self
                    .buffers
                    .iter()
                    .map(|spec| wgpu::BindGroupEntry {
                        binding: spec.binding,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer,
                            offset: 0,
                            size: std::num::NonZeroU64::new(u64::from(spec.size)),
                        }),
                    })
                    .collect::<Vec<_>>(),
            })
        })?;
        let mut views: [Option<&wgpu::TextureView>; TEXTURES] = [None; TEXTURES];
        for (view, (binding, _)) in views.iter_mut().zip(&self.textures) {
            *view = Some(textures(*binding).ok_or_else(|| {
                format!("native texture slot {} has not been bound", binding / 2)
            })?);
        }
        let sampled = scoped(device, || {
            crate::gpu::stats::create_bind_group(&device, &wgpu::BindGroupDescriptor {
                label: Some("native kept textures"),
                layout: &self.layouts[1],
                entries: &self
                    .textures
                    .iter()
                    .zip(views.iter().flatten())
                    .map(|((binding, _), view)| wgpu::BindGroupEntry {
                        binding: *binding,
                        resource: wgpu::BindingResource::TextureView(view),
                    })
                    .collect::<Vec<_>>(),
            })
        })?;
        Ok(Bindings {
            resources: ResourceHandle::Inline(GroupResources {
                pipeline: self.pipeline.clone(),
                read_only: self.read_only,
                groups: [constants, sampled, self.samplers.clone()],
            }),
            offsets,
            count,
            #[cfg(any(test, feature = "test-support"))]
            proof: preparation_bytes(&self.buffers, &held[..count]),
        })
    }

    /// `bindings` with the caller's lookups by `binding(slot, stage)`.
    pub fn bind_with<'a>(
        &'a self,
        arena: &Arena,
        constants: impl Fn(u32) -> Option<&'a [u8]>,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView>,
    ) -> Result<Bindings, String> {
        self.bind_with_capture(arena, constants, textures, false)
            .map(|(bindings, _)| bindings)
    }

    /// Capture only a successful effective pipeline and its immutable resources. The caller
    /// must verify selection and resource identities before reusing the returned program.
    /// Constant bytes and arena offsets are never retained across draws.
    pub fn bind_program_with<'a>(
        &'a self,
        arena: &Arena,
        constants: impl Fn(u32) -> Option<&'a [u8]>,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView>,
    ) -> Result<(Bindings, BindingProgram), String> {
        self.bind_with_capture(arena, constants, textures, true)
            .map(|(bindings, program)| (bindings, program.expect("successful capture")))
    }

    pub fn sampled_texture<'a>(&'a self, key: u32, binding: &'a TextureBinding) -> &'a wgpu::TextureView {
        self.sampled_view(key, &binding.view, binding.array.is_some())
    }

    fn sampled_view<'a>(
        &'a self,
        key: u32,
        view: &'a wgpu::TextureView,
        plain: bool,
    ) -> &'a wgpu::TextureView {
        // Sampling a native two-dimensional SRV through TextureCube returns zero.
        if let Some(zero) = &self.null_cube
            && self
                .textures
                .iter()
                .any(|(slot, kind)| *slot == key && *kind == super::TextureKind::Cube)
            && (plain
                || (view.texture().dimension() == wgpu::TextureDimension::D2
                    && view.texture().depth_or_array_layers() == 1))
        {
            zero
        } else {
            view
        }
    }

    /// What this compiled pass reads of each constant buffer it binds, in the order of the
    /// constant group's dynamic offsets: the physical size and the byte ranges the compiled
    /// programs read after dead-code elimination ([`Buffer::reads`]). A caller that draws the
    /// pass only while its inputs change keys on exactly these bytes.
    pub fn constant_reads(&self) -> &[Buffer] {
        &self.core.buffers
    }

    /// The texture slots this compiled pass samples, after dead-code elimination: a slot the
    /// caller binds that is not here is read by no program of the pass.
    pub fn sampled_slots(&self) -> impl Iterator<Item = u32> + '_ {
        self.core.textures.iter().map(|(slot, _)| *slot)
    }

    /// Whether the pass took the rewrites its [`Pass::instanced`] and [`Pass::stream`] asked
    /// for; a program that could not take them keeps its plain pipeline, refused by name on the
    /// standard error where it was translated.
    pub fn reads_per_instance(&self) -> bool {
        !self.rows.is_empty() || self.stream.is_some()
    }

    /// The program of a pass that reads its rows per instance: its pipeline, textures and
    /// samplers, without staging any constants. The caller binds its own buffer
    /// ([`BindingProgram::bind_rows`]), where the rows stand. `stream` is the instanced vertex
    /// stream's words and each instance's first vertex in them, which a pass that reads a stream
    /// per instance ([`Pass::stream`]) must be given.
    pub fn rows_program<'a>(
        &self,
        arena: &Arena,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView>,
        stream: Option<[wgpu::Buffer; 2]>,
    ) -> Result<BindingProgram, String> {
        let device = arena.device();
        // A texture no sampler of this device can filter draws through the variant built for it,
        // as a staged bind does.
        if self.fallback.is_some() {
            let mask = self.unfilterable(device, &textures);
            if mask != 0 {
                let variant = self.variant(device, mask)?;
                return variant.rows_program(arena, textures, stream);
            }
        }
        let stream = match (self.stream, stream) {
            (Some(_), None) => {
                return Err(format!(
                    "{}: this pass reads a vertex stream per instance and was given none",
                    self.label
                ));
            }
            (None, _) => None,
            (Some(_), given) => given,
        };
        let views = self
            .textures
            .iter()
            .map(|(binding, _)| {
                textures(*binding)
                    .map(|view| self.sampled_view(*binding, view, false).clone())
                    .ok_or_else(|| {
                        format!(
                            "{}: native texture slot {} has not been bound",
                            self.label,
                            binding / 2
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let group = scoped(device, || {
            crate::gpu::stats::create_bind_group(&device, &wgpu::BindGroupDescriptor {
                label: Some("native textures"),
                layout: &self.layouts[1],
                entries: &self
                    .textures
                    .iter()
                    .zip(&views)
                    .map(|((binding, _), view)| wgpu::BindGroupEntry {
                        binding: *binding,
                        resource: wgpu::BindingResource::TextureView(view),
                    })
                    .collect::<Vec<_>>(),
            })
        })
        .map_err(|reason| format!("{}: {reason}", self.label))?;
        use std::hash::{Hash, Hasher};
        let mut hasher = std::hash::DefaultHasher::new();
        views.iter().map(Some).collect::<Vec<_>>().hash(&mut hasher);
        Ok(BindingProgram {
            pipeline: self.pipeline.clone(),
            read_only: self.read_only,
            samplers: self.samplers.clone(),
            textures: group,
            texture_slots: self.textures.iter().map(|(slot, _)| *slot).collect(),
            views,
            hash: hasher.finish(),
            buffers: self.buffers.clone(),
            rows: self.rows.clone(),
            stream,
            layouts: self.layouts.clone(),
            groups: Arc::clone(&self.groups),
            label: self.label.clone(),
        })
    }

    fn bind_with_capture<'a>(
        &'a self,
        arena: &Arena,
        constants: impl Fn(u32) -> Option<&'a [u8]>,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView>,
        capture: bool,
    ) -> Result<(Bindings, Option<BindingProgram>), String> {
        let sampled = |key| {
            let view = textures(key)?;
            Some(self.sampled_view(key, view, false))
        };
        self.bind(arena, constants, sampled, capture)
            .map(|(bindings, mut program)| {
                if let Some(program) = &mut program {
                    self.label.clone_into(&mut program.label);
                }
                (bindings, program)
            })
            .map_err(|reason| format!("{}: {reason}", self.label))
    }

    fn bind<'a>(
        &self,
        arena: &Arena,
        constants: impl Fn(u32) -> Option<&'a [u8]>,
        textures: impl Fn(u32) -> Option<&'a wgpu::TextureView>,
        capture: bool,
    ) -> Result<(Bindings, Option<BindingProgram>), String> {
        if self.fallback.is_some() {
            let device = arena.device();
            let mask = self.unfilterable(device, &textures);
            if mask != 0 {
                let variant = self.variant(device, mask)?;
                return variant.bind(arena, constants, textures, capture);
            }
        }
        if !self.rows.is_empty() || self.stream.is_some() {
            return Err("this pass reads its rows per instance and binds its own buffer".into());
        }
        // Compilation refuses registers past D3D11's, so both lists fit their arrays.
        let mut blocks: [(&[u8], u64); BUFFERS] = [(&[], 0); BUFFERS];
        for (block, buffer) in blocks.iter_mut().zip(&self.buffers) {
            let data = constants(buffer.binding).ok_or_else(|| {
                format!(
                    "native constant buffer {} has not been bound",
                    buffer.binding / 2
                )
            })?;
            *block = (data, u64::from(buffer.size));
        }
        let mut found: [Option<&wgpu::TextureView>; TEXTURES] = [None; TEXTURES];
        for (view, (binding, _)) in found.iter_mut().zip(&self.textures) {
            *view = Some(textures(*binding).ok_or_else(|| {
                format!("native texture slot {} has not been bound", binding / 2)
            })?);
        }
        let views = &found[..self.textures.len()];
        let count = self.buffers.len();
        let mut offsets = [0; BUFFERS];
        let buffer = arena.constants(&blocks[..count], &mut offsets[..count])?;
        let frame = arena.frame();
        let device = arena.device();
        let mut groups = self
            .groups
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let groups = &mut *groups;
        groups.advance(frame);
        let constants = match groups
            .constants
            .iter_mut()
            .find(|(cached, _, _)| cached.as_slice() == [buffer.clone()])
        {
            Some((_, group, used)) => {
                used.set(frame);
                group.clone()
            }
            None => {
                let group = scoped(device, || {
                    crate::gpu::stats::create_bind_group(&device, &wgpu::BindGroupDescriptor {
                        label: Some("native constants"),
                        layout: &self.layouts[0],
                        entries: &self
                            .buffers
                            .iter()
                            .map(|spec| wgpu::BindGroupEntry {
                                binding: spec.binding,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &buffer,
                                    offset: 0,
                                    size: std::num::NonZeroU64::new(u64::from(spec.size)),
                                }),
                            })
                            .collect::<Vec<_>>(),
                    })
                })?;
                groups
                    .constants
                    .push((vec![buffer], group.clone(), GroupStamp::new(frame)));
                group
            }
        };
        let same = |cached: &[wgpu::TextureView]| cached.iter().map(Some).eq(views.iter().copied());
        let last = groups
            .last
            .filter(|_| groups.clock.hint.load(std::sync::atomic::Ordering::Acquire))
            .filter(|(hash, index)| same(&groups.textures[hash][*index].0));
        let (hash, found) = match last {
            Some((hash, index)) => (hash, Some(index)),
            None => {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::hash::DefaultHasher::new();
                views.hash(&mut hasher);
                let hash = hasher.finish();
                let found = groups
                    .textures
                    .get(&hash)
                    .and_then(|entries| entries.iter().position(|(cached, _, _)| same(cached)));
                (hash, found)
            }
        };
        let entries = groups.textures.entry(hash).or_default();
        let textures = match found {
            Some(index) => {
                let (_, group, used) = &mut entries[index];
                used.set(frame);
                groups.last = Some((hash, index));
                group.clone()
            }
            None => {
                let group = scoped(device, || {
                    crate::gpu::stats::create_bind_group(&device, &wgpu::BindGroupDescriptor {
                        label: Some("native textures"),
                        layout: &self.layouts[1],
                        entries: &self
                            .textures
                            .iter()
                            .zip(views.iter().flatten())
                            .map(|((binding, _), view)| wgpu::BindGroupEntry {
                                binding: *binding,
                                resource: wgpu::BindingResource::TextureView(view),
                            })
                            .collect::<Vec<_>>(),
                    })
                })?;
                let views = views.iter().flatten().map(|view| (*view).clone()).collect();
                entries.push((views, group.clone(), GroupStamp::new(frame)));
                groups.last = Some((hash, entries.len() - 1));
                group
            }
        };
        groups
            .clock
            .hint
            .store(true, std::sync::atomic::Ordering::Release);
        let program = capture.then(|| BindingProgram {
            pipeline: self.pipeline.clone(),
            read_only: self.read_only,
            samplers: self.samplers.clone(),
            textures: textures.clone(),
            views: views.iter().flatten().map(|view| (*view).clone()).collect(),
            texture_slots: self.textures.iter().map(|(slot, _)| *slot).collect(),
            hash,
            buffers: self.buffers.clone(),
            rows: self.rows.clone(),
            stream: None,
            layouts: self.layouts.clone(),
            groups: Arc::clone(&self.groups),
            label: self.label.clone(),
        });

        Ok((
            Bindings {
                resources: ResourceHandle::Inline(GroupResources {
                    pipeline: self.pipeline.clone(),
                    read_only: self.read_only,
                    groups: [constants, textures, self.samplers.clone()],
                }),
                offsets,
                count,
                #[cfg(any(test, feature = "test-support"))]
                proof: preparation_bytes(&self.buffers, &blocks[..count]),
            },
            program,
        ))
    }

    /// Check a physical native allocation whose initialized bytes are only partly known.
    /// Consumers call this before `bindings`; slots absent from the program need no bytes.
    pub fn check_initialized(
        &self,
        binding: u32,
        physical: u32,
        known: &[std::ops::Range<u32>],
    ) -> Result<(), String> {
        for buffer in self
            .buffers
            .iter()
            .filter(|buffer| buffer.binding == binding)
        {
            buffer.check_initialized(physical, known)?;
        }
        Ok(())
    }

    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, draw: &Draw) {
        self.draw_after(pass, draw, None);
    }

    /// Draw after another packed submission in the same pass, without redundant binds.
    /// Callers forget the previous draw whenever any other path changes render state.
    pub fn draw_after(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        draw: &Draw,
        previous: Option<&Draw>,
    ) {
        // A null blend-factor pointer gives D3D11's all-ones blend constant.
        if previous.is_none() {
            crate::gpu::stats::state();
            pass.set_blend_constant(wgpu::Color::WHITE);
        }
        draw.bindings
            .bind_after(pass, previous.map(|draw| &draw.bindings));
        if previous.is_none_or(|previous| {
            previous.vertices.buffer != draw.vertices.buffer
                || previous.vertices.offset != draw.vertices.offset
                || previous.vertices.size != draw.vertices.size
        }) {
            crate::gpu::stats::state();
            pass.set_vertex_buffer(0, draw.vertices.slice());
        }
        crate::gpu::stats::draw();
        pass.draw(0..draw.count, 0..1);
    }
}

/// Held while a device on the process's instance is made or let go: the main map and a pop-out
/// start and stop on threads of their own, once each, so they take turns. Never held while another
/// lock is taken.
pub fn device_lifecycle() -> std::sync::MutexGuard<'static, ()> {
    static LIFECYCLE: Mutex<()> = Mutex::new(());
    LIFECYCLE.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Cache entries that hold a device, let go under [`device_lifecycle`] once the cache's own lock
/// is released: declared before that lock is taken, it drops after it.
pub struct Retired<T>(pub Vec<T>);

impl<T> Drop for Retired<T> {
    fn drop(&mut self) {
        if !self.0.is_empty() {
            let _held = device_lifecycle();
            self.0.clear();
        }
    }
}

/// The process's one wgpu instance, which every device it makes comes from. wgpu tells devices
/// apart by an id each instance numbers in a registry of its own, and the device-keyed caches here
/// and in the map compare devices by it: a device from a second instance can take the id of one
/// from the first, and be handed that device's pipelines, layouts and views.
pub fn instance() -> wgpu::Instance {
    static INSTANCE: OnceLock<wgpu::Instance> = OnceLock::new();
    INSTANCE
        .get_or_init(|| wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env()))
        .clone()
}

/// DX11 reads explicit null SRVs as zero; WebGPU zero-initialises resources.
pub(crate) fn null_texture(device: &wgpu::Device, kind: super::TextureKind) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("native null texture"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: if kind == super::TextureKind::Cube {
                6
            } else {
                1
            },
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: if kind == super::TextureKind::D3 {
            wgpu::TextureDimension::D3
        } else {
            wgpu::TextureDimension::D2
        },
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(kind.view()),
        ..Default::default()
    })
}

/// Validation and out-of-memory errors while creating a GPU object, as a named failure.
pub fn scoped<T>(device: &wgpu::Device, create: impl FnOnce() -> T) -> Result<T, String> {
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let result = create();
    match pollster::block_on(validation.pop()).or(pollster::block_on(memory.pop())) {
        Some(error) => Err(error.to_string()),
        None => Ok(result),
    }
}

/// Native blend descriptor defaults and lowering.
pub fn blend(
    program: &Program,
    technique: &Technique,
) -> Result<(Option<wgpu::BlendState>, wgpu::ColorWrites), String> {
    let state = program.blend(technique);
    let factor = |code: u32| {
        [
            wgpu::BlendFactor::Zero,
            wgpu::BlendFactor::One,
            wgpu::BlendFactor::Src,
            wgpu::BlendFactor::OneMinusSrc,
            wgpu::BlendFactor::SrcAlpha,
            wgpu::BlendFactor::OneMinusSrcAlpha,
            wgpu::BlendFactor::DstAlpha,
            wgpu::BlendFactor::OneMinusDstAlpha,
            wgpu::BlendFactor::Dst,
            wgpu::BlendFactor::OneMinusDst,
            wgpu::BlendFactor::SrcAlphaSaturated,
            wgpu::BlendFactor::Constant,
            wgpu::BlendFactor::OneMinusConstant,
            wgpu::BlendFactor::Src1,
            wgpu::BlendFactor::OneMinusSrc1,
            wgpu::BlendFactor::Src1Alpha,
            wgpu::BlendFactor::OneMinusSrc1Alpha,
        ]
        .get(code as usize)
        .copied()
        .ok_or_else(|| format!("unknown native blend factor {code}"))
    };
    let operation = |code: u32| {
        [
            wgpu::BlendOperation::Add,
            wgpu::BlendOperation::Subtract,
            wgpu::BlendOperation::ReverseSubtract,
            wgpu::BlendOperation::Min,
            wgpu::BlendOperation::Max,
        ]
        .get(code as usize)
        .copied()
        .ok_or_else(|| format!("unknown native blend operation {code}"))
    };
    let mut blend = wgpu::BlendState {
        color: wgpu::BlendComponent {
            src_factor: factor(state.source)?,
            dst_factor: factor(state.dest)?,
            operation: operation(state.op)?,
        },
        alpha: wgpu::BlendComponent {
            src_factor: factor(state.source_alpha)?,
            dst_factor: factor(state.dest_alpha)?,
            operation: operation(state.op_alpha)?,
        },
    };
    let enabled = state.enable;
    let mask = wgpu::ColorWrites::from_bits_truncate(state.write_mask);
    // Native defaults ONE/ZERO/ADD form a sentinel: even an explicitly authored sentinel
    // inherits the colour equation. This includes SRC_ALPHA on the alpha channel.
    if blend.alpha == wgpu::BlendComponent::REPLACE {
        blend.alpha = blend.color;
    }
    // D3D ignores factors for MIN/MAX; wgpu requires One explicitly.
    for component in [&mut blend.color, &mut blend.alpha] {
        if matches!(
            component.operation,
            wgpu::BlendOperation::Min | wgpu::BlendOperation::Max
        ) {
            component.src_factor = wgpu::BlendFactor::One;
            component.dst_factor = wgpu::BlendFactor::One;
        }
    }
    Ok((enabled.then_some(blend), mask))
}

/// Native depth-stencil lowering. Comparisons and stencil operations both use the authored
/// index plus one in D3D11 order, anything else the first. The front-face stencil state applies
/// to both faces and the masks are their low bytes. D3D11's depth bias is rasterizer state but
/// wgpu's is depth state, so it comes from
/// `rasterizer`. The stencil reference is the caller's dynamic `set_stencil_reference`.
pub fn depth_stencil(
    state: &DepthStencilState,
    rasterizer: &RasterizerState,
    format: wgpu::TextureFormat,
) -> (wgpu::DepthStencilState, u32) {
    use wgpu::{CompareFunction as C, StencilOperation as O};
    let compare = |code: u32| {
        [
            C::Never,
            C::Less,
            C::Equal,
            C::LessEqual,
            C::Greater,
            C::NotEqual,
            C::GreaterEqual,
            C::Always,
        ]
        .get(code as usize)
        .copied()
        .unwrap_or(C::Never)
    };
    let operation = |code: u32| {
        [
            O::Keep,
            O::Zero,
            O::Replace,
            O::IncrementClamp,
            O::DecrementClamp,
            O::Invert,
            O::IncrementWrap,
            O::DecrementWrap,
        ]
        .get(code as usize)
        .copied()
        .unwrap_or(O::Keep)
    };
    let stencil = if state.stencil_enable {
        let face = wgpu::StencilFaceState {
            compare: compare(state.front_function),
            fail_op: operation(state.front_fail),
            depth_fail_op: operation(state.front_depth_fail),
            pass_op: operation(state.front_pass),
        };
        wgpu::StencilState {
            front: face,
            back: face,
            read_mask: state.stencil_read_mask & 0xFF,
            write_mask: state.stencil_write_mask & 0xFF,
        }
    } else {
        wgpu::StencilState::default()
    };
    let descriptor = wgpu::DepthStencilState {
        format,
        // A disabled depth test neither tests nor writes.
        depth_write_enabled: Some(state.depth_enable && state.depth_write_mask == 1),
        depth_compare: Some(if state.depth_enable {
            compare(state.depth_function)
        } else {
            C::Always
        }),
        stencil,
        bias: wgpu::DepthBiasState {
            constant: rasterizer.depth_bias,
            slope_scale: rasterizer.slope_scaled_depth_bias,
            clamp: rasterizer.depth_bias_clamp,
        },
    };
    (descriptor, state.stencil_ref)
}

/// Native rasterizer lowering: only `fill_solid` fills, cull mode follows the authored index,
/// winding is authored, and depth clipping is always on. Multisample
/// and line antialiasing come from the renderer, not the authored value. The second value is
/// the authored scissor enable; the rectangle is the caller's dynamic `set_scissor_rect`.
pub fn primitive(
    state: &RasterizerState,
    topology: wgpu::PrimitiveTopology,
) -> (wgpu::PrimitiveState, bool) {
    let primitive = wgpu::PrimitiveState {
        topology,
        front_face: if state.front_ccw {
            wgpu::FrontFace::Ccw
        } else {
            wgpu::FrontFace::Cw
        },
        cull_mode: match state.cull_mode {
            1 => Some(wgpu::Face::Front),
            2 => Some(wgpu::Face::Back),
            _ => None,
        },
        polygon_mode: if state.fill_mode == 2 {
            wgpu::PolygonMode::Fill
        } else {
            wgpu::PolygonMode::Line
        },
        ..Default::default()
    };
    (primitive, state.scissor)
}
