//! STAGE 3 app-shell integration of the focus-tree adapter.
//!
//! [`FocusShell`] owns the parsed focus CATALOG (loaded once from the resolver),
//! and — when the overlay is OPEN — the placed+bound tree for the observed
//! country at the current hour, plus the pan/zoom camera + hover state. It is
//! re-bound on every scrub tick (a FRESH placed tree per hour; never mutated in
//! place — the no-fabrication immutability rule). All input here is READ-ONLY
//! navigation (pan/zoom/hover); nothing mutates the snapshot or the game.

use assets::resolver::GameResolver;
use focus_tree::{
    bind_node, eval_context, load_catalog, place, Camera, FocusCatalog, FocusDomain, PlacedNode,
    PlacedTree, PosFidelity,
};
use snapshot::Snapshot;

/// The focus-tree overlay's per-frame inputs. The placed tree + bound domain are
/// rebuilt by the shell on each scrub tick; the camera is the shell's persistent input
/// state. The `white` texture is a process-owned solid pixel. (Lives here now that the
/// view module — not `compose` — consumes it.)
pub struct FocusOverlay<'a> {
    pub tree: &'a focus_tree::PlacedTree,
    pub domain: &'a focus_tree::FocusDomain,
    pub camera: focus_tree::Camera,
    pub white: &'a render::Texture,
    pub tag: String,
    pub date: String,
    /// optional id under the cursor (for the hover tooltip line).
    pub hover: Option<String>,
    /// optional user-selected focus id, preserved across scrub until explicitly changed.
    pub selected: Option<String>,
    /// Current factual selected-node detail, or None when the selected id is missing.
    pub selected_detail: Option<FocusSelectedDetail>,
}

/// Where the overlay's graph viewport sits (below the header, above the scrubber).
const HEADER_H: f32 = 56.0;
const FOOTER_H: f32 = 72.0; // leave room for the scrubber + tooltip line

pub struct FocusShell {
    catalog: Option<FocusCatalog>,
    /// open/closed visibility (toggled by the F key).
    pub open: bool,
    /// the placed tree for the current country+hour (None until first bind).
    placed: Option<PlacedTree>,
    domain: FocusDomain,
    camera: Option<Camera>,
    /// the country whose tree we last bound (re-bind on country change).
    bound_tag: Option<String>,
    bound_tree_id: Option<String>,
    date: String,
    hover: Option<String>,
    selected: Option<String>,
    click_candidate: Option<String>,
    drag_moved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusSelectedDetail {
    pub title: String,
    pub rows: Vec<FocusDetailRow>,
}

impl FocusSelectedDetail {
    fn new(title: impl Into<String>) -> FocusSelectedDetail {
        FocusSelectedDetail {
            title: title.into(),
            rows: Vec::new(),
        }
    }

    fn with_row(
        mut self,
        label: &'static str,
        text: impl Into<String>,
        fidelity: FocusDetailFidelity,
        reason: Option<String>,
    ) -> FocusSelectedDetail {
        self.rows.push(FocusDetailRow {
            label,
            text: text.into(),
            fidelity,
            reason,
        });
        self
    }

