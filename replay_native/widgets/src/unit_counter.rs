//! Compact unit-counter foundation for replay map/list polish.
//!
//! A counter has a fixed footprint: four asset slots (owner flag, unit icon,
//! formation badge, leader portrait), one honest count [`Atom`], and strength/org
//! bars. Availability changes never alter slot or bar geometry. Missing and
//! unavailable assets draw visible placeholders with compact badges, while missing
//! text/bar values use the shared no-fabrication glyph via [`Atom`] / [`Reason`].

use render::Renderer;

use crate::common::{
    AssetRef, Atom, DrawAssets, Fidelity, Reason, DERIVED_TINT, DIM_TINT, PLACEHOLDER_TINT,
    TEXT_TINT, UNAVAIL_TINT,
};
use crate::plan::{Cmd, Plan, SpriteDraw};

/// Fixed counter footprint in screen pixels.
pub const UNIT_COUNTER_W: f32 = 168.0;
pub const UNIT_COUNTER_H: f32 = 58.0;
pub const UNIT_COUNTER_SLOT: f32 = 22.0;
pub const UNIT_COUNTER_BAR_H: f32 = 5.0;

const PAD: f32 = 6.0;
const SLOT_GAP: f32 = 4.0;
const COUNT_W: f32 = 42.0;
const BADGE_W: f32 = 11.0;
const BADGE_H: f32 = 10.0;

const BG: [f32; 4] = [0.075, 0.082, 0.095, 0.96];
const BORDER: [f32; 4] = [0.28, 0.30, 0.34, 1.0];
const SLOT_BG: [f32; 4] = [0.13, 0.14, 0.17, 1.0];
const EMPTY_SLOT_BG: [f32; 4] = [0.095, 0.10, 0.12, 1.0];
const COUNT_BG: [f32; 4] = [0.12, 0.13, 0.16, 1.0];
const BAR_TRACK: [f32; 4] = [0.05, 0.055, 0.065, 1.0];
const STRENGTH_FILL: [f32; 4] = [0.36, 0.66, 0.34, 1.0];
const ORG_FILL: [f32; 4] = [0.36, 0.55, 0.84, 1.0];
const MISSING_BADGE_BG: [f32; 4] = [0.48, 0.32, 0.18, 1.0];
const UNAVAIL_BADGE_BG: [f32; 4] = [0.32, 0.22, 0.22, 1.0];

/// The asset state for one fixed counter slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitAsset {
    /// No asset by design. The slot remains present but empty.
    None,
    /// A sprite/portrait/flag name the draw resolver should resolve.
    Named(String),
    /// A named sprite frame, used by formation shields and other atlas-backed art.
    Framed { name: String, frame: u32 },
    /// A sprite was expected and is known missing before draw time.
    Missing(String),
    /// The asset field is blocked/unfed.
    Unavailable,
}

impl UnitAsset {
    pub fn named(name: impl Into<String>) -> UnitAsset {
        let name = name.into();
        if name.is_empty() {
            UnitAsset::Missing(name)
        } else {
            UnitAsset::Named(name)
        }
    }

    pub fn missing(name: impl Into<String>) -> UnitAsset {
        UnitAsset::Missing(name.into())
    }

    pub fn framed(name: impl Into<String>, frame: u32) -> UnitAsset {
        let name = name.into();
        if name.is_empty() {
            UnitAsset::Missing(name)
        } else {
            UnitAsset::Framed { name, frame }
        }
    }

    pub fn unavailable() -> UnitAsset {
        UnitAsset::Unavailable
    }

    pub fn name(&self) -> Option<&str> {
        match self {
            UnitAsset::Named(name) | UnitAsset::Framed { name, .. } => Some(name),
            _ => None,
        }
    }

    pub fn frame(&self) -> Option<u32> {
        match self {
            UnitAsset::Framed { frame, .. } => Some(*frame),
            _ => None,
        }
    }

    fn plan_name(&self) -> Option<String> {
        match self {
            UnitAsset::Named(name) => Some(name.clone()),
            UnitAsset::Framed { name, frame } => Some(format!("{name}#{frame}")),
            _ => None,
        }
    }
}

impl Default for UnitAsset {
    fn default() -> Self {
        UnitAsset::None
    }
}

impl From<AssetRef> for UnitAsset {
    fn from(value: AssetRef) -> Self {
        match value {
            AssetRef::None => UnitAsset::None,
            AssetRef::Named(name) => UnitAsset::named(name),
            AssetRef::Unavailable => UnitAsset::Unavailable,
        }
    }
}

