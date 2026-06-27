//! N3 — the TRAIT-TREE primitive: a node grid with prerequisite + mutually-exclusive
//! lines and per-node STATE, built as a THIN specialization of N1 ([`crate::node_graph`]).
//! Consumers: army/navy trait trees, command_character traits, officer_corps.
//!
//! Codex N3 hardening: a trait carries a RICH [`TraitState`] (not just a colour) —
//! owned / available / blocked / mutually-excluded / insufficient-xp / unknown /
//! unavailable — so a blocked or no-data trait BADGES rather than fabricates. Grid
//! placement (`gx,gy` cells) is kept SEPARATE from the visual edges, so hit-testing and
//! accounting do not depend on lines. Prereq/XOR edges to a MISSING trait id are
//! dropped at conversion (no phantom node is invented).
//!
//! The widget converts a [`TraitTree`] into an N1 [`NodeGraph`] and then reuses N1's
//! camera, plan, draw, and pick — so it inherits N1's pan/zoom + no-fabrication
//! machinery for free.
//!
//! ## Usage
//! ```ignore
//! use widgets::trait_tree::*;
//! use widgets::common::{Atom, Canvas};
//! let tree = TraitTree::new(vec![
//!     TraitNode::cell("infantry_leader", 0, 0, Atom::exact("Infantry Leader"), TraitState::Owned),
//!     TraitNode::cell("trickster",       1, 0, Atom::exact("Trickster"),       TraitState::Available),
//! ])
//! .prereq("infantry_leader", "trickster")
//! .mutually_exclusive("trickster", "ambusher");
//! let graph = tree.to_graph();
//! let cam = Canvas::fit(graph.bounds(), [0.0, 0.0, 600.0, 400.0], 20.0);
//! // widgets::node_graph::draw_graph(r, &cam, &graph, &assets);
//! ```

use crate::common::Atom;
use crate::node_graph::{Edge, GraphNode, NodeBadge, NodeGraph};

/// The grid cell size in canvas px (a trait node + spacing).
pub const CELL_W: f32 = 80.0;
pub const CELL_H: f32 = 80.0;
pub const NODE_W: f32 = 64.0;
pub const NODE_H: f32 = 64.0;

/// A trait's state (Codex N3: richer than a colour). Drives the node fill + badge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraitState {
    /// The leader HAS this trait.
    Owned,
    /// Prerequisites met + assignable now.
    Available,
    /// Prerequisites unmet (locked).
    Blocked,
    /// Excluded because a mutually-exclusive sibling is owned.
    MutuallyExcluded,
    /// Available except the leader lacks the XP (a distinct, honest state).
    InsufficientXp,
    /// We KNOW the trait exists but its state is not in the snapshot — badged unknown,
    /// never guessed as one of the above.
    Unknown,
    /// The trait slot itself is blocked/unfed — drawn as a `—` no-data node.
    Unavailable,
}

impl TraitState {
    /// The node-body palette for this state.
    pub fn color(&self) -> [f32; 4] {
        match self {
            TraitState::Owned => [0.85, 0.69, 0.22, 1.0],
            TraitState::Available => [0.35, 0.55, 0.38, 1.0],
            TraitState::Blocked => [0.28, 0.30, 0.34, 1.0],
            TraitState::MutuallyExcluded => [0.45, 0.22, 0.22, 1.0],
            TraitState::InsufficientXp => [0.40, 0.40, 0.24, 1.0],
            TraitState::Unknown => [0.22, 0.22, 0.26, 1.0],
            TraitState::Unavailable => crate::common::PLACEHOLDER_TINT,
        }
    }
}

/// One trait placed in the grid (cell coords kept separate from edges, Codex N3).
#[derive(Debug, Clone, PartialEq)]
pub struct TraitNode {
    pub id: String,
    /// Grid cell column / row (visual placement; NOT pixels).
    pub gx: i32,
    pub gy: i32,
    pub label: Atom,
    pub state: TraitState,
}

impl TraitNode {
    pub fn cell(
        id: impl Into<String>,
        gx: i32,
        gy: i32,
        label: Atom,
        state: TraitState,
    ) -> TraitNode {
        TraitNode {
            id: id.into(),
            gx,
            gy,
            label,
            state,
        }
    }
}

/// A trait tree: nodes + prereq + mutually-exclusive id pairs (resolved to edges at
/// conversion; pairs to a missing id are dropped).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TraitTree {
    pub nodes: Vec<TraitNode>,
    pub prereqs: Vec<(String, String)>,
    pub exclusives: Vec<(String, String)>,
}

impl TraitTree {
    pub fn new(nodes: Vec<TraitNode>) -> TraitTree {
        TraitTree {
            nodes,
            prereqs: Vec::new(),
            exclusives: Vec::new(),
        }
    }
    /// Add a prerequisite `from → to` (builder-chained).
    pub fn prereq(mut self, from: impl Into<String>, to: impl Into<String>) -> TraitTree {
        self.prereqs.push((from.into(), to.into()));
        self
    }
    /// Add a mutually-exclusive pair (builder-chained).
    pub fn mutually_exclusive(mut self, a: impl Into<String>, b: impl Into<String>) -> TraitTree {
        self.exclusives.push((a.into(), b.into()));
        self
    }
    /// True iff an id is a real node (used to drop phantom edges).
    fn has(&self, id: &str) -> bool {
        self.nodes.iter().any(|n| n.id == id)
    }

