//! N4 — the DESIGNER MODULE-SLOT LAYOUT: a slot grid + a module icon per slot + the
//! per-slot stat contribution rows. Consumers: all 5 designers (ship / tank / plane /
//! and the two others).
//!
//! Codex N4 hardening: a slot's occupancy is an explicit [`SlotModule`] —
//! `Empty` (a MEASURED empty slot) vs `Known(module)` vs `Unavailable{reason}` (the
//! slot or its module is blocked/unfed) — never an `Option<ModuleIcon>` that conflates
//! the three. A module's icon is an [`AssetRef`] (a missing icon draws a placeholder,
//! not nothing). Each stat contribution keeps its identity + an [`Atom`] value (a
//! blocked contribution badges `—`, never a fabricated 0).
//!
//! ## Usage
//! ```ignore
//! use widgets::designer::*;
//! use widgets::common::{Atom, AssetRef};
//! let layout = ModuleLayout::new(vec![
//!     Slot::known("fixed_main_1", 0, 0,
//!         Module::new(AssetRef::Named("GFX_ship_battery".into()), Atom::exact("Heavy Battery"))
//!             .level(2)
//!             .stat("Attack", Atom::exact("120"))
//!             .stat("Range",  Atom::unavailable())),
//!     Slot::empty("fixed_aux_1", 1, 0),
//!     Slot::unavailable("rear_1", 2, 0, "module behind a gfx wall"),
//! ]);
//! // draw_modules(renderer, &layout, area, &assets, resolve_sprite);
//! ```

use render::Renderer;

use crate::common::{AssetRef, Atom, DrawAssets, DIM_TINT, TEXT_TINT, UNAVAIL_TINT};
use crate::plan::{Cmd, Plan, SpriteDraw};

/// The slot cell footprint in screen px (icon box + the stat rows beneath).
pub const SLOT_W: f32 = 150.0;
pub const SLOT_H: f32 = 120.0;
pub const ICON_BOX: f32 = 56.0;

/// One stat contribution of a module: its identity + an [`Atom`] value (no-fabrication;
/// a blocked stat is `—`). Kept as a labelled pair so the row never loses the stat name.
#[derive(Debug, Clone, PartialEq)]
pub struct Contribution {
    pub name: String,
    pub value: Atom,
}

/// A module sitting in a slot: an icon [`AssetRef`] + a name [`Atom`] + an optional
/// level/variant badge + its stat contributions.
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub icon: AssetRef,
    pub name: Atom,
    /// An optional level/variant marker (e.g. "II") — None = no badge (not a fake 0).
    pub level: Option<u32>,
    pub contributions: Vec<Contribution>,
}

impl Module {
    pub fn new(icon: AssetRef, name: Atom) -> Module {
        Module {
            icon,
            name,
            level: None,
            contributions: Vec::new(),
        }
    }
    pub fn level(mut self, level: u32) -> Module {
        self.level = Some(level);
        self
    }
    /// Add a stat contribution (builder-chained).
    pub fn stat(mut self, name: impl Into<String>, value: Atom) -> Module {
        self.contributions.push(Contribution {
            name: name.into(),
            value,
        });
        self
    }
}

/// A slot's occupancy — the Codex N4 trichotomy (do NOT conflate empty / known /
/// blocked).
#[derive(Debug, Clone, PartialEq)]
pub enum SlotModule {
    /// A MEASURED empty slot (we know it holds no module) — drawn as an empty frame.
    Empty,
    /// A module is in the slot.
    Known(Module),
    /// The slot or its contents are blocked/unfed — drawn as a `—` no-data slot.
    Unavailable { reason: String },
}

/// One placed slot: a grid cell + a name + its occupancy.
#[derive(Debug, Clone, PartialEq)]
pub struct Slot {
    pub id: String,
    pub gx: i32,
    pub gy: i32,
    /// The slot's own name (e.g. "Fixed Main Battery").
    pub slot_name: Atom,
    pub module: SlotModule,
}

