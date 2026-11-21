use shader_pipeline::gpu::stats::CountedPasses as _;
use eframe::{egui, egui_wgpu, wgpu};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

#[cfg(test)]
mod tests;

type SharedRenderer = egui::mutex::RwLock<egui_wgpu::Renderer>;

pub(crate) struct RendererSession;

struct Registry<K, S> {
    entries: Vec<(Weak<K>, Arc<S>)>,
}

impl<K, S> Registry<K, S> {
    const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn find(&mut self, key: &Arc<K>) -> Option<Arc<S>> {
        self.entries
            .retain(|(renderer, _)| renderer.strong_count() > 0);
        self.entries
            .iter()
            .find(|(renderer, _)| std::ptr::eq(renderer.as_ptr(), Arc::as_ptr(key)))
            .map(|(_, session)| Arc::clone(session))
    }

    fn bind(&mut self, key: &Arc<K>, make: impl FnOnce() -> S) -> Arc<S> {
        if let Some(session) = self.find(key) {
            return session;
        }
        let session = Arc::new(make());
        self.entries
            .push((Arc::downgrade(key), Arc::clone(&session)));
        session
    }
}

static SESSIONS: Mutex<Registry<SharedRenderer, RendererSession>> = Mutex::new(Registry::new());

fn sessions() -> MutexGuard<'static, Registry<SharedRenderer, RendererSession>> {
    SESSIONS.lock().unwrap_or_else(PoisonError::into_inner)
}

pub(crate) fn bind(state: &egui_wgpu::RenderState) -> Arc<RendererSession> {
    sessions().bind(&state.renderer, || RendererSession)
}

pub(crate) fn of(state: &egui_wgpu::RenderState) -> Option<Arc<RendererSession>> {
    sessions().find(&state.renderer)
}

pub(crate) fn local(state: &egui_wgpu::RenderState) -> Option<Local<'_>> {
    of(state).map(|session| Local { session, state })
}

pub(crate) struct Target {
    pub(crate) view: wgpu::TextureView,
    pub(crate) filter: wgpu::FilterMode,
}

pub(crate) struct Local<'a> {
    session: Arc<RendererSession>,
    state: &'a egui_wgpu::RenderState,
}

impl Local<'_> {
    pub(crate) fn register(&self, target: Target) -> egui::TextureId {
        let Reply::Registered(id) = self.run(Command::Register(target)) else {
            unreachable!("a registration is answered with its id");
        };
        id
    }

    pub(crate) fn rebind(&self, id: egui::TextureId, target: Target) {
        self.run(Command::Rebind(id, target));
    }

    pub(crate) fn free(&self, id: egui::TextureId) {
        self.run(Command::Free(id));
    }

    fn run(&self, command: Command) -> Reply {
        self.session.execute(self.state, command)
    }
}

enum Command {
    Register(Target),
    Rebind(egui::TextureId, Target),
    Free(egui::TextureId),
}

enum Reply {
    Registered(egui::TextureId),
    Done,
}

impl RendererSession {
    fn execute(&self, state: &egui_wgpu::RenderState, command: Command) -> Reply {
        match command {
            Command::Register(Target { view, filter }) => Reply::Registered(
                state
                    .renderer
                    .write()
                    .register_native_texture(&state.device, &view, filter),
            ),
            Command::Rebind(id, Target { view, filter }) => {
                state
                    .renderer
                    .write()
                    .update_egui_texture_from_wgpu_texture(&state.device, &view, filter, id);
                Reply::Done
            }
            Command::Free(id) => {
                state.renderer.write().free_texture(&id);
                Reply::Done
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Painted {
    pub acquire: f32,
    pub presented: bool,
}

pub(crate) struct WindowSurface {
    surface: wgpu::Surface<'static>,
    configured: Option<([u32; 2], egui_wgpu::SurfaceConfig)>,
}

impl WindowSurface {
    pub(crate) fn new(surface: wgpu::Surface<'static>) -> Self {
        Self { surface, configured: None }
    }

    fn configure(&mut self, state: &egui_wgpu::RenderState, size: [u32; 2], config: egui_wgpu::SurfaceConfig) -> Result<(), String> {
        let default = self
            .surface
            .get_default_config(&state.adapter, size[0], size[1])
            .ok_or("Couldn’t draw in the map’s window: the graphics adapter doesn’t support it")?;
        let mut configuration = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: state.target_format,
            present_mode: config.present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![state.target_format],
            ..default
        };
        if let Some(latency) = config.desired_maximum_frame_latency {
            configuration.desired_maximum_frame_latency = latency;
        }
        self.surface.configure(&state.device, &configuration);
        self.configured = Some((size, config));
        Ok(())
    }

    #[expect(clippy::too_many_arguments, reason = "one frame's paint inputs, as eframe's painter takes them")]
    pub(crate) fn paint(
        &mut self,
        state: &egui_wgpu::RenderState,
        config: egui_wgpu::SurfaceConfig,
        size: Option<[u32; 2]>,
        pixels_per_point: f32,
        clear_color: [f32; 4],
        clipped_primitives: &[egui::ClippedPrimitive],
        textures_delta: &mut egui::TexturesDelta,
    ) -> Result<Painted, String> {
        let unpresented = |acquire| Ok(Painted { acquire, presented: false });
        let Some(size) = size else {
            update_textures(state, textures_delta);
            free_textures(state, textures_delta);
            return unpresented(0.0);
        };
        if skip_present() {
            return unpresented(paint_unpresented(state, size, pixels_per_point, clear_color, clipped_primitives, textures_delta));
        }
        if self.configured != Some((size, config)) {
            self.configure(state, size, config)?;
        }
        let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point };
        let mut encoder = state.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("map frame") });
        update_textures(state, textures_delta);
        let user = state.renderer.write().update_buffers(&state.device, &state.queue, &mut encoder, clipped_primitives, &screen);
        let waiting = std::time::Instant::now();
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.configured = None;
                update_textures(state, textures_delta);
                free_textures(state, textures_delta);
                return unpresented(0.0);
            }
            _ => {
                update_textures(state, textures_delta);
                free_textures(state, textures_delta);
                return unpresented(0.0);
            }
        };
        let acquire = waiting.elapsed().as_secs_f32();
        let view = texture.texture.create_view(&wgpu::TextureViewDescriptor::default());
        encode(state, encoder, user, &view, &screen, clear_color, clipped_primitives);
        free_textures(state, textures_delta);
        state.queue.present(texture);
        crate::screen_count::readout::presented();
        Ok(Painted { acquire, presented: true })
    }
}

