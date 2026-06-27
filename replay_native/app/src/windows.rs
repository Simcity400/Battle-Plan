//! The WINDOW STACK + front-to-back hit resolution (Stage 1 deliverable 4).
//!
//! The GUI is a STACK of windows over the map. The topbar is always present at the
//! BOTTOM of the GUI stack; the state panel (and any future panel) pushes ON TOP.
//! Hit resolution walks the stack TOP-DOWN (`gui_runtime::hit_test_stack` already
//! iterates `.rev()`), so the front-most window that captures wins. A window that is
//! a pure coordinate frame (the full-screen topbar root) passes clicks through its
//! empty regions — its real controls still capture (`Node::is_hit_capable`), and a
//! panel with a real background captures everywhere inside it (blocking the map).
//!
//! Critically, this layer answers ONE question per click: did the GUI capture, and
//! if so which window + leaf? If the GUI did NOT capture, the app falls through to
//! the MAP layer (province pick). That "GUI first, map on miss" order is the correct
//! front-to-back compositing order (Codex §3): the map is the bottom layer, so it is
//! only reached when nothing above it captured.

use gui_runtime::scene::Rect;
use gui_runtime::{hit_test, scene::Node, Hit};

/// A logical window in the stack, with a stable KIND the drill registry keys on.
pub struct StackWindow {
    /// A stable identifier for the window (e.g. "topbar", "state_panel").
    pub id: String,
    /// The window's KIND, used by the drill registry to route a hit (see
    /// [`crate::drill`]). The topbar is `Topbar`; a state panel is `StatePanel`.
    pub kind: WindowKind,
    /// The laid-out root nodes of this window (one .gui can yield several roots).
    pub roots: Vec<Node>,
    /// Whether the window is currently open/visible (closed windows are skipped).
    pub open: bool,
    /// An explicit MAP-BLOCKING bounding rect. A click inside this rect that does NOT
    /// hit a capturing leaf is still CONSUMED (returns [`GuiHit::Blocked`]) rather
    /// than falling through to the map. This guards the Codex §3 trap: a the game panel
    /// root is often a pure `containerWindowType` (no background that captures), so
    /// without an explicit block rect a click on the panel's empty interior would
    /// re-pick a province BEHIND the panel. `None` = the window does not block the map
    /// (e.g. the full-screen topbar, whose empty regions MUST fall through).
    pub block_rect: Option<Rect>,
}

/// The kind of a stacked window (drives drill routing + close behaviour).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowKind {
    /// The always-on topbar (never closes in Stage 1).
    Topbar,
    /// A state panel opened by a province pick (closes via its control).
    StatePanel,
}

/// The result of resolving a click against the GUI stack.
#[derive(Debug, Clone)]
pub enum GuiHit {
    /// The GUI captured the click: this window (by id + kind) and the hit leaf path.
    Captured {
        window_id: String,
        window_kind: WindowKind,
        hit: Hit,
    },
    /// The click landed inside an open window's MAP-BLOCKING rect but hit no capturing
    /// leaf (e.g. the panel's empty interior). It is CONSUMED — the app does nothing
    /// and must NOT fall through to the map (no province re-pick behind the panel).
    Blocked {
        window_id: String,
        window_kind: WindowKind,
    },
    /// No GUI window captured or blocked — the app falls through to the MAP layer.
    Miss,
}

/// The window stack: bottom-to-top order. The topbar is element 0 (bottom); panels
/// are pushed at the end (top). Hit resolution walks top-down.
#[derive(Default)]
pub struct WindowStack {
    windows: Vec<StackWindow>,
}

