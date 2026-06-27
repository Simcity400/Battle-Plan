//! BATCH 1 foundation — the no-fabrication value types every primitive uses at its
//! PUBLIC boundary (Codex's sharpest note: the honesty contract must live in the type,
//! not in caller discipline), plus the shared draw substrate (a 1x1-white-texture
//! `DrawAssets` bundle + fill/line/disc helpers) and the generic pan/zoom `Canvas`.
//!
//! ## Why a local [`Atom`] instead of the app's `DisplayAtom`?
//! The `widgets` crate must NOT depend on `app` (the host) — clusters in `app` depend
//! on `widgets`, so the reverse would be a cycle. [`Atom`] is the in-crate mirror of
//! the host's `DisplayAtom`: real measured `Text` + a fidelity badge, or the
//! not-captured glyph `"—"` carrying a [`Reason`]. The app's cluster builders turn a
//! `host::DisplayAtom` into a [`widgets::Atom`] with the one-line [`Atom::from_parts`]
//! / the `From`-style helpers — there is no path that yields a fabricated `0`.

use render::{Renderer, Texture};

/// The not-captured glyph (matches the host's `bind::format::NOT_CAPTURED`).
pub const NOT_CAPTURED: &str = "—";

/// Honesty badge for a measured value (mirrors `bind::Fidelity` without the dep).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fidelity {
    /// Read directly from the snapshot — a real measured value.
    Exact,
    /// Computed/approximated from measured inputs (badged so the UI can flag it).
    Derived,
    /// Not captured / blocked — renders the not-captured glyph, never a number.
    Unavailable,
}

/// Why a value is not captured (kept distinct so a UI badges missing/null/blocked
/// differently — the same trichotomy the host's `UnavailableReason` carries).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// A key along the path was absent — never captured.
    Absent,
    /// Present but JSON null — captured-but-unobtained.
    Null,
    /// Schema-present-but-unfed / runtime-computed value we do not store.
    Blocked,
}

/// A value READY FOR THE SCREEN: either real measured text (with its fidelity badge)
/// or the not-captured glyph. EVERY visible label/title/stat/cell on a widget is an
/// `Atom`, so a primitive's public API cannot be handed a bare `String`/`0` and
/// silently fabricate it (Codex (b): enforce no-fabrication at the boundary).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Atom {
    /// Real, measured text + its honesty badge.
    Text { text: String, fidelity: Fidelity },
    /// A blocked/absent/null field: render the not-captured glyph. Carries NO numeric
    /// payload by construction; the `reason` preserves the distinction for a badge.
    Unavailable { reason: Reason },
}

impl Atom {
    /// A measured-EXACT atom (the common case for a real snapshot read).
    pub fn exact(text: impl Into<String>) -> Atom {
        Atom::Text { text: text.into(), fidelity: Fidelity::Exact }
    }
    /// A measured-DERIVED atom (computed from measured inputs).
    pub fn derived(text: impl Into<String>) -> Atom {
        Atom::Text { text: text.into(), fidelity: Fidelity::Derived }
    }
    /// The not-captured atom (Absent reason by default).
    pub fn unavailable() -> Atom {
        Atom::Unavailable { reason: Reason::Absent }
    }
    /// The not-captured atom with an explicit reason.
    pub fn unavailable_because(reason: Reason) -> Atom {
        Atom::Unavailable { reason }
    }
    /// Build from the host's `(text, fidelity, reason)` triple WITHOUT a dep on the
    /// host crate: `Fidelity::Unavailable` collapses to the glyph regardless of text,
    /// so a cluster CANNOT smuggle a fabricated number past a `—` badge.
    pub fn from_parts(text: String, fidelity: Fidelity, reason: Reason) -> Atom {
        match fidelity {
            Fidelity::Unavailable => Atom::Unavailable { reason },
            f => Atom::Text { text, fidelity: f },
        }
    }

