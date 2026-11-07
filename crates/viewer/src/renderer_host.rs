use eframe::{egui, egui_wgpu, wgpu};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

#[cfg(test)]
mod tests;

type SharedRenderer = egui::mutex::RwLock<egui_wgpu::Renderer>;

pub(crate) struct RendererSession;

// A weak renderer key prevents address reuse without keeping retired renderers alive.
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
        // Commands finish at their call points; no queue changes registration or paint order.
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

    #[expect(clippy::too_many_arguments)]
    pub(crate) fn paint(
        &self,
        painter: &mut egui_wgpu::winit::Painter,
        viewport_id: egui::ViewportId,
        pixels_per_point: f32,
        clear_color: [f32; 4],
        clipped_primitives: &[egui::ClippedPrimitive],
        textures_delta: &mut egui::TexturesDelta,
        capture_data: Vec<egui::UserData>,
        window: &Arc<eframe::PaintWindow>,
    ) -> f32 {
        if skip_present() && capture_data.is_empty()
            && let Some(state) = painter.render_state()
        {
            return paint_unpresented(&state, pixels_per_point, clear_color, clipped_primitives, textures_delta, window);
        }
        painter.paint_and_update_textures(
            viewport_id,
            pixels_per_point,
            clear_color,
            clipped_primitives,
            textures_delta,
            capture_data,
            window,
        )
    }
}

/// Measurement only (`BATTLE_PLAN_PERF_SKIP_MAP_PRESENT`): every frame is drawn as usual but into a
/// texture of the window's size, never presented, so a run can compare the page's frame rate with
/// the map drawing but not composited against the map presenting.
fn skip_present() -> bool {
    static SKIP: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("BATTLE_PLAN_PERF_SKIP_MAP_PRESENT").or_else(|| std::env::var_os("HOI4_PERF_SKIP_MAP_PRESENT")).is_some());
    *SKIP
}

thread_local! {
    /// The texture unpresented frames are drawn into, kept while the window keeps its size.
    static UNPRESENTED: std::cell::RefCell<Option<(wgpu::TextureView, [u32; 2], wgpu::TextureFormat)>> =
        const { std::cell::RefCell::new(None) };
}

fn paint_unpresented(
    state: &egui_wgpu::RenderState,
    pixels_per_point: f32,
    clear_color: [f32; 4],
    clipped_primitives: &[egui::ClippedPrimitive],
    textures_delta: &mut egui::TexturesDelta,
    window: &eframe::PaintWindow,
) -> f32 {
    let size = window.inner_size();
    let size = [size.width.max(1), size.height.max(1)];
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
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point };
    let mut encoder = state.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("unpresented map frame") });
    let user = {
        let mut renderer = state.renderer.write();
        for (id, deltas) in textures_delta.set.drain() {
            for delta in deltas {
                renderer.update_texture(&state.device, &state.queue, id, &delta);
            }
        }
        renderer.update_buffers(&state.device, &state.queue, &mut encoder, clipped_primitives, &screen)
    };
    {
        let renderer = state.renderer.read();
        let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("unpresented map frame"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &view,
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
        renderer.render(&mut pass.forget_lifetime(), clipped_primitives, &screen);
    }
    state.queue.submit(user.into_iter().chain([encoder.finish()]));
    {
        let mut renderer = state.renderer.write();
        for id in textures_delta.free.drain() {
            renderer.free_texture(&id);
        }
    }
    // Paced by the display's compositions, as a presented frame is by its swap chain.
    let started = std::time::Instant::now();
    #[cfg(windows)]
    {
        #[link(name = "dwmapi")]
        unsafe extern "system" {
            fn DwmFlush() -> i32;
        }
        // SAFETY: DwmFlush takes no arguments and only waits for the next composition.
        unsafe {
            DwmFlush();
        }
    }
    started.elapsed().as_secs_f32()
}
