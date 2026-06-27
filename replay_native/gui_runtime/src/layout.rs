//! The LAYOUT MODEL — resolve each scene-graph node to an absolute pixel rect.
//!
//! Top-down: each node is given its parent's resolved rect; it computes its own
//! anchor point inside the parent (`orientation`), its size, then its top-left
//! (`origo`), recurses, and propagates the clip rect.
//!
//! The trickiest rules (Codex-refuted and corrected, 2026-06-26):
//!
//! * **orientation** picks WHICH point of the PARENT box the child anchors to;
//!   **origo** picks WHICH point of the CHILD's own box sits on that anchor.
//!   Both use the same 9-anchor `(fx,fy)` fractions ([`Anchor::fractions`]).
//!   Default origo = `UPPER_LEFT` (NOT "same as orientation").
//!
//! * **position is a raw screen-axis offset** added to the anchor — there is NO
//!   sign flip for right/bottom orientations. `UPPER_RIGHT` + `x:-399` lands 399
//!   px LEFT of the parent's right edge.
//!
//! * **percent / negative sizes fill to a parent-relative FAR EDGE** from the
//!   positioned reference point (Codex's key correction):
//!     `ref = anchor + position`  (origo not yet applied)
//!     `width = P/100 * parent_w - ref_x`            for `"N%%"`
//!     `width = parent_w + neg     - ref_x`          for a NEGATIVE px size
//!   So the root `topbar` (`pos.x=-4`, `width="100%%"`) resolves to width
//!   `screen_w + 4`, its right edge landing exactly on the screen edge.
//!
//! * **size `-1` / `0` / absent** on a sprite element -> the sprite's native
//!   frame size (content size). On a container -> a zero-size group (children
//!   may still extend outside; only `clipping` rejects them).

use crate::model::Scalar;
use crate::scene::{ElementKind, Node, Rect};
use assets::SpriteManifest;

/// How a node's intrinsic (content) size is found when size is auto/absent.
pub trait ContentSize {
    /// Native (width,height) for a node, if it has intrinsic content (a sprite
    /// or measured text). Returns None for a pure container.
    fn content_size(&self, node: &Node) -> Option<(f32, f32)>;
}

/// Content-size source backed by the sprite manifest + a text measurer.
pub struct ManifestContent<'a, M: Fn(&str, &str) -> Option<f32>> {
    pub manifest: &'a SpriteManifest,
    /// Measure a string's pixel width given (text, font); None if no font loaded.
    pub measure: M,
}

impl<'a, M: Fn(&str, &str) -> Option<f32>> ContentSize for ManifestContent<'a, M> {
    fn content_size(&self, node: &Node) -> Option<(f32, f32)> {
        match &node.kind {
            ElementKind::Icon { sprite, .. }
            | ElementKind::Background { sprite }
            | ElementKind::NineSlice { sprite }
            | ElementKind::ProgressBar { sprite, .. } => sprite_size(self.manifest, sprite),
            ElementKind::Button { sprite, text, .. } => {
                if let Some(s) = sprite {
                    if let Some(sz) = sprite_size(self.manifest, s) {
                        return Some(sz);
                    }
                }
                text.as_ref()
                    .map(|t| (t.max_width.unwrap_or(0.0), t.max_height.unwrap_or(0.0)))
            }
            ElementKind::Text(t) => {
                // A text box's content box is maxWidth x maxHeight when given,
                // else the measured single line.
                let w = t
                    .max_width
                    .or_else(|| (self.measure)(&t.text, &t.font))
                    .unwrap_or(0.0);
                let h = t.max_height.unwrap_or(0.0);
                Some((w, h))
            }
            ElementKind::Container { background } => background
                .as_ref()
                .and_then(|s| sprite_size(self.manifest, s)),
        }
    }
}

/// Native size of a sprite: per-frame width (`texture_w / no_of_frames`) x
/// height. Falls back to the manifest `size` then 0.
fn sprite_size(manifest: &SpriteManifest, name: &str) -> Option<(f32, f32)> {
    let e = manifest.get(name)?;
    let frames = e.no_of_frames.max(1);
    if let (Some(w), Some(h)) = (e.width, e.height) {
        return Some((w as f32 / frames as f32, h as f32));
    }
    if let Some(sz) = e.size {
        return Some((sz[0] as f32, sz[1] as f32));
    }
    None
}