    /// The string to DRAW for this atom (measured text or the not-captured glyph).
    pub fn as_str(&self) -> &str {
        match self {
            Atom::Text { text, .. } => text,
            Atom::Unavailable { .. } => NOT_CAPTURED,
        }
    }
    /// True iff this atom is the not-captured glyph.
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Atom::Unavailable { .. })
    }
    pub fn fidelity(&self) -> Fidelity {
        match self {
            Atom::Text { fidelity, .. } => *fidelity,
            Atom::Unavailable { .. } => Fidelity::Unavailable,
        }
    }
    pub fn reason(&self) -> Option<Reason> {
        match self {
            Atom::Unavailable { reason } => Some(*reason),
            Atom::Text { .. } => None,
        }
    }
    /// A drawing tint for the atom: a normal colour for measured text, a dim
    /// "unavailable" tint for the glyph (so a `—` never looks like a real `0`).
    pub fn tint(&self, normal: [f32; 4]) -> [f32; 4] {
        match self {
            Atom::Text { fidelity: Fidelity::Derived, .. } => DERIVED_TINT,
            Atom::Text { .. } => normal,
            Atom::Unavailable { .. } => UNAVAIL_TINT,
        }
    }
}

/// A reference to a sprite/portrait/flag ASSET, kept DISTINCT (Codex N4/N5/N8: an
/// `Option<String>` conflates "no asset by design", "asset name present but the file
/// is missing", and "blocked"). A widget draws a placeholder for `Named` whose texture
/// the caller could not resolve — never silently nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssetRef {
    /// There is, by design, NO asset here (e.g. an alert with no portrait).
    None,
    /// A named `GFX_*` sprite the caller should resolve; if resolution fails the
    /// widget draws a visible no-asset placeholder, never blank.
    Named(String),
    /// The asset slot is BLOCKED/unknown (e.g. portrait behind a gfx wall) — drawn as
    /// an explicit unavailable placeholder, distinct from `None`.
    Unavailable,
}

impl AssetRef {
    /// The sprite name to attempt to resolve, if any.
    pub fn name(&self) -> Option<&str> {
        match self {
            AssetRef::Named(s) => Some(s),
            _ => None,
        }
    }
    pub fn is_none(&self) -> bool {
        matches!(self, AssetRef::None)
    }
}

/// Standard widget tints (RGBA straight-alpha). Shared so every primitive badges the
/// same way (a derived value and an unavailable glyph look identical across widgets).
pub const TEXT_TINT: [f32; 4] = [0.93, 0.93, 0.90, 1.0];
pub const DIM_TINT: [f32; 4] = [0.62, 0.64, 0.68, 1.0];
pub const DERIVED_TINT: [f32; 4] = [0.78, 0.74, 0.52, 1.0];
pub const UNAVAIL_TINT: [f32; 4] = [0.55, 0.45, 0.40, 1.0];
pub const PLACEHOLDER_TINT: [f32; 4] = [0.20, 0.18, 0.22, 1.0];

/// The assets a primitive's draw fn needs, resolved per app/demo (mirrors the
/// `focus_tree`/`combat_panel` `DrawAssets` so widgets stay headlessly testable: the
/// layout/pick/no-fabrication logic never touches a GPU — only `draw_*` does).
pub struct DrawAssets<'a> {
    /// A 1x1 white texture, tinted per draw → solid rects / lines / discs / chrome.
    pub white: &'a Texture,
    /// Optional BMFont for labels. `None` → labels skipped (geometry still draws).
    pub font: Option<&'a assets::BmFont>,
    pub font_pages: &'a [Texture],
}

/// Build a 1x1 white texture (the substrate every widget tints). One per app.
pub fn white_texture(renderer: &Renderer) -> Texture {
    renderer.upload_rgba8_image(1, 1, &[255, 255, 255, 255])
}

/// Fill a solid rect (screen px), clipped to `clip`.
pub fn fill_rect(
    r: &mut Renderer,
    white: &Texture,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    c: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    r.draw_sprite_frame_clipped(white, x, y, w, h, 0, 1, c, clip);
}

/// Draw an axis-aligned rect OUTLINE of `thick` px (four fills), clipped.
#[allow(clippy::too_many_arguments)]
pub fn stroke_rect(
    r: &mut Renderer,
    white: &Texture,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    thick: f32,
    c: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    let t = thick.max(1.0);
    fill_rect(r, white, x, y, w, t, c, clip); // top
    fill_rect(r, white, x, y + h - t, w, t, c, clip); // bottom
    fill_rect(r, white, x, y, t, h, c, clip); // left
    fill_rect(r, white, x + w - t, y, t, h, c, clip); // right
}

