//! N1 — the generic NODE-GRAPH / TREE primitive. HIGHEST LEVERAGE: it backs
//! tech_tree, doctrine, research, power_balance, focus, AND N3 (trait trees).
//!
//! It is a PLACED-graph renderer, NOT a layout engine: the caller supplies absolute
//! canvas-space node positions (focus/tech/doctrine each compute their own placement
//! from the script grid; power_balance plots a 2-axis state space). The widget owns
//! the pan/zoom [`Canvas`], the edge-line layer, the node-box layer, hit-testing, and
//! the honesty badges — nothing more.
//!
//! No-fabrication (Codex): every node `label` is an [`Atom`] (a blocked node draws
//! `—`, never a fabricated id); a node carries an explicit [`NodeBadge`]; an edge whose
//! endpoint id is MISSING from the graph draws NOTHING (it never invents a position).
//! A dependency edge whose satisfaction state is unavailable is drawn with a neutral
//! unavailable tint, never as satisfied or unsatisfied.
//! Edge kinds are data-driven ([`EdgeKind`]) so the same primitive expresses focus
//! prereq/mutually-exclusive links AND a power-balance influence/reference arrow.
//!
//! ## Usage
//! ```ignore
//! use widgets::node_graph::*;
//! use widgets::common::{Atom, Canvas};
//! let g = NodeGraph::new(
//!     vec![
//!         GraphNode::box_at("A", 0.0, 0.0, 96.0, 96.0, Atom::exact("Alpha"), [0.85,0.69,0.22,1.0]),
//!         GraphNode::box_at("B", 0.0, 130.0, 96.0, 96.0, Atom::exact("Beta"), [0.35,0.55,0.38,1.0]),
//!     ],
//!     vec![Edge::dependency("A", "B", true)],
//! );
//! let cam = Canvas::fit(g.bounds(), [0.0, 0.0, 800.0, 600.0], 24.0);
//! // draw_graph(renderer, &cam, &g, &assets);            // GPU draw
//! let hit = g.pick(&cam, 48.0, 48.0);                    // CPU hit-test
//! ```

use render::Renderer;

use crate::common::{Atom, Canvas, DrawAssets, Fidelity, DIM_TINT, TEXT_TINT};
use crate::plan::{Cmd, Plan};

/// A node's POSITION-fidelity badge (distinct from the label's value-fidelity). Mirrors
/// `focus_tree::PosFidelity`: an approximate placement (an unsupported offset trigger)
/// is flagged, never silently drawn as if exact.
#[derive(Debug, Clone, PartialEq)]
pub enum NodeBadge {
    /// Position + state are exact.
    Exact,
    /// Position approximate (e.g. an unsupported offset trigger) — drawn with a corner
    /// tab so it is never silently wrong.
    Approximate { reason: String },
    /// The node itself is blocked/unavailable (drawn hollow with a `—`).
    Unavailable,
}

/// One placed node: an absolute canvas-space box + an [`Atom`] label + a state colour.
/// `w`/`h` are in canvas units (scaled by the camera). The label is an `Atom`, so a
/// blocked node draws `—` rather than a fabricated id (Codex (b)).
#[derive(Debug, Clone, PartialEq)]
pub struct GraphNode {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// The visible label (no-fabrication-safe).
    pub label: Atom,
    /// The node-body fill colour (the consumer's state palette: completed/available/…).
    pub state_color: [f32; 4],
    pub badge: NodeBadge,
}

impl GraphNode {
    /// A node box with an exact position + a measured label.
    pub fn box_at(
        id: impl Into<String>,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        label: Atom,
        state_color: [f32; 4],
    ) -> GraphNode {
        GraphNode {
            id: id.into(),
            x,
            y,
            w,
            h,
            label,
            state_color,
            badge: NodeBadge::Exact,
        }
    }
    /// Mark this node's POSITION approximate (carried through from the placement pass).
    pub fn approximate(mut self, reason: impl Into<String>) -> GraphNode {
        self.badge = NodeBadge::Approximate {
            reason: reason.into(),
        };
        self
    }
    /// Mark this node blocked/unavailable (drawn hollow with the not-captured glyph).
    pub fn unavailable(mut self) -> GraphNode {
        self.badge = NodeBadge::Unavailable;
        self.label = Atom::unavailable();
        self
    }
    /// The node's canvas-space centre (used for edge endpoints + picking).
    pub fn center(&self) -> (f32, f32) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
    fn finite(&self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.w.is_finite() && self.h.is_finite()
    }
}

