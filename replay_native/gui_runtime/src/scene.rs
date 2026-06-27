//! The RETAINED SCENE-GRAPH: typed nodes produced by deep-cloning the parsed
//! `.gui` tree ([`crate::model::RawNode`]) into a tree the layout/draw/hit-test
//! passes operate on. Each node carries its kind, the raw layout inputs
//! (position/size/orientation/origo), the passive-element payload, and — after
//! [`crate::layout::layout`] — a resolved absolute pixel rect.

use crate::model::{RawNode, Scalar};

/// 9-anchor box-model corner/edge. Used for BOTH `orientation` (which point of
/// the PARENT box the child anchors to) and `origo` (which point of the CHILD's
/// own box sits on that anchor). The `(fx, fy)` fractions are identical for both
/// roles — that symmetry is the whole point of the model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Anchor {
    UpperLeft,
    UpperRight,
    LowerLeft,
    LowerRight,
    Center,
    CenterUp,
    CenterDown,
    CenterLeft,
    CenterRight,
}

impl Anchor {
    /// Parse a Clausewitz orientation/origo token, CASE-INSENSITIVELY (source
    /// mixes `UPPER_RIGHT`, `upper_right`, `UPPER_right`). Unknown -> UpperLeft.
    pub fn parse(s: &str) -> Anchor {
        match s.to_ascii_uppercase().as_str() {
            "UPPER_LEFT" | "UPPERLEFT" | "TOP_LEFT" => Anchor::UpperLeft,
            "UPPER_RIGHT" | "UPPERRIGHT" | "TOP_RIGHT" => Anchor::UpperRight,
            "LOWER_LEFT" | "LOWERLEFT" | "BOTTOM_LEFT" => Anchor::LowerLeft,
            "LOWER_RIGHT" | "LOWERRIGHT" | "BOTTOM_RIGHT" => Anchor::LowerRight,
            "CENTER" | "CENTRE" | "CENTER_MIDDLE" => Anchor::Center,
            "CENTER_UP" | "CENTERUP" | "CENTER_TOP" | "UPPER_CENTER" => Anchor::CenterUp,
            "CENTER_DOWN" | "CENTERDOWN" | "CENTER_BOTTOM" | "LOWER_CENTER" => Anchor::CenterDown,
            "LEFT" | "CENTER_LEFT" | "LEFT_CENTER" => Anchor::CenterLeft,
            "RIGHT" | "CENTER_RIGHT" | "RIGHT_CENTER" => Anchor::CenterRight,
            _ => Anchor::UpperLeft,
        }
    }

    /// The (fx, fy) unit fractions: 0 = left/top edge, 1 = right/bottom edge,
    /// 0.5 = centre. Multiplying by a box's (w, h) gives the offset from the
    /// box's top-left to the anchor point.
    pub fn fractions(self) -> (f32, f32) {
        match self {
            Anchor::UpperLeft => (0.0, 0.0),
            Anchor::UpperRight => (1.0, 0.0),
            Anchor::LowerLeft => (0.0, 1.0),
            Anchor::LowerRight => (1.0, 1.0),
            Anchor::Center => (0.5, 0.5),
            Anchor::CenterUp => (0.5, 0.0),
            Anchor::CenterDown => (0.5, 1.0),
            Anchor::CenterLeft => (0.0, 0.5),
            Anchor::CenterRight => (1.0, 0.5),
        }
    }
}

/// Horizontal text alignment within the text box's `maxWidth`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Right,
    Center,
}

impl TextAlign {
    pub fn parse(s: &str) -> TextAlign {
        match s.to_ascii_lowercase().as_str() {
            "right" => TextAlign::Right,
            "center" | "centre" => TextAlign::Center,
            _ => TextAlign::Left,
        }
    }
}

/// A resolved text label payload (instant/textBox or a button's caption).
#[derive(Debug, Clone)]
pub struct TextSpec {
    /// The RESOLVED display string (loc-expanded; markup left literal in M2).
    pub text: String,
    /// The raw `.gui` text token before loc resolution (a key or a literal).
    pub raw: String,
    pub font: String,
    pub align: TextAlign,
    pub max_width: Option<f32>,
    pub max_height: Option<f32>,
}