impl From<&str> for UnitAsset {
    fn from(value: &str) -> Self {
        UnitAsset::named(value)
    }
}

impl From<String> for UnitAsset {
    fn from(value: String) -> Self {
        UnitAsset::named(value)
    }
}

/// A strength/org bar value. Fractions are clamped to [0, 1] at plan time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnitBar {
    /// A measured 0..=1 fraction. Out-of-range values badge unavailable.
    Exact(f32),
    /// A derived 0..=1 fraction. Out-of-range values badge unavailable.
    Derived(f32),
    /// A measured value the caller explicitly wants clamped into [0, 1].
    ClampedExact(f32),
    /// A derived value the caller explicitly wants clamped into [0, 1].
    ClampedDerived(f32),
    Unavailable {
        reason: Reason,
    },
}

impl UnitBar {
    pub fn exact(fraction: f32) -> UnitBar {
        UnitBar::Exact(fraction)
    }

    pub fn derived(fraction: f32) -> UnitBar {
        UnitBar::Derived(fraction)
    }

    pub fn clamped_exact(fraction: f32) -> UnitBar {
        UnitBar::ClampedExact(fraction)
    }

    pub fn clamped_derived(fraction: f32) -> UnitBar {
        UnitBar::ClampedDerived(fraction)
    }

    pub fn unavailable() -> UnitBar {
        UnitBar::Unavailable {
            reason: Reason::Absent,
        }
    }

    pub fn unavailable_because(reason: Reason) -> UnitBar {
        UnitBar::Unavailable { reason }
    }

    pub fn clamped_fraction(&self) -> Option<f32> {
        match self {
            UnitBar::Exact(f) | UnitBar::Derived(f) => fraction_if_valid(*f),
            UnitBar::ClampedExact(f) | UnitBar::ClampedDerived(f) => Some(clamp_fraction(*f)),
            UnitBar::Unavailable { .. } => None,
        }
    }

    pub fn fidelity(&self) -> Fidelity {
        match self {
            UnitBar::Exact(_) | UnitBar::ClampedExact(_) => Fidelity::Exact,
            UnitBar::Derived(_) | UnitBar::ClampedDerived(_) => Fidelity::Derived,
            UnitBar::Unavailable { .. } => Fidelity::Unavailable,
        }
    }

    fn fill_tint(&self, base: [f32; 4]) -> [f32; 4] {
        match self {
            UnitBar::Derived(_) | UnitBar::ClampedDerived(_) => DERIVED_TINT,
            UnitBar::Exact(_) | UnitBar::ClampedExact(_) => base,
            UnitBar::Unavailable { .. } => UNAVAIL_TINT,
        }
    }
}

impl Default for UnitBar {
    fn default() -> Self {
        UnitBar::unavailable()
    }
}

/// Fixed geometry for a unit counter. This is returned with each plan so callers can
/// prove data availability did not move slots or bars.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitCounterGeometry {
    pub area: gui_runtime::Rect,
    pub owner: gui_runtime::Rect,
    pub unit: gui_runtime::Rect,
    pub formation: gui_runtime::Rect,
    pub leader: gui_runtime::Rect,
    pub count: gui_runtime::Rect,
    pub strength: gui_runtime::Rect,
    pub org: gui_runtime::Rect,
}

impl UnitCounterGeometry {
    pub fn asset_slots(&self) -> [gui_runtime::Rect; 4] {
        [self.owner, self.unit, self.formation, self.leader]
    }
}

/// Model for one unit counter.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitCounter {
    pub count: Atom,
    pub owner: UnitAsset,
    pub unit: UnitAsset,
    pub formation: UnitAsset,
    pub leader: UnitAsset,
    pub strength: UnitBar,
    pub org: UnitBar,
}

impl UnitCounter {
    pub fn new(count: Atom) -> UnitCounter {
        UnitCounter {
            count,
            owner: UnitAsset::None,
            unit: UnitAsset::None,
            formation: UnitAsset::None,
            leader: UnitAsset::None,
            strength: UnitBar::unavailable(),
            org: UnitBar::unavailable(),
        }
    }

    pub fn owner(mut self, asset: impl Into<UnitAsset>) -> UnitCounter {
        self.owner = asset.into();
        self
    }

    pub fn unit(mut self, asset: impl Into<UnitAsset>) -> UnitCounter {
        self.unit = asset.into();
        self
    }

    pub fn formation(mut self, asset: impl Into<UnitAsset>) -> UnitCounter {
        self.formation = asset.into();
        self
    }

    pub fn leader(mut self, asset: impl Into<UnitAsset>) -> UnitCounter {
        self.leader = asset.into();
        self
    }

