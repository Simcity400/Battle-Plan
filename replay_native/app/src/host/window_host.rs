//! BATCH 0 deliverable 2 — the generic WINDOW / MODAL HOST.
//!
//! Owns the open-view stack, z-order, the modal stack, hit-test routing, scroll/drag
//! capture, and open/close routing — all OUT of `main.rs`. The host iterates
//! REGISTERED open view instances with ZERO per-view branches: it `bind`s each, hit-
//! tests each, draws each in z-order, and routes input top-down. There is no closed
//! `WindowKind`/`DrillAction` enum: routing is `ViewCommand` data over `ViewId`.
//!
//! Hit order (front-to-back, the correct compositing order, Codex §3): modal overlays
//! first (a modal captures everywhere), then normal windows top-down (a captured leaf,
//! else a `block_rect` consume), else MISS → the app falls through to the map layer.

use std::collections::HashMap;

use crate::host::context::BindContext;
use crate::host::draw::FrameDraw;
use crate::host::input::ViewInput;
use crate::host::registry::{ViewId, ViewRegistry};
use crate::host::view::{BoundView, HostPolicy, InstanceId, ViewCommand, ViewInstance, ZBand};

/// One open view in the host: its instance + a stable instance id + open order (for
/// within-band stacking) + whether it is a SINGLETON (one instance per view id, the
/// usual case — opening it again focuses the existing one).
struct OpenView {
    instance_id: InstanceId,
    view: Box<dyn ViewInstance>,
    open_seq: u64,
}

/// The result of resolving a click against the host's open views (front-to-back).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostHit {
    /// A view captured the click: its instance + the leaf control name (if any).
    Captured {
        instance_id: InstanceId,
        view_id: ViewId,
        leaf: Option<String>,
    },
    /// A click consumed by a view's map-blocking rect (no leaf) — NOT a map re-pick.
    Blocked {
        instance_id: InstanceId,
        view_id: ViewId,
    },
    /// No view captured/blocked — the app falls through to the MAP layer.
    Miss,
}

/// The generic window host.
pub struct WindowHost {
    open: Vec<OpenView>,
    next_instance: u64,
    next_seq: u64,
    /// instance id of the view currently capturing pointer input (a drag), if any.
    pointer_capture: Option<InstanceId>,
}

impl Default for WindowHost {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowHost {
    pub fn new() -> WindowHost {
        WindowHost {
            open: Vec::new(),
            next_instance: 1,
            next_seq: 1,
            pointer_capture: None,
        }
    }

    pub fn open_count(&self) -> usize {
        self.open.len()
    }
    pub fn is_open(&self, view_id: ViewId) -> bool {
        self.open.iter().any(|o| o.view.view_id() == view_id)
    }

    /// Open a view from a built instance. SINGLETON by view id: opening an already-open
    /// view replaces it (fresh instance) and moves it to the top of its band. Returns
    /// the new instance id.
    pub fn open_instance(&mut self, view: Box<dyn ViewInstance>) -> InstanceId {
        let view_id = view.view_id();
        // singleton: drop any existing instance of this view id.
        self.open.retain(|o| o.view.view_id() != view_id);
        let instance_id = InstanceId(self.next_instance);
        self.next_instance += 1;
        let open_seq = self.next_seq;
        self.next_seq += 1;
        self.open.push(OpenView {
            instance_id,
            view,
            open_seq,
        });
        self.sort_z();
        instance_id
    }

    /// Open a view through the registry by id (the menubar / command path).
    pub fn open_view(
        &mut self,
        registry: &ViewRegistry,
        id: ViewId,
        arg: Option<i64>,
    ) -> Option<InstanceId> {
        let inst = registry.open(id, arg)?;
        Some(self.open_instance(inst))
    }

    /// Close a view by id (a singleton). No-op if not open.
    pub fn close_view(&mut self, id: ViewId) {
        self.open.retain(|o| o.view.view_id() != id);
        if let Some(cap) = self.pointer_capture {
            if !self.open.iter().any(|o| o.instance_id == cap) {
                self.pointer_capture = None;
            }
        }
    }

    /// Close a view by instance id.
    pub fn close_instance(&mut self, iid: InstanceId) {
        self.open.retain(|o| o.instance_id != iid);
        if self.pointer_capture == Some(iid) {
            self.pointer_capture = None;
        }
    }

    /// Re-sort the open stack by z-band ascending, then open order within the band —
    /// so the draw order is back-to-front and the hit order (reversed) is front-to-back.
    fn sort_z(&mut self) {
        self.open.sort_by(|a, b| {
            let za = a.view.policy().z_band as i32;
            let zb = b.view.policy().z_band as i32;
            za.cmp(&zb).then(a.open_seq.cmp(&b.open_seq))
        });
    }

    /// Is ANY open view modal (captures all input)? Returns the front-most modal's id.
    fn front_modal(&self) -> Option<InstanceId> {
        self.open
            .iter()
            .rev()
            .find(|o| o.view.policy().modal)
            .map(|o| o.instance_id)
    }

