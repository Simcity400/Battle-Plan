//! CORE SURFACE 5 (migrated) — the LAND-COMBAT overlay (bubbles over the map + a
//! stacked read-only panel), an Overlay-band view wrapping the existing
//! [`crate::combat::CombatShell`]. The shell already separates persistent state (which
//! combat is open) from the per-hour `rebind` (aggregated view-models), so it maps onto
//! the host: `ViewInstance::rebind` → `shell.rebind`, bubble pick / close → `on_input`,
//! and `bind` → a `Drawable` that draws the bubbles + the stacked panel + the holding-
//! tag flag. Read-only; every battle-detail field stays badged unavailable.
//!
//! NOTE: the combat overlay is NOT modal — its bubbles sit over the map but a click off
//! a bubble must still reach the map (province pick). So it uses a non-modal policy and
//! reports hits only on bubbles / the open panel's close control.

use std::collections::HashMap;

use combat_panel::bubbles::Bubble;
use combat_panel::{CombatVM, SideTag};

use crate::camera::MapCamera;
use crate::combat::CombatShell;
use crate::host::context::BindContext;
use crate::host::draw::FrameDraw;
use crate::host::input::{PointerButton, ViewInput};
use crate::host::registry::{ViewDescriptor, ViewId};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InputOutcome, InstanceId, RebindContext,
    ViewCommand, ViewInstance, ZBand,
};
use crate::host::FidelityReport;
use crate::register_view;

pub const COMBAT_VIEW_ID: ViewId = ViewId("core.land_combat");

/// The combat overlay view: owns the shell + the white texture + the live camera (kept
/// so `on_input` can hit-test bubbles in screen space) + the current screen size (for
/// the close-button geometry, which is a PURE function of screen size — no draw-to-
/// input feedback channel needed).
pub struct CombatView {
    shell: CombatShell,
    white: &'static render::Texture,
    camera: MapCamera,
    screen: (u32, u32),
}

impl CombatView {
    pub fn new(
        shell: CombatShell,
        white: &'static render::Texture,
        camera: MapCamera,
        screen: (u32, u32),
    ) -> CombatView {
        CombatView {
            shell,
            white,
            camera,
            screen,
        }
    }
}

/// The panel close-button screen rect — a PURE function of the screen size, mirroring
/// `combat_panel::view::draw_panel`'s placement (`px+pw-26, py+5, 20, 20`). Because it
/// is pure, `on_input` recomputes it without any draw→input feedback channel.
fn panel_close_rect(screen_w: u32, screen_h: u32) -> (f32, f32, f32, f32) {
    let (w, h) = (screen_w as f32, screen_h as f32);
    let pw = (w * 0.46).clamp(360.0, 720.0);
    let _ph = (h - 120.0).clamp(280.0, 640.0);
    let px = w - pw - 24.0;
    let py = 64.0;
    (px + pw - 26.0, py + 5.0, 20.0, 20.0)
}

/// An owned per-frame copy of the combat overlay inputs (so the `Drawable` is self-
/// contained, no borrow of the shell).
struct OwnedCombatOverlay {
    bubbles: Vec<Bubble>,
    open: Option<(CombatVM, Option<String>)>,
    white: &'static render::Texture,
    /// the screen size at bind time — a panel-placement draw input (part of the digest).
    screen: (u32, u32),
}

pub struct CombatDrawable {
    overlay: OwnedCombatOverlay,
}

impl Drawable for CombatDrawable {
    /// Combat's pixel inputs: each bubble's screen position + radius + selected id +
    /// the open combat's id + window title (the panel's content). Detects a moved
    /// bubble / changed selection / different panel — what the empty node digest misses.
    fn draw_digest(&self) -> String {
        let mut s = format!("COMBAT screen={:?} bubbles=[", self.overlay.screen);
        for b in &self.overlay.bubbles {
            // include defender_tag — it drives the bubble COLOUR (Codex), so a colour
            // change must perturb the digest.
            s.push_str(&format!(
                "{}@{:.2},{:.2},r{:.2},def{:?};",
                b.combat_id, b.sx, b.sy, b.radius, b.defender_tag
            ));
        }
        s.push(']');
        match &self.overlay.open {
            Some((vm, ideo)) => s.push_str(&format!(
                " open={} kind={} prov={} ideo={:?} title={} atk_ref={} def_ref={} def_tag={:?}",
                vm.combat_id,
                vm.kind,
                vm.location_province,
                ideo,
                vm.window_title(),
                vm.attacker.ref_count(),
                vm.defender.ref_count(),
                vm.defender.tag.tag(),
            )),
            None => s.push_str(" open=none"),
        }
        s
    }

    fn draw(&self, fd: &mut FrameDraw) {
        draw_combat_overlay(fd, &self.overlay);
    }
}

impl ViewInstance for CombatView {
    fn view_id(&self) -> ViewId {
        COMBAT_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        // NOT modal: a click off a bubble must fall through to the map pick.
        HostPolicy::window(ZBand::Overlay)
    }

