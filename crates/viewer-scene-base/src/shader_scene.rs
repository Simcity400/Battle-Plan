use eframe::wgpu;
use shader_pipeline::gpu::stats::CountedPasses as _;
use viewer_gpu::gpu_contract::Targets;
use shader_pipeline::{gpu, stream, vertices::NativeBuffer};
use std::sync::MutexGuard;
use std::{
    collections::BTreeMap,
    io,
    ops::Range,
    sync::{Arc, Mutex},
};

pub struct Program {
    selection: shader_pipeline::library::Selection,
    layouts: Vec<gpu::VertexLayout>,
    state: gpu::State,
    raster: content_reader_gui::pdxshader::RasterizerState,
    depth: content_reader_gui::pdxshader::DepthStencilState,
}

impl Program {
    #[cfg(any(test, feature = "test-support"))]
    pub fn selection(&self) -> &shader_pipeline::library::Selection {
        &self.selection
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn layouts(&self) -> &[gpu::VertexLayout] {
        &self.layouts
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn state(&self) -> &gpu::State {
        &self.state
    }

    pub fn new(
        selection: &shader_pipeline::library::Selection,
        layouts: &[gpu::VertexLayout],
        state: &gpu::State,
        raster: &content_reader_gui::pdxshader::RasterizerState,
        depth: &content_reader_gui::pdxshader::DepthStencilState,
    ) -> Self {
        Self {
            selection: selection.clone(),
            layouts: layouts.to_vec(),
            state: state.clone(),
            raster: raster.clone(),
            depth: depth.clone(),
        }
    }

    pub fn bind(
        &self,
        retained: &mut State,
        buffers: &BTreeMap<u32, impl AsRef<[u8]>>,
        textures: &BTreeMap<u32, wgpu::TextureView>,
    ) {
        self.bind_registers(retained, buffers);
        self.bind_textures(retained, textures);
    }

    pub fn bind_formatted(
        &self,
        retained: &mut State,
        buffers: &BTreeMap<u32, impl AsRef<[u8]>>,
        textures: &BTreeMap<u32, gpu::TextureBinding>,
    ) {
        self.bind_registers(retained, buffers);
        self.bind_images(
            retained,
            textures.iter().map(|(key, image)| (*key, image.clone())),
        );
    }

    fn bind_registers(&self, retained: &mut State, buffers: &BTreeMap<u32, impl AsRef<[u8]>>) {
        use content_reader_gui::pdxshader::program::Stage;
        self.select(retained);
        let stream = &mut retained.stream;
        for (slot, layout) in self.layouts.iter().enumerate() {
            stream.bind_vertex_stride(slot as u32, layout.stride);
        }
        for (&key, bytes) in buffers {
            let stage = if key % 2 == 0 {
                Stage::Vertex
            } else {
                Stage::Pixel
            };
            if key / 2 == 0 {
                stream.bind_partial_constants(0, stage, Some(bytes.as_ref()), &[0..348, 352..408]);
            } else {
                stream.bind_constants(key / 2, stage, Some(bytes.as_ref()));
            }
        }
    }

    pub fn sampler(&self, key: u32) -> Option<&content_reader_gui::pdxshader::program::Sampler> {
        self.state.samplers.get(&key)
    }

    pub fn select(&self, retained: &mut State) {
        let stream = &mut retained.stream;
        stream.select_program(&self.selection, &self.layouts);
        stream.bind_blend(self.state.blend, self.state.write_mask);
        stream.bind_rasterizer(self.raster.clone());
        stream.bind_depth_stencil(self.depth.clone());
    }

    pub fn bind_textures(&self, retained: &mut State, textures: &BTreeMap<u32, wgpu::TextureView>) {
        self.bind_images(
            retained,
            textures
                .iter()
                .map(|(key, image)| (*key, image.clone().into())),
        );
    }

    fn bind_images(
        &self,
        retained: &mut State,
        textures: impl Iterator<Item = (u32, gpu::TextureBinding)>,
    ) {
        use content_reader_gui::pdxshader::program::Stage;
        let stream = &mut retained.stream;
        for (key, texture) in textures {
            let stage = if key % 2 == 0 {
                Stage::Vertex
            } else {
                Stage::Pixel
            };
            stream.bind_formatted_texture(
                key / 2,
                stage,
                Some(texture),
                self.state.samplers.get(&key),
            );
        }
    }
}

pub struct Partial {
    pub bytes: Vec<u8>,
    pub size: u64,
    pub known: Vec<Range<u64>>,
}

#[derive(Clone)]
enum Bytes {
    Complete(Arc<[u8]>),
    Partial(Arc<Mutex<Partial>>),
}

#[derive(Clone)]
struct Buffer {
    bytes: Bytes,
    offset: u64,
}

#[derive(Default)]
pub struct State {
    pub stream: stream::Stream,
    vertices: BTreeMap<u32, Buffer>,
}

impl State {
    pub fn begin_frame(&mut self) {
        self.stream.begin_frame();
        self.vertices.clear();
    }

    pub fn inherit(&mut self, previous: &Self) {
        self.stream.inherit_bindings(&previous.stream);
        self.vertices = previous.vertices.clone();
    }

    pub fn bind_vertex(&mut self, slot: u32, stride: u64, bytes: Arc<[u8]>) {
        self.stream.bind_vertex_stride(slot, stride);
        self.vertices.insert(
            slot,
            Buffer {
                bytes: Bytes::Complete(bytes),
                offset: 0,
            },
        );
    }

    pub fn bind_partial_vertex(
        &mut self,
        slot: u32,
        stride: u64,
        allocation: Arc<Mutex<Partial>>,
        offset: u64,
    ) {
        self.stream.bind_vertex_stride(slot, stride);
        self.vertices.insert(
            slot,
            Buffer {
                bytes: Bytes::Partial(allocation),
                offset,
            },
        );
    }

    pub fn prepare(
        &mut self,
        arena: &gpu::Arena,
        label: impl std::fmt::Display,
        template: &gpu::Pass,
        vertices: u32,
        instances: u32,
    ) -> io::Result<stream::NativeSubmission> {
        self.prepare_with(arena, label, template, vertices, instances, None, None)
    }

    #[expect(clippy::too_many_arguments, reason = "the arena, the label and template, the counts, and the prepared and previous submissions")]
    pub fn prepare_with(
        &mut self,
        arena: &gpu::Arena,
        label: impl std::fmt::Display,
        template: &gpu::Pass,
        vertices: u32,
        instances: u32,
        prepared: Option<stream::NativePrepared<'_>>,
        previous: Option<&stream::PassSubmission>,
    ) -> io::Result<stream::NativeSubmission> {
        self.prepare_with_cache(
            arena, label, template, vertices, instances, prepared, previous, None,
        )
    }

    #[expect(clippy::too_many_arguments, reason = "the arena, the label and template, the counts, the prepared and previous submissions, and the cache")]
    pub fn prepare_with_cache(
        &mut self,
        arena: &gpu::Arena,
        label: impl std::fmt::Display,
        template: &gpu::Pass,
        vertices: u32,
        instances: u32,
        prepared: Option<stream::NativePrepared<'_>>,
        previous: Option<&stream::PassSubmission>,
        cache: Option<&mut stream::NativePassCache>,
    ) -> io::Result<stream::NativeSubmission> {
        if prepared.is_some() {
            return self
                .stream
                .prepare_native_pass_with_cache(
                    arena,
                    label,
                    template,
                    &BTreeMap::new(),
                    vertices,
                    instances,
                    prepared,
                    previous,
                    cache,
                )
                .map_err(io::Error::other);
        }
        let stream = &mut self.stream;
        with_buffers(&self.vertices, |buffers| {
            stream
                .prepare_native_pass_with_cache(
                    arena, label, template, buffers, vertices, instances, None, previous, cache,
                )
                .map_err(io::Error::other)
        })
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn packed_bytes(
        &self,
        vertices: u32,
        instances: u32,
    ) -> io::Result<shader_pipeline::vertices::PackedBuffers> {
        with_buffers(&self.vertices, |buffers| {
            shader_pipeline::vertices::pack_native(
                &self.stream.vertex_layouts()?,
                buffers,
                vertices,
                instances,
            )
        })
        .map_err(io::Error::other)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn snapshot(&self) -> Self {
        let mut snapshot = Self::default();
        snapshot.inherit(self);
        for buffer in snapshot.vertices.values_mut() {
            if let Bytes::Partial(allocation) = &buffer.bytes {
                let copy = {
                    let allocation = allocation.lock().unwrap_or_else(|e| e.into_inner());
                    Partial {
                        bytes: allocation.bytes.clone(),
                        size: allocation.size,
                        known: allocation.known.clone(),
                    }
                };
                buffer.bytes = Bytes::Partial(Arc::new(Mutex::new(copy)));
            }
        }
        snapshot
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn same_bindings(&self, other: &Self) -> bool {
        self.stream.same_bindings(&other.stream)
            && self.vertices.len() == other.vertices.len()
            && self.vertices.iter().all(|(slot, a)| {
                let Some(b) = other.vertices.get(slot) else {
                    return false;
                };
                if a.offset != b.offset {
                    return false;
                }
                match (&a.bytes, &b.bytes) {
                    (Bytes::Complete(a), Bytes::Complete(b)) => a == b,
                    (Bytes::Partial(a), Bytes::Partial(b)) => {
                        if Arc::ptr_eq(a, b) {
                            return true;
                        }
                        let a = a.lock().unwrap_or_else(|e| e.into_inner());
                        let b = b.lock().unwrap_or_else(|e| e.into_inner());
                        a.size == b.size && a.bytes == b.bytes && a.known == b.known
                    }
                    _ => false,
                }
            })
    }
}

fn with_buffers<R>(
    vertices: &BTreeMap<u32, Buffer>,
    work: impl FnOnce(&BTreeMap<u32, Option<NativeBuffer<'_>>>) -> R,
) -> R {
    enum Locked<'a> {
        Complete(&'a [u8], [Range<u64>; 1]),
        Partial(MutexGuard<'a, Partial>),
    }
    let locked: Vec<_> = vertices
        .iter()
        .map(|(slot, buffer)| {
            let bytes = match &buffer.bytes {
                Bytes::Complete(bytes) => Locked::Complete(
                    bytes,
                    [Range {
                        start: 0,
                        end: bytes.len() as u64,
                    }],
                ),
                Bytes::Partial(allocation) => {
                    Locked::Partial(allocation.lock().unwrap_or_else(|error| error.into_inner()))
                }
            };
            (*slot, buffer.offset, bytes)
        })
        .collect();
    let buffers = locked
        .iter()
        .map(|(slot, offset, bytes)| {
            let buffer = match bytes {
                Locked::Complete(bytes, known) => NativeBuffer {
                    bytes,
                    size: bytes.len() as u64,
                    offset: *offset,
                    known,
                },
                Locked::Partial(allocation) => NativeBuffer {
                    bytes: &allocation.bytes,
                    size: allocation.size,
                    offset: *offset,
                    known: &allocation.known,
                },
            };
            (*slot, Some(buffer))
        })
        .collect();
    work(&buffers)
}

pub fn world_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    targets: &Targets,
    label: &str,
) -> wgpu::RenderPass<'a> {
    encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(targets.scene_attachment(wgpu::LoadOp::Load))],
        depth_stencil_attachment: Some(targets.depth_attachment(wgpu::LoadOp::Load)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}