    /// REBIND every open view's hour-dependent derived state from the new snapshot
    /// (deliverable 7): called once per scrub-tick on EVERY open registered window,
    /// BEFORE `bind_all`. A plain window's default `rebind` is a no-op; a custom
    /// overlay recomputes its placed-graph / view-models (idempotent for the hour).
    pub fn rebind_all(&mut self, rc: &crate::host::view::RebindContext) {
        for o in self.open.iter_mut() {
            o.view.rebind(rc);
        }
    }

    /// BIND every open view for the current frame (deliverable 7 / Codex (f)): each
    /// `bind` is pure + reads only the immutable snapshot + the instance's own UI
    /// state, so re-binding the same hour reproduces the same `BoundView`. Returns the
    /// bound views in BACK-TO-FRONT draw order. The returned `BoundView`s carry the
    /// CORRECT instance ids (stamped here from the open stack).
    pub fn bind_all(&self, ctx: &BindContext) -> Vec<BoundView> {
        self.open
            .iter()
            .map(|o| {
                let mut bv = o.view.bind(ctx);
                bv.instance = o.instance_id;
                bv
            })
            .collect()
    }

    /// DRAW the already-bound views in z-order (back to front). ZERO per-view branches:
    /// each bound view self-draws via its `Drawable`. Within a band, modal overlays are
    /// drawn last so they sit above their peers.
    pub fn draw_all(&self, bound: &[BoundView], fd: &mut FrameDraw) {
        for bv in bound {
            bv.drawable.draw(fd);
        }
    }

    /// Resolve a click against the open views, front-to-back. A modal overlay (if any)
    /// is consulted first and — being modal — ALWAYS captures (nothing below it nor the
    /// map sees the click). Otherwise normal windows are walked top-down: a captured
    /// leaf wins; else a `block_rect` consume; else fall through to the next window /
    /// finally a MISS (the app then does a map pick).
    ///
    /// `bound` is THIS frame's bound views (their hit models). The host indexes them by
    /// instance id so the hit model matches what's on screen.
    pub fn resolve_click(&self, bound: &[BoundView], px: f32, py: f32) -> HostHit {
        let by_iid: HashMap<InstanceId, &BoundView> =
            bound.iter().map(|b| (b.instance, b)).collect();

        // 1) a modal overlay captures everything (front-most modal first).
        if let Some(modal_iid) = self.front_modal() {
            if let Some(bv) = by_iid.get(&modal_iid) {
                if let Some(hit) = bv.hit.hit_test(px, py) {
                    return HostHit::Captured {
                        instance_id: modal_iid,
                        view_id: bv.view_id,
                        leaf: hit.leaf().map(|s| s.to_string()),
                    };
                }
                // modal with no leaf under the cursor -> still consumes (blocked).
                return HostHit::Blocked {
                    instance_id: modal_iid,
                    view_id: bv.view_id,
                };
            }
        }

        // 2) normal windows, top-down (front-most first).
        for o in self.open.iter().rev() {
            let bv = match by_iid.get(&o.instance_id) {
                Some(b) => b,
                None => continue,
            };
            if let Some(hit) = bv.hit.hit_test(px, py) {
                return HostHit::Captured {
                    instance_id: o.instance_id,
                    view_id: bv.view_id,
                    leaf: hit.leaf().map(|s| s.to_string()),
                };
            }
            if let Some(rect) = bv.hit.block_rect {
                if rect.contains(px, py) {
                    return HostHit::Blocked {
                        instance_id: o.instance_id,
                        view_id: bv.view_id,
                    };
                }
            }
        }
        HostHit::Miss
    }

    /// Route an input event to the open views (front-to-back), letting a view CONSUME
    /// it (and capture the pointer for a drag). Returns the accumulated commands +
    /// whether a redraw is needed. A modal view sees the event first.
    pub fn route_input(&mut self, ev: &ViewInput) -> RouteResult {
        let mut result = RouteResult::default();

        // Honour an active pointer capture, but ONLY for the DRAG STREAM that the
        // capture owns: the `PointerMove`/`PointerUp` events that follow the
        // `PointerDown` which grabbed the capture. A non-drag event (a fresh
        // `PointerDown`, a `Wheel`, or a `Key` such as Escape) is NOT diverted to the
        // capture owner — it routes normally (modal-first, then top-down), so a modal
        // opened mid-drag still sees a subsequent Escape/click before the captured
        // lower view (Codex H2-adjacent). The drag stream stays with its owner whether
        // or not the owner consumes a given move/up — it is NOT re-dispatched to the
        // same instance (nor lower views) by the top-down loop below, which would be a
        // double-dispatch (Codex H3).
        if is_drag_stream(ev) {
            if let Some(cap) = self.pointer_capture {
                if let Some(o) = self.open.iter_mut().find(|o| o.instance_id == cap) {
                    let out = o.view.on_input(ev);
                    result.needs_redraw |= out.needs_redraw;
                    result.commands.extend(out.commands);
                    if let ViewInput::PointerUp { .. } = ev {
                        self.pointer_capture = None;
                    }
                    // The drag-stream event belongs to the capture owner; do not fall
                    // through to the normal loop (no double dispatch). `consumed`
                    // mirrors the owner's answer for the caller's bookkeeping.
                    result.consumed = out.consumed;
                    result.emitter = Some(cap);
                    return result;
                } else {
                    // the capturing instance was closed -> drop the stale capture.
                    self.pointer_capture = None;
                }
            }
        }

        // modal-first, else top-down.
        let order: Vec<usize> = {
            let mut idx: Vec<usize> = (0..self.open.len()).collect();
            // front-to-back = reverse of the z-sorted stack.
            idx.reverse();
            idx
        };
        for i in order {
            let modal = self.open[i].view.policy().modal;
            let out = self.open[i].view.on_input(ev);
            result.needs_redraw |= out.needs_redraw;
            result.commands.extend(out.commands);
            if out.consumed {
                // a pointer-down that a view consumed grabs the drag capture.
                if let ViewInput::PointerDown { .. } = ev {
                    self.pointer_capture = Some(self.open[i].instance_id);
                }
                result.consumed = true;
                result.emitter = Some(self.open[i].instance_id);
                return result;
            }
            // a modal view blocks routing to lower views even if it didn't consume.
            if modal {
                result.consumed = true;
                result.emitter = Some(self.open[i].instance_id);
                return result;
            }
        }
        result
    }