/// The typed element payload. M2 implements the passive kinds the topbar uses;
/// every other Clausewitz kind degrades to [`ElementKind::Container`] (a pure
/// coordinate frame) so an unknown view still lays out and recurses.
#[derive(Debug, Clone)]
pub enum ElementKind {
    /// containerWindowType / windowType / positionType: a coordinate frame,
    /// optionally with a `background` sprite, recurses into children.
    Container { background: Option<String> },
    /// `background` element: a bare sprite that fills the parent (window chrome).
    Background { sprite: String },
    /// iconType: a (optionally framed) sprite.
    Icon { sprite: String, frame: u32 },
    /// buttonType / guiButtonType: a sprite (state frame) + optional caption.
    Button {
        sprite: Option<String>,
        frame: u32,
        text: Option<TextSpec>,
    },
    /// instantTextBoxType / textBoxType: BMFont text.
    Text(TextSpec),
    /// corneredTileSpriteType-backed window chrome (9-slice). Promoted from a
    /// Container/Icon when its sprite is a 9-slice in the manifest.
    NineSlice { sprite: String },
    /// progressbartype: a bar drawn `fraction` of the way across.
    ProgressBar { sprite: String, fraction: f32 },
}

/// A laid-out absolute pixel rect (top-left origin, y down).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    /// Intersection of two rects (empty rect if disjoint).
    pub fn intersect(&self, o: &Rect) -> Rect {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        Rect {
            x: x0,
            y: y0,
            w: (x1 - x0).max(0.0),
            h: (y1 - y0).max(0.0),
        }
    }
    pub fn is_empty(&self) -> bool {
        self.w <= 0.0 || self.h <= 0.0
    }
}

/// One retained scene-graph node.
#[derive(Debug, Clone)]
pub struct Node {
    pub name: String,
    pub raw_kind: String,
    pub kind: ElementKind,

    // --- raw layout inputs (interpreted by layout.rs) ---
    pub pos: (f32, f32),
    /// Raw per-axis size scalars (px int / "100%%" / "-1"); None = unspecified.
    pub size_w: Option<Scalar>,
    pub size_h: Option<Scalar>,
    pub orientation: Anchor,
    pub origo: Anchor,

    pub clipping: bool,
    pub alwaystransparent: bool,

    // --- resolved by layout ---
    pub rect: Rect,
    /// The clip rect in force for this node (intersection of ancestor clips).
    pub clip: Option<Rect>,

    pub children: Vec<Node>,
}

impl Node {
    /// Deep-clone a parsed `RawNode` into a typed scene-graph node. Kind mapping
    /// and 9-slice/progressbar promotion are resolved against the sprite manifest
    /// so window chrome draws as a real 9-slice. `loc` resolves text keys.
    pub fn from_raw(
        raw: &RawNode,
        manifest: &assets::SpriteManifest,
        loc: &crate::loc::Loc,
    ) -> Node {
        let name = raw.name.clone().unwrap_or_default();
        // Resolve each POSITION axis to a pixel value: a literal/edge-offset is its
        // number, a RESOLVED `@named` anchor is its real coordinate, and an
        // unresolved anchor / position-percent / unknown token falls back to 0.0 as
        // an explicit chrome fallback (the raw token is preserved on the `Coord`).
        let pos = raw
            .position
            .as_ref()
            .map(|p| (p.x.px(), p.y.px()))
            .unwrap_or((0.0, 0.0));
        let (size_w, size_h) = match &raw.size {
            Some(s) => (s.w().cloned(), s.h().cloned()),
            None => (None, None),
        };
        let orientation = raw
            .orientation
            .as_deref()
            .map(Anchor::parse)
            .unwrap_or(Anchor::UpperLeft);
        let origo = raw
            .origo
            .as_deref()
            .map(Anchor::parse)
            .unwrap_or(Anchor::UpperLeft);
        let clipping = raw.clipping.map(|c| c.is_yes()).unwrap_or(false);
        let alwaystransparent = raw.alwaystransparent.map(|c| c.is_yes()).unwrap_or(false);

        let kind = classify(raw, manifest, loc);

        let children = raw
            .children
            .iter()
            .map(|c| Node::from_raw(c, manifest, loc))
            .collect();

        Node {
            name,
            raw_kind: raw.kind.clone(),
            kind,
            pos,
            size_w,
            size_h,
            orientation,
            origo,
            clipping,
            alwaystransparent,
            rect: Rect::default(),
            clip: None,
            children,
        }
    }

