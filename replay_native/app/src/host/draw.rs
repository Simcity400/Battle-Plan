//! The per-frame immediate-mode DRAW bundle a [`Drawable`](crate::host::view::Drawable)
//! uses to composite itself — the renderer + the cross-frame texture/font caches + the
//! static stage + the per-frame render context (screen size, etc).
//!
//! This is the seam that lets a view module OWN its draw (Codex (c)): the old
//! special-case draws — topbar flag sprite, the buildings gridbox + scrollbar, the
//! occupation bars, the combat flags, the focus header — move into their view modules
//! and call these helpers, instead of being branches in `compose.rs`.

use assets::SpriteManifest;
use gui_runtime::draw::{draw_window, DrawStats, TextureCache};
use gui_runtime::fonts::FontCache;
use gui_runtime::scene::Node;
use render::Renderer;

use crate::host::context::Stage;

/// Everything a view's [`Drawable`](crate::host::view::Drawable) needs to draw itself
/// this frame: the renderer (accumulating quads composited over the map), the shared
/// caches, the static stage, and the per-frame [`RenderContext`].
pub struct FrameDraw<'a> {
    pub renderer: &'a mut Renderer,
    pub textures: &'a mut TextureCache<'static>,
    pub fonts: &'a mut FontCache<'static>,
    pub stage: &'a Stage,
    pub rc: RenderContext,
    /// accumulated draw stats (informational).
    pub stats: DrawStats,
}

/// The per-frame, hour-independent render context (screen geometry + anything a
/// `Drawable` needs that is NOT the snapshot). The snapshot was already consumed by
/// `bind`; draw is replayable GPU work only.
#[derive(Debug, Clone, Copy)]
pub struct RenderContext {
    pub screen_w: u32,
    pub screen_h: u32,
}

impl<'a> FrameDraw<'a> {
    pub fn new(
        renderer: &'a mut Renderer,
        textures: &'a mut TextureCache<'static>,
        fonts: &'a mut FontCache<'static>,
        stage: &'a Stage,
        rc: RenderContext,
    ) -> FrameDraw<'a> {
        FrameDraw {
            renderer,
            textures,
            fonts,
            stage,
            rc,
            stats: DrawStats::default(),
        }
    }

    /// Draw a laid-out scene-graph (the plain-window path).
    pub fn draw_nodes(&mut self, roots: &[Node]) {
        for root in roots {
            draw_window(
                self.renderer,
                root,
                self.textures,
                self.fonts,
                &mut self.stats,
            );
        }
    }

    /// The static sprite manifest (for border sizes / frame counts in custom draws).
    pub fn manifest(&self) -> &SpriteManifest {
        &self.stage.manifest
    }

    pub fn screen(&self) -> (u32, u32) {
        (self.rc.screen_w, self.rc.screen_h)
    }
}