    /// Convert into an N1 [`NodeGraph`]: cell coords → canvas px, state → colour + badge,
    /// prereq/XOR pairs → edges (a pair to a MISSING id is dropped — no phantom node).
    pub fn to_graph(&self) -> NodeGraph {
        let nodes: Vec<GraphNode> = self
            .nodes
            .iter()
            .map(|t| {
                let x = t.gx as f32 * CELL_W;
                let y = t.gy as f32 * CELL_H;
                let mut gn = GraphNode::box_at(
                    t.id.clone(),
                    x,
                    y,
                    NODE_W,
                    NODE_H,
                    t.label.clone(),
                    t.state.color(),
                );
                gn.badge = match t.state {
                    TraitState::Unavailable => {
                        // a blocked trait slot -> hollow `—` node.
                        return gn.unavailable();
                    }
                    // an UNKNOWN trait state gets an explicit honesty badge (a corner tab),
                    // so "we don't know this trait's state" never relies on palette alone
                    // (Codex N3).
                    TraitState::Unknown => NodeBadge::Approximate {
                        reason: "trait state unknown".into(),
                    },
                    _ => NodeBadge::Exact,
                };
                gn
            })
            .collect();

        let mut edges = Vec::new();
        for (from, to) in &self.prereqs {
            if self.has(from) && self.has(to) {
                // a prereq is satisfied iff the source trait is owned.
                let satisfied = self
                    .nodes
                    .iter()
                    .find(|n| &n.id == from)
                    .map(|n| n.state == TraitState::Owned)
                    .unwrap_or(false);
                edges.push(Edge::dependency(from.clone(), to.clone(), satisfied));
            }
        }
        for (a, b) in &self.exclusives {
            if self.has(a) && self.has(b) {
                edges.push(Edge::mutually_exclusive(a.clone(), b.clone()));
            }
        }
        NodeGraph::new(nodes, edges)
    }

    /// The number of prereq/XOR pairs that reference a MISSING trait id (dropped at
    /// conversion). A diagnostics hook — a non-zero count flags a tree-data gap without
    /// fabricating a node.
    pub fn dangling_edge_count(&self) -> usize {
        let dangling = |pairs: &[(String, String)]| {
            pairs
                .iter()
                .filter(|(a, b)| !self.has(a) || !self.has(b))
                .count()
        };
        dangling(&self.prereqs) + dangling(&self.exclusives)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::Canvas;
    use crate::node_graph;

    fn tree() -> TraitTree {
        TraitTree::new(vec![
            TraitNode::cell("a", 0, 0, Atom::exact("Alpha"), TraitState::Owned),
            TraitNode::cell("b", 1, 0, Atom::exact("Beta"), TraitState::Available),
            TraitNode::cell("u", 2, 0, Atom::exact("Gamma"), TraitState::Unavailable),
        ])
        .prereq("a", "b")
        .mutually_exclusive("b", "ghost") // ghost is missing -> dropped
    }

    #[test]
    fn converts_cells_to_canvas_positions() {
        let g = tree().to_graph();
        assert_eq!(g.nodes.len(), 3);
        assert_eq!((g.node("a").unwrap().x, g.node("a").unwrap().y), (0.0, 0.0));
        assert_eq!(g.node("b").unwrap().x, CELL_W);
    }

    #[test]
    fn drops_edge_to_missing_trait_no_phantom() {
        let t = tree();
        let g = t.to_graph();
        // only the a->b prereq survives; the b<->ghost exclusive is dropped.
        assert_eq!(g.edges.len(), 1);
        assert_eq!(t.dangling_edge_count(), 1);
        // no phantom "ghost" node was created.
        assert!(g.node("ghost").is_none());
    }

    #[test]
    fn prereq_satisfied_iff_source_owned() {
        let g = tree().to_graph();
        let dep = g
            .edges
            .iter()
            .find(|e| e.from == "a" && e.to == "b")
            .unwrap();
        assert!(dep.satisfied); // "a" is Owned
    }

    #[test]
    fn unavailable_trait_becomes_glyph_node() {
        let g = tree().to_graph();
        let u = g.node("u").unwrap();
        assert!(matches!(u.badge, NodeBadge::Unavailable));
        assert!(u.label.is_unavailable());
    }

    #[test]
    fn unknown_trait_gets_explicit_badge_not_palette_only() {
        let t = TraitTree::new(vec![TraitNode::cell(
            "k",
            0,
            0,
            Atom::exact("K"),
            TraitState::Unknown,
        )]);
        let g = t.to_graph();
        let n = g.node("k").unwrap();
        // an Unknown state carries an Approximate honesty badge (a corner tab), not Exact.
        assert!(matches!(n.badge, NodeBadge::Approximate { .. }));
    }

    #[test]
    fn empty_tree_yields_empty_graph_no_panic() {
        let g = TraitTree::default().to_graph();
        assert!(g.nodes.is_empty() && g.edges.is_empty());
        let cam = Canvas::fit(g.bounds(), [0.0, 0.0, 100.0, 100.0], 10.0);
        let (plan, stats) = node_graph::plan_graph(&cam, &g);
        assert!(plan.is_empty());
        assert_eq!(stats.nodes_drawn, 0);
    }
}