    pub fn digest(&self) -> String {
        let mut s = format!("title={:?}", self.title);
        for row in &self.rows {
            s.push_str(&format!(
                ";{}=text={:?},fidelity={:?},reason={:?}",
                row.label, row.text, row.fidelity, row.reason
            ));
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusDetailRow {
    pub label: &'static str,
    pub text: String,
    pub fidelity: FocusDetailFidelity,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusDetailFidelity {
    Exact,
    Derived,
    Unavailable,
}

impl FocusShell {
    /// Load the catalog once. A failure leaves the shell present-but-empty (the
    /// overlay just won't open) — never a panic, never fabricated data.
    pub fn load(resolver: &GameResolver) -> FocusShell {
        let catalog = match load_catalog(resolver) {
            Ok(c) => {
                eprintln!(
                    "[focus] catalog: {} trees, {} shared focuses",
                    c.trees.len(),
                    c.shared.len()
                );
                Some(c)
            }
            Err(e) => {
                eprintln!("[focus] WARNING: focus catalog not loaded ({e}); overlay disabled.");
                None
            }
        };
        FocusShell {
            catalog,
            open: false,
            placed: None,
            domain: FocusDomain::default(),
            camera: None,
            bound_tag: None,
            bound_tree_id: None,
            date: String::new(),
            hover: None,
            selected: None,
            click_candidate: None,
            drag_moved: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn has_catalog(&self) -> bool {
        self.catalog.is_some()
    }

    /// Toggle the overlay. On OPEN, (re)bind against the current snapshot/country
    /// and fit the camera to the viewport.
    pub fn toggle(&mut self, snap: &Snapshot, tag: &str, screen: (u32, u32)) {
        self.open = !self.open;
        if self.open {
            self.rebind(snap, tag, screen, true);
        } else {
            self.hover = None;
            self.click_candidate = None;
            self.drag_moved = false;
        }
    }

    fn viewport(screen: (u32, u32)) -> [f32; 4] {
        [
            0.0,
            HEADER_H,
            screen.0 as f32,
            (screen.1 as f32 - HEADER_H - FOOTER_H).max(1.0),
        ]
    }

    /// Rebuild the placed tree + bound domain for `tag` at the snapshot's hour.
    /// Called on toggle-open and on every scrub tick while open. `refit` re-fits
    /// the camera (true on open / country change; false on a same-tree scrub so
    /// the user's pan/zoom is preserved).
    pub fn rebind(&mut self, snap: &Snapshot, tag: &str, screen: (u32, u32), refit: bool) {
        let catalog = match &self.catalog {
            Some(c) => c,
            None => return,
        };
        self.domain = snap
            .country(tag)
            .map(FocusDomain::from_country)
            .unwrap_or_default();
        self.date = snap.date.clone().unwrap_or_default();

        // pick the tree the country's focus domain names; fall back to a
        // tag-derived guess only as a last resort (never fabricates STATE — an
        // unknown tree just renders nothing).
        let tree_id = self
            .domain
            .focus_tree
            .clone()
            .or_else(|| guess_tree_id(catalog, tag));
        let tree_id = match tree_id {
            Some(id) => id,
            None => {
                eprintln!("[focus] no focus tree for {tag}; overlay empty.");
                self.placed = None;
                return;
            }
        };

        let tree = match catalog.tree(&tree_id) {
            Some(t) => t,
            None => {
                eprintln!("[focus] tree `{tree_id}` not in catalog; overlay empty.");
                self.placed = None;
                return;
            }
        };
        let ctx = eval_context(tag, &self.domain, None);
        let placed = place(&tree, &ctx);
        if placed.approximate_count > 0 {
            eprintln!(
                "[focus] {} of {} nodes have APPROXIMATE positions (unsupported offset triggers, badged).",
                placed.approximate_count, placed.nodes.len()
            );
        }

        let viewport = Self::viewport(screen);
        let changed_tree = self.bound_tree_id.as_deref() != Some(tree_id.as_str())
            || self.bound_tag.as_deref() != Some(tag);
        if refit || changed_tree || self.camera.is_none() {
            self.camera = Some(Camera::fit(&placed, viewport, 24.0));
        } else if let Some(cam) = &mut self.camera {
            cam.viewport = viewport; // keep pan/zoom, just track resize
        }
        self.placed = Some(placed);
        self.bound_tag = Some(tag.to_string());
        self.bound_tree_id = Some(tree_id);
    }

    /// Update the viewport on a window resize (keeps pan/zoom).
    pub fn resize(&mut self, screen: (u32, u32)) {
        if let Some(cam) = &mut self.camera {
            cam.viewport = Self::viewport(screen);
        }
    }

    /// Whether the cursor is inside the overlay's interactive region (so the app
    /// routes pan/zoom here instead of to the map). Only true when open.
    pub fn captures(&self, sx: f32, sy: f32) -> bool {
        if !self.open {
            return false;
        }
        match &self.camera {
            Some(c) => {
                let v = c.viewport;
                sx >= v[0] && sx <= v[0] + v[2] && sy >= v[1] && sy <= v[1] + v[3]
            }
            None => self.open, // empty tree still captures (modal)
        }
    }

    /// Pan by a screen-space delta (left-drag inside the overlay). READ-ONLY.
    pub fn pan(&mut self, dx: f32, dy: f32) {
        if let Some(cam) = &mut self.camera {
            cam.pan(dx, dy);
        }
    }

    /// Zoom around the cursor (wheel inside the overlay). READ-ONLY.
    pub fn zoom_at(&mut self, sx: f32, sy: f32, factor: f32) {
        if let Some(cam) = &mut self.camera {
            cam.zoom_at(sx, sy, factor);
        }
    }

    /// Update the hover-highlighted node from the cursor.
    pub fn hover(&mut self, sx: f32, sy: f32) {
        self.hover = self.pick(sx, sy);
    }

    /// Pick a focus node at a screen-space point, if one is under the cursor.
    pub fn pick(&self, sx: f32, sy: f32) -> Option<String> {
        match (&self.camera, &self.placed) {
            (Some(cam), Some(tree)) => focus_tree::pick(cam, tree, sx, sy),
            _ => None,
        }
    }

    /// Start a click candidate. Dragging cancels it; pointer-up must hit the same node.
    pub fn begin_click(&mut self, sx: f32, sy: f32) {
        self.click_candidate = self.pick(sx, sy);
        self.drag_moved = false;
    }

    /// Cancel the pending click because the pointer became a pan drag.
    pub fn cancel_click_for_drag(&mut self) {
        self.drag_moved = true;
        self.click_candidate = None;
    }

    /// Commit a click selection if pointer-down/up hit the same node and no drag happened.
    pub fn commit_click(&mut self, sx: f32, sy: f32) -> bool {
        let candidate = if !self.drag_moved {
            match (self.click_candidate.take(), self.pick(sx, sy)) {
                (Some(down), Some(up)) if down == up => Some(down),
                _ => None,
            }
        } else {
            self.click_candidate = None;
            None
        };
        self.drag_moved = false;
        match candidate {
            Some(id) => {
                let changed = self.selected.as_deref() != Some(id.as_str());
                self.selected = Some(id);
                changed
            }
            None => false,
        }
    }

    pub fn selected_id(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    pub fn selected_detail(&self) -> Option<FocusSelectedDetail> {
        let id = self.selected.as_deref()?;
        let tree = self.placed.as_ref()?;
        let node = tree.node(id)?;
        Some(focus_selected_detail(node, &self.domain))
    }

    /// Build the per-frame overlay descriptor for the compose pass (when open +
    /// bound). `white` is the app-owned solid texture.
    pub fn overlay<'a>(&'a self, white: &'a render::Texture) -> Option<FocusOverlay<'a>> {
        if !self.open {
            return None;
        }
        let (tree, camera) = match (&self.placed, &self.camera) {
            (Some(t), Some(c)) => (t, *c),
            _ => return None,
        };
        Some(FocusOverlay {
            tree,
            domain: &self.domain,
            camera,
            white,
            tag: self.bound_tag.clone().unwrap_or_default(),
            date: self.date.clone(),
            hover: self.hover.clone(),
            selected: self.selected.clone(),
            selected_detail: self.selected_detail(),
        })
    }
}

fn focus_selected_detail(node: &PlacedNode, domain: &FocusDomain) -> FocusSelectedDetail {
    let binding = bind_node(&node.id, &node.prerequisites, domain);
    let (state, state_fidelity) = match binding.state {
        focus_tree::NodeState::Completed => ("completed", FocusDetailFidelity::Exact),
        focus_tree::NodeState::Current => ("in progress", FocusDetailFidelity::Exact),
        focus_tree::NodeState::PrereqsMet => ("prereqs met", FocusDetailFidelity::Derived),
        focus_tree::NodeState::PrereqsUnmet => ("prereqs unmet", FocusDetailFidelity::Derived),
    };
    let mut detail = FocusSelectedDetail::new(node.id.clone())
        .with_row("ID", node.id.clone(), FocusDetailFidelity::Exact, None)
        .with_row("State", state, state_fidelity, None);
    detail = match node.cost {
        Some(cost) => detail.with_row(
            "Cost",
            format_focus_number(cost),
            FocusDetailFidelity::Exact,
            None,
        ),
        None => detail.with_row(
            "Cost",
            "—",
            FocusDetailFidelity::Unavailable,
            Some("absent".to_string()),
        ),
    };
    if binding.state == focus_tree::NodeState::Current {
        detail = match binding.progress_days {
            Some(days) => detail.with_row(
                "Progress days",
                days.to_string(),
                FocusDetailFidelity::Exact,
                None,
            ),
            None => detail.with_row(
                "Progress days",
                "—",
                FocusDetailFidelity::Unavailable,
                Some("absent".to_string()),
            ),
        };
    }
    detail = detail.with_row(
        "Prereq groups",
        node.prerequisites.len().to_string(),
        FocusDetailFidelity::Exact,
        None,
    );
    match &node.fidelity {
        PosFidelity::Exact => {
            detail.with_row("Position", "exact", FocusDetailFidelity::Exact, None)
        }
        PosFidelity::Approximate { reason } => detail.with_row(
            "Position",
            "approximate",
            FocusDetailFidelity::Derived,
            Some(reason.clone()),
        ),
    }
}

fn format_focus_number(n: f32) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

/// Best-effort tree-id guess from a tag when the snapshot doesn't carry
/// `focus.focus_tree` (e.g. an early frame). Returns a catalog tree whose id
/// starts with the lowercased tag, else None. This only picks WHICH tree to
/// show, never the node STATE (which stays empty/available without a domain).
fn guess_tree_id(catalog: &FocusCatalog, tag: &str) -> Option<String> {
    let lt = tag.to_lowercase();
    catalog
        .trees
        .iter()
        .find(|t| t.id.starts_with(&lt))
        .map(|t| t.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use focus_tree::{FocusNode, FocusTree};
    use serde_json::json;

    fn focus_node(
        id: &str,
        y: i32,
        relative_to: Option<&str>,
        cost: Option<f32>,
        prereqs: Vec<focus_tree::PrereqGroup>,
    ) -> FocusNode {
        FocusNode {
            id: id.to_string(),
            icon: Some(format!("GFX_{id}")),
            base_x: 0,
            base_y: y,
            relative_to: relative_to.map(str::to_string),
            cost,
            offsets: Vec::new(),
            prerequisites: prereqs,
            mutually_exclusive: Vec::new(),
        }
    }

    fn test_tree() -> FocusTree {
        FocusTree {
            id: "german_focus".to_string(),
            nodes: vec![
                focus_node("GER_a", 0, None, Some(10.0), Vec::new()),
                focus_node(
                    "GER_b",
                    1,
                    Some("GER_a"),
                    Some(7.0),
                    vec![focus_tree::PrereqGroup {
                        any_of: vec!["GER_a".to_string()],
                    }],
                ),
                focus_node(
                    "GER_c",
                    1,
                    Some("GER_b"),
                    None,
                    vec![focus_tree::PrereqGroup {
                        any_of: vec!["GER_b".to_string()],
                    }],
                ),
            ],
        }
    }

    fn shell() -> FocusShell {
        FocusShell {
            catalog: Some(FocusCatalog {
                trees: vec![test_tree()],
                shared: Vec::new(),
            }),
            open: true,
            placed: None,
            domain: FocusDomain::default(),
            camera: None,
            bound_tag: None,
            bound_tree_id: None,
            date: String::new(),
            hover: None,
            selected: None,
            click_candidate: None,
            drag_moved: false,
        }
    }

    fn snap_a() -> Snapshot {
        serde_json::from_value(json!({
            "date": "1936.1.1.12",
            "countries": {
                "GER": { "focus": {
                    "focus_tree": "german_focus",
                    "completed": ["GER_a"],
                    "current": "GER_b",
                    "progress": 12
                }}
            }
        }))
        .unwrap()
    }

    fn snap_b() -> Snapshot {
        serde_json::from_value(json!({
            "date": "1936.2.1.12",
            "countries": {
                "GER": { "focus": {
                    "focus_tree": "german_focus",
                    "completed": ["GER_a", "GER_b"],
                    "current": "GER_c",
                    "progress": 20
                }}
            }
        }))
        .unwrap()
    }

    fn snap_current_without_progress() -> Snapshot {
        serde_json::from_value(json!({
            "date": "1936.3.1.12",
            "countries": {
                "GER": { "focus": {
                    "focus_tree": "german_focus",
                    "completed": ["GER_a", "GER_b"],
                    "current": "GER_c"
                }}
            }
        }))
        .unwrap()
    }

    fn node_center(shell: &FocusShell, id: &str) -> (f32, f32) {
        let tree = shell.placed.as_ref().expect("placed tree");
        let node = tree.node(id).expect("placed node");
        let cam = shell.camera.expect("camera");
        cam.to_screen(
            node.px + focus_tree::view::NODE_W / 2.0,
            node.py + focus_tree::view::NODE_H / 2.0,
        )
    }

    fn click(shell: &mut FocusShell, id: &str) -> bool {
        let (x, y) = node_center(shell, id);
        shell.begin_click(x, y);
        shell.commit_click(x, y)
    }

    #[test]
    fn focus_shell_click_selection_populates_factual_detail() {
        let mut shell = shell();
        shell.rebind(&snap_a(), "GER", (1600, 900), true);

        assert!(click(&mut shell, "GER_b"));
        assert_eq!(shell.selected_id(), Some("GER_b"));
        let digest = shell.selected_detail().expect("selected detail").digest();
        assert!(digest.contains("title=\"GER_b\""), "{digest}");
        assert!(digest.contains("ID=text=\"GER_b\""), "{digest}");
        assert!(
            digest.contains("State=text=\"in progress\",fidelity=Exact"),
            "{digest}"
        );
        assert!(
            digest.contains("Cost=text=\"7\",fidelity=Exact"),
            "{digest}"
        );
        assert!(
            digest.contains("Progress days=text=\"12\",fidelity=Exact"),
            "{digest}"
        );
        assert!(digest.contains("Prereq groups=text=\"1\""), "{digest}");
        assert!(digest.contains("Position=text=\"exact\""), "{digest}");
        assert!(!digest.contains("0 days"), "{digest}");
    }

    #[test]
    fn focus_shell_drag_pan_does_not_select_pending_click() {
        let mut shell = shell();
        shell.rebind(&snap_a(), "GER", (1600, 900), true);
        let (x, y) = node_center(&shell, "GER_b");

        shell.begin_click(x, y);
        shell.cancel_click_for_drag();
        assert!(!shell.commit_click(x, y));
        assert_eq!(shell.selected_id(), None);
        assert!(shell.selected_detail().is_none());
    }

    #[test]
    fn focus_shell_missing_selected_fails_closed_without_fallback() {
        let mut shell = shell();
        shell.rebind(&snap_a(), "GER", (1600, 900), true);
        shell.selected = Some("NOT_A_FOCUS".to_string());

        assert_eq!(shell.selected_id(), Some("NOT_A_FOCUS"));
        assert!(
            shell.selected_detail().is_none(),
            "missing selected focus must not fall back to another node"
        );
    }

    #[test]
    fn focus_shell_selected_detail_scrub_a_b_a_rebuilds_current_state() {
        let mut shell = shell();
        let a = snap_a();
        let b = snap_b();
        shell.rebind(&a, "GER", (1600, 900), true);
        click(&mut shell, "GER_b");
        let a1 = shell.selected_detail().expect("A detail").digest();
        assert!(a1.contains("State=text=\"in progress\""), "{a1}");
        assert!(a1.contains("Progress days=text=\"12\""), "{a1}");

        shell.rebind(&b, "GER", (1600, 900), false);
        let b_digest = shell.selected_detail().expect("B detail").digest();
        assert!(b_digest.contains("State=text=\"completed\""), "{b_digest}");
        assert!(
            !b_digest.contains("Progress days"),
            "completed selected focus must not carry active progress rows: {b_digest}"
        );
        assert_ne!(a1, b_digest, "B must rebuild from current-hour focus state");

        shell.rebind(&a, "GER", (1600, 900), false);
        let a2 = shell.selected_detail().expect("A detail again").digest();
        assert_eq!(a1, a2, "selected focus detail A->B->A must restore");
    }

    #[test]
    fn focus_selected_detail_missing_cost_and_progress_are_unavailable_not_zero() {
        let mut shell = shell();
        shell.rebind(&snap_current_without_progress(), "GER", (1600, 900), true);
        click(&mut shell, "GER_c");

        let digest = shell.selected_detail().expect("selected detail").digest();
        assert!(
            digest.contains("Cost=text=\"—\",fidelity=Unavailable"),
            "{digest}"
        );
        assert!(
            digest.contains("Progress days=text=\"—\",fidelity=Unavailable"),
            "{digest}"
        );
        assert!(digest.contains("reason=Some(\"absent\")"), "{digest}");
        assert!(!digest.contains("text=\"0\""), "{digest}");
        assert!(!digest.contains("0 days"), "{digest}");
    }
}
