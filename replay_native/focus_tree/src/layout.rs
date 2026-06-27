//! Resolve a [`crate::graph::FocusTree`] into ABSOLUTE pixel positions.
//!
//! Two effects compose, in the engine's order:
//!   1. `relative_position_id` — a node's (x,y) is an OFFSET from its anchor's
//!      RESOLVED position (chains resolve transitively; cycles are broken + a
//!      node missing its anchor falls back to its own base x/y).
//!   2. the conditional `offset = { x y trigger }` branches — evaluated by §E:
//!      the FIRST branch whose trigger is TRUE adds (dx,dy). An UNSUPPORTED
//!      trigger anywhere on the node poisons it: we apply NO offset and BADGE
//!      the node `approximate` (never fabricate the branch).
//!
//! Grid units are converted to pixels by [`GRID_X`]/[`GRID_Y`] (the in-game
//! focus grid step), matching `nationalfocusview`'s node spacing.

use std::collections::HashMap;

use crate::eval::{eval_trigger, Eval, EvalContext};
use crate::graph::{FocusNode, FocusTree};

/// In-game focus grid step in pixels — the `focus_spacing` positionType in
/// `nationalfocusview.gui` ({x:96, y:130}). One grid unit in the focus script's
/// `x`/`y` maps to this many pixels; the node prototype itself is 165x128.
pub const GRID_X: f32 = 96.0;
pub const GRID_Y: f32 = 130.0;

/// Why a node's position might be flagged approximate.
#[derive(Debug, Clone, PartialEq)]
pub enum PosFidelity {
    /// Position is exact (base + resolved relatives + a supported/true offset or
    /// no offsets at all).
    Exact,
    /// At least one offset trigger was UNSUPPORTED → base position used, BADGED.
    Approximate { reason: String },
}

/// A node placed at an absolute pixel position.
#[derive(Debug, Clone)]
pub struct PlacedNode {
    pub id: String,
    pub icon: Option<String>,
    pub px: f32,
    pub py: f32,
    /// Grid coordinates (pre-pixel), handy for tests + tooltips.
    pub gx: i32,
    pub gy: i32,
    pub fidelity: PosFidelity,
    pub prerequisites: Vec<Vec<String>>,
    pub mutually_exclusive: Vec<String>,
    pub cost: Option<f32>,
}

/// The fully laid-out tree.
#[derive(Debug, Clone)]
pub struct PlacedTree {
    pub id: String,
    pub nodes: Vec<PlacedNode>,
    /// Count of nodes that fell back to approximate (for the log line).
    pub approximate_count: usize,
}

impl PlacedTree {
    pub fn node(&self, id: &str) -> Option<&PlacedNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
    pub fn bounds(&self) -> (f32, f32, f32, f32) {
        let mut min_x = f32::MAX;
        let mut min_y = f32::MAX;
        let mut max_x = f32::MIN;
        let mut max_y = f32::MIN;
        for n in &self.nodes {
            min_x = min_x.min(n.px);
            min_y = min_y.min(n.py);
            max_x = max_x.max(n.px);
            max_y = max_y.max(n.py);
        }
        if self.nodes.is_empty() {
            (0.0, 0.0, 0.0, 0.0)
        } else {
            (min_x, min_y, max_x, max_y)
        }
    }
}

/// Resolve grid coordinates for one node, following `relative_position_id`
/// transitively with cycle protection. Returns (gx, gy) in grid units.
fn resolve_grid(
    id: &str,
    by_id: &HashMap<&str, &FocusNode>,
    memo: &mut HashMap<String, (i32, i32)>,
    visiting: &mut Vec<String>,
) -> (i32, i32) {
    if let Some(p) = memo.get(id) {
        return *p;
    }
    let node = match by_id.get(id) {
        Some(n) => *n,
        None => return (0, 0),
    };
    let pos = match &node.relative_to {
        None => (node.base_x, node.base_y),
        Some(anchor) => {
            if anchor == id || visiting.iter().any(|v| v == anchor) || by_id.get(anchor.as_str()).is_none() {
                // self-anchor, cycle, or missing anchor → fall back to base
                (node.base_x, node.base_y)
            } else {
                visiting.push(id.to_string());
                let (ax, ay) = resolve_grid(anchor, by_id, memo, visiting);
                visiting.pop();
                (ax + node.base_x, ay + node.base_y)
            }
        }
    };
    memo.insert(id.to_string(), pos);
    pos
}

/// Apply the offset evaluator to a node's grid position. Returns the (possibly
/// shifted) grid coords + the fidelity badge.
fn apply_offsets(node: &FocusNode, gx: i32, gy: i32, ctx: &EvalContext) -> (i32, i32, PosFidelity) {
    let mut fx = gx;
    let mut fy = gy;
    for off in &node.offsets {
        match &off.trigger {
            // unconditional offset (no trigger) — always applies
            None => {
                fx += off.dx;
                fy += off.dy;
            }
            Some(trig) => match eval_trigger(trig, ctx) {
                Eval::True => {
                    fx += off.dx;
                    fy += off.dy;
                    // first matching conditional offset wins; later ones ignored
                    return (fx, fy, PosFidelity::Exact);
                }
                Eval::False => { /* this branch not active; try next */ }
                Eval::Unsupported { reason } => {
                    // poison: do not guess — keep base, badge approximate
                    return (
                        gx,
                        gy,
                        PosFidelity::Approximate {
                            reason: format!("offset trigger `{reason}`"),
                        },
                    );
                }
            },
        }
    }
    (fx, fy, PosFidelity::Exact)
}

