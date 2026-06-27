//! Render the placed+bound focus GRAPH into the `nationalfocusview` drag_scroll
//! canvas: a pan/zoom/clip viewport, a link-line layer (prerequisite +
//! mutually_exclusive), and the node prototype coloured by bound state.
//!
//! This is the imperative ADAPTER's draw side (plan §B.5). Geometry comes from
//! [`crate::layout`] (absolute pixels), node STATE from [`crate::bind`]. We reuse
//! the `render` crate's quad/sprite/text primitives — a 1x1 white texture tinted
//! per state gives solid node frames + link lines + the current-focus glow,
//! every primitive CLIPPED to the canvas viewport (no leak past the frame).

use std::collections::HashMap;

use render::{Renderer, Texture};

use crate::bind::{FocusDomain, NodeState};
use crate::layout::{PlacedNode, PlacedTree, PosFidelity};

/// The node prototype footprint (the `national_focus_item` window: 165x128),
/// drawn centred on the grid cell. We draw a slightly inset symbol box.
pub const NODE_W: f32 = 96.0;
pub const NODE_H: f32 = 96.0;

/// A pan/zoom view transform over canvas (tree) space. `scale` is the zoom,
/// `(ox, oy)` the canvas-space point shown at the viewport's top-left.
#[derive(Debug, Clone, Copy)]
pub struct Camera {
    pub scale: f32,
    pub ox: f32,
    pub oy: f32,
    /// The on-screen viewport rect [x, y, w, h] the canvas is clipped to.
    pub viewport: [f32; 4],
}

impl Camera {
    /// Fit the whole tree into `viewport` with a small margin (used for the
    /// offscreen overview PNG + the initial app view).
    pub fn fit(tree: &PlacedTree, viewport: [f32; 4], margin: f32) -> Camera {
        let (minx, miny, maxx, maxy) = tree.bounds();
        let tw = (maxx - minx + NODE_W).max(1.0);
        let th = (maxy - miny + NODE_H).max(1.0);
        let sx = (viewport[2] - 2.0 * margin) / tw;
        let sy = (viewport[3] - 2.0 * margin) / th;
        let scale = sx.min(sy).min(1.0).max(0.02);
        Camera {
            scale,
            ox: minx - margin / scale,
            oy: miny - margin / scale,
            viewport,
        }
    }

    /// Canvas (tree) space → screen pixels.
    pub fn to_screen(&self, cx: f32, cy: f32) -> (f32, f32) {
        (
            self.viewport[0] + (cx - self.ox) * self.scale,
            self.viewport[1] + (cy - self.oy) * self.scale,
        )
    }

    pub fn clip(&self) -> [f32; 4] {
        self.viewport
    }

    /// Pan by a screen-space delta (drag). Read-only navigation.
    pub fn pan(&mut self, dx_screen: f32, dy_screen: f32) {
        self.ox -= dx_screen / self.scale;
        self.oy -= dy_screen / self.scale;
    }

    /// Zoom around a screen-space anchor (mouse), clamped.
    pub fn zoom_at(&mut self, anchor_x: f32, anchor_y: f32, factor: f32) {
        let (cx, cy) = self.from_screen(anchor_x, anchor_y);
        self.scale = (self.scale * factor).clamp(0.02, 4.0);
        // keep the anchor fixed
        self.ox = cx - (anchor_x - self.viewport[0]) / self.scale;
        self.oy = cy - (anchor_y - self.viewport[1]) / self.scale;
    }

    pub fn from_screen(&self, sx: f32, sy: f32) -> (f32, f32) {
        (
            self.ox + (sx - self.viewport[0]) / self.scale,
            self.oy + (sy - self.viewport[1]) / self.scale,
        )
    }
}

/// The colour palette for node states (RGBA, straight alpha). Distinct, legible
/// in the overview PNG; the real sprites layer on top in the app shell.
fn state_color(state: &NodeState) -> [f32; 4] {
    match state {
        // completed = solid gold
        NodeState::Completed => [0.85, 0.69, 0.22, 1.0],
        // current = bright cyan (the "working on it" glow drawn separately)
        NodeState::Current => [0.20, 0.80, 0.95, 1.0],
        // prereqs-met = muted green
        NodeState::PrereqsMet => [0.35, 0.55, 0.38, 1.0],
        // prereqs-unmet = dim slate
        NodeState::PrereqsUnmet => [0.28, 0.30, 0.34, 1.0],
    }
}

/// A 1x1 white texture for solid tinted rects/lines.
pub fn white_texture(renderer: &Renderer) -> Texture {
    renderer.upload_rgba8_image(1, 1, &[255, 255, 255, 255])
}

/// Draw a solid filled rect (screen space), clipped to `clip`.
fn fill_rect(r: &mut Renderer, white: &Texture, x: f32, y: f32, w: f32, h: f32, c: [f32; 4], clip: [f32; 4]) {
    r.draw_sprite_frame_clipped(white, x, y, w, h, 0, 1, c, Some(clip));
}

