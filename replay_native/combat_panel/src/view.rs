//! Draw side of the combat-panel adapter (plan §C.2 + §G): the map BUBBLES and the
//! stacked COMBAT PANEL (landcombat view), reusing the `render` quad/sprite/text
//! primitives. A 1x1 white texture tinted per element gives the bubble discs, the
//! panel chrome, and the "UNAVAILABLE — " badges; real country flags load through
//! the `flag_path` resolver when available.
//!
//! No-fabrication is enforced VISUALLY: every battle-DETAIL row renders the dash
//! "—" with an UNAVAILABLE tag, and the attacker side shows its TAG as "—" (we
//! never paint a fabricated owner colour or flag for the unresolved attacker).

use render::{Renderer, Texture};

use crate::model::{CombatVM, Field, SideTag};

/// A 1x1 white texture (tinted per draw) — the substrate for discs/chrome/badges.
pub fn white_texture(renderer: &Renderer) -> Texture {
    renderer.upload_rgba8_image(1, 1, &[255, 255, 255, 255])
}

/// Assets the draw needs (resolved per app/demo): the white pixel, an optional
/// font, and optional per-tag flag textures (keyed by TAG; absent → no flag drawn).
pub struct DrawAssets<'a> {
    pub white: &'a Texture,
    pub font: Option<&'a assets::BmFont>,
    pub font_pages: &'a [Texture],
    /// TAG -> resolved flag texture. Only DERIVED holding tags get a flag; the
    /// unavailable attacker never does.
    pub flags: &'a std::collections::HashMap<String, Texture>,
}

/// Colours (RGBA straight-alpha, 0..1).
const PANEL_BG: [f32; 4] = [0.10, 0.11, 0.14, 0.97];
const PANEL_HEADER: [f32; 4] = [0.16, 0.18, 0.23, 1.0];
const ATTACKER_C: [f32; 4] = [0.86, 0.36, 0.30, 1.0];
const DEFENDER_C: [f32; 4] = [0.34, 0.58, 0.86, 1.0];
const CONTESTED_C: [f32; 4] = [0.80, 0.70, 0.25, 1.0];
const TEXT_C: [f32; 4] = [0.93, 0.93, 0.90, 1.0];
const DIM_C: [f32; 4] = [0.62, 0.64, 0.68, 1.0];
const UNAVAIL_C: [f32; 4] = [0.55, 0.45, 0.40, 1.0];
const ROW_ALT: [f32; 4] = [0.13, 0.14, 0.18, 1.0];

fn fill(
    r: &mut Renderer,
    white: &Texture,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    c: [f32; 4],
    clip: [f32; 4],
) {
    r.draw_sprite_frame_clipped(white, x, y, w, h, 0, 1, c, Some(clip));
}

/// Draw a filled disc by stacking horizontal scanline rects (cheap, crisp enough
/// for a small bubble; avoids needing a circle texture).
fn disc(
    r: &mut Renderer,
    white: &Texture,
    cx: f32,
    cy: f32,
    radius: f32,
    c: [f32; 4],
    clip: [f32; 4],
) {
    let rad = radius.max(1.0);
    let steps = (rad * 2.0).ceil() as i32;
    for i in 0..steps {
        let dy = -rad + (i as f32) + 0.5;
        if dy.abs() > rad {
            continue;
        }
        let half = (rad * rad - dy * dy).max(0.0).sqrt();
        fill(r, white, cx - half, cy + dy, half * 2.0, 1.0, c, clip);
    }
}

/// Per-frame draw stats (verification + the legend).
#[derive(Debug, Default, Clone)]
pub struct CombatDrawStats {
    pub bubbles_drawn: usize,
    pub panel_drawn: bool,
    pub unavailable_rows: usize,
    pub exact_rows: usize,
    pub derived_rows: usize,
}

