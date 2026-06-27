//! The focus node+edge GRAPH model and the file → graph builder.
//!
//! Each `focus = { … }` becomes a [`FocusNode`] with its base grid position
//! (`x`/`y`, in 1-unit grid steps), an optional `relative_position_id` anchor,
//! its conditional `offset` branches, its icon, cost, and the edges harvested
//! from `prerequisite = { focus = … }` (AND-of-ORs) and `mutually_exclusive`.
//!
//! Positions here are still GRID positions (relative chains UNRESOLVED). The
//! [`crate::layout`] pass resolves `relative_position_id` chains + applies the
//! offset evaluator (§E) to produce absolute pixel positions.

use crate::pdx::{Entry, Value};

/// A single conditional `offset = { x y trigger = { … } }` branch.
#[derive(Debug, Clone)]
pub struct Offset {
    pub dx: i32,
    pub dy: i32,
    /// The raw trigger block (evaluated by [`crate::eval`]). `None` = no trigger
    /// (an unconditional offset; rare, applied always).
    pub trigger: Option<Value>,
}

/// A prerequisite group: satisfied iff ANY of its focus ids is completed
/// (Clausewitz `prerequisite = { focus = A focus = B }` is an OR). Multiple
/// `prerequisite` lines AND together.
#[derive(Debug, Clone)]
pub struct PrereqGroup {
    pub any_of: Vec<String>,
}

/// One focus node parsed from the script (geometry still in grid units).
#[derive(Debug, Clone)]
pub struct FocusNode {
    pub id: String,
    pub icon: Option<String>,
    pub base_x: i32,
    pub base_y: i32,
    pub relative_to: Option<String>,
    pub cost: Option<f32>,
    pub offsets: Vec<Offset>,
    pub prerequisites: Vec<PrereqGroup>,
    pub mutually_exclusive: Vec<String>,
}

/// A whole parsed focus tree (one `focus_tree = { id = … }` block).
#[derive(Debug, Clone)]
pub struct FocusTree {
    pub id: String,
    pub nodes: Vec<FocusNode>,
}

impl FocusTree {
    pub fn node(&self, id: &str) -> Option<&FocusNode> {
        self.nodes.iter().find(|n| n.id == id)
    }
}

fn scalar_i32(v: &Value, key: &str) -> Option<i32> {
    v.get_scalar(key)
        .and_then(|s| s.parse::<f32>().ok())
        .map(|f| f as i32)
}

fn parse_offset(block: &Value) -> Offset {
    Offset {
        dx: scalar_i32(block, "x").unwrap_or(0),
        dy: scalar_i32(block, "y").unwrap_or(0),
        trigger: block.get("trigger").cloned(),
    }
}

fn parse_focus(block: &Value) -> Option<FocusNode> {
    let id = block.get_scalar("id")?.to_string();
    let mut prerequisites = Vec::new();
    for e in block.get_all("prerequisite") {
        let any_of: Vec<String> = e
            .value
            .get_all("focus")
            .filter_map(|fe| fe.value.as_scalar().map(|s| s.to_string()))
            .collect();
        if !any_of.is_empty() {
            prerequisites.push(PrereqGroup { any_of });
        }
    }
    let mut mutually_exclusive = Vec::new();
    for e in block.get_all("mutually_exclusive") {
        for fe in e.value.get_all("focus") {
            if let Some(s) = fe.value.as_scalar() {
                mutually_exclusive.push(s.to_string());
            }
        }
    }
    let offsets: Vec<Offset> = block
        .get_all("offset")
        .map(|e| parse_offset(&e.value))
        .collect();

    Some(FocusNode {
        id,
        icon: block.get_scalar("icon").map(|s| s.to_string()),
        base_x: scalar_i32(block, "x").unwrap_or(0),
        base_y: scalar_i32(block, "y").unwrap_or(0),
        relative_to: block
            .get_scalar("relative_position_id")
            .map(|s| s.to_string()),
        cost: block.get_scalar("cost").and_then(|s| s.parse().ok()),
        offsets,
        prerequisites,
        mutually_exclusive,
    })
}

/// Build all focus trees defined in a set of parsed top-level entries (one file
/// may define several `focus_tree` blocks; `shared_focus` blocks at top level
/// are also harvested into a synthetic catalog so cross-file anchors resolve).
pub fn trees_from_entries(top: &[Entry]) -> Vec<FocusTree> {
    let mut out = Vec::new();
    for e in top {
        if e.key == "focus_tree" {
            let id = e.value.get_scalar("id").unwrap_or("unknown").to_string();
            let nodes: Vec<FocusNode> = e
                .value
                .get_all("focus")
                .filter_map(|fe| parse_focus(&fe.value))
                .collect();
            out.push(FocusTree { id, nodes });
        }
    }
    out
}

/// Harvest top-level `shared_focus = { … }` blocks (defined outside any
/// `focus_tree`, referenced by trees that `shared_focus = ID`). Returned as a
/// flat id→node catalog so a tree can splice them in.
pub fn shared_focuses(top: &[Entry]) -> Vec<FocusNode> {
    top.iter()
        .filter(|e| e.key == "shared_focus")
        .filter_map(|e| parse_focus(&e.value))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdx::parse_file;

    #[test]
    fn builds_nodes_and_edges() {
        let src = r#"
            focus_tree = {
                id = t
                focus = { id = A icon = GFX_a x = 10 y = 0 cost = 5 }
                focus = { id = B x = 0 y = 1
                    relative_position_id = A
                    prerequisite = { focus = A }
                    mutually_exclusive = { focus = C }
                    offset = { x = -2 y = 0 trigger = { tag = GER } }
                }
            }
        "#;
        let top = parse_file(src);
        let trees = trees_from_entries(&top);
        assert_eq!(trees.len(), 1);
        let t = &trees[0];
        assert_eq!(t.nodes.len(), 2);
        let b = t.node("B").unwrap();
        assert_eq!(b.relative_to.as_deref(), Some("A"));
        assert_eq!(b.prerequisites[0].any_of, vec!["A".to_string()]);
        assert_eq!(b.mutually_exclusive, vec!["C".to_string()]);
        assert_eq!(b.offsets.len(), 1);
        assert_eq!(b.offsets[0].dx, -2);
        assert!(b.offsets[0].trigger.is_some());
        let a = t.node("A").unwrap();
        assert_eq!(a.icon.as_deref(), Some("GFX_a"));
        assert_eq!(a.cost, Some(5.0));
    }

    #[test]
    fn or_prerequisite_group() {
        let src = r#"
            focus_tree = { id = t
                focus = { id = X x = 0 y = 0
                    prerequisite = { focus = A focus = B }
                    prerequisite = { focus = C }
                }
            }
        "#;
        let top = parse_file(src);
        let t = &trees_from_entries(&top)[0];
        let x = t.node("X").unwrap();
        assert_eq!(x.prerequisites.len(), 2);
        assert_eq!(
            x.prerequisites[0].any_of,
            vec!["A".to_string(), "B".to_string()]
        );
        assert_eq!(x.prerequisites[1].any_of, vec!["C".to_string()]);
    }
}
