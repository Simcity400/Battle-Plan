//! CPU shadows of native IA bindings for missing-program transitions.
use eframe::wgpu;
use shader_pipeline::{gpu, stream, vertices::NativeBuffer};
use std::sync::MutexGuard;
use std::{
    collections::BTreeMap,
    io,
    ops::Range,
    sync::{Arc, Mutex},
};

/// The native commands of a compiled world producer, before a later null
/// technique retains them. Physical binds are independent of reflected usage.
pub struct Program {
    selection: shader_pipeline::library::Selection,
    layouts: Vec<gpu::VertexLayout>,
    state: gpu::State,
    raster: installed_content_core::pdxshader::RasterizerState,
    depth: installed_content_core::pdxshader::DepthStencilState,
}

impl Program {
    /// The technique it draws with.
    #[cfg(test)]
    pub fn selection(&self) -> &shader_pipeline::library::Selection {
        &self.selection
    }

    /// The vertex layouts it draws with.
    #[cfg(test)]
    pub fn layouts(&self) -> &[gpu::VertexLayout] {
        &self.layouts
    }

    /// The state it draws with.
    #[cfg(test)]
    pub fn state(&self) -> &gpu::State {
        &self.state
    }

    pub fn new(
        selection: &shader_pipeline::library::Selection,
        layouts: &[gpu::VertexLayout],
        state: &gpu::State,
        raster: &installed_content_core::pdxshader::RasterizerState,
        depth: &installed_content_core::pdxshader::DepthStencilState,
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
        use installed_content_core::pdxshader::program::Stage;
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
        self.bind_textures(retained, textures);
    }

    /// The sampler this program binds with a texture at `key`.
    pub fn sampler(&self, key: u32) -> Option<&installed_content_core::pdxshader::program::Sampler> {
        self.state.samplers.get(&key)
    }

    pub fn select(&self, retained: &mut State) {
        let stream = &mut retained.stream;
        stream.bind_program(Some(self.selection.clone()), self.layouts.clone());
        stream.bind_blend(self.state.blend, self.state.write_mask);
        stream.bind_rasterizer(self.raster.clone());
        stream.bind_depth_stencil(self.depth.clone());
    }

    pub fn bind_textures(&self, retained: &mut State, textures: &BTreeMap<u32, wgpu::TextureView>) {
        use installed_content_core::pdxshader::program::Stage;
        let stream = &mut retained.stream;
        for (&key, texture) in textures {
            let stage = if key % 2 == 0 {
                Stage::Vertex
            } else {
                Stage::Pixel
            };
            stream.bind_texture(
                key / 2,
                stage,
                Some(texture.clone()),
                self.state.samplers.get(&key),
            );
        }
    }
}

/// A native vertex allocation of `size` bytes: its written prefix and the sorted, disjoint
/// known ranges within it.
pub struct Partial {
    pub bytes: Vec<u8>,
    pub size: u64,
    pub known: Vec<Range<u64>>,
}

/// D3D11 binds a vertex buffer by handle, so a ring's NO_OVERWRITE append after a bind is
/// what a later draw through that bind reads.
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
    pub fn inherit(&mut self, previous: &Self) {
        self.stream.inherit_bindings(&previous.stream);
        self.vertices = previous.vertices.clone();
    }

    /// A complete allocation, every byte known.
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
        label: &str,
        template: &gpu::Pass,
        vertices: u32,
        instances: u32,
    ) -> io::Result<stream::NativeSubmission> {
        enum Locked<'a> {
            Complete(&'a [u8], [Range<u64>; 1]),
            Partial(MutexGuard<'a, Partial>),
        }
        let locked: Vec<_> = self
            .vertices
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
                    Bytes::Partial(allocation) => Locked::Partial(
                        allocation.lock().unwrap_or_else(|error| error.into_inner()),
                    ),
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
        self.stream
            .prepare_native_pass(arena, label, template, &buffers, vertices, instances)
            .map_err(io::Error::other)
    }
}