/// Draw an L-shaped axis-aligned link (vertical then horizontal segment) between two
/// screen points — the focus/tech link-line style. NaN-safe: a non-finite endpoint
/// draws nothing.
#[allow(clippy::too_many_arguments)]
pub fn link_l(
    r: &mut Renderer,
    white: &Texture,
    from: (f32, f32),
    to: (f32, f32),
    thick: f32,
    c: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    if !(from.0.is_finite() && from.1.is_finite() && to.0.is_finite() && to.1.is_finite()) {
        return;
    }
    let (x0, y0) = from;
    let (x1, y1) = to;
    let t = thick.max(1.0);
    let midy = y1;
    let (vy0, vy1) = if y0 <= midy { (y0, midy) } else { (midy, y0) };
    fill_rect(r, white, x0 - t / 2.0, vy0, t, (vy1 - vy0).max(t), c, clip);
    let (hx0, hx1) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
    fill_rect(r, white, hx0, midy - t / 2.0, (hx1 - hx0).max(t), t, c, clip);
}

/// Draw a straight line between two screen points as a thin rect (for non-stepped
/// edges — influence/reference). NaN-safe.
#[allow(clippy::too_many_arguments)]
pub fn line_seg(
    r: &mut Renderer,
    white: &Texture,
    from: (f32, f32),
    to: (f32, f32),
    thick: f32,
    c: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    if !(from.0.is_finite() && from.1.is_finite() && to.0.is_finite() && to.1.is_finite()) {
        return;
    }
    let (x0, y0) = from;
    let (x1, y1) = to;
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.5 {
        return;
    }
    // approximate a rotated segment with a chain of small dots (we have no rotated
    // quad primitive) — cheap + crisp enough for the few influence edges drawn.
    let t = thick.max(1.0);
    let steps = (len / (t.max(1.0))).ceil() as i32;
    for i in 0..=steps {
        let f = i as f32 / steps.max(1) as f32;
        let px = x0 + dx * f;
        let py = y0 + dy * f;
        fill_rect(r, white, px - t / 2.0, py - t / 2.0, t, t, c, clip);
    }
}

/// Draw a filled disc by stacking horizontal scanline rects (no circle texture). The
/// shared bubble/marker primitive (copied shape from `combat_panel`, NOT imported).
pub fn disc(
    r: &mut Renderer,
    white: &Texture,
    cx: f32,
    cy: f32,
    radius: f32,
    c: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    if !(cx.is_finite() && cy.is_finite() && radius.is_finite()) {
        return;
    }
    let rad = radius.max(1.0);
    let steps = (rad * 2.0).ceil() as i32;
    for i in 0..steps {
        let dy = -rad + (i as f32) + 0.5;
        if dy.abs() > rad {
            continue;
        }
        let half = (rad * rad - dy * dy).max(0.0).sqrt();
        fill_rect(r, white, cx - half, cy + dy, half * 2.0, 1.0, c, clip);
    }
}

/// Draw a label `Atom` at (x,y) with its honesty tint, clipped, IF a font is present.
/// The single text-draw seam: a `—` atom draws the glyph in the unavailable tint.
pub fn draw_atom(
    r: &mut Renderer,
    assets: &DrawAssets,
    atom: &Atom,
    x: f32,
    y: f32,
    normal: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    if let Some(font) = assets.font {
        r.draw_text_clipped(font, assets.font_pages, atom.as_str(), x, y, atom.tint(normal), clip);
    }
}

/// Draw a plain (already-honest) string at (x,y). For widget chrome captions that are
/// NOT data fields (e.g. a "BATTLE DETAIL" section header).
#[allow(clippy::too_many_arguments)]
pub fn draw_text(
    r: &mut Renderer,
    assets: &DrawAssets,
    text: &str,
    x: f32,
    y: f32,
    c: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    if let Some(font) = assets.font {
        r.draw_text_clipped(font, assets.font_pages, text, x, y, c, clip);
    }
}

// ===========================================================================
// The generic pan/zoom CANVAS camera — the shared transform N1 (and thus N3)
// build on. SAME math as `focus_tree::Camera` (canvas space ⇄ screen), but OWNED
// here so `widgets` does not depend on `focus_tree` (Codex build-risk guard).
// ===========================================================================

/// A pan/zoom view transform over canvas space. `scale` is the zoom; `(ox, oy)` is the
/// canvas-space point shown at the viewport's top-left. All ops are NaN/degenerate-safe
/// (Codex: clamp/ignore non-finite + zero/negative scale + infinite bounds).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Canvas {
    pub scale: f32,
    pub ox: f32,
    pub oy: f32,
    /// The on-screen viewport rect [x, y, w, h] the canvas is clipped to.
    pub viewport: [f32; 4],
}

