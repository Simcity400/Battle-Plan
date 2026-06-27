//! `GpuView` — the winit window's wgpu SURFACE + the present pass (Stage 1
//! deliverable 1, GPU half).
//!
//! The app creates `Instance -> Surface(window) -> Adapter(compatible_surface) ->
//! Device/Queue` here and injects that device into `render::Renderer::from_device`,
//! so the SURFACE and the existing draw/map pipelines share ONE device (Codex §1:
//! cross-device resources fail wgpu validation). The existing engine still renders
//! into its OFFSCREEN `Rgba8Unorm` compositor target (untouched golden path); each
//! frame, after the map + UI have composited into that target, [`present`] samples it
//! through a tiny present pipeline and writes it into the acquired surface frame,
//! converting to the surface's colour space in-shader (Codex §1 sRGB trap). No
//! per-frame CPU readback (Codex §2): the compositor texture stays on the GPU.

use std::sync::Arc;

use render::Renderer;
use winit::window::Window;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PresentParams {
    srgb_out: f32,
    _pad: [f32; 3],
}

/// The GPU surface view: owns the surface, the shared device (via `Renderer`), the
/// present pipeline, and the per-frame present logic.
pub struct GpuView {
    /// The window (kept alive for the surface; `Arc` so the surface can borrow it
    /// for `'static`).
    _window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    surface_format: wgpu::TextureFormat,
    config: wgpu::SurfaceConfiguration,

    /// The reused engine renderer (offscreen compositor) over the SAME device.
    pub renderer: Renderer,

    present_pipeline: wgpu::RenderPipeline,
    present_params_buf: wgpu::Buffer,
    present_params_bg: wgpu::BindGroup,
}

impl GpuView {
    /// Build the surface + shared device + present pipeline for `window` at
    /// `(width,height)`. Returns an error string on any wgpu setup failure (the app
    /// reports it as a BLOCKER — never a silent fallback).
    pub fn new(window: Arc<Window>, width: u32, height: u32) -> Result<GpuView, String> {
        pollster::block_on(Self::new_async(window, width, height))
    }

    async fn new_async(window: Arc<Window>, width: u32, height: u32) -> Result<GpuView, String> {
        let width = width.max(1);
        let height = height.max(1);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY | wgpu::Backends::GL,
            ..Default::default()
        });

        // The surface borrows the window; the Arc<Window> keeps it alive for 'static.
        let surface = instance
            .create_surface(window.clone())
            .map_err(|e| format!("create_surface failed: {e}"))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                // CRITICAL: the adapter MUST be compatible with this surface, or the
                // device it yields cannot present to it (Codex §1).
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .ok_or_else(|| "no wgpu adapter compatible with the surface".to_string())?;
        let adapter_info = adapter.get_info();
        let bc_supported = adapter
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);

        let mut features = wgpu::Features::empty();
        if bc_supported {
            features |= wgpu::Features::TEXTURE_COMPRESSION_BC;
        }
        // Raise limits to the adapter's resolution: real the game UI textures exceed the
        // 2048 downlevel default (the topbar strip is 2346 px) — M7 hardening note.
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: Some("app-surface-device"),
                    required_features: features,
                    required_limits: limits,
                },
                None,
            )
            .await
            .map_err(|e| format!("request_device failed: {e}"))?;

        // Surface format: prefer a NON-sRGB format to avoid the gamma re-encode trap;
        // fall back to whatever the surface offers (and decode in the present shader).
        let caps = surface.get_capabilities(&adapter);
        if caps.formats.is_empty() {
            return Err("surface reports no supported formats".to_string());
        }
        let surface_format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let srgb_out = surface_format.is_srgb();

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: caps
                .present_modes
                .iter()
                .copied()
                .find(|m| *m == wgpu::PresentMode::Fifo)
                .unwrap_or(caps.present_modes[0]),
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        // Build the engine renderer over the SAME device (shared with the surface).
        let renderer =
            Renderer::from_device(device, queue, adapter_info, bc_supported, width, height);

        // --- present pipeline: sample the offscreen target -> surface, sRGB-aware. ---
        let device = &renderer.device;
        let present_params_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("present-params"),
            size: std::mem::size_of::<PresentParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        renderer.queue.write_buffer(
            &present_params_buf,
            0,
            bytemuck::bytes_of(&PresentParams {
                srgb_out: if srgb_out { 1.0 } else { 0.0 },
                _pad: [0.0; 3],
            }),
        );

        let params_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("present-params-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let present_params_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("present-params-bg"),
            layout: &params_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: present_params_buf.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("present-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("present.wgsl").into()),
        });
        // group 0 = the engine's texture+sampler layout (so offscreen_bind_group fits);
        // group 1 = the present params.
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("present-pl"),
            bind_group_layouts: &[renderer.tex_bind_group_layout(), &params_layout],
            push_constant_ranges: &[],
        });
        let present_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("present-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        Ok(GpuView {
            _window: window,
            surface,
            surface_format,
            config,
            renderer,
            present_pipeline,
            present_params_buf,
            present_params_bg,
        })
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }
    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    /// Resize the surface + the offscreen compositor target. A zero dimension or a
    /// no-op size is ignored.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if width == self.config.width && height == self.config.height {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&self.renderer.device, &self.config);
        self.renderer.resize(width, height);
    }

    /// PRESENT the already-composited offscreen target to the surface. The caller has
    /// (this frame) rendered the map + UI into `self.renderer`'s offscreen target via
    /// the no-readback flush path; this acquires the surface texture, runs the present
    /// pass (sample offscreen -> surface, colour-converted), and presents. Returns an
    /// error string on a surface-acquire failure the caller can react to (e.g.
    /// `Lost`/`Outdated` -> reconfigure).
    pub fn present(&mut self) -> Result<(), wgpu::SurfaceError> {
        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let offscreen_bg = self.renderer.offscreen_bind_group();
        let mut enc =
            self.renderer
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("present-enc"),
                });
        {
            let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("present-rp"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            rp.set_pipeline(&self.present_pipeline);
            rp.set_bind_group(0, &offscreen_bg, &[]);
            rp.set_bind_group(1, &self.present_params_bg, &[]);
            rp.draw(0..3, 0..1);
        }
        self.renderer.queue.submit(Some(enc.finish()));
        frame.present();
        // touch the params buffer field so the borrow is considered used across
        // resizes (kept for a future runtime sRGB-mode toggle).
        let _ = &self.present_params_buf;
        Ok(())
    }

    /// Block until all submitted GPU work has completed. Used by `--smoke` before a
    /// hard `process::exit` so the surface's frame/semaphores are released cleanly
    /// (otherwise a Vulkan teardown race panics: "SurfaceSemaphores still in use").
    pub fn wait_idle(&self) {
        self.renderer.device.poll(wgpu::Maintain::Wait);
    }
}