    pub fn strength(mut self, value: UnitBar) -> UnitCounter {
        self.strength = value;
        self
    }

    pub fn org(mut self, value: UnitBar) -> UnitCounter {
        self.org = value;
        self
    }
}

/// A unit-counter plan plus the geometry/fill rects used to produce it.
#[derive(Debug, Clone, PartialEq)]
pub struct UnitCounterPlan {
    pub plan: Plan,
    pub geometry: UnitCounterGeometry,
    pub strength_fill: Option<gui_runtime::Rect>,
    pub org_fill: Option<gui_runtime::Rect>,
}

impl UnitCounterPlan {
    pub fn digest(&self) -> String {
        self.plan.digest()
    }
}

/// Return the fixed geometry for a counter whose top-left corner is `(x, y)`.
pub fn layout_unit_counter(x: f32, y: f32) -> UnitCounterGeometry {
    let x = sane(x);
    let y = sane(y);
    let area = gui_runtime::Rect {
        x,
        y,
        w: UNIT_COUNTER_W,
        h: UNIT_COUNTER_H,
    };
    let slot_y = y + PAD;
    let slot = |i: usize| gui_runtime::Rect {
        x: x + PAD + i as f32 * (UNIT_COUNTER_SLOT + SLOT_GAP),
        y: slot_y,
        w: UNIT_COUNTER_SLOT,
        h: UNIT_COUNTER_SLOT,
    };
    let count = gui_runtime::Rect {
        x: x + UNIT_COUNTER_W - PAD - COUNT_W,
        y: y + PAD + 2.0,
        w: COUNT_W,
        h: 18.0,
    };
    let strength = gui_runtime::Rect {
        x: x + PAD,
        y: y + 36.0,
        w: UNIT_COUNTER_W - 2.0 * PAD,
        h: UNIT_COUNTER_BAR_H,
    };
    let org = gui_runtime::Rect {
        x: x + PAD,
        y: y + 47.0,
        w: UNIT_COUNTER_W - 2.0 * PAD,
        h: UNIT_COUNTER_BAR_H,
    };
    UnitCounterGeometry {
        area,
        owner: slot(0),
        unit: slot(1),
        formation: slot(2),
        leader: slot(3),
        count,
        strength,
        org,
    }
}

/// Build a pure render plan for one fixed-footprint unit counter.
pub fn plan_unit_counter(counter: &UnitCounter, x: f32, y: f32) -> UnitCounterPlan {
    let g = layout_unit_counter(x, y);
    let clip = Some([g.area.x, g.area.y, g.area.w, g.area.h]);
    let mut p = Plan::new();

    p.push(Cmd::Rect {
        x: g.area.x,
        y: g.area.y,
        w: g.area.w,
        h: g.area.h,
        tint: BG,
        clip,
    });
    push_border(&mut p, g.area, BORDER, clip);

    push_asset_slot(&mut p, g.owner, &counter.owner, clip);
    push_asset_slot(&mut p, g.unit, &counter.unit, clip);
    push_asset_slot(&mut p, g.formation, &counter.formation, clip);
    push_asset_slot(&mut p, g.leader, &counter.leader, clip);

    p.push(Cmd::Rect {
        x: g.count.x,
        y: g.count.y,
        w: g.count.w,
        h: g.count.h,
        tint: COUNT_BG,
        clip,
    });
    p.push(Cmd::Text {
        x: g.count.x + 5.0,
        y: g.count.y + 3.0,
        text: counter.count.as_str().into(),
        tint: counter.count.tint(TEXT_TINT),
        clip: Some([g.count.x, g.count.y, g.count.w, g.count.h]),
    });

    let strength_fill = push_bar(&mut p, g.strength, counter.strength, STRENGTH_FILL, clip);
    let org_fill = push_bar(&mut p, g.org, counter.org, ORG_FILL, clip);

    UnitCounterPlan {
        plan: p,
        geometry: g,
        strength_fill,
        org_fill,
    }
}

/// Draw one unit counter by executing its plan. Named sprites are resolved by the
/// caller; resolver misses draw the standard placeholder.
pub fn draw_unit_counter(
    r: &mut Renderer,
    counter: &UnitCounter,
    x: f32,
    y: f32,
    assets: &DrawAssets,
    resolve: impl FnMut(&mut Renderer, SpriteDraw) -> bool,
) {
    plan_unit_counter(counter, x, y)
        .plan
        .execute(r, assets, resolve);
}