/// Draw the COMBAT BUBBLES over the map. Owner-aware: the disc is tinted by the
/// DEFENDER holding tag's side colour when known, else a neutral contested tint;
/// `selected` (the open combat) gets a bright ring. Clipped to `clip`.
pub fn draw_bubbles(
    r: &mut Renderer,
    white: &Texture,
    bubbles: &[crate::bubbles::Bubble],
    selected: Option<&str>,
    clip: [f32; 4],
) -> usize {
    let mut n = 0;
    for b in bubbles {
        let base = if b.defender_tag.is_some() {
            DEFENDER_C
        } else {
            CONTESTED_C
        };
        // outer ring (selection highlight or subtle dark rim)
        let ring = if selected == Some(b.combat_id.as_str()) {
            [1.0, 0.95, 0.55, 1.0]
        } else {
            [0.05, 0.05, 0.07, 0.9]
        };
        disc(r, white, b.sx, b.sy, b.radius + 2.0, ring, clip);
        disc(r, white, b.sx, b.sy, b.radius, base, clip);
        // inner highlight dot
        disc(
            r,
            white,
            b.sx,
            b.sy,
            (b.radius * 0.35).max(2.0),
            [1.0, 1.0, 1.0, 0.85],
            clip,
        );
        n += 1;
    }
    n
}

