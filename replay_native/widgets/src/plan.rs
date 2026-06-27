//! The shared RENDER-PLAN — an ordered list of pure draw commands a primitive emits
//! from its (data, camera, layout) inputs. This is the headless-testability + scrub-
//! byte-identity seam (roadmap VALIDATION: "a render-plan digest where applicable"):
//!
//!   * a primitive's `plan_*` fn turns its layout into a [`Plan`] with NO GPU — so the
//!     full draw LOGIC (positions, tints, clip, no-fabrication `—` vs measured text)
//!     is unit-tested without wgpu;
//!   * a primitive's `draw_*` fn just EXECUTES a `Plan` against the `render::Renderer`
//!     (one tiny dispatcher), so the GPU path runs exactly what the plan describes;
//!   * [`Plan::digest`] is a stable string hash of the command stream → two frames
//!     produce the same pixels iff their plans' digests match (the scrubber's A→B→A
//!     byte-identity check).
//!
//! Commands carry the RESOLVED inputs (screen px + tint + the literal text to draw),
//! so the digest changes iff something visible changes — exactly what scrub-identity
//! needs.

use render::Renderer;

use crate::common::DrawAssets;

/// One pure draw command (screen-space px). `Text` carries the literal resolved string
/// (a `—` for an unavailable atom), so the digest distinguishes a measured value from
/// the not-captured glyph.
#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    /// A solid filled rect (tinted white texture).
    Rect { x: f32, y: f32, w: f32, h: f32, tint: [f32; 4], clip: Option<[f32; 4]> },
    /// A filled disc (bubble/marker).
    Disc { cx: f32, cy: f32, r: f32, tint: [f32; 4], clip: Option<[f32; 4]> },
    /// An L-shaped step link (focus/tech edge).
    LinkL { from: (f32, f32), to: (f32, f32), thick: f32, tint: [f32; 4], clip: Option<[f32; 4]> },
    /// A straight segment (influence/reference edge).
    Line { from: (f32, f32), to: (f32, f32), thick: f32, tint: [f32; 4], clip: Option<[f32; 4]> },
    /// A text run (the literal resolved string + its tint).
    Text { x: f32, y: f32, text: String, tint: [f32; 4], clip: Option<[f32; 4]> },
    /// A named sprite to resolve+draw (icon/portrait/flag). `name=None` is a
    /// placeholder fill (a no-asset / unavailable slot) — never silently nothing.
    Sprite { x: f32, y: f32, w: f32, h: f32, name: Option<String>, tint: [f32; 4], clip: Option<[f32; 4]> },
}

/// An ordered render plan: the exact, replayable command stream a primitive emits.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    pub cmds: Vec<Cmd>,
}

impl Plan {
    pub fn new() -> Plan {
        Plan { cmds: Vec::new() }
    }
    pub fn push(&mut self, c: Cmd) {
        self.cmds.push(c);
    }
    pub fn len(&self) -> usize {
        self.cmds.len()
    }
    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }
    /// Count the `Text` commands whose literal is the not-captured glyph — the
    /// no-fabrication accounting hook (a test asserts blocked fields drew `—`).
    pub fn unavailable_text_count(&self) -> usize {
        self.cmds
            .iter()
            .filter(|c| matches!(c, Cmd::Text { text, .. } if text == crate::common::NOT_CAPTURED))
            .count()
    }
    /// Count the placeholder (name=None) sprite commands — a no-asset slot drew a
    /// visible placeholder, not nothing.
    pub fn placeholder_sprite_count(&self) -> usize {
        self.cmds
            .iter()
            .filter(|c| matches!(c, Cmd::Sprite { name: None, .. }))
            .count()
    }

    /// A STABLE digest of the command stream — the scrub byte-identity key. Two plans
    /// digest-equal iff every command (kind + resolved coords + tint + literal text)
    /// matches in order. Floats are quantised to 0.01 px so sub-pixel jitter from a
    /// harmless re-layout does not falsely perturb identity (matches the app's other
    /// digests).
    pub fn digest(&self) -> String {
        let mut s = String::with_capacity(self.cmds.len() * 24);
        for c in &self.cmds {
            match c {
                Cmd::Rect { x, y, w, h, tint, clip } => {
                    s.push_str(&format!("R{},{},{},{}|{}|{};", q(*x), q(*y), q(*w), q(*h), qt(tint), qc(clip)));
                }
                Cmd::Disc { cx, cy, r, tint, clip } => {
                    s.push_str(&format!("D{},{},{}|{}|{};", q(*cx), q(*cy), q(*r), qt(tint), qc(clip)));
                }
                Cmd::LinkL { from, to, thick, tint, clip } => {
                    s.push_str(&format!("L{},{}->{},{}@{}|{}|{};", q(from.0), q(from.1), q(to.0), q(to.1), q(*thick), qt(tint), qc(clip)));
                }
                Cmd::Line { from, to, thick, tint, clip } => {
                    s.push_str(&format!("N{},{}->{},{}@{}|{}|{};", q(from.0), q(from.1), q(to.0), q(to.1), q(*thick), qt(tint), qc(clip)));
                }
                Cmd::Text { x, y, text, tint, clip } => {
                    s.push_str(&format!("T{},{}:{:?}|{}|{};", q(*x), q(*y), text, qt(tint), qc(clip)));
                }
                Cmd::Sprite { x, y, w, h, name, tint, clip } => {
                    s.push_str(&format!("S{},{},{},{}:{:?}|{}|{};", q(*x), q(*y), q(*w), q(*h), name, qt(tint), qc(clip)));
                }
            }
        }
        s
    }

    /// EXECUTE this plan against the renderer (the one GPU dispatcher every `draw_*`
    /// funnels through). A `Sprite` with a name is handed to the caller-supplied
    /// `resolve` closure (texture cache), which receives the FULL placement
    /// ([`SpriteDraw`] — name + rect + tint + clip) so it can draw the texture at the
    /// right rect; it returns `true` if it drew. A miss / `None` / a `false` return
    /// draws a tinted placeholder rect — never silently nothing (Codex N4/N5/N8: a
    /// no-asset slot is always visible).
    pub fn execute(
        &self,
        r: &mut Renderer,
        assets: &DrawAssets,
        mut resolve: impl FnMut(&mut Renderer, SpriteDraw) -> bool,
    ) {
        for c in &self.cmds {
            match c {
                Cmd::Rect { x, y, w, h, tint, clip } => {
                    crate::common::fill_rect(r, assets.white, *x, *y, *w, *h, *tint, *clip);
                }
                Cmd::Disc { cx, cy, r: rad, tint, clip } => {
                    crate::common::disc(r, assets.white, *cx, *cy, *rad, *tint, *clip);
                }
                Cmd::LinkL { from, to, thick, tint, clip } => {
                    crate::common::link_l(r, assets.white, *from, *to, *thick, *tint, *clip);
                }
                Cmd::Line { from, to, thick, tint, clip } => {
                    crate::common::line_seg(r, assets.white, *from, *to, *thick, *tint, *clip);
                }
                Cmd::Text { x, y, text, tint, clip } => {
                    if let Some(font) = assets.font {
                        r.draw_text_clipped(font, assets.font_pages, text, *x, *y, *tint, *clip);
                    }
                }
                Cmd::Sprite { x, y, w, h, name, tint, clip } => {
                    let drawn = match name {
                        Some(n) => resolve(
                            r,
                            SpriteDraw { name: n, x: *x, y: *y, w: *w, h: *h, tint: *tint, clip: *clip },
                        ),
                        None => false,
                    };
                    if !drawn {
                        // a visible placeholder (no-asset / unavailable) — never blank.
                        crate::common::fill_rect(r, assets.white, *x, *y, *w, *h, crate::common::PLACEHOLDER_TINT, *clip);
                        crate::common::stroke_rect(r, assets.white, *x, *y, *w, *h, 1.0, crate::common::DIM_TINT, *clip);
                    }
                }
            }
        }
    }
}

