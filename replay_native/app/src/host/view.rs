//! BATCH 0 deliverable 2 — the VIEW traits: `ViewInstance` (persistent, input-driven)
//! + `BoundView` (the pure per-tick output) + `Drawable` (self-rendering).
//!
//! Codex (a): a "view" is NOT just a `Vec<Node>`. It is a registered route + a host
//! policy + PERSISTENT instance state (scroll / drag / camera / hover) + a PURE
//! per-snapshot `bind` + a self-rendering bound output. Splitting persistent input
//! state ([`ViewInstance`]) from the per-tick binding ([`ViewInstance::bind`], which is
//! `&self` — read-only over instance state) is what preserves scrub A→B→A byte
//! identity: re-binding the SAME instance at the SAME hour reproduces the SAME
//! [`BoundView`] (the camera/hover it reads were set by `input`, not by `bind`).
//!
//! There is deliberately NO closed `enum WindowKind` and NO `enum BoundView` — that
//! would just relocate the per-view hardcoding. Polymorphism is `dyn ViewInstance` +
//! `dyn Drawable`; the host iterates them with zero per-view branches.

use gui_runtime::scene::Node;
use gui_runtime::{Hit, Rect};
use render::modes::MapMode;

use crate::host::context::{classify_value, BindContext, Captured, FidelityReport};
use crate::host::draw::FrameDraw;
use crate::host::registry::ViewId;

/// A stable per-open-window instance id (unique within a host session).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstanceId(pub u64);

/// The context for the per-tick [`ViewInstance::rebind`] hour refresh: the new
/// snapshot, the observed country, the screen size, and whether to refit a camera
/// (false on a same-tree scrub so pan/zoom is preserved). No GPU — pure derive.
pub struct RebindContext<'a> {
    pub snapshot: &'a snapshot::Snapshot,
    pub country: &'a str,
    pub screen: (u32, u32),
    /// True only when the surface should re-fit its camera (open / country change).
    pub refit: bool,
}

impl<'a> RebindContext<'a> {
    /// Read a dotted country path during the pre-bind refresh, preserving the same
    /// Absent/Null/Measured trichotomy as [`BindContext::country_field`].
    pub fn country_field(&self, tag: &str, dotted: &str) -> Captured<&'a serde_json::Value> {
        classify_value(self.snapshot.country_path(tag, dotted))
    }

    /// Read a dotted state path during the pre-bind refresh.
    pub fn state_field(&self, sid: &str, dotted: &str) -> Captured<&'a serde_json::Value> {
        classify_value(self.snapshot.state_path(sid, dotted))
    }

    /// The top-level `order_of_battle` domain during the pre-bind refresh.
    pub fn order_of_battle(&self) -> Captured<&'a serde_json::Value> {
        classify_value(self.snapshot.domain("order_of_battle"))
    }

    /// Country ids and payloads for iteration-only accounting. Per-field reads still
    /// go through [`Self::country_field`].
    pub fn countries(&self) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
        self.snapshot.countries()
    }

    /// State ids and payloads for iteration-only accounting. Per-field reads still
    /// go through [`Self::state_field`].
    pub fn states(&self) -> Option<&'a serde_json::Map<String, serde_json::Value>> {
        self.snapshot.states()
    }
}

/// How the window host treats a view: where it sits in the stack, whether it blocks
/// the map, and whether it is modal (captures all input above it). This REPLACES the
/// closed `WindowKind` enum — it is data the host reads, not a per-view match.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HostPolicy {
    /// Base z-order band. Higher = nearer the viewer. The host keeps within-band
    /// insertion order, so opening two same-band views stacks them by open time.
    pub z_band: ZBand,
    /// A view that, while open, captures ALL pointer input (nothing below it, nor the
    /// map, receives a click) — e.g. the focus-tree overlay. A non-modal window only
    /// captures where its own controls / block-rect are.
    pub modal: bool,
    /// True if this view should be re-bound every scrub tick (essentially always, but
    /// a purely-static chrome view could opt out). Kept explicit for the scrubber loop.
    pub rebind_each_tick: bool,
}

impl HostPolicy {
    pub const fn window(z_band: ZBand) -> HostPolicy {
        HostPolicy {
            z_band,
            modal: false,
            rebind_each_tick: true,
        }
    }
    pub const fn modal_overlay(z_band: ZBand) -> HostPolicy {
        HostPolicy {
            z_band,
            modal: true,
            rebind_each_tick: true,
        }
    }
}

/// The coarse z-stacking bands (ascending = nearer the viewer). The scrubber/HUD is
/// drawn by the host *above* all bands (transport chrome, not a view).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ZBand {
    /// The bottom layer: the interactive map (province pick is the fall-through target).
    Background = 0,
    /// The always-on topbar frame.
    Chrome = 1,
    /// Normal panels (state panel, cluster windows).
    Panel = 2,
    /// Stacked, map-covering overlays (focus tree, combat).
    Overlay = 3,
}

