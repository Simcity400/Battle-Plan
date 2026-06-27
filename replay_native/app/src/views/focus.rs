//! CORE SURFACE 4 (migrated) — the FOCUS-TREE OVERLAY, a modal Overlay-band view that
//! wraps the existing [`crate::focus::FocusShell`] (placed graph + pan/zoom camera +
//! hover). The shell ALREADY separates persistent UI state (camera/hover) from the
//! per-hour `rebind`, so it maps directly onto the host model: `ViewInstance::rebind`
//! → `shell.rebind`, `on_input` → pan/zoom/hover, and `bind` → a `Drawable` that
//! re-runs the focus draw. Read-only navigation; nothing mutates the snapshot/game.

use std::cell::RefCell;

use crate::focus::FocusShell;
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

pub const FOCUS_VIEW_ID: ViewId = ViewId("core.focus_tree");

/// The focus overlay view instance: owns the shell + the white texture handle + the
/// last cursor (for pan-drag deltas) + the observed country.
pub struct FocusView {
    shell: FocusShell,
    white: &'static render::Texture,
    country: String,
    last_cursor: Option<(f32, f32)>,
    screen: (u32, u32),
}

impl FocusView {
    /// Build from a loaded shell + the app-owned white texture (leaked `'static`). The
    /// shell is opened (toggle) by the binary before the view is added to the host.
    pub fn new(
        mut shell: FocusShell,
        white: &'static render::Texture,
        country: impl Into<String>,
        snap: &snapshot::Snapshot,
        screen: (u32, u32),
    ) -> FocusView {
        let country = country.into();
        if !shell.is_open() {
            shell.toggle(snap, &country, screen);
        }
        FocusView {
            shell,
            white,
            country,
            last_cursor: None,
            screen,
        }
    }
}

/// The focus overlay's `Drawable` — re-runs the existing focus-overlay draw. The
/// `FocusOverlay` borrows the shell's placed tree + camera, so we build it inside a
/// RefCell-free closure: the drawable holds the shell by `&'static`? No — instead it
/// captures the laid-out overlay descriptor via the compose helper.
pub struct FocusDrawable {
    /// The bound overlay snapshot for this frame (placed tree + camera + header text).
    /// Owned so the drawable has no borrow of the shell.
    overlay: RefCell<Option<OwnedFocusOverlay>>,
}

/// An owned copy of the focus overlay's per-frame inputs (so the `Drawable` is self-
/// contained). The placed tree + domain are `Clone`-cheap snapshots of the shell state.
struct OwnedFocusOverlay {
    tree: focus_tree::PlacedTree,
    domain: focus_tree::FocusDomain,
    camera: focus_tree::Camera,
    white: &'static render::Texture,
    tag: String,
    date: String,
    hover: Option<String>,
}

impl Drawable for FocusDrawable {
    /// Focus's pixel inputs: the tree id + each placed node's id/position/fidelity + the
    /// camera (scale/offset/viewport) + header text + hover. Detects a moved node /
    /// changed camera / different hover — what the empty node digest misses.
    fn draw_digest(&self) -> String {
        let ov = self.overlay.borrow();
        let ov = match ov.as_ref() {
            Some(o) => o,
            None => return "FOCUS closed".into(),
        };
        let mut s = format!(
            "FOCUS tree={} tag={} date={} cam={:.3},{:.2},{:.2},vp{:?} hover={:?} nodes=[",
            ov.tree.id,
            ov.tag,
            ov.date,
            ov.camera.scale,
            ov.camera.ox,
            ov.camera.oy,
            ov.camera.viewport,
            ov.hover
        );
        for n in &ov.tree.nodes {
            // include the prereq/exclusive edges + icon — they drive LINK draw + node
            // colour (Codex), so a changed edge set must perturb the digest.
            s.push_str(&format!(
                "{}@{:.2},{:.2},f{:?},icon{:?},pre{:?},mx{:?};",
                n.id, n.px, n.py, n.fidelity, n.icon, n.prerequisites, n.mutually_exclusive
            ));
        }
        s.push(']');
        // the bound FocusDomain drives per-node STATE colour (completed/current/...),
        // so digest its completed/available sets — a domain change must perturb it.
        s.push_str(&format!(" domain={}", focus_domain_digest(&ov.domain)));
        s
    }