fn push_asset_slot(
    p: &mut Plan,
    rect: gui_runtime::Rect,
    asset: &UnitAsset,
    clip: Option<[f32; 4]>,
) {
    p.push(Cmd::Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        tint: SLOT_BG,
        clip,
    });
    push_border(p, rect, BORDER, clip);

    match asset {
        UnitAsset::None => {
            p.push(Cmd::Rect {
                x: rect.x + 2.0,
                y: rect.y + 2.0,
                w: rect.w - 4.0,
                h: rect.h - 4.0,
                tint: EMPTY_SLOT_BG,
                clip,
            });
        }
        UnitAsset::Named(_) | UnitAsset::Framed { .. } => {
            p.push(Cmd::Sprite {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
                name: asset.plan_name(),
                tint: [1.0; 4],
                clip,
            });
        }
        UnitAsset::Missing(_) => {
            p.push(Cmd::Sprite {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
                name: None,
                tint: PLACEHOLDER_TINT,
                clip,
            });
            push_badge(p, rect, "M", MISSING_BADGE_BG, clip);
        }
        UnitAsset::Unavailable => {
            p.push(Cmd::Sprite {
                x: rect.x,
                y: rect.y,
                w: rect.w,
                h: rect.h,
                name: None,
                tint: PLACEHOLDER_TINT,
                clip,
            });
            push_badge(p, rect, crate::common::NOT_CAPTURED, UNAVAIL_BADGE_BG, clip);
        }
    }
}

fn push_badge(
    p: &mut Plan,
    rect: gui_runtime::Rect,
    text: &str,
    bg: [f32; 4],
    clip: Option<[f32; 4]>,
) {
    let x = rect.x + rect.w - BADGE_W;
    let y = rect.y + rect.h - BADGE_H;
    p.push(Cmd::Rect {
        x,
        y,
        w: BADGE_W,
        h: BADGE_H,
        tint: bg,
        clip,
    });
    p.push(Cmd::Text {
        x: x + 2.0,
        y: y - 2.0,
        text: text.into(),
        tint: TEXT_TINT,
        clip: Some([x, y, BADGE_W, BADGE_H]),
    });
}

fn push_bar(
    p: &mut Plan,
    rect: gui_runtime::Rect,
    bar: UnitBar,
    fill: [f32; 4],
    clip: Option<[f32; 4]>,
) -> Option<gui_runtime::Rect> {
    p.push(Cmd::Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: rect.h,
        tint: BAR_TRACK,
        clip,
    });
    let fill_rect = match bar.clamped_fraction() {
        Some(fraction) => {
            let fill_rect = gui_runtime::Rect {
                x: rect.x,
                y: rect.y,
                w: rect.w * fraction,
                h: rect.h,
            };
            p.push(Cmd::Rect {
                x: fill_rect.x,
                y: fill_rect.y,
                w: fill_rect.w,
                h: fill_rect.h,
                tint: bar.fill_tint(fill),
                clip,
            });
            Some(fill_rect)
        }
        None => {
            p.push(Cmd::Text {
                x: rect.x + 2.0,
                y: rect.y - 5.0,
                text: crate::common::NOT_CAPTURED.into(),
                tint: UNAVAIL_TINT,
                clip: Some([rect.x, rect.y - 5.0, rect.w, rect.h + 8.0]),
            });
            None
        }
    };
    push_border(p, rect, DIM_TINT, clip);
    fill_rect
}

fn push_border(p: &mut Plan, rect: gui_runtime::Rect, tint: [f32; 4], clip: Option<[f32; 4]>) {
    let t = 1.0;
    p.push(Cmd::Rect {
        x: rect.x,
        y: rect.y,
        w: rect.w,
        h: t,
        tint,
        clip,
    });
    p.push(Cmd::Rect {
        x: rect.x,
        y: rect.y + rect.h - t,
        w: rect.w,
        h: t,
        tint,
        clip,
    });
    p.push(Cmd::Rect {
        x: rect.x,
        y: rect.y,
        w: t,
        h: rect.h,
        tint,
        clip,
    });
    p.push(Cmd::Rect {
        x: rect.x + rect.w - t,
        y: rect.y,
        w: t,
        h: rect.h,
        tint,
        clip,
    });
}

fn sane(v: f32) -> f32 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