/// Resolve one size axis to pixels, applying the percent/negative far-edge rule.
/// `ref_axis` = the positioned reference (anchor + position) on this axis, in
/// ABSOLUTE screen coords; `parent_origin` = the parent's x/y on this axis;
/// `parent_extent` = parent's width/height; `content` = native content size on
/// this axis (used only for auto/0/-1/absent). Returns the resolved pixel extent.
///
/// The far-edge rules are measured against the PARENT's local frame, so they must
/// subtract the parent origin from the absolute reference (Codex FIX-A): the prior
/// code dropped `parent_origin`, which only happened to be correct when the parent
/// sat at the screen origin (x=y=0) — nested parents resolved to the wrong far edge.
fn resolve_axis(
    raw: Option<&Scalar>,
    ref_axis: f32,
    parent_origin: f32,
    parent_extent: f32,
    content: Option<f32>,
) -> f32 {
    // the reference point EXPRESSED IN THE PARENT'S LOCAL FRAME.
    let local_ref = ref_axis - parent_origin;
    match raw {
        Some(Scalar::Str(s)) => {
            let t = s.trim();
            // percent: "N%%" or "N%"
            let pct = t.trim_end_matches('%');
            if t.ends_with('%') {
                if let Ok(p) = pct.trim().parse::<f32>() {
                    return (p / 100.0) * parent_extent - local_ref;
                }
            }
            // a numeric string (possibly negative)
            if let Ok(v) = t.parse::<f32>() {
                return px_or_faredge(v, local_ref, parent_extent, content);
            }
            content.unwrap_or(0.0)
        }
        Some(Scalar::Int(i)) => px_or_faredge(*i as f32, local_ref, parent_extent, content),
        Some(Scalar::Float(f)) => px_or_faredge(*f as f32, local_ref, parent_extent, content),
        Some(Scalar::Bool(_)) | None => content.unwrap_or(0.0),
    }
}

/// A positive px size is literal; `0` or `-1` mean auto (content/sprite native
/// size, per the documented Clausewitz model — Codex FIX-B: a literal `0` used to
/// resolve to a 0x0 extent, making a sprite vanish + un-hittable); any other
/// NEGATIVE px is a far-edge fill (`parent_extent + neg - local_ref`).
/// `local_ref` is the positioned reference IN THE PARENT'S LOCAL FRAME.
fn px_or_faredge(v: f32, local_ref: f32, parent_extent: f32, content: Option<f32>) -> f32 {
    if v == -1.0 || v == 0.0 {
        content.unwrap_or(0.0)
    } else if v > 0.0 {
        v
    } else {
        parent_extent + v - local_ref
    }
}