/// Draw the stacked, read-only COMBAT PANEL for one combat at screen rect
/// `[px,py,pw,ph]`. Returns the draw stats (incl. the unavailable-row count, the
/// no-fabrication proof). `close_btn` (out) receives the close control's screen
/// rect so the shell can route a click there.
#[allow(clippy::too_many_arguments)]
pub fn draw_panel(
    r: &mut Renderer,
    vm: &CombatVM,
    assets: &DrawAssets,
    px: f32,
    py: f32,
    pw: f32,
    ph: f32,
) -> (CombatDrawStats, CloseButton) {
    let clip = [px, py, pw, ph];
    let mut stats = CombatDrawStats {
        panel_drawn: true,
        ..Default::default()
    };
    let white = assets.white;

    // panel background + header.
    fill(r, white, px, py, pw, ph, PANEL_BG, clip);
    let hh = 30.0;
    fill(r, white, px, py, pw, hh, PANEL_HEADER, clip);

    let pad = 12.0;
    let mut y = py + hh + 8.0;
    let line = 20.0;

    // header: title + close button (top-right).
    if let Some(f) = assets.font {
        let title = format!("LAND COMBAT  \u{2014}  {}", vm.window_title());
        r.draw_text_clipped(
            f,
            assets.font_pages,
            &title,
            px + pad,
            py + 7.0,
            TEXT_C,
            Some(clip),
        );
    }
    let cb = CloseButton {
        x: px + pw - 26.0,
        y: py + 5.0,
        w: 20.0,
        h: 20.0,
    };
    fill(
        r,
        white,
        cb.x,
        cb.y,
        cb.w,
        cb.h,
        [0.45, 0.16, 0.16, 1.0],
        clip,
    );
    if let Some(f) = assets.font {
        r.draw_text_clipped(
            f,
            assets.font_pages,
            "X",
            cb.x + 6.0,
            cb.y + 3.0,
            TEXT_C,
            Some(clip),
        );
    }

    // --- EXACT / DERIVED lifecycle + location block ---
    let label = |r: &mut Renderer, y: f32, k: &str, v: &str, vc: [f32; 4], assets: &DrawAssets| {
        if let Some(f) = assets.font {
            r.draw_text_clipped(
                f,
                assets.font_pages,
                k,
                px + pad,
                y,
                DIM_C,
                Some([px, py, pw, ph]),
            );
            r.draw_text_clipped(
                f,
                assets.font_pages,
                v,
                px + pad + 168.0,
                y,
                vc,
                Some([px, py, pw, ph]),
            );
        }
    };

    label(r, y, "Combat id (exact)", &vm.combat_id, TEXT_C, assets);
    y += line;
    label(r, y, "Kind (exact)", &vm.kind, TEXT_C, assets);
    y += line;
    label(
        r,
        y,
        "Location province (exact)",
        &vm.location_province.to_string(),
        TEXT_C,
        assets,
    );
    y += line;
    let (loc_v, loc_c) = match &vm.location_name {
        Field::Value(s) => (s.clone(), TEXT_C),
        Field::Unavailable { .. } => ("\u{2014}".into(), UNAVAIL_C),
    };
    label(r, y, "Province name (derived)", &loc_v, loc_c, assets);
    if vm.location_name.is_unavailable() {
        stats.unavailable_rows += 1
    } else {
        stats.derived_rows += 1
    }
    y += line;
    let (dur_v, dur_c) = match &vm.duration_days {
        Field::Value(d) => (format!("{d} days"), TEXT_C),
        Field::Unavailable { .. } => ("\u{2014}".into(), UNAVAIL_C),
    };
    label(r, y, "Duration (derived, \u{a7}G)", &dur_v, dur_c, assets);
    if vm.duration_days.is_unavailable() {
        stats.unavailable_rows += 1
    } else {
        stats.derived_rows += 1
    }
    y += line + 6.0;
    stats.exact_rows += 3; // id, kind, province id

    // --- SIDE ROSTERS (the §C.2 aggregation) ---
    let col_w = (pw - 2.0 * pad - 12.0) / 2.0;
    let side_top = y;
    let side_h = 132.0;
    draw_side(
        r,
        assets,
        vm,
        crate::model::Side::Attacker,
        px + pad,
        side_top,
        col_w,
        side_h,
        &mut stats,
    );
    draw_side(
        r,
        assets,
        vm,
        crate::model::Side::Defender,
        px + pad + col_w + 12.0,
        side_top,
        col_w,
        side_h,
        &mut stats,
    );
    y = side_top + side_h + 10.0;

    // --- BATTLE-DETAIL block: §G HARD LIMIT, every row UNAVAILABLE "—" ---
    if let Some(f) = assets.font {
        r.draw_text_clipped(
            f,
            assets.font_pages,
            "BATTLE DETAIL  (\u{a7}G hard limit \u{2014} uncracked / runtime-computed)",
            px + pad,
            y,
            [0.78, 0.66, 0.55, 1.0],
            Some(clip),
        );
    }
    y += line + 2.0;
    let details: [(&str, &dyn DetailField); 8] = [
        ("Combat width", &vm.combat_width),
        ("Terrain", &vm.terrain),
        ("Progress bar (-1..1)", &vm.progress_bar),
        ("Tactics", &vm.tactics),
        ("Combat modifiers", &vm.modifiers_vector),
        ("Air support", &vm.air_support),
        ("Leaders (name/skill)", &vm.leaders),
        ("Per-division stat rows", &vm.division_stat_rows),
    ];
    for (i, (k, fld)) in details.iter().enumerate() {
        if i % 2 == 1 {
            fill(
                r,
                white,
                px + pad - 4.0,
                y - 2.0,
                pw - 2.0 * pad + 8.0,
                line,
                ROW_ALT,
                clip,
            );
        }
        if let Some(f) = assets.font {
            r.draw_text_clipped(f, assets.font_pages, k, px + pad, y, DIM_C, Some(clip));
            r.draw_text_clipped(
                f,
                assets.font_pages,
                "\u{2014}  UNAVAILABLE",
                px + pad + 200.0,
                y,
                UNAVAIL_C,
                Some(clip),
            );
        }
        debug_assert!(
            fld.is_unavailable(),
            "battle-detail must be unavailable: {k}"
        );
        stats.unavailable_rows += 1;
        y += line;
    }

    (stats, cb)
}