fn update_textures(state: &egui_wgpu::RenderState, textures_delta: &mut egui::TexturesDelta) {
    let mut renderer = state.renderer.write();
    for (id, deltas) in textures_delta.set.drain() {
        for delta in deltas {
            renderer.update_texture(&state.device, &state.queue, id, &delta);
        }
    }
}

fn free_textures(state: &egui_wgpu::RenderState, textures_delta: &mut egui::TexturesDelta) {
    let mut renderer = state.renderer.write();
    for id in textures_delta.free.drain() {
        renderer.free_texture(&id);
    }
}

fn draw(
    state: &egui_wgpu::RenderState,
    view: &wgpu::TextureView,
    size: [u32; 2],
    pixels_per_point: f32,
    clear_color: [f32; 4],
    clipped_primitives: &[egui::ClippedPrimitive],
    textures_delta: &mut egui::TexturesDelta,
) {
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point };
    let mut encoder = state.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("map frame") });
    update_textures(state, textures_delta);
    let user = state.renderer.write().update_buffers(&state.device, &state.queue, &mut encoder, clipped_primitives, &screen);
    encode(state, encoder, user, view, &screen, clear_color, clipped_primitives);
    free_textures(state, textures_delta);
}

fn encode(
    state: &egui_wgpu::RenderState,
    mut encoder: wgpu::CommandEncoder,
    user: Vec<wgpu::CommandBuffer>,
    view: &wgpu::TextureView,
    screen: &egui_wgpu::ScreenDescriptor,
    clear_color: [f32; 4],
    clipped_primitives: &[egui::ClippedPrimitive],
) {
    {
        let renderer = state.renderer.read();
        let pass = encoder.counted_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("map frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: f64::from(clear_color[0]),
                        g: f64::from(clear_color[1]),
                        b: f64::from(clear_color[2]),
                        a: f64::from(clear_color[3]),
                    }),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        renderer.render(&mut pass.forget_lifetime(), clipped_primitives, screen);
    }
    state.queue.submit(user.into_iter().chain([encoder.finish()]));
}

fn skip_present() -> bool {
    static SKIP: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("BATTLEPLAN_PERF_SKIP_MAP_PRESENT").or_else(|| std::env::var_os("HOI4_PERF_SKIP_MAP_PRESENT")).is_some());
    *SKIP
}

thread_local! {
    static UNPRESENTED: std::cell::RefCell<Option<(wgpu::TextureView, [u32; 2], wgpu::TextureFormat)>> =
        const { std::cell::RefCell::new(None) };
}

fn paint_unpresented(
    state: &egui_wgpu::RenderState,
    size: [u32; 2],
    pixels_per_point: f32,
    clear_color: [f32; 4],
    clipped_primitives: &[egui::ClippedPrimitive],
    textures_delta: &mut egui::TexturesDelta,
) -> f32 {
    let size = [size[0].max(1), size[1].max(1)];
    let view = UNPRESENTED.with_borrow_mut(|held| {
        if !held.as_ref().is_some_and(|(_, at, format)| *at == size && *format == state.target_format) {
            let texture = state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("unpresented map frame"),
                size: wgpu::Extent3d { width: size[0], height: size[1], depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: state.target_format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });
            *held = Some((texture.create_view(&wgpu::TextureViewDescriptor::default()), size, state.target_format));
        }
        held.as_ref().map(|(view, _, _)| view.clone())
    });
    let Some(view) = view else { return 0.0 };
    draw(state, &view, size, pixels_per_point, clear_color, clipped_primitives, textures_delta);
    let started = std::time::Instant::now();
    #[cfg(windows)]
    {
        #[link(name = "dwmapi")]
        unsafe extern "system" {
            fn DwmFlush() -> i32;
        }
        unsafe {
            DwmFlush();
        }
    }
    started.elapsed().as_secs_f32()
}