    /// Execute a batch of [`ViewCommand`]s emitted by bound views / input, opening or
    /// closing registered views (the generalized drill open/close, deliverable 2).
    /// `emitter` is the instance that produced `CloseSelf`.
    pub fn execute_commands(
        &mut self,
        registry: &ViewRegistry,
        emitter: Option<InstanceId>,
        commands: impl IntoIterator<Item = ViewCommand>,
    ) -> bool {
        let mut changed = false;
        for cmd in commands {
            match cmd {
                ViewCommand::OpenView { view_id, arg } => {
                    if self.open_view(registry, view_id, arg).is_some() {
                        changed = true;
                    }
                }
                ViewCommand::CloseSelf => {
                    if let Some(iid) = emitter {
                        self.close_instance(iid);
                        changed = true;
                    }
                }
                ViewCommand::CloseView { view_id } => {
                    if self.is_open(view_id) {
                        self.close_view(view_id);
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    /// The front-most (highest-z, latest-open) view id, if any (for ESC "close the
    /// top-most overlay" semantics in the app).
    pub fn front_view(&self) -> Option<ViewId> {
        self.open.last().map(|o| o.view.view_id())
    }

    /// The front-most CLOSEABLE overlay/panel (anything above `Chrome`), for ESC.
    pub fn front_closeable(&self) -> Option<ViewId> {
        self.open
            .iter()
            .rev()
            .find(|o| o.view.policy().z_band > ZBand::Chrome)
            .map(|o| o.view.view_id())
    }

    /// Iterate open view ids in z-order (back to front) — for diagnostics / the scrub
    /// loop's "rebind every open window".
    pub fn open_view_ids(&self) -> impl Iterator<Item = ViewId> + '_ {
        self.open.iter().map(|o| o.view.view_id())
    }

    /// Mutable access to an open instance's view (e.g. to resize its camera), by id.
    pub fn instance_mut(&mut self, view_id: ViewId) -> Option<&mut (dyn ViewInstance + 'static)> {
        self.open
            .iter_mut()
            .find(|o| o.view.view_id() == view_id)
            .map(|o| o.view.as_mut())
    }

    /// Downcast the open view with `view_id` to a concrete type `T` (immutable). Lets
    /// the binary reach a view's owned state (the map camera, the combat shell) with NO
    /// per-view enum — the only place concrete view types are named is the binary glue.
    pub fn downcast_ref<T: 'static>(&self, view_id: ViewId) -> Option<&T> {
        self.open
            .iter()
            .find(|o| o.view.view_id() == view_id)
            .and_then(|o| o.view.as_any().downcast_ref::<T>())
    }

    /// Downcast the open view with `view_id` to a concrete type `T` (mutable).
    pub fn downcast_mut<T: 'static>(&mut self, view_id: ViewId) -> Option<&mut T> {
        self.open
            .iter_mut()
            .find(|o| o.view.view_id() == view_id)
            .and_then(|o| o.view.as_any_mut().downcast_mut::<T>())
    }
}

/// True iff `ev` is part of the DRAG STREAM a pointer capture owns: a `PointerMove`
/// (the drag itself) or the `PointerUp` that ends it. A fresh `PointerDown`, a
/// `Wheel`, or a `Key` is NOT part of the captured drag and routes normally.
fn is_drag_stream(ev: &ViewInput) -> bool {
    matches!(
        ev,
        ViewInput::PointerMove { .. } | ViewInput::PointerUp { .. }
    )
}

/// What `route_input` resolved.
#[derive(Default)]
pub struct RouteResult {
    pub consumed: bool,
    pub needs_redraw: bool,
    pub commands: Vec<ViewCommand>,
    /// The instance that consumed the event (the emitter for a `CloseSelf` command).
    pub emitter: Option<InstanceId>,
}

#[allow(unused_imports)]
use HostPolicy as _HostPolicyUsed;
