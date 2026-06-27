//! HIT-TESTING — front-to-back tree walk returning the topmost node under a
//! point, honouring `alwaystransparent` (pass-through) and clipping.
//!
//! Rules (Codex-refuted/confirmed 2026-06-26):
//! * Walk children in REVERSE declaration order (topmost-drawn first), since
//!   z-order = declaration order (later siblings draw on top).
//! * `alwaystransparent = yes` -> the node itself never captures the click (it
//!   passes through), BUT its children are STILL tested. This is exactly the v1
//!   "shine overlay ate the click" bug, inverted: get the pass-through right.
//! * **Clipping** is the only hard reject: a point outside an ancestor's clip
//!   rect cannot hit anything inside it. A non-clipping (group/zero-size)
//!   container does NOT reject its children just because the point is outside the
//!   container's own rect — Clausewitz containers are often zero-size groups
//!   whose children extend beyond them.

use crate::scene::Node;

/// The result of a hit-test: a path of node names from root to the hit leaf,
/// plus the leaf's index path (for the caller to resolve a datum/verb later).
#[derive(Debug, Clone, Default)]
pub struct Hit {
    /// Names root..leaf of the hit node's ancestry (inclusive of the leaf).
    pub path: Vec<String>,
}

impl Hit {
    /// The hit leaf's name (the deepest non-transparent node under the point).
    pub fn leaf(&self) -> Option<&str> {
        self.path.last().map(|s| s.as_str())
    }
}

/// Hit-test a single root node tree at screen point (px,py). Returns the topmost
/// non-`alwaystransparent` node under the point, or None.
pub fn hit_test(root: &Node, px: f32, py: f32) -> Option<Hit> {
    let mut path = Vec::new();
    if hit_node(root, px, py, &mut path) {
        Some(Hit { path })
    } else {
        None
    }
}

/// Hit-test a STACK of windows (front-to-back compositor order: last = topmost).
/// Returns the first hit from the topmost window down.
pub fn hit_test_stack(windows: &[Node], px: f32, py: f32) -> Option<Hit> {
    for w in windows.iter().rev() {
        if let Some(h) = hit_test(w, px, py) {
            return Some(h);
        }
    }
    None
}