impl Slot {
    pub fn known(id: impl Into<String>, gx: i32, gy: i32, module: Module) -> Slot {
        Slot {
            id: id.into(),
            gx,
            gy,
            slot_name: Atom::exact(""),
            module: SlotModule::Known(module),
        }
    }
    pub fn empty(id: impl Into<String>, gx: i32, gy: i32) -> Slot {
        Slot {
            id: id.into(),
            gx,
            gy,
            slot_name: Atom::exact(""),
            module: SlotModule::Empty,
        }
    }
    pub fn unavailable(id: impl Into<String>, gx: i32, gy: i32, reason: impl Into<String>) -> Slot {
        Slot {
            id: id.into(),
            gx,
            gy,
            slot_name: Atom::exact(""),
            module: SlotModule::Unavailable {
                reason: reason.into(),
            },
        }
    }
    /// Set the slot's display name (builder-chained).
    pub fn named(mut self, name: Atom) -> Slot {
        self.slot_name = name;
        self
    }
    /// The slot's screen rect within `area` (gridded by [`SLOT_W`]/[`SLOT_H`]).
    pub fn rect(&self, area: gui_runtime::Rect) -> gui_runtime::Rect {
        gui_runtime::Rect {
            x: area.x + self.gx as f32 * SLOT_W,
            y: area.y + self.gy as f32 * SLOT_H,
            w: SLOT_W - 6.0,
            h: SLOT_H - 6.0,
        }
    }
}

/// The full designer layout: the slots.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModuleLayout {
    pub slots: Vec<Slot>,
}

impl ModuleLayout {
    pub fn new(slots: Vec<Slot>) -> ModuleLayout {
        ModuleLayout { slots }
    }
    /// Hit-test a click against the slots within `area`. Returns the slot id under the
    /// point (front-most wins). `None` off every slot.
    pub fn pick(&self, area: gui_runtime::Rect, x: f32, y: f32) -> Option<String> {
        let mut hit = None;
        for s in &self.slots {
            if s.rect(area).contains(x, y) {
                hit = Some(s.id.clone());
            }
        }
        hit
    }
    /// Counts (empty / known / unavailable) for the per-view accounting test.
    pub fn occupancy_counts(&self) -> (usize, usize, usize) {
        let (mut e, mut k, mut u) = (0, 0, 0);
        for s in &self.slots {
            match s.module {
                SlotModule::Empty => e += 1,
                SlotModule::Known(_) => k += 1,
                SlotModule::Unavailable { .. } => u += 1,
            }
        }
        (e, k, u)
    }
}

const SLOT_BG: [f32; 4] = [0.10, 0.11, 0.14, 1.0];
const SLOT_BORDER: [f32; 4] = [0.30, 0.32, 0.38, 1.0];
const EMPTY_BORDER: [f32; 4] = [0.22, 0.24, 0.28, 1.0];
const ICON_BG: [f32; 4] = [0.16, 0.17, 0.21, 1.0];