fn clamp_fraction(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

fn fraction_if_valid(v: f32) -> Option<f32> {
    if v.is_finite() && (0.0..=1.0).contains(&v) {
        Some(v)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counter() -> UnitCounter {
        UnitCounter::new(Atom::exact("24"))
            .owner(UnitAsset::named("GFX_flag_GER"))
            .unit(UnitAsset::named("GFX_unit_infantry"))
            .formation(UnitAsset::named("GFX_army_group"))
            .leader(UnitAsset::named("GFX_leader_portrait"))
            .strength(UnitBar::exact(0.75))
            .org(UnitBar::derived(0.5))
    }

    #[test]
    fn stable_geometry_and_digest() {
        let first = plan_unit_counter(&counter(), 10.0, 20.0);
        let second = plan_unit_counter(&counter(), 10.0, 20.0);
        assert_eq!(first.geometry, second.geometry);
        assert_eq!(first.digest(), second.digest());

        let unavailable = UnitCounter::new(Atom::unavailable())
            .owner(UnitAsset::Unavailable)
            .unit(UnitAsset::Missing("GFX_missing_unit".into()))
            .formation(UnitAsset::None)
            .leader(UnitAsset::Unavailable)
            .strength(UnitBar::unavailable_because(Reason::Blocked))
            .org(UnitBar::unavailable_because(Reason::Null));
        let unavailable_plan = plan_unit_counter(&unavailable, 10.0, 20.0);
        assert_eq!(first.geometry, unavailable_plan.geometry);
        assert_eq!(
            first.geometry.asset_slots(),
            unavailable_plan.geometry.asset_slots()
        );
    }

    #[test]
    fn missing_and_unavailable_assets_emit_fixed_placeholder_badges() {
        let model = UnitCounter::new(Atom::exact("3"))
            .owner(UnitAsset::Missing("GFX_missing_flag".into()))
            .unit(UnitAsset::Unavailable)
            .formation(UnitAsset::None)
            .leader(UnitAsset::named("GFX_leader"));
        let planned = plan_unit_counter(&model, 0.0, 0.0);

        assert_eq!(planned.plan.placeholder_sprite_count(), 2);
        assert!(planned
            .plan
            .cmds
            .iter()
            .any(|cmd| matches!(cmd, Cmd::Text { text, .. } if text == "M")));
        assert!(planned.plan.cmds.iter().any(
            |cmd| matches!(cmd, Cmd::Text { text, .. } if text == crate::common::NOT_CAPTURED)
        ));

        let named_sprite_count = planned
            .plan
            .cmds
            .iter()
            .filter(|cmd| matches!(cmd, Cmd::Sprite { name: Some(_), .. }))
            .count();
        assert_eq!(named_sprite_count, 1);
    }

    #[test]
    fn bars_are_clamped_and_nonfinite_safe() {
        let model = UnitCounter::new(Atom::exact("8"))
            .strength(UnitBar::clamped_exact(-0.25))
            .org(UnitBar::clamped_exact(1.4));
        let planned = plan_unit_counter(&model, f32::NAN, f32::INFINITY);

        assert_eq!(planned.geometry.area.x, 0.0);
        assert_eq!(planned.geometry.area.y, 0.0);
        assert_eq!(planned.strength_fill.unwrap().w, 0.0);
        assert_eq!(planned.org_fill.unwrap().w, planned.geometry.org.w);

        let nan_model = UnitCounter::new(Atom::exact("8"))
            .strength(UnitBar::clamped_exact(f32::NAN))
            .org(UnitBar::clamped_derived(f32::INFINITY));
        let nan_plan = plan_unit_counter(&nan_model, 0.0, 0.0);
        assert_eq!(nan_plan.strength_fill.unwrap().w, 0.0);
        assert_eq!(nan_plan.org_fill.unwrap().w, 0.0);
        assert_eq!(
            nan_plan.digest(),
            plan_unit_counter(&nan_model, 0.0, 0.0).digest()
        );
    }

    #[test]
    fn out_of_range_fraction_badges_unavailable_unless_clamped() {
        let model = UnitCounter::new(Atom::exact("12"))
            .strength(UnitBar::exact(229.4))
            .org(UnitBar::derived(-0.1));
        let planned = plan_unit_counter(&model, 0.0, 0.0);

        assert!(planned.strength_fill.is_none());
        assert!(planned.org_fill.is_none());
        assert!(planned.plan.unavailable_text_count() >= 2);
    }

    #[test]
    fn framed_formation_asset_survives_digest_key() {
        let model = UnitCounter::new(Atom::exact("5"))
            .formation(UnitAsset::framed("GFX_theatre_army_shield", 2));
        let planned = plan_unit_counter(&model, 0.0, 0.0);

        assert!(planned.plan.cmds.iter().any(
            |cmd| matches!(cmd, Cmd::Sprite { name: Some(name), .. } if name == "GFX_theatre_army_shield#2")
        ));
        assert_eq!(
            planned.digest(),
            plan_unit_counter(&model, 0.0, 0.0).digest()
        );
    }
}