/// One side's roster column (flag + tag + ref count + the no-division badge).
#[allow(clippy::too_many_arguments)]
fn draw_side(
    r: &mut Renderer,
    assets: &DrawAssets,
    vm: &CombatVM,
    side: crate::model::Side,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    stats: &mut CombatDrawStats,
) {
    let clip = [x, y, w, h];
    let white = assets.white;
    let (roster, accent, title) = match side {
        crate::model::Side::Attacker => (&vm.attacker, ATTACKER_C, "ATTACKER"),
        crate::model::Side::Defender => (&vm.defender, DEFENDER_C, "DEFENDER"),
    };
    fill(r, white, x, y, w, h, [0.08, 0.085, 0.11, 1.0], clip);
    fill(r, white, x, y, w, 22.0, accent, clip);
    if let Some(f) = assets.font {
        r.draw_text_clipped(
            f,
            assets.font_pages,
            title,
            x + 8.0,
            y + 4.0,
            [0.06, 0.06, 0.08, 1.0],
            Some(clip),
        );
    }

    // TAG + flag.
    let mut cy = y + 30.0;
    let flag_x = x + 8.0;
    let (tag_text, tag_c) = match &roster.tag {
        SideTag::HoldingTagDerived(t) => {
            if let Some(tex) = assets.flags.get(t) {
                r.draw_sprite_frame_clipped(
                    tex,
                    flag_x,
                    cy,
                    48.0,
                    30.0,
                    0,
                    1,
                    [1.0; 4],
                    Some(clip),
                );
            } else {
                fill(r, white, flag_x, cy, 48.0, 30.0, accent, clip);
            }
            (format!("{t}  (province controller, derived)"), TEXT_C)
        }
        SideTag::Unavailable => {
            // NO fabricated flag/colour for the unresolved attacker — a hollow slot.
            fill(
                r,
                white,
                flag_x,
                cy,
                48.0,
                30.0,
                [0.14, 0.14, 0.16, 1.0],
                clip,
            );
            if let Some(f) = assets.font {
                r.draw_text_clipped(
                    f,
                    assets.font_pages,
                    "\u{2014}",
                    flag_x + 18.0,
                    cy + 8.0,
                    UNAVAIL_C,
                    Some(clip),
                );
            }
            ("\u{2014}  (TAG unavailable)".to_string(), UNAVAIL_C)
        }
    };
    if let Some(f) = assets.font {
        r.draw_text_clipped(
            f,
            assets.font_pages,
            &tag_text,
            flag_x + 56.0,
            cy + 2.0,
            tag_c,
            Some(clip),
        );
    }
    if roster.tag == SideTag::Unavailable {
        stats.unavailable_rows += 1
    } else {
        stats.derived_rows += 1
    }
    cy += 38.0;

    // ref count — EXACT when the array is present (incl. a measured 0), "—" when
    // absent (Codex guardrail). Labelled "leader/command refs", NEVER "divisions".
    if let Some(f) = assets.font {
        let rcf = roster.ref_count_field();
        let rc = format!("Leader/command refs (exact): {}", rcf.display());
        r.draw_text_clipped(f, assets.font_pages, &rc, x + 8.0, cy, TEXT_C, Some(clip));
        if rcf.is_unavailable() {
            stats.unavailable_rows += 1
        } else {
            stats.exact_rows += 1
        }
        cy += 20.0;
        // the committed/reserve DIVISION roster is not present -> unavailable badge.
        r.draw_text_clipped(
            f,
            assets.font_pages,
            "Division roster: \u{2014} UNAVAILABLE",
            x + 8.0,
            cy,
            UNAVAIL_C,
            Some(clip),
        );
        stats.unavailable_rows += 1;
    }
}

/// The close button's screen rect (so the shell can route a click to close).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CloseButton {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl CloseButton {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// Tiny trait so the battle-detail loop can assert unavailability uniformly across
/// the differently-typed `Field<T>`s.
trait DetailField {
    fn is_unavailable(&self) -> bool;
}
impl<T> DetailField for Field<T> {
    fn is_unavailable(&self) -> bool {
        Field::is_unavailable(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_button_hit_test() {
        let cb = CloseButton {
            x: 100.0,
            y: 10.0,
            w: 20.0,
            h: 20.0,
        };
        assert!(cb.contains(110.0, 20.0));
        assert!(!cb.contains(99.0, 20.0));
        assert!(!cb.contains(110.0, 31.0));
    }
}