/// Build the render plan for the designer layout within `area`. A `Known` slot draws
/// its icon (a `Sprite` cmd the executor resolves; a miss → placeholder) + name + level
/// + stat rows; an `Empty` slot draws an empty dashed frame; an `Unavailable` slot draws
/// a `—` no-data frame. NEVER a fabricated module/stat.
pub fn plan_modules(layout: &ModuleLayout, area: gui_runtime::Rect) -> Plan {
    let mut p = Plan::new();
    let area_clip = Some([area.x, area.y, area.w, area.h]);
    for s in &layout.slots {
        let r = s.rect(area);
        let clip = Some([r.x, r.y, r.w, r.h]);
        // slot frame.
        p.push(Cmd::Rect {
            x: r.x,
            y: r.y,
            w: r.w,
            h: r.h,
            tint: SLOT_BG,
            clip: area_clip,
        });
        let icon_x = r.x + 6.0;
        let icon_y = r.y + 6.0;
        match &s.module {
            SlotModule::Known(m) => {
                push_border(&mut p, r, SLOT_BORDER, clip);
                // icon box backdrop + the module sprite (resolve→placeholder on miss).
                p.push(Cmd::Rect {
                    x: icon_x,
                    y: icon_y,
                    w: ICON_BOX,
                    h: ICON_BOX,
                    tint: ICON_BG,
                    clip,
                });
                p.push(Cmd::Sprite {
                    x: icon_x,
                    y: icon_y,
                    w: ICON_BOX,
                    h: ICON_BOX,
                    name: m.icon.name().map(|s| s.to_string()),
                    tint: [1.0; 4],
                    clip,
                });
                // module name (right of icon).
                let tx = icon_x + ICON_BOX + 6.0;
                p.push(Cmd::Text {
                    x: tx,
                    y: icon_y,
                    text: m.name.as_str().into(),
                    tint: m.name.tint(TEXT_TINT),
                    clip,
                });
                if let Some(lvl) = m.level {
                    p.push(Cmd::Text {
                        x: tx,
                        y: icon_y + 16.0,
                        text: format!("Mk {}", roman(lvl)),
                        tint: DIM_TINT,
                        clip,
                    });
                }
                // stat contribution rows beneath the icon.
                let mut sy = icon_y + ICON_BOX + 6.0;
                for c in &m.contributions {
                    p.push(Cmd::Text {
                        x: r.x + 6.0,
                        y: sy,
                        text: c.name.clone(),
                        tint: DIM_TINT,
                        clip,
                    });
                    p.push(Cmd::Text {
                        x: r.x + 90.0,
                        y: sy,
                        text: c.value.as_str().into(),
                        tint: c.value.tint(TEXT_TINT),
                        clip,
                    });
                    sy += 16.0;
                }
            }
            SlotModule::Empty => {
                // a measured-empty slot: a dim dashed frame + "Empty" caption.
                push_border(&mut p, r, EMPTY_BORDER, clip);
                p.push(Cmd::Rect {
                    x: icon_x,
                    y: icon_y,
                    w: ICON_BOX,
                    h: ICON_BOX,
                    tint: crate::common::PLACEHOLDER_TINT,
                    clip,
                });
                p.push(Cmd::Text {
                    x: icon_x + ICON_BOX + 6.0,
                    y: icon_y,
                    text: "Empty".into(),
                    tint: DIM_TINT,
                    clip,
                });
            }
            SlotModule::Unavailable { .. } => {
                // a blocked slot: a `—` no-data frame (distinct from Empty).
                push_border(&mut p, r, EMPTY_BORDER, clip);
                p.push(Cmd::Rect {
                    x: icon_x,
                    y: icon_y,
                    w: ICON_BOX,
                    h: ICON_BOX,
                    tint: crate::common::PLACEHOLDER_TINT,
                    clip,
                });
                p.push(Cmd::Text {
                    x: icon_x + ICON_BOX + 6.0,
                    y: icon_y,
                    text: crate::common::NOT_CAPTURED.into(),
                    tint: UNAVAIL_TINT,
                    clip,
                });
            }
        }
        // the slot's own name across the bottom (if any).
        if let Atom::Text { text, .. } = &s.slot_name {
            if !text.is_empty() {
                p.push(Cmd::Text {
                    x: r.x + 6.0,
                    y: r.y + r.h - 16.0,
                    text: text.clone(),
                    tint: DIM_TINT,
                    clip,
                });
            }
        }
    }
    p
}

/// Draw the designer layout (plan + execute). `resolve` resolves a module sprite name
/// to a texture; a miss draws the placeholder (Codex N4: a missing icon is visible).
pub fn draw_modules(
    r: &mut Renderer,
    layout: &ModuleLayout,
    area: gui_runtime::Rect,
    assets: &DrawAssets,
    resolve: impl FnMut(&mut Renderer, SpriteDraw) -> bool,
) {
    plan_modules(layout, area).execute(r, assets, resolve);
}