/// Draw a 1px-ish line between two screen points as a thin rotated-free rect.
/// Focus links are axis-aligned step routes, so we draw an L (vertical then
/// horizontal segment) which matches the in-game `GFX_focus_link_up_down` style.
fn link_line(
    r: &mut Renderer,
    white: &Texture,
    from: (f32, f32),
    to: (f32, f32),
    thick: f32,
    c: [f32; 4],
    clip: [f32; 4],
) {
    let (x0, y0) = from;
    let (x1, y1) = to;
    let t = thick.max(1.0);
    // vertical segment from `from` down/up to the midpoint y of `to`
    let midy = y1;
    let (vy0, vy1) = if y0 <= midy { (y0, midy) } else { (midy, y0) };
    fill_rect(r, white, x0 - t / 2.0, vy0, t, (vy1 - vy0).max(t), c, clip);
    // horizontal segment at midy from x0 to x1
    let (hx0, hx1) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    fill_rect(r, white, hx0, midy - t / 2.0, (hx1 - hx0).max(t), t, c, clip);
}

/// Inputs the renderer needs that come from the asset side (resolved per app).
pub struct DrawAssets<'a> {
    pub white: &'a Texture,
    /// Optional BMFont for node labels (id/name). When `None`, labels are
    /// skipped (the PNG still shows frames + links + state colours).
    pub font: Option<&'a assets::BmFont>,
    pub font_pages: &'a [Texture],
}

/// Per-render stats (for verification + the badge legend).
#[derive(Debug, Default, Clone)]
pub struct FocusDrawStats {
    pub nodes_drawn: usize,
    pub links_drawn: usize,
    pub exclusives_drawn: usize,
    pub completed: usize,
    pub current: usize,
    pub available: usize,
    pub locked: usize,
    pub approximate: usize,
}

/// Draw the whole focus tree through `cam` into `renderer`. READ-ONLY: nothing
/// here mutates the snapshot or the tree. Returns draw stats.
#[allow(clippy::too_many_arguments)]
pub fn draw_tree(
    renderer: &mut Renderer,
    cam: &Camera,
    tree: &PlacedTree,
    domain: &FocusDomain,
    assets: &DrawAssets,
) -> FocusDrawStats {
    let clip = cam.clip();
    let mut stats = FocusDrawStats::default();

    // Pre-resolve node center positions in screen space.
    let mut centers: HashMap<&str, (f32, f32)> = HashMap::new();
    for n in &tree.nodes {
        let (sx, sy) = cam.to_screen(n.px + NODE_W / 2.0, n.py + NODE_H / 2.0);
        centers.insert(n.id.as_str(), (sx, sy));
    }

    // 1) LINK LAYER first (under nodes). Prerequisite edges = satisfied (bright)
    //    vs unsatisfied (dim); mutually_exclusive = red crossbars.
    for n in &tree.nodes {
        let to = match centers.get(n.id.as_str()) {
            Some(p) => *p,
            None => continue,
        };
        for group in &n.prerequisites {
            for prereq in group {
                if let Some(&from) = centers.get(prereq.as_str()) {
                    let satisfied = domain.completed.contains(prereq);
                    let c = if satisfied {
                        [0.80, 0.80, 0.55, 0.95]
                    } else {
                        [0.45, 0.45, 0.50, 0.55]
                    };
                    link_line(renderer, assets.white, from, to, 2.0 * cam.scale.max(0.5), c, clip);
                    stats.links_drawn += 1;
                }
            }
        }
        for ex in &n.mutually_exclusive {
            if let Some(&other) = centers.get(ex.as_str()) {
                // draw once per unordered pair (only when id < ex to dedupe)
                if n.id.as_str() < ex.as_str() {
                    link_line(
                        renderer,
                        assets.white,
                        to,
                        other,
                        1.5 * cam.scale.max(0.5),
                        [0.80, 0.25, 0.25, 0.85],
                        clip,
                    );
                    stats.exclusives_drawn += 1;
                }
            }
        }
    }

    // 2) NODE LAYER.
    let nw = NODE_W * cam.scale;
    let nh = NODE_H * cam.scale;
    for n in &tree.nodes {
        let b = crate::bind::bind_node(&n.id, &n.prerequisites, domain);
        let (cx, cy) = centers[n.id.as_str()];
        let x = cx - nw / 2.0;
        let y = cy - nh / 2.0;

        // node body (state colour) + a darker border ring for legibility
        fill_rect(renderer, assets.white, x - 1.0, y - 1.0, nw + 2.0, nh + 2.0, [0.05, 0.05, 0.06, 1.0], clip);
        fill_rect(renderer, assets.white, x, y, nw, nh, state_color(&b.state), clip);

        // current-focus glow: a bright inset ring
        if b.state == NodeState::Current {
            let g = [0.40, 0.95, 1.0, 0.9];
            let t = (3.0 * cam.scale).max(1.5);
            fill_rect(renderer, assets.white, x, y, nw, t, g, clip);
            fill_rect(renderer, assets.white, x, y + nh - t, nw, t, g, clip);
            fill_rect(renderer, assets.white, x, y, t, nh, g, clip);
            fill_rect(renderer, assets.white, x + nw - t, y, t, nh, g, clip);
        }

        // APPROXIMATE-position badge: a magenta corner tab (never silent).
        if let PosFidelity::Approximate { .. } = n.fidelity {
            stats.approximate += 1;
            let s = (10.0 * cam.scale).max(4.0);
            fill_rect(renderer, assets.white, x + nw - s, y, s, s, [0.95, 0.15, 0.85, 1.0], clip);
        }

        // label (short focus id) when a font + room is available
        if let Some(font) = assets.font {
            if nw >= 28.0 {
                let label = short_label(&n.id);
                renderer.draw_text_clipped(
                    font,
                    assets.font_pages,
                    &label,
                    x + 3.0,
                    y + 2.0,
                    [0.95, 0.95, 0.92, 1.0],
                    Some([x, y, nw, nh]),
                );
            }
        }

        match b.state {
            NodeState::Completed => stats.completed += 1,
            NodeState::Current => stats.current += 1,
            NodeState::PrereqsMet => stats.available += 1,
            NodeState::PrereqsUnmet => stats.locked += 1,
        }
        stats.nodes_drawn += 1;
    }

    stats
}

