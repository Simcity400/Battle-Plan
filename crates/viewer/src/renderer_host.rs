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