    fn draw(&self, fd: &mut FrameDraw) {
        let ov = self.overlay.borrow();
        let ov = match ov.as_ref() {
            Some(o) => o,
            None => return,
        };
        draw_focus_overlay(fd, ov);
    }
}

impl ViewInstance for FocusView {
    fn view_id(&self) -> ViewId {
        FOCUS_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::modal_overlay(ZBand::Overlay)
    }

    fn rebind(&mut self, ctx: &RebindContext) {
        self.screen = ctx.screen;
        if self.shell.is_open() {
            self.shell.rebind(
                ctx.snapshot,
                ctx.country
                    .is_empty()
                    .then(|| self.country.as_str())
                    .unwrap_or(ctx.country),
                ctx.screen,
                ctx.refit,
            );
        }
    }

    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                if self.shell.captures(x, y) {
                    self.last_cursor = Some((x, y));
                    return InputOutcome::consumed();
                }
                InputOutcome::ignored()
            }
            ViewInput::PointerMove { x, y, drag } => {
                let mut out = InputOutcome::ignored();
                if drag {
                    if let Some((ax, ay)) = self.last_cursor {
                        self.shell.pan(x - ax, y - ay);
                        self.last_cursor = Some((x, y));
                        out = InputOutcome::consumed();
                    }
                }
                self.shell.hover(x, y);
                out.needs_redraw = true;
                out
            }
            ViewInput::PointerUp { .. } => {
                self.last_cursor = None;
                InputOutcome::ignored()
            }
            ViewInput::Wheel { x, y, dy } => {
                if self.shell.captures(x, y) {
                    let factor = if dy >= 0.0 {
                        1.0 + dy * 0.12
                    } else {
                        1.0 / (1.0 - dy * 0.12)
                    };
                    self.shell.zoom_at(x, y, factor);
                    return InputOutcome::consumed();
                }
                InputOutcome::ignored()
            }
            ViewInput::Key {
                key: crate::host::input::ViewKey::Escape,
            } => {
                // ESC closes the overlay (the host removes this view).
                InputOutcome::consumed().with_command(ViewCommand::CloseSelf)
            }
            _ => InputOutcome::ignored(),
        }
    }

    fn bind(&self, _ctx: &BindContext) -> BoundView {
        // Snapshot the shell's overlay descriptor into an owned form for the drawable.
        let owned = self.shell.overlay(self.white).map(|ov| OwnedFocusOverlay {
            tree: ov.tree.clone(),
            domain: ov.domain.clone(),
            camera: ov.camera,
            white: self.white,
            tag: ov.tag,
            date: ov.date,
            hover: ov.hover,
        });
        // a modal overlay: it captures everything while open (no GUI leaves; the host's
        // modal policy consumes clicks). Empty hit model.
        BoundView {
            instance: InstanceId(0),
            view_id: FOCUS_VIEW_ID,
            policy: self.policy(),
            hit: HitModel::none(),
            drawable: Box::new(FocusDrawable {
                overlay: RefCell::new(owned),
            }),
            // the focus tree is a graph render, not a field-bound surface — its honesty
            // is per-node (badged in the legend). Count it as one derived surface.
            fidelity: {
                let mut f = FidelityReport::default();
                f.derived += 1;
                f
            },
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// A STABLE digest of the bound `FocusDomain` (the per-node STATE colour driver):
/// completed set (sorted for determinism) + current + progress. A domain change (e.g.
/// a focus completed between hours) perturbs this, so the focus digest detects a node-
/// colour change the placed-tree geometry alone would miss (Codex claim-1 fix).
fn focus_domain_digest(d: &focus_tree::FocusDomain) -> String {
    let mut completed: Vec<&String> = d.completed.iter().collect();
    completed.sort();
    format!(
        "tree={:?} completed={:?} current={:?} progress={:?}",
        d.focus_tree, completed, d.current, d.progress
    )
}

/// Draw the focus overlay through `FrameDraw` — byte-identical to the old
/// `compose::draw_focus_overlay` (same backdrop/header/legend/tooltip geometry).
fn draw_focus_overlay(fd: &mut FrameDraw, ov: &OwnedFocusOverlay) {
    let (w, h) = fd.screen();
    let (w, h) = (w as f32, h as f32);
    let full = [0.0, 0.0, w, h];
    fd.renderer.draw_sprite_frame_clipped(
        ov.white,
        0.0,
        0.0,
        w,
        h,
        0,
        1,
        [0.04, 0.05, 0.07, 0.92],
        Some(full),
    );
    let header_h = 56.0;
    fd.renderer.draw_sprite_frame_clipped(
        ov.white,
        0.0,
        0.0,
        w,
        header_h,
        0,
        1,
        [0.06, 0.07, 0.10, 1.0],
        Some(full),
    );

    let assets = focus_tree::DrawAssets {
        white: ov.white,
        font: None,
        font_pages: &[],
    };
    let stats = {
        let loaded = fd.fonts.get(fd.renderer, "hoi_16mbs");
        match loaded {
            Some(l) => {
                let a = focus_tree::DrawAssets {
                    white: ov.white,
                    font: Some(&l.font),
                    font_pages: &l.pages,
                };
                focus_tree::draw_tree(fd.renderer, &ov.camera, &ov.tree, &ov.domain, &a)
            }
            None => focus_tree::draw_tree(fd.renderer, &ov.camera, &ov.tree, &ov.domain, &assets),
        }
    };

    if let Some(loaded) = fd.fonts.get(fd.renderer, "hoi_18mbs") {
        let font = loaded.font.clone();
        let pages = &loaded.pages;
        let title = format!("FOCUS TREE  {}   {}   {}", ov.tree.id, ov.tag, ov.date);
        fd.renderer
            .draw_text(&font, pages, &title, 12.0, 6.0, [0.95, 0.95, 0.92, 1.0]);
        let legend = format!(
            "completed {}   current {}   prereqs-met {}   prereqs-unmet {}   approx-pos {}   [ESC closes]",
            stats.completed, stats.current, stats.available, stats.locked, stats.approximate
        );
        fd.renderer
            .draw_text(&font, pages, &legend, 12.0, 30.0, [0.85, 0.86, 0.84, 1.0]);
        if let Some(id) = &ov.hover {
            if let Some(node) = ov.tree.node(id) {
                let tt = focus_tree::tooltip(node, &ov.domain);
                let ty = h - 64.0;
                fd.renderer.draw_sprite_frame_clipped(
                    ov.white,
                    8.0,
                    ty - 4.0,
                    (tt.len() as f32) * 9.5 + 16.0,
                    26.0,
                    0,
                    1,
                    [0.05, 0.06, 0.09, 0.95],
                    Some(full),
                );
                fd.renderer
                    .draw_text(&font, pages, &tt, 14.0, ty, [0.95, 0.95, 0.85, 1.0]);
            }
        }
    }
}

/// The focus view is SEEDED by the binary (it needs the loaded shell + the white
/// texture + the current snapshot). Not opened cold via the registry.
fn open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    panic!("core.focus_tree is seeded by the binary (needs the loaded catalog + white texture)");
}

register_view!(ViewDescriptor {
    id: FOCUS_VIEW_ID,
    gui_root: None,
    required_domains: &["countries"],
    route: None,    // toggled by the F key; seeded by the binary (needs leaked textures)
    shortcut: None, // F is handled as Core seed-glue (the factory needs leaked assets)
    open,
    policy: HostPolicy::modal_overlay(ZBand::Overlay),
});

pub fn link() {}