/// The SEMANTIC kind of an edge (Codex N1: not just focus's link/exclusive). The style
/// is derived from the kind so the same primitive draws a focus prereq, a focus
/// mutually-exclusive crossbar, a doctrine influence arrow, or a power-balance reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeKind {
    /// A prerequisite/parent→child dependency (the focus/tech link). `satisfied`
    /// brightens it; an L-route step line.
    Dependency,
    /// A mutually-exclusive pair (the focus XOR crossbar; a straight red segment).
    MutualExclusive,
    /// A soft influence/contribution (doctrine, power-balance) — a straight tinted
    /// segment, optionally directed.
    Influence,
    /// A cross-reference (a thin neutral straight segment).
    Reference,
}

/// A graph EDGE: two endpoint ids + a kind + per-edge fidelity + an optional label.
/// Codex N1: directed flag, edge label, and fidelity are all supported. An edge whose
/// endpoint is absent from the node set is SKIPPED at draw time (no invented position).
#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    /// True if the edge has an arrowhead at `to` (dependency/influence direction).
    pub directed: bool,
    /// True if this edge's precondition is met (brightens a dependency line).
    pub satisfied: bool,
    /// An optional measured edge label (e.g. a contribution weight). No-fabrication.
    pub label: Option<Atom>,
    /// The edge's honesty (Derived for a computed influence weight, etc).
    pub fidelity: Fidelity,
}

impl Edge {
    /// A prerequisite dependency `from → to` (the focus/tech link). Directed.
    pub fn dependency(from: impl Into<String>, to: impl Into<String>, satisfied: bool) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            kind: EdgeKind::Dependency,
            directed: true,
            satisfied,
            label: None,
            fidelity: Fidelity::Exact,
        }
    }
    /// A prerequisite dependency whose satisfied/unsatisfied state is unavailable.
    pub fn dependency_unavailable(from: impl Into<String>, to: impl Into<String>) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            kind: EdgeKind::Dependency,
            directed: true,
            satisfied: false,
            label: None,
            fidelity: Fidelity::Unavailable,
        }
    }
    /// A mutually-exclusive pair `a ↔ b` (the focus XOR crossbar). Undirected.
    pub fn mutually_exclusive(a: impl Into<String>, b: impl Into<String>) -> Edge {
        Edge {
            from: a.into(),
            to: b.into(),
            kind: EdgeKind::MutualExclusive,
            directed: false,
            satisfied: false,
            label: None,
            fidelity: Fidelity::Exact,
        }
    }
    /// A soft influence `from → to` with a measured/derived weight label.
    pub fn influence(from: impl Into<String>, to: impl Into<String>, label: Atom) -> Edge {
        Edge {
            from: from.into(),
            to: to.into(),
            kind: EdgeKind::Influence,
            directed: true,
            satisfied: true,
            label: Some(label),
            fidelity: Fidelity::Derived,
        }
    }
    /// A neutral cross-reference (undirected, thin).
    pub fn reference(a: impl Into<String>, b: impl Into<String>) -> Edge {
        Edge {
            from: a.into(),
            to: b.into(),
            kind: EdgeKind::Reference,
            directed: false,
            satisfied: false,
            label: None,
            fidelity: Fidelity::Exact,
        }
    }
}

/// The placed graph: nodes + edges. The widget never lays out positions — it draws +
/// hit-tests what the caller placed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NodeGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<Edge>,
}