/// A sprite placement handed to the [`Plan::execute`] resolver: the sprite name + its
/// screen rect + tint + clip, so the caller draws the resolved texture exactly where the
/// plan placed it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpriteDraw<'a> {
    pub name: &'a str,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub tint: [f32; 4],
    pub clip: Option<[f32; 4]>,
}

fn q(v: f32) -> i64 {
    if v.is_finite() {
        (v * 100.0).round() as i64
    } else {
        i64::MIN // a non-finite coord is a distinct, stable token
    }
}
fn qt(t: &[f32; 4]) -> String {
    format!("{},{},{},{}", q(t[0]), q(t[1]), q(t[2]), q(t[3]))
}
fn qc(c: &Option<[f32; 4]>) -> String {
    match c {
        Some(r) => format!("c{},{},{},{}", q(r[0]), q(r[1]), q(r[2]), q(r[3])),
        None => "c-".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_stable_and_order_sensitive() {
        let mut a = Plan::new();
        a.push(Cmd::Rect { x: 1.0, y: 2.0, w: 3.0, h: 4.0, tint: [1.0; 4], clip: None });
        a.push(Cmd::Text { x: 5.0, y: 6.0, text: "hi".into(), tint: [1.0; 4], clip: None });
        let mut b = a.clone();
        assert_eq!(a.digest(), b.digest());
        // reorder -> different digest.
        b.cmds.reverse();
        assert_ne!(a.digest(), b.digest());
    }

    #[test]
    fn digest_distinguishes_measured_from_unavailable_text() {
        let mut zero = Plan::new();
        zero.push(Cmd::Text { x: 0.0, y: 0.0, text: "0".into(), tint: [1.0; 4], clip: None });
        let mut dash = Plan::new();
        dash.push(Cmd::Text { x: 0.0, y: 0.0, text: "—".into(), tint: [1.0; 4], clip: None });
        // a measured 0 and a not-captured — never collide.
        assert_ne!(zero.digest(), dash.digest());
        assert_eq!(dash.unavailable_text_count(), 1);
        assert_eq!(zero.unavailable_text_count(), 0);
    }

    #[test]
    fn nonfinite_coord_is_stable_token_not_panic() {
        let mut p = Plan::new();
        p.push(Cmd::Rect { x: f32::NAN, y: 0.0, w: 1.0, h: 1.0, tint: [1.0; 4], clip: None });
        // digesting a NaN coord does not panic and is deterministic.
        assert_eq!(p.digest(), p.digest());
    }

    #[test]
    fn quantises_subpixel_jitter() {
        let mut a = Plan::new();
        a.push(Cmd::Rect { x: 10.000, y: 0.0, w: 1.0, h: 1.0, tint: [1.0; 4], clip: None });
        let mut b = Plan::new();
        b.push(Cmd::Rect { x: 10.0001, y: 0.0, w: 1.0, h: 1.0, tint: [1.0; 4], clip: None });
        assert_eq!(a.digest(), b.digest()); // <0.01px jitter collapses
    }
}