/// The hard scale clamp (shared so every graph zooms within the same range).
pub const CANVAS_MIN_SCALE: f32 = 0.02;
pub const CANVAS_MAX_SCALE: f32 = 6.0;

impl Canvas {
    /// An identity canvas over a viewport (no pan, scale 1).
    pub fn new(viewport: [f32; 4]) -> Canvas {
        Canvas { scale: 1.0, ox: 0.0, oy: 0.0, viewport: sane_viewport(viewport) }
    }

    /// Fit a canvas-space bounds `(minx,miny,maxx,maxy)` into `viewport` with a margin.
    /// A degenerate/empty/non-finite bounds falls back to an identity canvas (never a
    /// NaN scale).
    pub fn fit(bounds: (f32, f32, f32, f32), viewport: [f32; 4], margin: f32) -> Canvas {
        let viewport = sane_viewport(viewport);
        let (minx, miny, maxx, maxy) = bounds;
        if !(minx.is_finite() && miny.is_finite() && maxx.is_finite() && maxy.is_finite()) {
            return Canvas::new(viewport);
        }
        let tw = (maxx - minx).max(1.0);
        let th = (maxy - miny).max(1.0);
        let m = margin.max(0.0);
        let sx = (viewport[2] - 2.0 * m) / tw;
        let sy = (viewport[3] - 2.0 * m) / th;
        let mut scale = sx.min(sy);
        if !scale.is_finite() || scale <= 0.0 {
            scale = 1.0;
        }
        let scale = scale.clamp(CANVAS_MIN_SCALE, CANVAS_MAX_SCALE);
        Canvas { scale, ox: minx - m / scale, oy: miny - m / scale, viewport }
    }

    /// The scale to project with — coerced to a safe positive value so a caller that
    /// poked a `0`/NaN/negative into the public `scale` field cannot poison the
    /// transform (Codex: the frozen struct must make degenerate input safe, not just the
    /// constructors).
    fn safe_scale(&self) -> f32 {
        if self.scale.is_finite() && self.scale > 0.0 {
            self.scale
        } else {
            1.0
        }
    }
    /// Canvas space → screen pixels.
    pub fn to_screen(&self, cx: f32, cy: f32) -> (f32, f32) {
        let s = self.safe_scale();
        (self.viewport[0] + (cx - self.ox) * s, self.viewport[1] + (cy - self.oy) * s)
    }
    /// Screen pixels → canvas space.
    pub fn from_screen(&self, sx: f32, sy: f32) -> (f32, f32) {
        let s = self.safe_scale();
        (self.ox + (sx - self.viewport[0]) / s, self.oy + (sy - self.viewport[1]) / s)
    }
    /// The viewport clip rect.
    pub fn clip(&self) -> [f32; 4] {
        self.viewport
    }
    /// True iff a screen point is inside the viewport (the canvas's hit region).
    pub fn contains(&self, sx: f32, sy: f32) -> bool {
        let [x, y, w, h] = self.viewport;
        sx >= x && sx < x + w && sy >= y && sy < y + h
    }
    /// Update the viewport (on resize); pan/zoom preserved.
    pub fn set_viewport(&mut self, viewport: [f32; 4]) {
        self.viewport = sane_viewport(viewport);
    }
    /// Pan by a screen-space delta (drag). Non-finite deltas are ignored.
    pub fn pan(&mut self, dx_screen: f32, dy_screen: f32) {
        if !(dx_screen.is_finite() && dy_screen.is_finite()) {
            return;
        }
        let s = self.safe_scale();
        self.ox -= dx_screen / s;
        self.oy -= dy_screen / s;
    }
    /// Zoom about a screen-space anchor (mouse), clamped to [MIN,MAX]. A non-finite
    /// factor/anchor is ignored; the anchor point stays fixed under the cursor. A
    /// degenerate stored `scale` is first coerced safe so the result is always valid.
    pub fn zoom_at(&mut self, anchor_x: f32, anchor_y: f32, factor: f32) {
        if !(anchor_x.is_finite() && anchor_y.is_finite() && factor.is_finite()) || factor <= 0.0 {
            return;
        }
        let (cx, cy) = self.from_screen(anchor_x, anchor_y);
        let new_scale = (self.safe_scale() * factor).clamp(CANVAS_MIN_SCALE, CANVAS_MAX_SCALE);
        if new_scale <= 0.0 {
            return;
        }
        self.scale = new_scale;
        self.ox = cx - (anchor_x - self.viewport[0]) / self.scale;
        self.oy = cy - (anchor_y - self.viewport[1]) / self.scale;
    }
}