impl NodeGraph {
    pub fn new(nodes: Vec<GraphNode>, edges: Vec<Edge>) -> NodeGraph {
        NodeGraph { nodes, edges }
    }
    /// The first node with `id` (duplicate ids → deterministic first match; Codex N1).
    pub fn node(&self, id: &str) -> Option<&GraphNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
    /// Canvas-space bounds `(minx,miny,maxx,maxy)` of ALL node boxes (so a camera can
    /// `fit`). Empty graph → a unit box at the origin (never an inverted/NaN bound).
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let mut b = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        let mut any = false;
        for n in self.nodes.iter().filter(|n| n.finite()) {
            any = true;
            b.0 = b.0.min(n.x);
            b.1 = b.1.min(n.y);
            b.2 = b.2.max(n.x + n.w);
            b.3 = b.3.max(n.y + n.h);
        }
        if any {
            b
        } else {
            (0.0, 0.0, 1.0, 1.0)
        }
    }

    /// Hit-test a SCREEN point against the placed nodes. Front-most wins (later nodes
    /// draw on top); returns the node id. `None` outside the camera viewport or off
    /// every node. Read-only — no fabrication.
    pub fn pick(&self, cam: &Canvas, sx: f32, sy: f32) -> Option<String> {
        if !cam.contains(sx, sy) {
            return None;
        }
        let mut hit = None;
        for n in self.nodes.iter().filter(|n| n.finite()) {
            let (cx, cy) = n.center();
            let (scx, scy) = cam.to_screen(cx, cy);
            let nw = (n.w * cam.scale).max(1.0);
            let nh = (n.h * cam.scale).max(1.0);
            let x = scx - nw / 2.0;
            let y = scy - nh / 2.0;
            if sx >= x && sx < x + nw && sy >= y && sy < y + nh {
                hit = Some(n.id.clone()); // later nodes win (drawn on top)
            }
        }
        hit
    }
}

/// Per-render stats (verification + the badge legend).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct GraphDrawStats {
    pub nodes_drawn: usize,
    pub edges_drawn: usize,
    pub edges_skipped_missing_endpoint: usize,
    pub approximate_nodes: usize,
    pub unavailable_nodes: usize,
}

const DEP_SATISFIED: [f32; 4] = [0.80, 0.80, 0.55, 0.95];
const DEP_UNSATISFIED: [f32; 4] = [0.45, 0.45, 0.50, 0.55];
const DEP_UNAVAILABLE: [f32; 4] = [0.34, 0.36, 0.46, 0.42];
const XOR_C: [f32; 4] = [0.80, 0.25, 0.25, 0.85];
const INFLUENCE_C: [f32; 4] = [0.45, 0.62, 0.85, 0.85];
const REFERENCE_C: [f32; 4] = [0.55, 0.56, 0.60, 0.7];
const BORDER_C: [f32; 4] = [0.05, 0.05, 0.06, 1.0];
const APPROX_TAB_C: [f32; 4] = [0.95, 0.15, 0.85, 1.0];