/// Lay out a tree: resolve relatives, apply offsets, convert to pixels.
pub fn place(tree: &FocusTree, ctx: &EvalContext) -> PlacedTree {
    let by_id: HashMap<&str, &FocusNode> =
        tree.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let mut memo: HashMap<String, (i32, i32)> = HashMap::new();
    let mut placed = Vec::with_capacity(tree.nodes.len());
    let mut approximate_count = 0;

    for node in &tree.nodes {
        let mut visiting = Vec::new();
        let (gx, gy) = resolve_grid(&node.id, &by_id, &mut memo, &mut visiting);
        let (fx, fy, fidelity) = apply_offsets(node, gx, gy, ctx);
        if let PosFidelity::Approximate { reason } = &fidelity {
            approximate_count += 1;
            eprintln!(
                "[focus_tree] node `{}` position APPROXIMATE: {reason} (fell back to base x/y)",
                node.id
            );
        }
        placed.push(PlacedNode {
            id: node.id.clone(),
            icon: node.icon.clone(),
            px: fx as f32 * GRID_X,
            py: fy as f32 * GRID_Y,
            gx: fx,
            gy: fy,
            fidelity,
            prerequisites: node.prerequisites.iter().map(|g| g.any_of.clone()).collect(),
            mutually_exclusive: node.mutually_exclusive.clone(),
            cost: node.cost,
        });
    }

    PlacedTree { id: tree.id.clone(), nodes: placed, approximate_count }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::trees_from_entries;
    use crate::pdx::parse_file;

    fn tree(src: &str) -> FocusTree {
        trees_from_entries(&parse_file(src)).into_iter().next().unwrap()
    }

    #[test]
    fn relative_position_chain_resolves() {
        // A at (10,0); B relative to A at (0,1) -> (10,1); C relative to B at
        // (5,0) -> (15,1).
        let t = tree(
            r#"focus_tree = { id = t
                focus = { id = A x = 10 y = 0 }
                focus = { id = B x = 0 y = 1 relative_position_id = A }
                focus = { id = C x = 5 y = 0 relative_position_id = B }
            }"#,
        );
        let p = place(&t, &EvalContext::default());
        assert_eq!((p.node("A").unwrap().gx, p.node("A").unwrap().gy), (10, 0));
        assert_eq!((p.node("B").unwrap().gx, p.node("B").unwrap().gy), (10, 1));
        assert_eq!((p.node("C").unwrap().gx, p.node("C").unwrap().gy), (15, 1));
        // pixel conversion
        assert_eq!(p.node("C").unwrap().px, 15.0 * GRID_X);
    }

    #[test]
    fn missing_anchor_falls_back_to_base() {
        let t = tree(
            r#"focus_tree = { id = t
                focus = { id = B x = 3 y = 4 relative_position_id = GHOST }
            }"#,
        );
        let p = place(&t, &EvalContext::default());
        assert_eq!((p.node("B").unwrap().gx, p.node("B").unwrap().gy), (3, 4));
    }

    #[test]
    fn cycle_is_broken() {
        let t = tree(
            r#"focus_tree = { id = t
                focus = { id = A x = 1 y = 1 relative_position_id = B }
                focus = { id = B x = 2 y = 2 relative_position_id = A }
            }"#,
        );
        // must terminate; both fall back somewhere finite
        let p = place(&t, &EvalContext::default());
        assert_eq!(p.nodes.len(), 2);
    }

    #[test]
    fn supported_offset_shifts_position() {
        let mut ctx = EvalContext { tag: "GER".into(), ..Default::default() };
        ctx.completed_focuses.insert("X".into());
        let t = tree(
            r#"focus_tree = { id = t
                focus = { id = A x = 10 y = 0
                    offset = { x = -3 y = 0 trigger = { tag = GER has_completed_focus = X } }
                }
            }"#,
        );
        let p = place(&t, &ctx);
        let a = p.node("A").unwrap();
        assert_eq!((a.gx, a.gy), (7, 0));
        assert_eq!(a.fidelity, PosFidelity::Exact);
        assert_eq!(p.approximate_count, 0);
    }

    #[test]
    fn unsupported_offset_falls_back_and_badges() {
        let ctx = EvalContext { tag: "GER".into(), ..Default::default() };
        let t = tree(
            r#"focus_tree = { id = t
                focus = { id = A x = 10 y = 0
                    offset = { x = -3 y = 0 trigger = { num_of_factories > 5 } }
                }
            }"#,
        );
        let p = place(&t, &ctx);
        let a = p.node("A").unwrap();
        // position stays at BASE (10,0); never fabricated to (7,0)
        assert_eq!((a.gx, a.gy), (10, 0));
        assert!(matches!(a.fidelity, PosFidelity::Approximate { .. }));
        assert_eq!(p.approximate_count, 1);
    }

    #[test]
    fn false_offset_keeps_base_but_stays_exact() {
        let ctx = EvalContext { tag: "USA".into(), ..Default::default() };
        let t = tree(
            r#"focus_tree = { id = t
                focus = { id = A x = 10 y = 0
                    offset = { x = -3 y = 0 trigger = { tag = GER } }
                }
            }"#,
        );
        let p = place(&t, &ctx);
        let a = p.node("A").unwrap();
        assert_eq!((a.gx, a.gy), (10, 0));
        // a FALSE (not unsupported) offset is exact — we KNOW it doesn't apply
        assert_eq!(a.fidelity, PosFidelity::Exact);
    }
}