/// Recursive helper. Pushes this node's name onto `path` if it (or a descendant)
/// is the hit; returns true if a hit was recorded in this subtree.
fn hit_node(node: &Node, px: f32, py: f32, path: &mut Vec<String>) -> bool {
    // Clipping hard-reject: if this node's clip rect (intersection of ancestor
    // clips, set during layout) excludes the point, nothing here can be hit.
    if let Some(clip) = node.clip {
        if !clip.contains(px, py) {
            return false;
        }
    }

    // Children first (topmost-drawn = last declared), front-to-back.
    for child in node.children.iter().rev() {
        path.push(node.name.clone());
        if hit_node(child, px, py, path) {
            return true;
        }
        path.pop();
    }

    // No child captured. This node captures iff the point is in its own rect,
    // it is not pass-through, AND it is a hit-capable surface (Codex-corrected:
    // a pure coordinate-frame container does NOT self-capture, so a full-screen
    // `topbar` window passes the click through to whatever is behind it). A
    // zero-size / group container with no rect hit also simply doesn't capture.
    if !node.alwaystransparent && node.is_hit_capable() && node.rect.contains(px, py) {
        path.push(node.name.clone());
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Anchor, ElementKind, Node, Rect};

    fn leaf(name: &str, rect: Rect, transparent: bool) -> Node {
        Node {
            name: name.into(),
            raw_kind: "iconType".into(),
            kind: ElementKind::Icon {
                sprite: "s".into(),
                frame: 0,
            },
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

    fn container(name: &str, rect: Rect, children: Vec<Node>) -> Node {
        let mut c = leaf(name, rect, false);
        c.raw_kind = "containerWindowType".into();
        c.kind = ElementKind::Container { background: None };
        c.children = children;
        c
    }

    const R: fn(f32, f32, f32, f32) -> Rect = |x, y, w, h| Rect { x, y, w, h };

    #[test]
    fn topmost_sibling_wins() {
        // two overlapping leaves; the later-declared one is on top.
        let root = container(
            "root",
            R(0.0, 0.0, 100.0, 100.0),
            vec![
                leaf("under", R(0.0, 0.0, 100.0, 100.0), false),
                leaf("over", R(0.0, 0.0, 100.0, 100.0), false),
            ],
        );
        let h = hit_test(&root, 50.0, 50.0).unwrap();
        assert_eq!(h.leaf(), Some("over"));
    }

    #[test]
    fn alwaystransparent_passes_through_to_lower_sibling() {
        // the v1 "shine ate the click" case: top element is transparent -> the
        // click falls through to the button beneath it.
        let root = container(
            "root",
            R(0.0, 0.0, 100.0, 100.0),
            vec![
                leaf("button", R(0.0, 0.0, 100.0, 100.0), false),
                leaf("shine", R(0.0, 0.0, 100.0, 100.0), true), // transparent overlay
            ],
        );
        let h = hit_test(&root, 50.0, 50.0).unwrap();
        assert_eq!(h.leaf(), Some("button"));
    }

    #[test]
    fn transparent_node_children_still_tested() {
        // a transparent container with a real child -> the child captures.
        let mut transparent_container = container(
            "tcont",
            R(0.0, 0.0, 100.0, 100.0),
            vec![leaf("inner", R(10.0, 10.0, 30.0, 30.0), false)],
        );
        transparent_container.alwaystransparent = true;
        let h = hit_test(&transparent_container, 20.0, 20.0).unwrap();
        assert_eq!(h.leaf(), Some("inner"));
        // a point only over the transparent container (not the child) -> no hit
        assert!(hit_test(&transparent_container, 90.0, 90.0).is_none());
    }

    #[test]
    fn zero_size_group_does_not_reject_outside_children() {
        // a zero-size container whose child extends outside it (Codex caveat):
        // a point over the child still hits even though it's outside the parent.
        let group = container(
            "group",
            R(0.0, 0.0, 0.0, 0.0),
            vec![leaf("child", R(200.0, 50.0, 40.0, 40.0), false)],
        );
        let h = hit_test(&group, 210.0, 60.0).unwrap();
        assert_eq!(h.leaf(), Some("child"));
    }

    #[test]
    fn clipping_hard_rejects_outside_point() {
        // a clipping container: a child drawn outside the clip is NOT hittable.
        let mut clip = container(
            "clip",
            R(0.0, 0.0, 100.0, 100.0),
            vec![leaf("child", R(200.0, 50.0, 40.0, 40.0), false)],
        );
        clip.clipping = true;
        clip.clip = Some(R(0.0, 0.0, 100.0, 100.0));
        clip.children[0].clip = Some(R(0.0, 0.0, 100.0, 100.0));
        // point over the child but outside the clip rect -> no hit
        assert!(hit_test(&clip, 210.0, 60.0).is_none());
    }

    #[test]
    fn stack_topmost_window_wins() {
        // windows with a background surface DO capture (a real opaque panel);
        // the topmost (last-stacked) window wins.
        let mut under = container("w_under", R(0.0, 0.0, 100.0, 100.0), vec![]);
        let mut over = container("w_over", R(0.0, 0.0, 100.0, 100.0), vec![]);
        under.kind = ElementKind::Container {
            background: Some("bg".into()),
        };
        over.kind = ElementKind::Container {
            background: Some("bg".into()),
        };
        let h = hit_test_stack(&[under, over], 50.0, 50.0).unwrap();
        assert_eq!(h.leaf(), Some("w_over"));
    }

    #[test]
    fn pure_container_passes_through_to_what_is_behind() {
        // a full-screen pure container (no background) must NOT capture — the
        // click falls through (e.g. the topbar window over the map). With no
        // children and no background, there is no hit at all.
        let frame = container("topbar", R(0.0, 0.0, 1920.0, 1080.0), vec![]);
        assert!(hit_test(&frame, 960.0, 540.0).is_none());
    }
}
