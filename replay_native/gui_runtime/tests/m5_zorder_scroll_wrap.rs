//! M5 integration test — the three Codex-required foundations, exercised through
//! the PUBLIC `gui_runtime` API exactly as the m5_demo uses them:
//!
//!   (a) the gridbox ROW-FACTORY: one instance per array element, correct per-row
//!       grid layout, the viewport CLIP, and the SCROLL offset;
//!   (b) the WRAPPED, marked-up text whose wrap is computed from glyph metrics
//!       AFTER `$VAR$`/`§colour`/`£icon` expansion;
//!   (c) the Z-ORDER / `alwaystransparent` hit-test: a click on a painted panel
//!       element hits the panel; a click on a TRANSPARENT panel region falls
//!       through to the map behind; overlapping windows resolve front-to-back.
//!
//! This is the deterministic, GPU-free regression of M5 deliverable #5 (the demo
//! binary runs the same checks against the REAL assets + snapshot).

use std::collections::HashMap;

use assets::bmfont::Glyph;
use assets::BmFont;
use gui_runtime::gridbox::{self, GridSpec};
use gui_runtime::markup::{expand, wrap};
use gui_runtime::scene::{Anchor, ElementKind, Node, Rect, TextAlign, TextSpec};
use gui_runtime::{hit_test_stack, ContentSize};

// ---------------------------------------------------------------------------
// test scaffolding
// ---------------------------------------------------------------------------

fn test_font() -> BmFont {
    let mut glyphs = HashMap::new();
    for cp in 32u32..=126 {
        let adv = if cp == 32 { 6 } else { 10 };
        glyphs.insert(
            cp,
            Glyph {
                id: cp,
                page: 0,
                x: 0,
                y: 0,
                width: if cp == 32 { 0 } else { 8 },
                height: 10,
                xoffset: 0,
                yoffset: 0,
                xadvance: adv,
                chnl: 15,
            },
        );
    }
    BmFont {
        face: "t".into(),
        size: 14,
        line_height: 16,
        base: 12,
        scale_w: 256,
        scale_h: 256,
        page_files: Default::default(),
        page_paths: Default::default(),
        glyphs,
        kernings: Default::default(),
    }
}

struct SizedContent;
impl ContentSize for SizedContent {
    fn content_size(&self, node: &Node) -> Option<(f32, f32)> {
        match &node.kind {
            ElementKind::Text(_) => Some((40.0, 18.0)),
            ElementKind::Icon { .. } | ElementKind::Button { .. } => Some((32.0, 32.0)),
            _ => None,
        }
    }
}

