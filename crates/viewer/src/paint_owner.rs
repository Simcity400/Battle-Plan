use crate::renderer_host::{self, RendererSession};
use eframe::{egui, egui_wgpu::winit::Painter};
use std::sync::{Arc, OnceLock};

#[derive(Default)]
struct SessionPaint {
    // A painter retains one renderer; the app creator binds it before its first paint.
    session: OnceLock<Option<Arc<RendererSession>>>,
}

impl SessionPaint {
    fn session(&self, painter: &Painter) -> Option<&Arc<RendererSession>> {
        let session = match self.session.get() {
            Some(session) => session,
            None => {
                let state = painter.render_state()?;
                self.session.get_or_init(|| renderer_host::of(&state))
            }
        };
        session.as_ref()
    }
}

impl eframe::PaintOwner for SessionPaint {
    fn paint(
        &self,
        painter: &mut Painter,
        viewport_id: egui::ViewportId,
        pixels_per_point: f32,
        clear_color: [f32; 4],
        clipped_primitives: &[egui::ClippedPrimitive],
        textures_delta: &mut egui::TexturesDelta,
        capture_data: Vec<egui::UserData>,
        window: &Arc<eframe::PaintWindow>,
    ) -> f32 {
        match self.session(painter) {
            Some(session) => session.paint(
                painter,
                viewport_id,
                pixels_per_point,
                clear_color,
                clipped_primitives,
                textures_delta,
                capture_data,
                window,
            ),
            None => painter.paint_and_update_textures(
                viewport_id,
                pixels_per_point,
                clear_color,
                clipped_primitives,
                textures_delta,
                capture_data,
                window,
            ),
        }
    }
}

pub(crate) fn owner() -> Arc<dyn eframe::PaintOwner> {
    Arc::new(SessionPaint::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_options_default_to_no_owner_and_clone_the_session_owner() {
        assert!(eframe::NativeOptions::default().paint_owner.is_none());
        let owner = owner();
        let options = eframe::NativeOptions {
            paint_owner: Some(Arc::clone(&owner)),
            ..Default::default()
        };
        assert!(Arc::ptr_eq(
            options.clone().paint_owner.as_ref().unwrap(),
            &owner
        ));
    }
}