/// Coerce a viewport to finite, non-negative dimensions (a NaN/zero viewport would
/// poison every transform).
fn sane_viewport(v: [f32; 4]) -> [f32; 4] {
    let f = |x: f32| if x.is_finite() { x } else { 0.0 };
    [f(v[0]), f(v[1]), f(v[2]).max(0.0), f(v[3]).max(0.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atom_no_fabrication_trichotomy() {
        // a measured 0 is REAL — it draws "0", never the glyph.
        let m = Atom::exact("0");
        assert!(!m.is_unavailable());
        assert_eq!(m.as_str(), "0");
        assert_eq!(m.fidelity(), Fidelity::Exact);
        // an unavailable atom draws the glyph regardless.
        let u = Atom::unavailable_because(Reason::Blocked);
        assert!(u.is_unavailable());
        assert_eq!(u.as_str(), NOT_CAPTURED);
        assert_eq!(u.reason(), Some(Reason::Blocked));
        // from_parts CANNOT smuggle a number past an Unavailable fidelity.
        let smuggled = Atom::from_parts("999".into(), Fidelity::Unavailable, Reason::Blocked);
        assert!(smuggled.is_unavailable());
        assert_eq!(smuggled.as_str(), NOT_CAPTURED);
    }

    #[test]
    fn asset_ref_distinguishes_none_named_unavailable() {
        assert!(AssetRef::None.is_none());
        assert_eq!(AssetRef::Named("GFX_x".into()).name(), Some("GFX_x"));
        assert_eq!(AssetRef::Unavailable.name(), None);
        assert!(!AssetRef::Unavailable.is_none()); // distinct from None
    }

    #[test]
    fn canvas_roundtrip_and_fit() {
        let cam = Canvas::fit((0.0, 0.0, 100.0, 80.0), [0.0, 0.0, 800.0, 600.0], 20.0);
        let (sx, sy) = cam.to_screen(50.0, 40.0);
        let (cx, cy) = cam.from_screen(sx, sy);
        assert!((cx - 50.0).abs() < 0.01 && (cy - 40.0).abs() < 0.01);
    }

    #[test]
    fn canvas_degenerate_inputs_never_nan() {
        // empty bounds -> identity, finite scale.
        let cam = Canvas::fit((0.0, 0.0, 0.0, 0.0), [0.0, 0.0, 100.0, 100.0], 0.0);
        assert!(cam.scale.is_finite() && cam.scale > 0.0);
        // NaN bounds -> identity.
        let nanc = Canvas::fit((f32::NAN, 0.0, 1.0, 1.0), [0.0, 0.0, 100.0, 100.0], 0.0);
        assert!(nanc.scale.is_finite());
        // zoom with a non-finite factor is a no-op.
        let mut c = Canvas::new([0.0, 0.0, 100.0, 100.0]);
        let before = c;
        c.zoom_at(10.0, 10.0, f32::INFINITY);
        assert_eq!(c, before);
        // scale clamps.
        let mut c2 = Canvas::new([0.0, 0.0, 100.0, 100.0]);
        for _ in 0..100 {
            c2.zoom_at(50.0, 50.0, 2.0);
        }
        assert!(c2.scale <= CANVAS_MAX_SCALE + 1e-3);
    }

    #[test]
    fn canvas_degenerate_public_scale_field_is_safe() {
        // a caller poking scale=0 into the public field must not produce a NaN transform.
        let mut c = Canvas::new([0.0, 0.0, 100.0, 100.0]);
        c.scale = 0.0;
        let (sx, sy) = c.to_screen(10.0, 10.0);
        assert!(sx.is_finite() && sy.is_finite());
        let (cx, cy) = c.from_screen(50.0, 50.0);
        assert!(cx.is_finite() && cy.is_finite());
        c.scale = f32::NAN;
        assert!(c.to_screen(1.0, 1.0).0.is_finite());
    }

    #[test]
    fn canvas_focal_zoom_keeps_anchor_fixed() {
        let mut c = Canvas::new([0.0, 0.0, 800.0, 600.0]);
        let (ax, ay) = (300.0, 200.0);
        let before = c.from_screen(ax, ay);
        c.zoom_at(ax, ay, 2.0);
        let after = c.from_screen(ax, ay);
        assert!((before.0 - after.0).abs() < 0.01 && (before.1 - after.1).abs() < 0.01);
    }
}