/// A short human label for a node: drop the `TAG_` prefix, keep it terse.
fn short_label(id: &str) -> String {
    let s = id.splitn(2, '_').nth(1).unwrap_or(id);
    s.chars().take(14).collect()
}

/// Hit-test a screen point against the tree's nodes (front-most = last drawn).
/// Returns the focus id under the cursor, for tooltips/selection. READ-ONLY.
pub fn pick(cam: &Camera, tree: &PlacedTree, sx: f32, sy: f32) -> Option<String> {
    let nw = NODE_W * cam.scale;
    let nh = NODE_H * cam.scale;
    let mut hit = None;
    for n in &tree.nodes {
        let (cx, cy) = cam.to_screen(n.px + NODE_W / 2.0, n.py + NODE_H / 2.0);
        let x = cx - nw / 2.0;
        let y = cy - nh / 2.0;
        if sx >= x && sx <= x + nw && sy >= y && sy <= y + nh {
            hit = Some(n.id.clone()); // later nodes win (drawn on top)
        }
    }
    hit
}

/// Build the one-line tooltip for a node: name + state + progress + position
/// fidelity badge text. No-fabrication: progress only when current.
pub fn tooltip(node: &PlacedNode, domain: &FocusDomain) -> String {
    let b = crate::bind::bind_node(&node.id, &node.prerequisites, domain);
    let state = match b.state {
        NodeState::Completed => "Completed".to_string(),
        NodeState::Current => match b.progress_days {
            Some(d) => format!("In progress ({d} days)"),
            None => "In progress".to_string(),
        },
        // Honest labels: we only know the PREREQUISITES are/aren't met, not the
        // full the game availability (available/bypass/exclusive/capitulation gates
        // are not in our snapshot) — never claim "Available".
        NodeState::PrereqsMet => "Prereqs met".to_string(),
        NodeState::PrereqsUnmet => "Prereqs unmet".to_string(),
    };
    let mut s = format!("{} — {}", node.id, state);
    if let PosFidelity::Approximate { reason } = &node.fidelity {
        s.push_str(&format!("  [position approximate: {reason}]"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::{place, PlacedTree};
    use crate::{graph::trees_from_entries, pdx::parse_file, EvalContext};

    fn placed() -> PlacedTree {
        let t = trees_from_entries(&parse_file(
            r#"focus_tree = { id = t
                focus = { id = A x = 0 y = 0 }
                focus = { id = B x = 0 y = 1 relative_position_id = A prerequisite = { focus = A } }
            }"#,
        ))
        .into_iter()
        .next()
        .unwrap();
        place(&t, &EvalContext::default())
    }

    #[test]
    fn camera_fit_and_roundtrip() {
        let p = placed();
        let cam = Camera::fit(&p, [0.0, 0.0, 800.0, 600.0], 20.0);
        let (sx, sy) = cam.to_screen(50.0, 60.0);
        let (cx, cy) = cam.from_screen(sx, sy);
        assert!((cx - 50.0).abs() < 0.01);
        assert!((cy - 60.0).abs() < 0.01);
    }

    #[test]
    fn pick_finds_node() {
        let p = placed();
        let cam = Camera::fit(&p, [0.0, 0.0, 800.0, 600.0], 20.0);
        // center of node A in screen space
        let a = p.node("A").unwrap();
        let (cx, cy) = cam.to_screen(a.px + NODE_W / 2.0, a.py + NODE_H / 2.0);
        assert_eq!(pick(&cam, &p, cx, cy).as_deref(), Some("A"));
        // far away misses
        assert!(pick(&cam, &p, -1000.0, -1000.0).is_none());
    }

    #[test]
    fn tooltip_no_fabrication() {
        let p = placed();
        let domain = FocusDomain::default();
        let a = p.node("A").unwrap();
        let tt = tooltip(a, &domain);
        // empty domain -> A has no prereqs -> "Prereqs met"; never the
        // over-claiming "Available", never a fabricated "0 days".
        assert!(tt.contains("Prereqs met"));
        assert!(!tt.contains("Available"));
        assert!(!tt.contains("0 days"));
    }
}