/// The result of binding ONE view for ONE frame: its self-rendering [`Drawable`], its
/// hit model (for the host's hit-test), the fidelity report, and any commands it wants
/// the host to execute (open/close other views, etc). PURE output — produced by
/// [`ViewInstance::bind`], owns no borrow of app globals.
pub struct BoundView {
    pub instance: InstanceId,
    pub view_id: ViewId,
    pub policy: HostPolicy,
    pub hit: HitModel,
    pub drawable: Box<dyn Drawable>,
    pub fidelity: FidelityReport,
    pub commands: Vec<ViewCommand>,
}

impl BoundView {
    /// Passive overlay plan for the current hover, if the drawable exposes one.
    /// This is deliberately not part of [`HitModel`]: tooltips never consume input.
    pub fn tooltip_plan(&self) -> Option<&widgets::TooltipPlan> {
        self.drawable.tooltip_plan()
    }
}

/// How the host hit-tests a bound view. Most views are a laid-out `Vec<Node>` scene
/// graph (front-to-back `hit_test`); a view may ALSO declare a map-blocking rect (a
/// pure-container panel whose empty interior must consume clicks, not re-pick the map —
/// the Codex §3 guard) and/or be modal (captures everywhere while open).
pub struct HitModel {
    /// The laid-out scene-graph roots to hit-test (front-to-back). May be empty for a
    /// pure custom overlay that uses only `block_rect` / `modal`.
    pub roots: Vec<Node>,
    /// An explicit map-blocking rect: a click inside it that hits no capturing leaf is
    /// CONSUMED (not a fall-through to the map). `None` = no blocking (e.g. the topbar).
    pub block_rect: Option<Rect>,
}

impl HitModel {
    pub fn from_roots(roots: Vec<Node>) -> HitModel {
        HitModel {
            roots,
            block_rect: None,
        }
    }
    pub fn with_block_rect(mut self, rect: Rect) -> HitModel {
        self.block_rect = Some(rect);
        self
    }
    /// An empty hit model (a pure custom overlay relies on `modal`/`block_rect`).
    pub fn none() -> HitModel {
        HitModel {
            roots: Vec::new(),
            block_rect: None,
        }
    }
    /// Front-to-back hit-test this view's roots for a captured leaf.
    pub fn hit_test(&self, px: f32, py: f32) -> Option<Hit> {
        for root in self.roots.iter().rev() {
            if let Some(h) = gui_runtime::hit_test(root, px, py) {
                return Some(h);
            }
        }
        None
    }
}

/// A command a bound view asks the host to run AFTER the frame's input is resolved
/// (so a builder stays pure — it never reaches in and mutates the host). The set is
/// OPEN-ended via `OpenView`/`CloseView` keyed by [`ViewId`]; routing is data, not a
/// hardcoded match. (Existing drill semantics map onto these.)
#[derive(Debug, Clone, PartialEq)]
pub enum ViewCommand {
    /// Open (or focus) the view with this id, optionally carrying an integer argument
    /// (e.g. a state id for the state panel) the factory interprets.
    OpenView { view_id: ViewId, arg: Option<i64> },
    /// Recenter the live native map camera on a raster/world coordinate. This is a
    /// read-only replay navigation command; the app shell applies it only when the
    /// seeded `core.map` view is live.
    RecenterMap { raster_x: f32, raster_y: f32 },
    /// Switch the seeded native `core.map` instance to an existing render map mode.
    /// The generic host does not own that seeded map state; the app shell applies it.
    SetMapMode { mode: MapMode },
    /// Close the view instance that emitted this (or, by id, a named singleton).
    CloseSelf,
    /// Close a named singleton view by id.
    CloseView { view_id: ViewId },
}

/// A self-rendering bound output. The view module owns its draw: a plain GUI window
/// produces a [`NodeDrawable`]; the focus tree / combat overlay produce their own.
/// The host calls `draw` for every open view in z-order with ZERO per-view branches
/// (Codex (c)). `draw` takes the immediate-mode [`FrameDraw`] (renderer + caches +
/// per-frame render context) — matching this app's accumulate-quads compose pass.
pub trait Drawable {
    fn draw(&self, fd: &mut FrameDraw);

    /// Passive tooltip overlay emitted by this drawable. The host draws it last and
    /// includes it in the deterministic digest, but never hit-tests it.
    fn tooltip_plan(&self) -> Option<&widgets::TooltipPlan> {
        None
    }