fn push_border(p: &mut Plan, r: gui_runtime::Rect, c: [f32; 4], clip: Option<[f32; 4]>) {
    let t = 1.0;
    p.push(Cmd::Rect {
        x: r.x,
        y: r.y,
        w: r.w,
        h: t,
        tint: c,
        clip,
    });
    p.push(Cmd::Rect {
        x: r.x,
        y: r.y + r.h - t,
        w: r.w,
        h: t,
        tint: c,
        clip,
    });
    p.push(Cmd::Rect {
        x: r.x,
        y: r.y,
        w: t,
        h: r.h,
        tint: c,
        clip,
    });
    p.push(Cmd::Rect {
        x: r.x + r.w - t,
        y: r.y,
        w: t,
        h: r.h,
        tint: c,
        clip,
    });
}

/// Tiny roman numeral for a module mark (1..=10; >10 falls back to the decimal).
fn roman(n: u32) -> String {
    match n {
        1 => "I",
        2 => "II",
        3 => "III",
        4 => "IV",
        5 => "V",
        6 => "VI",
        7 => "VII",
        8 => "VIII",
        9 => "IX",
        10 => "X",
        _ => return n.to_string(),
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> ModuleLayout {
        ModuleLayout::new(vec![
            Slot::known(
                "main",
                0,
                0,
                Module::new(
                    AssetRef::Named("GFX_battery".into()),
                    Atom::exact("Heavy Battery"),
                )
                .level(2)
                .stat("Attack", Atom::exact("120"))
                .stat("Range", Atom::unavailable()),
            ),
            Slot::empty("aux", 1, 0),
            Slot::unavailable("rear", 2, 0, "behind a gfx wall"),
        ])
    }

    #[test]
    fn occupancy_counts_distinguish_three_states() {
        assert_eq!(layout().occupancy_counts(), (1, 1, 1));
    }

    #[test]
    fn empty_layout_no_panic() {
        let l = ModuleLayout::default();
        let area = gui_runtime::Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 300.0,
        };
        assert_eq!(l.pick(area, 10.0, 10.0), None);
        assert!(plan_modules(&l, area).is_empty());
    }

    #[test]
    fn pick_returns_slot_under_point() {
        let l = layout();
        let area = gui_runtime::Rect {
            x: 0.0,
            y: 0.0,
            w: 600.0,
            h: 300.0,
        };
        let r = l.slots[1].rect(area);
        assert_eq!(l.pick(area, r.x + 4.0, r.y + 4.0).as_deref(), Some("aux"));
    }

    #[test]
    fn known_slot_with_missing_icon_emits_resolvable_sprite() {
        let l = layout();
        let area = gui_runtime::Rect {
            x: 0.0,
            y: 0.0,
            w: 600.0,
            h: 300.0,
        };
        let p = plan_modules(&l, area);
        // the known slot emitted a Sprite cmd (executor resolves -> placeholder on miss).
        let has_sprite = p
            .cmds
            .iter()
            .any(|c| matches!(c, Cmd::Sprite { name: Some(_), .. }));
        assert!(has_sprite);
    }

    #[test]
    fn blocked_stat_draws_glyph_not_zero() {
        let l = layout();
        let area = gui_runtime::Rect {
            x: 0.0,
            y: 0.0,
            w: 600.0,
            h: 300.0,
        };
        let p = plan_modules(&l, area);
        // the "Range" unavailable stat + the unavailable slot caption -> at least 2 "—".
        assert!(p.unavailable_text_count() >= 2);
        // and the measured Attack "120" is present (a real value, not collapsed).
        assert!(p
            .cmds
            .iter()
            .any(|c| matches!(c, Cmd::Text { text, .. } if text == "120")));
    }

    #[test]
    fn plan_is_deterministic() {
        let l = layout();
        let area = gui_runtime::Rect {
            x: 0.0,
            y: 0.0,
            w: 600.0,
            h: 300.0,
        };
        assert_eq!(
            plan_modules(&l, area).digest(),
            plan_modules(&l, area).digest()
        );
    }
}