fn leaf(name: &str, kind: ElementKind, rect: Rect, transparent: bool) -> Node {
    Node {
        name: name.into(),
        raw_kind: "iconType".into(),
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

fn icon(name: &str, rect: Rect) -> Node {
    leaf(
        name,
        ElementKind::Icon {
            sprite: "s".into(),
            frame: 0,
        },
        rect,
        false,
    )
}

fn building_template() -> Node {
    let level = Node {
        name: "level".into(),
        raw_kind: "instantTextBoxType".into(),
        kind: ElementKind::Text(TextSpec {
            text: "0".into(),
            raw: "0".into(),
            font: "f".into(),
            align: TextAlign::Center,
            max_width: Some(60.0),
            max_height: Some(20.0),
        }),
        pos: (5.0, 54.0),
        size_w: None,
        size_h: None,
        orientation: Anchor::UpperLeft,
        origo: Anchor::UpperLeft,
        clipping: false,
        alwaystransparent: false,
        rect: Rect::default(),
        clip: None,
        children: vec![],
    };
    let pic = leaf(
        "building_picture",
        ElementKind::Button {
            sprite: Some("GFX_buildings_strip".into()),
            frame: 0,
            text: None,
        },
        Rect::default(),
        false,
    );
    Node {
        name: "state_building_entry".into(),
        raw_kind: "containerWindowType".into(),
        kind: ElementKind::Container {
            background: Some("GFX_entry_bg".into()),
        },
        pos: (0.0, 0.0),
        size_w: Some(gui_runtime::model::Scalar::Int(140)),
        size_h: Some(gui_runtime::model::Scalar::Int(70)),
        orientation: Anchor::UpperLeft,
        origo: Anchor::UpperLeft,
        clipping: false,
        alwaystransparent: false,
        rect: Rect::default(),
        clip: None,
        children: vec![pic, level],
    }
}

// ---------------------------------------------------------------------------
// (a) gridbox row-factory: per-element instance, per-row layout, clip, scroll
// ---------------------------------------------------------------------------

#[test]
fn foundation_a_gridbox_row_factory_overflow_clip_scroll() {
    let tmpl = building_template();
    // a 480x105 viewport = 3 columns × 1.5 visible rows (cell 140x70).
    let viewport = Rect {
        x: 193.0,
        y: 155.0,
        w: 480.0,
        h: 105.0,
    };
    let mut gb = tmpl.clone();
    gb.rect = viewport;
    let spec = GridSpec::from_gridbox(&gb, &tmpl, 14.0);
    assert_eq!(spec.columns, 3, "480px box / 140px cell = 3 columns");

    // 10 buildings (the real Brandenburg count: 7 state + 3 province) overflow.
    let n = 10;
    let inst0 = gridbox::instantiate(&tmpl, n, viewport, &spec, 0.0, &SizedContent);
    assert_eq!(inst0.rows.len(), n, "exactly one instance per element");
    // 10/3 = 4 grid-rows × 70 = 280 content height > 105 viewport -> overflow.
    assert_eq!(inst0.content_height, 280.0);
    assert!(inst0.overflowing);
    assert!(inst0.scrollbar.is_some());

    // per-row grid layout: element 0 col0 row0, element 4 col1 row1, etc.
    assert_eq!(inst0.rows[0].rect.x, viewport.x);
    assert_eq!(inst0.rows[0].rect.y, viewport.y);
    assert_eq!(inst0.rows[4].rect.x, viewport.x + 140.0); // col1
    assert_eq!(inst0.rows[4].rect.y, viewport.y + 70.0); // grid-row1
                                                         // every row carries the viewport clip, including the building_picture child.
    for r in &inst0.rows {
        assert_eq!(r.clip, Some(viewport));
        assert_eq!(r.children[0].clip, Some(viewport));
    }

    // SCROLL: scroll down by one grid-row (70px) shifts every row up by 70.
    let scrolled = gridbox::instantiate(&tmpl, n, viewport, &spec, 70.0, &SizedContent);
    for i in 0..n {
        assert_eq!(
            scrolled.rows[i].rect.y,
            inst0.rows[i].rect.y - 70.0,
            "row {i} not uniformly scroll-shifted"
        );
    }
    // the top row is now ABOVE the viewport; a hit there must be clip-rejected.
    let top = &scrolled.rows[0];
    assert!(top.rect.y < viewport.y);
    let inside_row_but_above_vp = (top.rect.x + 5.0, top.rect.y + 5.0);
    assert!(
        hit_test_stack(
            std::slice::from_ref(top),
            inside_row_but_above_vp.0,
            inside_row_but_above_vp.1
        )
        .is_none(),
        "a scrolled-out row must be rejected by the viewport clip"
    );
    // a row scrolled INTO view (grid-row1 -> now at the top) IS hittable.
    let visible = &scrolled.rows[3]; // element 3 was grid-row1 col0 -> now y=vp.y
    assert_eq!(visible.rect.y, viewport.y);
    let pt = (visible.rect.x + 5.0, visible.rect.y + 5.0);
    assert!(hit_test_stack(std::slice::from_ref(visible), pt.0, pt.1).is_some());
}

// ---------------------------------------------------------------------------
// (b) wrapped, marked-up text: wrap from glyph metrics AFTER expansion
// ---------------------------------------------------------------------------

#[test]
fn foundation_b_wrap_uses_glyph_metrics_after_markup_expansion() {
    let font = test_font();
    let mut vars = HashMap::new();
    vars.insert("STATE".to_string(), "Brandenburg".to_string());
    vars.insert("CAT".to_string(), "megalopolis".to_string());

    // markup with a $VAR$, two §colour runs, and a £icon — exactly the demo's form.
    let src = "The state of §Y$STATE$§! is a §G$CAT$§! region £victory_points holding many points";
    let exp = expand(src, &vars, [0.9, 0.9, 0.86, 1.0]);

    // the EXPANDED plain text is markup-stripped + var-substituted (no §/£/$).
    assert!(!exp.plain.contains('§'));
    assert!(!exp.plain.contains('£'));
    assert!(!exp.plain.contains('$'));
    assert!(exp.plain.contains("Brandenburg"));
    assert!(exp.plain.contains("megalopolis"));

    // wrap at 200px; every produced line MUST be <= the budget (proves the wrap is
    // measured, not char-counted) and there must be >= 2 lines.
    let wrapped = wrap(&exp, &font, 200.0);
    assert!(wrapped.lines.len() >= 2, "text must wrap to multiple lines");
    for l in &wrapped.lines {
        assert!(
            l.width <= 200.0 + 0.01,
            "line width {} exceeds budget",
            l.width
        );
    }
    // the measured width of line 0 equals BMFont.measure of its concatenated text
    // (icon placeholder excluded) — i.e. the wrap used glyph metrics post-expansion.
    let l0 = &wrapped.lines[0];
    let l0_text: String = l0
        .spans
        .iter()
        .filter(|s| s.icon.is_none())
        .map(|s| s.text.as_str())
        .collect();
    // line0 has no icon in this layout; assert exact glyph-measured width.
    if l0.spans.iter().all(|s| s.icon.is_none()) {
        assert_eq!(l0.width, font.measure(&l0_text) as f32);
    }
    // colour survived: "Brandenburg" is yellow somewhere in the wrapped output.
    assert!(wrapped
        .lines
        .iter()
        .flat_map(|l| &l.spans)
        .any(|s| s.text.contains("Brandenburg") && s.color == [0.93, 0.82, 0.27, 1.0]));
    // the £icon became an icon span carried through wrapping.
    assert!(wrapped
        .lines
        .iter()
        .flat_map(|l| &l.spans)
        .any(|s| s.icon.as_deref() == Some("victory_points")));
}

// ---------------------------------------------------------------------------
// (c) z-order / alwaystransparent: panel over map, front-to-back, fall-through
// ---------------------------------------------------------------------------

/// Build a faithful mini-stack: a full-screen MAP window (opaque, captures), and
/// a state PANEL window whose ROOT is a pure container (no background -> does NOT
/// self-capture) carrying an opaque painted child (the state_name) and a
/// transparent overlay region.
fn map_and_panel() -> (Vec<Node>, Vec<Node>) {
    // map: a full-screen opaque background surface (a real province fill).
    let map = leaf(
        "map_province",
        ElementKind::Background {
            sprite: "GFX_map".into(),
        },
        Rect {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        },
        false,
    );

    // panel root: a pure coordinate-frame container (no background) — by the
    // runtime's is_hit_capable rule it does NOT eat clicks; only its painted
    // children do. This is exactly how the real countrystateview behaves.
    let painted = icon(
        "state_name",
        Rect {
            x: 60.0,
            y: 140.0,
            w: 300.0,
            h: 30.0,
        },
    );
    // a transparent overlay covering part of the panel bbox (the "shine" case):
    let transparent = leaf(
        "shine_overlay",
        ElementKind::Icon {
            sprite: "GFX_shine".into(),
            frame: 0,
        },
        Rect {
            x: 40.0,
            y: 110.0,
            w: 460.0,
            h: 650.0,
        },
        true, // alwaystransparent
    );
    let mut panel_root = Node {
        name: "countrystateview".into(),
        raw_kind: "containerWindowType".into(),
        kind: ElementKind::Container { background: None }, // NOT hit-capable
        pos: (40.0, 110.0),
        size_w: Some(gui_runtime::model::Scalar::Int(460)),
        size_h: Some(gui_runtime::model::Scalar::Int(650)),
        orientation: Anchor::UpperLeft,
        origo: Anchor::UpperLeft,
        clipping: false,
        alwaystransparent: false,
        rect: Rect {
            x: 40.0,
            y: 110.0,
            w: 460.0,
            h: 650.0,
        },
        clip: None,
        children: vec![painted, transparent],
    };
    panel_root.rect = Rect {
        x: 40.0,
        y: 110.0,
        w: 460.0,
        h: 650.0,
    };

    (vec![map], vec![panel_root])
}

#[test]
fn foundation_c_zorder_alwaystransparent_fallthrough() {
    let (map, panel) = map_and_panel();
    // the compositor stack front-to-back: [map (bottom), panel (top)].
    let stack = |x: f32, y: f32| {
        // panel is the topmost window; hit it first, else the map.
        hit_test_stack(&panel, x, y)
            .map(|h| ("panel", h.leaf().unwrap_or("").to_string()))
            .or_else(|| {
                hit_test_stack(&map, x, y).map(|h| ("map", h.leaf().unwrap_or("").to_string()))
            })
    };

    // (i) a click on the PAINTED panel element hits the panel.
    let on_panel = stack(200.0, 150.0).expect("a hit");
    assert_eq!(on_panel.0, "panel");
    assert_eq!(on_panel.1, "state_name");

    // (ii) a click on a TRANSPARENT panel region falls through to the map. The
    // shine overlay covers (40,110,460,650) and is alwaystransparent; a point over
    // it but NOT over the painted state_name must pass through to the map below.
    let trans_pt = (450.0, 600.0); // inside the transparent overlay, not on state_name
    let through = stack(trans_pt.0, trans_pt.1).expect("falls through to a hit");
    assert_eq!(
        through.0, "map",
        "transparent region must fall through to the map"
    );
    assert_eq!(through.1, "map_province");

    // a point over the panel BBOX but outside every painted child ALSO falls
    // through (the pure-container root does not self-capture).
    let bare = stack(45.0, 700.0).expect("falls through");
    assert_eq!(bare.0, "map");

    // (iii) front-to-back z-order: where the panel HAS a painted opaque element
    // over the map, the panel wins (it is the topmost window); where it does not,
    // the map wins. Both proven above — assert the ordering invariant directly:
    // the painted state_name sits over the map at (200,150) and the panel wins.
    let over = stack(200.0, 150.0).unwrap();
    assert_eq!(over.0, "panel", "the topmost window must win front-to-back");
    // and the map is independently alive (bottom layer reachable) off-panel.
    let off_panel = stack(1500.0, 800.0).unwrap();
    assert_eq!(off_panel.0, "map");
}

/// Codex (c) caveat: a true 3-window front-to-back test where the PANEL OVERLAPS
/// the TOPBAR. The compositor stack is [map, topbar, panel] (last = topmost). At
/// an OVERLAP point the panel (topmost) must win over the topbar; where only the
/// topbar is painted the topbar wins; where neither covers, the map wins.
#[test]
fn foundation_c_panel_over_topbar_over_map_front_to_back() {
    // map (bottom): full-screen opaque.
    let map = vec![leaf(
        "map_province",
        ElementKind::Background {
            sprite: "GFX_map".into(),
        },
        Rect {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        },
        false,
    )];
    // topbar (middle): an opaque band across the TOP, with a hit-capable element.
    let topbar = vec![{
        let mut w = leaf(
            "topbar",
            ElementKind::Container { background: None }, // frame, not hit-capable
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 200.0,
            },
            false,
        );
        w.raw_kind = "containerWindowType".into();
        w.children = vec![icon(
            "DateText",
            Rect {
                x: 0.0,
                y: 0.0,
                w: 1920.0,
                h: 200.0,
            },
        )];
        w
    }];
    // panel (top): OVERLAPS the topbar band (its painted element sits at y in
    // [120,180], inside the topbar's [0,200]).
    let panel = vec![{
        let mut w = leaf(
            "countrystateview",
            ElementKind::Container { background: None },
            Rect {
                x: 40.0,
                y: 110.0,
                w: 460.0,
                h: 650.0,
            },
            false,
        );
        w.raw_kind = "containerWindowType".into();
        w.children = vec![icon(
            "state_name",
            Rect {
                x: 60.0,
                y: 120.0,
                w: 300.0,
                h: 60.0,
            },
        )];
        w
    }];

    // front-to-back resolver over the real 3-window stack.
    let stack = |x: f32, y: f32| -> Option<&'static str> {
        if let Some(h) = hit_test_stack(&panel, x, y) {
            return Some(if h.leaf() == Some("state_name") {
                "panel"
            } else {
                "panel?"
            });
        }
        if let Some(h) = hit_test_stack(&topbar, x, y) {
            return Some(if h.leaf() == Some("DateText") {
                "topbar"
            } else {
                "topbar?"
            });
        }
        if hit_test_stack(&map, x, y).is_some() {
            return Some("map");
        }
        None
    };

    // OVERLAP point: covered by BOTH the topbar element AND the panel element.
    // (90,150) is inside state_name (60..360 x, 120..180 y) AND DateText (full band).
    assert_eq!(
        stack(90.0, 150.0),
        Some("panel"),
        "panel must win over topbar at overlap"
    );
    // a point in the topbar band but NOT over the panel element → topbar wins.
    assert_eq!(
        stack(1500.0, 100.0),
        Some("topbar"),
        "topbar wins where panel absent"
    );
    // a point below both bands → the map wins.
    assert_eq!(
        stack(1500.0, 900.0),
        Some("map"),
        "map wins where neither covers"
    );
}