    /// A stable digest of the DRAW-DETERMINING DATA this drawable carries — the pixel
    /// inputs `bind` computed (Codex (c)): for a custom GPU overlay (map/focus/combat),
    /// the scene-graph digest is empty, so THIS is what makes the surface byte-
    /// comparable headlessly. The map returns its LUT-bytes hash + camera uniforms +
    /// badge; combat returns its bubble positions + open-panel id + close rect; focus
    /// returns its placed-node positions + camera. A plain `NodeDrawable` returns "" (it
    /// is fully covered by the node digest). Two drawables produce the same pixels iff
    /// their node digest AND this digest match (modulo the GPU itself, which is fixed).
    fn draw_digest(&self) -> String {
        String::new()
    }
}

/// A persistent, input-driven view instance living in the window host. It owns its UI
/// state (scroll/drag/camera/hover) across ticks; `bind` reads that state but does not
/// mutate it (so re-binding the same hour is byte-identical).
pub trait ViewInstance {
    /// The registered id of this view (its descriptor key).
    fn view_id(&self) -> ViewId;

    /// The host policy (z-band / modal / rebind). Usually a const from the descriptor.
    fn policy(&self) -> HostPolicy;

    /// Feed a host input event; MAY mutate ONLY this instance's UI state (scroll/drag/
    /// camera/hover) and returns any commands. Returns `false` if the event was not
    /// consumed (the host may route it elsewhere). Default: consume nothing.
    fn on_input(&mut self, _ev: &crate::host::input::ViewInput) -> InputOutcome {
        InputOutcome::ignored()
    }

    /// REBIND the instance's HOUR-DEPENDENT derived state from the new snapshot (the
    /// per-scrub-tick refresh). Called by the host on EVERY open view each time the
    /// timeline cursor moves, BEFORE `bind`. The default is a no-op (a plain GUI window
    /// derives everything inside `bind`); a custom overlay (focus tree / combat) that
    /// caches a placed-graph / aggregated view-models recomputes them here.
    ///
    /// CRITICAL (Codex (f)): rebinding the SAME hour with the SAME persistent UI state
    /// (camera/hover) MUST reproduce the SAME derived state — it is a pure function of
    /// (snapshot, persistent-state). It must NOT fold in transient per-frame inputs, or
    /// scrub A→B→A would differ. `refit` is false on a same-tree scrub so the user's
    /// pan/zoom is preserved.
    fn rebind(&mut self, _ctx: &RebindContext) {}

    /// PURE bind for the current frame: read the immutable snapshot + this instance's
    /// UI state, return a fresh [`BoundView`]. MUST NOT mutate instance state (Codex
    /// (f): a hover/camera mutation here would break scrub byte identity).
    fn bind(&self, ctx: &BindContext) -> BoundView;

    /// Downcast support so the binary can reach a concrete view's owned state (the map
    /// CAMERA, the combat shell) without a per-view enum. Default impls are provided by
    /// the [`impl_view_any!`] macro; a view that owns mutable state the shell drives
    /// implements them (one line each).
    fn as_any(&self) -> &dyn std::any::Any;
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

/// One-line `as_any`/`as_any_mut` for a `ViewInstance` so the binary can downcast it.
#[macro_export]
macro_rules! impl_view_any {
    () => {
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
            self
        }
    };
}

/// The result of feeding an input event to a view instance.
pub struct InputOutcome {
    /// True iff the view consumed the event (the host stops routing it).
    pub consumed: bool,
    /// Commands the view wants the host to run.
    pub commands: Vec<ViewCommand>,
    /// True iff this input changed something that needs a redraw.
    pub needs_redraw: bool,
}

impl InputOutcome {
    pub fn ignored() -> InputOutcome {
        InputOutcome {
            consumed: false,
            commands: Vec::new(),
            needs_redraw: false,
        }
    }
    pub fn consumed() -> InputOutcome {
        InputOutcome {
            consumed: true,
            commands: Vec::new(),
            needs_redraw: true,
        }
    }
    pub fn with_command(mut self, c: ViewCommand) -> InputOutcome {
        self.commands.push(c);
        self
    }
}

/// The default [`Drawable`] for a plain GUI window: a laid-out `Vec<Node>` scene-graph
/// drawn front-to-back through `gui_runtime::draw_window`. Views that need extra,
/// non-`.gui` draws (a flag sprite, a gridbox, occupation bars) compose a SMALL custom
/// drawable that calls into their own module — never a branch in `compose.rs`.
pub struct NodeDrawable {
    pub roots: Vec<Node>,
}

impl Drawable for NodeDrawable {
    fn draw(&self, fd: &mut FrameDraw) {
        for root in &self.roots {
            fd.draw_nodes(std::slice::from_ref(root));
        }
    }
}