/// Lay out a node tree in place. `parent` is the parent's resolved rect; `clip`
/// is the clip rect inherited from ancestors (None = unclipped). The root call
/// passes the full screen rect as `parent` and `None` clip.
pub fn layout<C: ContentSize>(node: &mut Node, parent: Rect, clip: Option<Rect>, content: &C) {
    // 1. anchor point inside the parent (orientation).
    let (ofx, ofy) = node.orientation.fractions();
    let anchor_x = parent.x + ofx * parent.w;
    let anchor_y = parent.y + ofy * parent.h;

    // 2. positioned reference point (anchor + raw position). origo NOT yet
    //    applied — the percent/far-edge size rule is defined against this point.
    let ref_x = anchor_x + node.pos.0;
    let ref_y = anchor_y + node.pos.1;

    // 3. size. Auto/absent/-1 -> content size; percent/negative -> far edge.
    let csize = content.content_size(node);
    let w = resolve_axis(
        node.size_w.as_ref(),
        ref_x,
        parent.x,
        parent.w,
        csize.map(|c| c.0),
    );
    let h = resolve_axis(
        node.size_h.as_ref(),
        ref_y,
        parent.y,
        parent.h,
        csize.map(|c| c.1),
    );

    // 4. apply origo: the child's own (gx,gy) corner sits on the anchor; back
    //    the top-left off by the origo fraction of the child's own size.
    let (gfx, gfy) = node.origo.fractions();
    let left = ref_x - gfx * w;
    let top = ref_y - gfy * h;

    node.rect = Rect {
        x: left,
        y: top,
        w,
        h,
    };

    // 5. clip propagation: a clipping node intersects its rect into the clip
    //    stack; the child clip is what descendants inherit.
    let child_clip = if node.clipping {
        Some(match clip {
            Some(c) => c.intersect(&node.rect),
            None => node.rect,
        })
    } else {
        clip
    };
    node.clip = child_clip;

    // 6. recurse. Children anchor inside THIS node's rect.
    for child in node.children.iter_mut() {
        layout(child, node.rect, child_clip, content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{Anchor, ElementKind, Rect};

    fn n(kind: ElementKind) -> Node {
        Node {
            name: "t".into(),
            raw_kind: "containerWindowType".into(),
            tooltip: None,
            kind,
            pos: (0.0, 0.0),
            size_w: None,
            size_h: None,
            orientation: Anchor::UpperLeft,
            origo: Anchor::UpperLeft,
            clipping: false,
            alwaystransparent: false,
            rect: Rect::default(),
            clip: None,
            children: vec![],
        }
    }

    struct NoContent;
    impl ContentSize for NoContent {
        fn content_size(&self, _: &Node) -> Option<(f32, f32)> {
            None
        }
    }
    struct Fixed(f32, f32);
    impl ContentSize for Fixed {
        fn content_size(&self, _: &Node) -> Option<(f32, f32)> {
            Some((self.0, self.1))
        }
    }

    const SCREEN: Rect = Rect {
        x: 0.0,
        y: 0.0,
        w: 1920.0,
        h: 1080.0,
    };

    #[test]
    fn root_percent_with_negative_pos_lands_right_edge_on_screen() {
        // topbar: pos(-4,-1), size 100%%/100%% -> rect (-4,-1) sized 1924x1081,
        // right edge = -4 + 1924 = 1920 = screen edge.
        let mut root = n(ElementKind::Container { background: None });
        root.pos = (-4.0, -1.0);
        root.size_w = Some(Scalar::Str("100%%".into()));
        root.size_h = Some(Scalar::Str("100%%".into()));
        layout(&mut root, SCREEN, None, &NoContent);
        assert_eq!((root.rect.x, root.rect.y), (-4.0, -1.0));
        assert_eq!(root.rect.w, 1924.0);
        assert_eq!(root.rect.x + root.rect.w, 1920.0);
        assert_eq!(root.rect.y + root.rect.h, 1080.0);
    }

    #[test]
    fn upper_right_negative_x_lands_left_of_right_edge_no_sign_flip() {
        let parent = Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 100.0,
        };
        let mut c = n(ElementKind::Icon {
            sprite: "s".into(),
            frame: 0,
        });
        c.orientation = Anchor::UpperRight;
        c.pos = (-399.0, -2.0);
        c.size_w = Some(Scalar::Int(50));
        c.size_h = Some(Scalar::Int(20));
        layout(&mut c, parent, None, &NoContent);
        // anchor = right edge = 1000; left = 1000 + (-399) = 601
        assert_eq!(c.rect.x, 601.0);
        assert_eq!(c.rect.y, -2.0);
    }

    #[test]
    fn origo_center_centers_child_on_anchor() {
        let parent = Rect {
            x: 0.0,
            y: 0.0,
            w: 200.0,
            h: 200.0,
        };
        let mut c = n(ElementKind::Icon {
            sprite: "s".into(),
            frame: 0,
        });
        c.orientation = Anchor::Center; // anchor at (100,100)
        c.origo = Anchor::Center; // child centered on anchor
        c.size_w = Some(Scalar::Int(40));
        c.size_h = Some(Scalar::Int(20));
        layout(&mut c, parent, None, &NoContent);
        // left = 100 - 0.5*40 = 80 ; top = 100 - 0.5*20 = 90
        assert_eq!((c.rect.x, c.rect.y), (80.0, 90.0));
    }

    #[test]
    fn lower_left_anchors_to_parent_bottom() {
        let parent = Rect {
            x: 0.0,
            y: 0.0,
            w: 500.0,
            h: 300.0,
        };
        let mut c = n(ElementKind::Icon {
            sprite: "s".into(),
            frame: 0,
        });
        c.orientation = Anchor::LowerLeft; // anchor (0,300)
        c.pos = (188.0, -22.0);
        c.size_w = Some(Scalar::Int(40));
        c.size_h = Some(Scalar::Int(20));
        layout(&mut c, parent, None, &NoContent);
        assert_eq!(c.rect.x, 188.0);
        assert_eq!(c.rect.y, 278.0); // 300 - 22
    }

    #[test]
    fn auto_size_uses_content() {
        let parent = Rect {
            x: 0.0,
            y: 0.0,
            w: 500.0,
            h: 300.0,
        };
        let mut c = n(ElementKind::Icon {
            sprite: "s".into(),
            frame: 0,
        });
        // no explicit size -> content size
        layout(&mut c, parent, None, &Fixed(27.0, 27.0));
        assert_eq!((c.rect.w, c.rect.h), (27.0, 27.0));
    }

    #[test]
    fn negative_px_size_fills_to_far_edge() {
        let parent = Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 100.0,
        };
        let mut c = n(ElementKind::Container { background: None });
        c.pos = (10.0, 0.0);
        c.size_w = Some(Scalar::Int(-15)); // right edge 15px before parent right
        layout(&mut c, parent, None, &NoContent);
        // ref_x = 0 + 10 = 10; w = 1000 + (-15) - 10 = 975; right = 10+975 = 985 = 1000-15
        assert_eq!(c.rect.w, 975.0);
        assert_eq!(c.rect.x + c.rect.w, 985.0);
    }

    #[test]
    fn percent_far_edge_honours_nonzero_parent_origin() {
        // BUG (Codex FIX-A): the percent/negative far-edge rule dropped parent.x/y,
        // so for a NESTED parent (origin != 0) the far edge landed at the wrong
        // place. A child with pos.x=10 and width="100%%" inside a parent at x=100
        // w=500 must have its RIGHT edge at the parent's right (100+500=600), i.e.
        // width = 600 - (100 + 10) = 490 — NOT the old (500 - 110 = 390).
        let parent = Rect {
            x: 100.0,
            y: 50.0,
            w: 500.0,
            h: 300.0,
        };
        let mut c = n(ElementKind::Container { background: None });
        c.pos = (10.0, 5.0);
        c.size_w = Some(Scalar::Str("100%%".into()));
        c.size_h = Some(Scalar::Str("100%%".into()));
        layout(&mut c, parent, None, &NoContent);
        assert_eq!(
            c.rect.w, 490.0,
            "100%% width must reach the parent's right edge"
        );
        assert_eq!(
            c.rect.x + c.rect.w,
            600.0,
            "right edge lands on the parent right edge"
        );
        assert_eq!(
            c.rect.h, 295.0,
            "100%% height must reach the parent's bottom edge"
        );
        assert_eq!(
            c.rect.y + c.rect.h,
            350.0,
            "bottom edge lands on the parent bottom edge"
        );
    }

    #[test]
    fn negative_px_far_edge_honours_nonzero_parent_origin() {
        // Same FIX-A for a NEGATIVE px size inside a non-zero-origin parent: -15
        // means "15px before the parent's RIGHT edge". Parent x=100 w=1000 -> right
        // edge 1100; child pos.x=10 -> ref_x=110; right edge should be 1100-15=1085,
        // so width = 1085 - 110 = 975 (NOT the old 1000-15-110 = 875).
        let parent = Rect {
            x: 100.0,
            y: 0.0,
            w: 1000.0,
            h: 100.0,
        };
        let mut c = n(ElementKind::Container { background: None });
        c.pos = (10.0, 0.0);
        c.size_w = Some(Scalar::Int(-15));
        layout(&mut c, parent, None, &NoContent);
        assert_eq!(c.rect.w, 975.0);
        assert_eq!(
            c.rect.x + c.rect.w,
            1085.0,
            "right edge = parent right - 15"
        );
    }

    #[test]
    fn explicit_size_zero_uses_content_like_minus_one() {
        // BUG (Codex FIX-B): the docstring says "size -1 / 0 / absent -> native
        // content size", but a literal 0 was treated as a literal zero extent, so a
        // sprite with size 0 vanished (0x0, un-hittable). A 0 must resolve to the
        // content size, exactly like -1.
        let parent = Rect {
            x: 0.0,
            y: 0.0,
            w: 500.0,
            h: 300.0,
        };
        let mut c = n(ElementKind::Icon {
            sprite: "s".into(),
            frame: 0,
        });
        c.size_w = Some(Scalar::Int(0));
        c.size_h = Some(Scalar::Int(0));
        layout(&mut c, parent, None, &Fixed(27.0, 27.0));
        assert_eq!(
            (c.rect.w, c.rect.h),
            (27.0, 27.0),
            "size 0 -> native content size"
        );
    }

    #[test]
    fn clipping_intersects_nested() {
        let mut root = n(ElementKind::Container { background: None });
        root.size_w = Some(Scalar::Int(100));
        root.size_h = Some(Scalar::Int(100));
        root.clipping = true;
        let mut child = n(ElementKind::Container { background: None });
        child.pos = (50.0, 50.0);
        child.size_w = Some(Scalar::Int(100));
        child.size_h = Some(Scalar::Int(100));
        child.clipping = true;
        root.children.push(child);
        layout(&mut root, SCREEN, None, &NoContent);
        let cc = root.children[0].clip.unwrap();
        // child clip = intersect((0,0,100,100),(50,50,100,100)) = (50,50,50,50)
        assert_eq!((cc.x, cc.y, cc.w, cc.h), (50.0, 50.0, 50.0, 50.0));
    }
}