    fn rebind(&mut self, ctx: &RebindContext) {
        self.screen = ctx.screen;
        self.shell.rebind(ctx.snapshot);
    }

    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                // 1) a click on the OPEN panel's close control closes it. The close-rect
                // is recomputed PURELY from the screen size (no draw feedback).
                if self.shell.is_open() {
                    let r = panel_close_rect(self.screen.0, self.screen.1);
                    if self.shell.try_close_at(r, x, y) {
                        return InputOutcome::consumed();
                    }
                }
                // 2) a click on a bubble opens its panel (consumes the click).
                if self.shell.try_open_at(&self.camera, x, y) {
                    return InputOutcome::consumed();
                }
                InputOutcome::ignored()
            }
            ViewInput::Key {
                key: crate::host::input::ViewKey::Escape,
            } => {
                if self.shell.is_open() {
                    self.shell.close();
                    return InputOutcome::consumed();
                }
                InputOutcome::ignored()
            }
            _ => InputOutcome::ignored(),
        }
    }

    fn bind(&self, ctx: &BindContext) -> BoundView {
        // bubbles are laid out from the LIVE map camera (read from the context; falls
        // back to the view's own cached camera if the context lacks one).
        let cam = ctx.map_camera.unwrap_or(self.camera);
        let bubbles = self.shell.bubbles(&cam);
        let open = self
            .shell
            .open_combat()
            .map(|(vm, ideo)| (vm.clone(), ideo.map(|s| s.to_string())));

        let overlay = OwnedCombatOverlay {
            bubbles,
            open,
            white: self.white,
            screen: self.screen,
        };

        // the combat overlay reports NO GUI leaf to the host (its bubble/close hit-test
        // is internal, in `on_input`), so its hit model is empty — a click that misses a
        // bubble/close falls through to the map. The view's `on_input` runs FIRST (it is
        // higher-z than the map), so it gets first refusal on bubbles.
        BoundView {
            instance: InstanceId(0),
            view_id: COMBAT_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::none(),
            drawable: Box::new(CombatDrawable { overlay }),
            // every battle-detail field is badged unavailable; the surface as a whole is
            // partial → record one unavailable to flag it honestly.
            fidelity: {
                let mut f = FidelityReport::default();
                f.unavailable += 1;
                f
            },
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

impl CombatView {
    /// Mutable access to the wrapped shell (the C key in the binary toggles it).
    pub fn shell_mut(&mut self) -> &mut CombatShell {
        &mut self.shell
    }
    pub fn set_camera(&mut self, cam: MapCamera) {
        self.camera = cam;
    }
}

/// Draw the combat overlay through `FrameDraw` — byte-identical to the old
/// `compose::draw_combat_overlay` (bubbles + stacked panel + holding-tag flag). Writes
/// the close-button rect into the overlay's out-cell for the next click.
fn draw_combat_overlay(fd: &mut FrameDraw, ov: &OwnedCombatOverlay) {
    let (w, h) = fd.screen();
    let (w, h) = (w as f32, h as f32);
    let full = [0.0, 0.0, w, h];

    let selected_id = ov.open.as_ref().map(|(vm, _)| vm.combat_id.as_str());
    combat_panel::view::draw_bubbles(fd.renderer, ov.white, &ov.bubbles, selected_id, full);

    let (vm, ideology) = match &ov.open {
        Some((vm, ideo)) => (vm, ideo.clone()),
        None => return,
    };

    let mut flags: HashMap<String, render::Texture> = HashMap::new();
    if let SideTag::HoldingTagDerived(tag) = &vm.defender.tag {
        let extracted = repo_extracted_flags();
        if let Some(p) = combat_panel::flag_path(
            &fd.stage.resolver,
            extracted.as_deref(),
            tag,
            ideology.as_deref(),
        ) {
            if let Ok(tex) = fd.renderer.load_image(&p) {
                flags.insert(tag.clone(), tex);
            }
        }
    }

    let pw = (w * 0.46).clamp(360.0, 720.0);
    let ph = (h - 120.0).clamp(280.0, 640.0);
    let px = w - pw - 24.0;
    let py = 64.0;

    let loaded = fd.fonts.get(fd.renderer, "hoi_16mbs");
    let (font_ref, pages_ref): (Option<&assets::BmFont>, &[render::Texture]) = match &loaded {
        Some(l) => (Some(&l.font), &l.pages),
        None => (None, &[]),
    };
    let assets = combat_panel::view::DrawAssets {
        white: ov.white,
        font: font_ref,
        font_pages: pages_ref,
        flags: &flags,
    };
    let (_stats, _close) = combat_panel::view::draw_panel(fd.renderer, vm, &assets, px, py, pw, ph);
}

/// The extracted-flag PNG dir (`game_assets/assets/flags`), if present (READ-ONLY).
fn repo_extracted_flags() -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .parent()?
        .join("game_assets")
        .join("assets")
        .join("flags");
    p.is_dir().then_some(p)
}

/// Seeded by the binary (needs the loaded shell + the white texture + the map camera).
fn open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    panic!("core.land_combat is seeded by the binary (needs the loaded shell + white texture)");
}

register_view!(ViewDescriptor {
    id: COMBAT_VIEW_ID,
    gui_root: None,
    required_domains: &["combats", "map"],
    route: None,    // toggled by the C key; seeded by the binary (needs leaked textures)
    shortcut: None, // C is handled as Core seed-glue (the factory needs leaked assets)
    open,
    policy: HostPolicy::window(ZBand::Overlay),
});

pub fn link() {}