/// Build the render PLAN for the graph (NO GPU — headlessly testable). Edges first
/// (under nodes), then node boxes. The returned `(Plan, stats)` is what `draw_graph`
/// executes; the plan's digest is the scrub byte-identity key. Returns the draw stats
/// (incl. the skipped-edge count — the no-fabrication proof a missing-endpoint edge
/// emitted no command).
pub fn plan_graph(cam: &Canvas, graph: &NodeGraph) -> (Plan, GraphDrawStats) {
    let clip = Some(cam.clip());
    let mut p = Plan::new();
    let mut stats = GraphDrawStats::default();

    let center = |id: &str| -> Option<(f32, f32)> {
        graph.node(id).filter(|n| n.finite()).map(|n| {
            let (cx, cy) = n.center();
            cam.to_screen(cx, cy)
        })
    };

    // 1) EDGE LAYER.
    for e in &graph.edges {
        let (from, to) = match (center(&e.from), center(&e.to)) {
            (Some(a), Some(b)) => (a, b),
            // a missing endpoint NEVER invents a position — emit nothing, count it.
            _ => {
                stats.edges_skipped_missing_endpoint += 1;
                continue;
            }
        };
        let thick = (2.0 * cam.scale).max(0.6);
        match e.kind {
            EdgeKind::Dependency => {
                let c = if e.fidelity == Fidelity::Unavailable {
                    DEP_UNAVAILABLE
                } else if e.satisfied {
                    DEP_SATISFIED
                } else {
                    DEP_UNSATISFIED
                };
                p.push(Cmd::LinkL {
                    from,
                    to,
                    thick,
                    tint: c,
                    clip,
                });
                if e.directed {
                    p.push(arrowhead(to, c, cam.scale, clip));
                }
            }
            EdgeKind::MutualExclusive => {
                // draw EVERY edge the caller supplied — dedup is the caller's job (it emits
                // one edge per pair). An id-order gate here silently dropped a legitimately
                // ordered pair like ("z","a") while still counting it (Codex).
                p.push(Cmd::Line {
                    from,
                    to,
                    thick: thick.max(1.0),
                    tint: XOR_C,
                    clip,
                });
            }
            EdgeKind::Influence => {
                p.push(Cmd::Line {
                    from,
                    to,
                    thick,
                    tint: INFLUENCE_C,
                    clip,
                });
                if e.directed {
                    p.push(arrowhead(to, INFLUENCE_C, cam.scale, clip));
                }
            }
            EdgeKind::Reference => {
                p.push(Cmd::Line {
                    from,
                    to,
                    thick: thick.max(0.8),
                    tint: REFERENCE_C,
                    clip,
                });
            }
        }
        if let Some(lbl) = &e.label {
            let mx = (from.0 + to.0) / 2.0;
            let my = (from.1 + to.1) / 2.0;
            p.push(Cmd::Text {
                x: mx + 2.0,
                y: my - 6.0,
                text: lbl.as_str().into(),
                tint: lbl.tint(DIM_TINT),
                clip,
            });
        }
        stats.edges_drawn += 1;
    }

    // 2) NODE LAYER.
    for n in graph.nodes.iter().filter(|n| n.finite()) {
        let (cx, cy) = n.center();
        let (scx, scy) = cam.to_screen(cx, cy);
        let nw = (n.w * cam.scale).max(1.0);
        let nh = (n.h * cam.scale).max(1.0);
        let x = scx - nw / 2.0;
        let y = scy - nh / 2.0;

        // body: a dark border ring + the state fill (hollow for an unavailable node).
        p.push(Cmd::Rect {
            x: x - 1.0,
            y: y - 1.0,
            w: nw + 2.0,
            h: nh + 2.0,
            tint: BORDER_C,
            clip,
        });
        if matches!(n.badge, NodeBadge::Unavailable) {
            stats.unavailable_nodes += 1;
            p.push(Cmd::Rect {
                x,
                y,
                w: nw,
                h: nh,
                tint: crate::common::PLACEHOLDER_TINT,
                clip,
            });
            // a dim outline so a blocked node reads as "no data".
            push_stroke(
                &mut p,
                x,
                y,
                nw,
                nh,
                (2.0 * cam.scale).max(1.0),
                DIM_TINT,
                clip,
            );
        } else {
            p.push(Cmd::Rect {
                x,
                y,
                w: nw,
                h: nh,
                tint: n.state_color,
                clip,
            });
        }

        if let NodeBadge::Approximate { .. } = n.badge {
            stats.approximate_nodes += 1;
            let s = (10.0 * cam.scale).max(4.0);
            p.push(Cmd::Rect {
                x: x + nw - s,
                y,
                w: s,
                h: s,
                tint: APPROX_TAB_C,
                clip,
            });
        }

        // label (clipped to the node box) when there is room.
        if nw >= 24.0 {
            p.push(Cmd::Text {
                x: x + 3.0,
                y: y + 2.0,
                text: n.label.as_str().into(),
                tint: n.label.tint(TEXT_TINT),
                clip: Some([x, y, nw, nh]),
            });
        }
        stats.nodes_drawn += 1;
    }

    (p, stats)
}

/// Draw the whole graph through `cam` (plan + execute). READ-ONLY. Returns draw stats.
pub fn draw_graph(
    r: &mut Renderer,
    cam: &Canvas,
    graph: &NodeGraph,
    assets: &DrawAssets,
) -> GraphDrawStats {
    let (plan, stats) = plan_graph(cam, graph);
    // N1 draws no sprites (icons are the consumer's job over the box) — resolve is a no-op.
    plan.execute(r, assets, |_, _| false);
    stats
}