    /// Count nodes in this subtree (inclusive).
    pub fn count(&self) -> usize {
        1 + self.children.iter().map(|c| c.count()).sum::<usize>()
    }

    /// Whether this node SELF-CAPTURES a hit on its own rect. Codex-corrected:
    /// a pure coordinate-frame container (containerWindowType / windowType /
    /// positionType / unknown -> [`ElementKind::Container`] with no background)
    /// must NOT eat clicks just because it has a rect — otherwise the full-screen
    /// `topbar` window would swallow every click meant for the map beneath it.
    /// Children of such a container remain hittable; only drawable/interactive
    /// surfaces (button, icon, text, background, 9-slice, progressbar) capture.
    pub fn is_hit_capable(&self) -> bool {
        match &self.kind {
            ElementKind::Container { background } => background.is_some(),
            ElementKind::Background { .. }
            | ElementKind::Icon { .. }
            | ElementKind::NineSlice { .. }
            | ElementKind::ProgressBar { .. }
            | ElementKind::Button { .. }
            | ElementKind::Text(_) => true,
        }
    }
}

/// Build a [`TextSpec`] from a raw text block, resolving the loc key.
fn text_spec(tb: &crate::model::TextBlock, loc: &crate::loc::Loc) -> Option<TextSpec> {
    let raw = tb.raw_text()?.as_text();
    if raw.is_empty() {
        return None;
    }
    // Resolve a loc key -> localized string; a non-key literal stays as-is; a
    // missing key shows the raw key (never blank) per the loc-layer contract.
    let resolved = loc.resolve(&raw);
    Some(TextSpec {
        text: resolved,
        raw,
        font: tb.font.clone().unwrap_or_else(|| "hoi_18mbs".to_string()),
        align: tb
            .format
            .as_deref()
            .map(TextAlign::parse)
            .unwrap_or(TextAlign::Left),
        max_width: tb.max_w(),
        max_height: tb.max_h(),
    })
}

/// Map a raw element to a typed [`ElementKind`], promoting 9-slice chrome and
/// progressbars by consulting the sprite manifest.
fn classify(
    raw: &RawNode,
    manifest: &assets::SpriteManifest,
    loc: &crate::loc::Loc,
) -> ElementKind {
    let kind = raw.kind.to_ascii_lowercase();
    let sprite = raw.sprite.clone();
    let sprite_entry = sprite.as_deref().and_then(|s| manifest.get(s));
    let is_nine = sprite_entry
        .map(|e| e.nine_slice.unwrap_or(false) || e.border_size.is_some())
        .unwrap_or(false);
    let is_bar = sprite_entry
        .map(|e| e.kind.eq_ignore_ascii_case("progressbartype"))
        .unwrap_or(false);
    let frame = raw.frame.map(|f| (f.max(1) as u32) - 1).unwrap_or(0);

    match kind.as_str() {
        "icontype" => {
            if let Some(s) = sprite {
                if is_bar {
                    ElementKind::ProgressBar {
                        sprite: s,
                        fraction: 1.0,
                    }
                } else if is_nine {
                    ElementKind::NineSlice { sprite: s }
                } else {
                    ElementKind::Icon { sprite: s, frame }
                }
            } else {
                ElementKind::Container { background: None }
            }
        }
        "background" => match sprite {
            Some(s) if is_nine => ElementKind::NineSlice { sprite: s },
            Some(s) => ElementKind::Background { sprite: s },
            None => ElementKind::Container { background: None },
        },
        "buttontype" | "guibuttontype" => ElementKind::Button {
            sprite,
            frame,
            text: raw.text.as_ref().and_then(|tb| text_spec(tb, loc)),
        },
        "instanttextboxtype" | "textboxtype" => match raw.text.as_ref().and_then(|tb| text_spec(tb, loc))
        {
            Some(t) => ElementKind::Text(t),
            None => ElementKind::Container { background: None },
        },
        "progressbartype" => match sprite {
            Some(s) => ElementKind::ProgressBar {
                sprite: s,
                fraction: 1.0,
            },
            None => ElementKind::Container { background: None },
        },
        // containerWindowType / windowType / positionType / unknown kinds: a
        // coordinate frame. A container may carry a `background` sprite child in
        // the game, but in this schema the bg is a separate `background` element, so
        // the container itself is background-less.
        _ => ElementKind::Container { background: sprite },
    }
}