impl WindowStack {
    pub fn new() -> WindowStack {
        WindowStack {
            windows: Vec::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Push a window onto the TOP of the stack (front-most).
    pub fn push(&mut self, w: StackWindow) {
        self.windows.push(w);
    }

    /// Iterate windows BOTTOM-to-TOP (the app's draw order — back to front).
    pub fn iter_draw_order(&self) -> impl Iterator<Item = &StackWindow> {
        self.windows.iter().filter(|w| w.open)
    }

    /// Mutable access to a window by id (e.g. to re-bind its roots on a scrub tick).
    pub fn get_mut(&mut self, id: &str) -> Option<&mut StackWindow> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// True if a window with this id is present AND open.
    pub fn is_open(&self, id: &str) -> bool {
        self.windows.iter().any(|w| w.id == id && w.open)
    }

    /// Open (or replace) a window: if an id already exists, replace its roots/kind and
    /// mark it open + move it to the TOP; otherwise push it on top.
    pub fn open_or_replace(&mut self, w: StackWindow) {
        if let Some(pos) = self.windows.iter().position(|x| x.id == w.id) {
            self.windows.remove(pos);
        }
        self.windows.push(w);
    }

    /// Close (remove) a window by id. The topbar should never be closed (the app
    /// only ever calls this for panels), but this is permissive.
    pub fn close(&mut self, id: &str) {
        self.windows.retain(|w| w.id != id);
    }

    /// Resolve a click against the stack, TOP-DOWN. Returns the first window whose
    /// tree captures the point (front-to-back), or [`GuiHit::Miss`] if none do (the
    /// app then falls through to the map). A closed window is skipped.
    pub fn resolve(&self, px: f32, py: f32) -> GuiHit {
        // top-down: last pushed is front-most.
        for w in self.windows.iter().rev() {
            if !w.open {
                continue;
            }
            for root in w.roots.iter().rev() {
                if let Some(hit) = hit_test(root, px, py) {
                    return GuiHit::Captured {
                        window_id: w.id.clone(),
                        window_kind: w.kind,
                        hit,
                    };
                }
            }
            // No leaf captured in this window — but if the click is inside its
            // map-blocking rect, CONSUME it here (don't fall through to a lower window
            // or the map). This is the explicit guard for a pure-container panel root.
            if let Some(rect) = w.block_rect {
                if rect.contains(px, py) {
                    return GuiHit::Blocked {
                        window_id: w.id.clone(),
                        window_kind: w.kind,
                    };
                }
            }
        }
        GuiHit::Miss
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gui_runtime::scene::{Anchor, ElementKind, Rect};

    fn node(name: &str, kind: ElementKind, rect: Rect, transparent: bool) -> Node {
        Node {
            name: name.into(),
            raw_kind: "containerWindowType".into(),
            kind,
            pos: (0.0, 0.0),
            size_w: None,
            size_h: None,
            orientation: Anchor::UpperLeft,
            origo: Anchor::UpperLeft,
            clipping: false,
            alwaystransparent: transparent,
            rect,
            clip: None,
            children: vec![],
        }
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    /// A full-screen TOPBAR pure container (no background) with a real button at the
    /// top, over a MAP layer. A click on the button captures; a click on the empty
    /// region falls THROUGH (Miss → map pick). This is the deliverable-4 / Codex-§3
    /// "pure container passes through, control captures" case.
    #[test]
    fn topbar_control_captures_empty_falls_through() {
        let mut topbar_root = node(
            "topbar",
            ElementKind::Container { background: None },
            rect(0.0, 0.0, 1920.0, 1080.0),
            false,
        );
        topbar_root.children.push(node(
            "settings_button",
            ElementKind::Button {
                sprite: Some("s".into()),
                frame: 0,
                text: None,
            },
            rect(1800.0, 0.0, 64.0, 40.0),
            false,
        ));
        let mut stack = WindowStack::new();
        stack.push(StackWindow {
            id: "topbar".into(),
            kind: WindowKind::Topbar,
            roots: vec![topbar_root],
            open: true,
            block_rect: None,
        });

        // click the button -> captured by topbar.
        match stack.resolve(1820.0, 20.0) {
            GuiHit::Captured {
                window_kind, hit, ..
            } => {
                assert_eq!(window_kind, WindowKind::Topbar);
                assert_eq!(hit.leaf(), Some("settings_button"));
            }
            other => panic!("the topbar button should capture, got {other:?}"),
        }
        // click empty centre -> Miss (falls through to the map).
        assert!(matches!(stack.resolve(960.0, 540.0), GuiHit::Miss));
    }

    /// A state panel with a REAL background stacked over the topbar: a click inside
    /// the panel is captured by the PANEL (front-most), and it BLOCKS the map (the
    /// panel background captures, doesn't leak). A click outside the panel but on a
    /// topbar control still routes to the topbar; a click on empty space misses.
    #[test]
    fn panel_over_topbar_front_to_back() {
        // topbar: a control on the right.
        let mut topbar_root = node(
            "topbar",
            ElementKind::Container { background: None },
            rect(0.0, 0.0, 1920.0, 1080.0),
            false,
        );
        topbar_root.children.push(node(
            "settings_button",
            ElementKind::Button {
                sprite: Some("s".into()),
                frame: 0,
                text: None,
            },
            rect(1800.0, 0.0, 64.0, 40.0),
            false,
        ));
        // panel: an OPAQUE background window on the left (it must block the map).
        let panel_root = node(
            "state_panel",
            ElementKind::Container {
                background: Some("panel_bg".into()),
            },
            rect(40.0, 110.0, 480.0, 700.0),
            false,
        );

        let mut stack = WindowStack::new();
        stack.push(StackWindow {
            id: "topbar".into(),
            kind: WindowKind::Topbar,
            roots: vec![topbar_root],
            open: true,
            block_rect: None,
        });
        stack.push(StackWindow {
            id: "state_panel".into(),
            kind: WindowKind::StatePanel,
            roots: vec![panel_root],
            open: true,
            block_rect: None,
        });

        // click inside the panel -> captured by the PANEL (front-most), blocks the map.
        match stack.resolve(200.0, 400.0) {
            GuiHit::Captured { window_kind, .. } => assert_eq!(window_kind, WindowKind::StatePanel),
            other => panic!("the panel background must capture (block the map), got {other:?}"),
        }
        // click the topbar control (outside the panel) -> topbar.
        match stack.resolve(1820.0, 20.0) {
            GuiHit::Captured { window_kind, .. } => assert_eq!(window_kind, WindowKind::Topbar),
            other => panic!("the topbar control should still capture, got {other:?}"),
        }
        // click empty space (not panel, not a control) -> Miss -> map.
        assert!(matches!(stack.resolve(960.0, 1000.0), GuiHit::Miss));
    }

    /// A transparent overlay over a button (the v1 "shine ate the click" bug,
    /// inverted): the click falls through the transparent overlay to the button.
    #[test]
    fn transparent_overlay_passes_to_control_below() {
        let mut root = node(
            "topbar",
            ElementKind::Container { background: None },
            rect(0.0, 0.0, 1920.0, 1080.0),
            false,
        );
        root.children.push(node(
            "real_button",
            ElementKind::Button {
                sprite: Some("s".into()),
                frame: 0,
                text: None,
            },
            rect(100.0, 0.0, 80.0, 40.0),
            false,
        ));
        // a transparent shine drawn AFTER (on top of) the button.
        root.children.push(node(
            "shine",
            ElementKind::Icon {
                sprite: "shine".into(),
                frame: 0,
            },
            rect(100.0, 0.0, 80.0, 40.0),
            true,
        ));
        let mut stack = WindowStack::new();
        stack.push(StackWindow {
            id: "topbar".into(),
            kind: WindowKind::Topbar,
            roots: vec![root],
            open: true,
            block_rect: None,
        });
        match stack.resolve(140.0, 20.0) {
            GuiHit::Captured { hit, .. } => assert_eq!(hit.leaf(), Some("real_button")),
            other => panic!("click should fall through the shine to the button, got {other:?}"),
        }
    }

    /// Closing a panel removes it from resolution: a click that the panel had
    /// captured now misses (falls to the map).
    #[test]
    fn closed_panel_no_longer_captures() {
        let panel_root = node(
            "state_panel",
            ElementKind::Container {
                background: Some("panel_bg".into()),
            },
            rect(40.0, 110.0, 480.0, 700.0),
            false,
        );
        let mut stack = WindowStack::new();
        stack.push(StackWindow {
            id: "state_panel".into(),
            kind: WindowKind::StatePanel,
            roots: vec![panel_root],
            open: true,
            block_rect: None,
        });
        assert!(matches!(
            stack.resolve(200.0, 400.0),
            GuiHit::Captured { .. }
        ));
        stack.close("state_panel");
        assert!(matches!(stack.resolve(200.0, 400.0), GuiHit::Miss));
    }

    /// THE Codex §3 guard: a panel whose root is a PURE container (no capturing
    /// background) but with an explicit `block_rect` must CONSUME a click on its empty
    /// interior (return `Blocked`), NOT let it fall through to the map. A click on its
    /// real close button still captures; a click OUTSIDE the block rect falls through.
    #[test]
    fn pure_container_panel_with_block_rect_consumes_interior_clicks() {
        // a pure container panel root (NO background) + one real button.
        let mut panel_root = node(
            "countrystateview",
            ElementKind::Container { background: None }, // pure container — would NOT capture
            rect(40.0, 110.0, 436.0, 496.0),
            false,
        );
        panel_root.children.push(node(
            "close_button",
            ElementKind::Button {
                sprite: Some("x".into()),
                frame: 0,
                text: None,
            },
            rect(450.0, 116.0, 20.0, 20.0),
            false,
        ));
        let mut stack = WindowStack::new();
        // a topbar UNDER it so a fall-through would be observable as a different result.
        stack.push(StackWindow {
            id: "topbar".into(),
            kind: WindowKind::Topbar,
            roots: vec![node(
                "topbar",
                ElementKind::Container { background: None },
                rect(0.0, 0.0, 1600.0, 900.0),
                false,
            )],
            open: true,
            block_rect: None,
        });
        stack.push(StackWindow {
            id: "state_panel".into(),
            kind: WindowKind::StatePanel,
            roots: vec![panel_root],
            open: true,
            block_rect: Some(rect(40.0, 110.0, 436.0, 496.0)),
        });

        // click the panel's EMPTY interior (no leaf) -> Blocked (consumed, NOT a map pick).
        match stack.resolve(200.0, 300.0) {
            GuiHit::Blocked { window_kind, .. } => assert_eq!(window_kind, WindowKind::StatePanel),
            other => panic!("empty panel interior must be Blocked, got {other:?}"),
        }
        // click the real close button -> Captured.
        match stack.resolve(458.0, 124.0) {
            GuiHit::Captured { hit, .. } => assert_eq!(hit.leaf(), Some("close_button")),
            other => panic!("the close button must capture, got {other:?}"),
        }
        // click OUTSIDE the panel block rect (and not a topbar control) -> Miss -> map.
        assert!(matches!(stack.resolve(900.0, 700.0), GuiHit::Miss));
    }
}