/// A tiny arrowhead disc at the edge's `to` end (no rotated triangle; a small filled
/// dot reads as a direction marker at any zoom).
fn arrowhead(to: (f32, f32), c: [f32; 4], scale: f32, clip: Option<[f32; 4]>) -> Cmd {
    Cmd::Disc {
        cx: to.0,
        cy: to.1,
        r: (3.0 * scale).clamp(2.0, 6.0),
        tint: c,
        clip,
    }
}

/// Push a 4-fill rect outline into a plan.
#[allow(clippy::too_many_arguments)]
fn push_stroke(
    p: &mut Plan,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    t: f32,
    c: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    let t = t.max(1.0);
    p.push(Cmd::Rect {
        x,
        y,
        w,
        h: t,
        tint: c,
        clip,
    });
    p.push(Cmd::Rect {
        x,
        y: y + h - t,
        w,
        h: t,
        tint: c,
        clip,
    });
    p.push(Cmd::Rect {
        x,
        y,
        w: t,
        h,
        tint: c,
        clip,
    });
    p.push(Cmd::Rect {
        x: x + w - t,
        y,
        w: t,
        h,
        tint: c,
        clip,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> NodeGraph {
        NodeGraph::new(
            vec![
                GraphNode::box_at(
                    "A",
                    0.0,
                    0.0,
                    96.0,
                    96.0,
                    Atom::exact("Alpha"),
                    [0.8, 0.7, 0.2, 1.0],
                ),
                GraphNode::box_at(
                    "B",
                    0.0,
                    130.0,
                    96.0,
                    96.0,
                    Atom::exact("Beta"),
                    [0.3, 0.5, 0.4, 1.0],
                ),
            ],
            vec![
                Edge::dependency("A", "B", true),
                Edge::mutually_exclusive("A", "B"),
                // an edge to a GHOST node — must be skipped, never positioned.
                Edge::dependency("A", "GHOST", false),
            ],
        )
    }

    #[test]
    fn bounds_cover_all_node_boxes() {
        let g = graph();
        let (minx, miny, maxx, maxy) = g.bounds();
        assert_eq!((minx, miny), (0.0, 0.0));
        assert_eq!((maxx, maxy), (96.0, 226.0));
    }

    #[test]
    fn empty_graph_has_sane_bounds_and_no_pick_panic() {
        let g = NodeGraph::default();
        let b = g.bounds();
        assert!(b.0.is_finite() && b.2 > b.0); // never inverted/NaN
        let cam = Canvas::new([0.0, 0.0, 100.0, 100.0]);
        assert_eq!(g.pick(&cam, 50.0, 50.0), None); // no panic on empty
    }

    #[test]
    fn pick_finds_front_node_and_misses_outside_viewport() {
        let g = graph();
        let cam = Canvas::fit(g.bounds(), [0.0, 0.0, 800.0, 600.0], 24.0);
        let a = g.node("A").unwrap();
        let (cx, cy) = cam.to_screen(a.center().0, a.center().1);
        assert_eq!(g.pick(&cam, cx, cy).as_deref(), Some("A"));
        // a point outside the viewport returns None (not a clamped fabrication).
        assert_eq!(g.pick(&cam, -50.0, -50.0), None);
    }

    #[test]
    fn duplicate_ids_resolve_to_first_match_deterministically() {
        let g = NodeGraph::new(
            vec![
                GraphNode::box_at("D", 0.0, 0.0, 10.0, 10.0, Atom::exact("first"), [1.0; 4]),
                GraphNode::box_at("D", 0.0, 0.0, 10.0, 10.0, Atom::exact("second"), [1.0; 4]),
            ],
            vec![],
        );
        assert_eq!(g.node("D").unwrap().label, Atom::exact("first"));
    }

    #[test]
    fn nan_node_position_is_ignored_not_drawn() {
        let mut g = graph();
        g.nodes.push(GraphNode::box_at(
            "N",
            f32::NAN,
            0.0,
            10.0,
            10.0,
            Atom::exact("nan"),
            [1.0; 4],
        ));
        // bounds ignore the NaN node; pick never returns it.
        let (_, _, maxx, _) = g.bounds();
        assert!(maxx.is_finite());
    }

    #[test]
    fn unavailable_node_badges_glyph_not_fabricated_label() {
        let n = GraphNode::box_at("U", 0.0, 0.0, 96.0, 96.0, Atom::exact("real"), [1.0; 4])
            .unavailable();
        assert!(n.label.is_unavailable());
        assert_eq!(n.label.as_str(), "—");
    }

    #[test]
    fn plan_skips_missing_endpoint_edge_and_is_deterministic() {
        let g = graph();
        let cam = Canvas::fit(g.bounds(), [0.0, 0.0, 800.0, 600.0], 24.0);
        let (plan, stats) = plan_graph(&cam, &g);
        // the A->GHOST edge emitted no command (no fabricated position).
        assert_eq!(stats.edges_skipped_missing_endpoint, 1);
        assert_eq!(stats.edges_drawn, 2);
        assert_eq!(stats.nodes_drawn, 2);
        // re-planning the same camera+graph is byte-identical (scrub A→A).
        let (plan2, _) = plan_graph(&cam, &g);
        assert_eq!(plan.digest(), plan2.digest());
    }

    #[test]
    fn unavailable_dependency_edge_uses_neutral_no_fabrication_tint() {
        let g = NodeGraph::new(
            vec![
                GraphNode::box_at("A", 0.0, 0.0, 96.0, 96.0, Atom::exact("A"), [1.0; 4]),
                GraphNode::box_at("B", 0.0, 130.0, 96.0, 96.0, Atom::exact("B"), [1.0; 4]),
            ],
            vec![Edge::dependency_unavailable("A", "B")],
        );
        assert_eq!(g.edges[0].fidelity, Fidelity::Unavailable);
        let cam = Canvas::fit(g.bounds(), [0.0, 0.0, 800.0, 600.0], 24.0);
        let (plan, stats) = plan_graph(&cam, &g);
        assert_eq!(stats.edges_drawn, 1);
        let link_tint = plan
            .cmds
            .iter()
            .find_map(|cmd| match cmd {
                Cmd::LinkL { tint, .. } => Some(*tint),
                _ => None,
            })
            .expect("dependency link emitted");
        assert_eq!(link_tint, DEP_UNAVAILABLE);
        assert_ne!(link_tint, DEP_UNSATISFIED);
        assert_ne!(link_tint, DEP_SATISFIED);
    }

    #[test]
    fn reverse_ordered_xor_edge_still_draws() {
        // a pair given as ("z","a") (from > to) must still emit a line — no id-order gate.
        let g = NodeGraph::new(
            vec![
                GraphNode::box_at("z", 0.0, 0.0, 96.0, 96.0, Atom::exact("Z"), [1.0; 4]),
                GraphNode::box_at("a", 0.0, 130.0, 96.0, 96.0, Atom::exact("A"), [1.0; 4]),
            ],
            vec![Edge::mutually_exclusive("z", "a")],
        );
        let cam = Canvas::fit(g.bounds(), [0.0, 0.0, 800.0, 600.0], 24.0);
        let (plan, stats) = plan_graph(&cam, &g);
        assert_eq!(stats.edges_drawn, 1);
        assert_eq!(
            plan.cmds
                .iter()
                .filter(|c| matches!(c, Cmd::Line { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn plan_for_unavailable_node_emits_glyph_text() {
        let g = NodeGraph::new(
            vec![
                GraphNode::box_at("U", 0.0, 0.0, 200.0, 200.0, Atom::exact("real"), [1.0; 4])
                    .unavailable(),
            ],
            vec![],
        );
        let cam = Canvas::fit(g.bounds(), [0.0, 0.0, 800.0, 600.0], 24.0);
        let (plan, stats) = plan_graph(&cam, &g);
        assert_eq!(stats.unavailable_nodes, 1);
        assert_eq!(plan.unavailable_text_count(), 1); // the "—" label
    }
}
