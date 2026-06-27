//! The designer view-model + its render PLAN + scrub digest.
//!
//! A [`DesignerModel`] is the PURE bound output of a designer's `bind` (no GPU): the
//! header atoms, the N4 [`ModuleLayout`] (module-slot grid OR division regiment grid), the
//! inline upgrade-track levels, the optional N2 [`TabBar`] (division normal/HQ), and the
//! COMPUTED stat panels badged `unavailable`. [`DesignerModel::plan`] turns it into a
//! [`widgets::Plan`] (N4 module plan + a header strip + the tab bar + the stat badges)
//! executed over the real `.gui` chrome; [`DesignerModel::digest`] is the scrub byte-
//! identity key (it captures every data-determining input).
//!
//! NO-FABRICATION: every visible value is a [`widgets::Atom`] — a blocked/absent field
//! draws the not-captured glyph (`—`), never a fabricated 0. The computed stat panels are
//! a labelled `Unavailable` badge list (the §G hard limit), never a number.

use gui_runtime::Rect;

use widgets::common::{Atom, DIM_TINT, TEXT_TINT, UNAVAIL_TINT};
use widgets::designer::{plan_modules, ModuleLayout};
use widgets::plan::{Cmd, Plan};
use widgets::tabs::{plan_tabs, TabBar};

use crate::host::context::{DisplayAtom, FidelityReport};

/// The label font the designer overlay draws with (matches the app's other chrome).
pub const LABEL_FONT: &str = "hoi_16mbs";

/// A computed stat-panel slot badged unavailable (the §G hard limit). Carries its label
/// so the overlay reads "Combat Stats: —" — an honest no-data row, never a fabricated 0.
#[derive(Debug, Clone, PartialEq)]
pub struct StatBadge {
    pub label: String,
}

/// The bound view-model for ONE designer view.
#[derive(Debug, Clone, Default)]
pub struct DesignerModel {
    /// header rows: `(label, value-atom)` — experience / name / archetype / version / etc.
    pub header: Vec<(String, Atom)>,
    /// the N4 module-slot grid (component designers) OR regiment grid (division designer).
    pub modules: ModuleLayout,
    /// inline upgrade tracks: `(track-name, level-atom)` (EXACT levels; derived names).
    pub upgrades: Vec<(String, Atom)>,
    /// the optional N2 tab bar (division designer: normal vs HQ).
    pub tabs: Option<TabBar>,
    /// the COMPUTED stat panels, badged unavailable (never fabricated).
    pub stat_badges: Vec<StatBadge>,
}

impl DesignerModel {
    pub fn new() -> DesignerModel {
        DesignerModel::default()
    }

    /// Push a header `(label, atom)` and tally it into the fidelity report.
    pub fn push_header(&mut self, report: &mut FidelityReport, label: &str, atom: DisplayAtom) {
        report.record(&atom);
        self.header
            .push((label.to_string(), super::to_watom(&atom)));
    }

    /// Push a header badged `unavailable` (a variant/template absent for this country).
    pub fn push_header_unavailable(&mut self, report: &mut FidelityReport, label: &str) {
        let atom =
            DisplayAtom::unavailable_because(crate::host::context::UnavailableReason::Absent);
        report.record(&atom);
        self.header
            .push((label.to_string(), super::to_watom(&atom)));
    }

    /// Push a COMPUTED stat panel badged unavailable (§G hard limit) — recorded in the
    /// fidelity report as unavailable, NEVER bound to a fabricated value.
    pub fn push_stat_unavailable(&mut self, report: &mut FidelityReport, label: &str) {
        let atom =
            DisplayAtom::unavailable_because(crate::host::context::UnavailableReason::Blocked);
        report.record(&atom);
        self.stat_badges.push(StatBadge {
            label: label.to_string(),
        });
    }

    /// A STABLE digest of every data-determining input (header atoms + N4 occupancy +
    /// per-slot module/contribution stream + upgrade levels + tab state + stat badges).
    /// Two models render the same pixels iff this is equal → the scrub A→B→A key.
    pub fn digest(&self, area: Rect) -> String {
        let mut s = String::new();
        // header.
        s.push_str("H[");
        for (l, a) in &self.header {
            s.push_str(&format!("{l}={}({});", a.as_str(), fid(a)));
        }
        s.push(']');
        // module / regiment grid: the N4 plan digest IS the geometry+icon+stat stream.
        let (e, k, u) = self.modules.occupancy_counts();
        s.push_str(&format!(
            "M[occ={e}/{k}/{u};{}]",
            plan_modules(&self.modules, area).digest()
        ));
        // upgrades.
        s.push_str("U[");
        for (l, a) in &self.upgrades {
            s.push_str(&format!("{l}={}({});", a.as_str(), fid(a)));
        }
        s.push(']');
        // tabs.
        match &self.tabs {
            Some(t) => s.push_str(&format!("T[sel={:?};n={}]", t.selected(), t.tabs.len())),
            None => s.push_str("T[none]"),
        }
        // stat badges (all unavailable).
        s.push_str("S[");
        for b in &self.stat_badges {
            s.push_str(&format!("{}=—;", b.label));
        }
        s.push(']');
        s
    }

    /// Build the render PLAN for the overlay within `area`: the header strip, the N4
    /// module/regiment grid, the upgrade-track list, the optional N2 tab bar, and the
    /// computed stat-badge column. Pure — no GPU, headlessly testable + digest-stable.
    pub fn plan(&self, area: Rect) -> Plan {
        let mut p = Plan::new();
        let clip = Some([area.x, area.y, area.w, area.h]);
        let mut y = area.y;

        // 1) header strip (label: value) — value atom tinted by fidelity.
        for (label, atom) in &self.header {
            p.push(Cmd::Text {
                x: area.x,
                y,
                text: format!("{label}:"),
                tint: DIM_TINT,
                clip,
            });
            p.push(Cmd::Text {
                x: area.x + 140.0,
                y,
                text: atom.as_str().into(),
                tint: atom.tint(TEXT_TINT),
                clip,
            });
            y += 18.0;
        }
        y += 8.0;

        // 2) the N2 tab bar (division designer) — sits above the grid when present.
        let grid_top = if let Some(tabs) = &self.tabs {
            let tab_view = Rect {
                x: area.x,
                y,
                w: area.w,
                h: area.h - (y - area.y),
            };
            let tab_plan = plan_tabs(tabs, tab_view);
            for c in tab_plan.cmds {
                p.push(c);
            }
            y + widgets::tabs::BAR_THICK + 6.0
        } else {
            y
        };

        // 3) the N4 module / regiment grid.
        let grid_area = Rect {
            x: area.x,
            y: grid_top,
            w: area.w,
            h: (area.h - (grid_top - area.y)).max(0.0),
        };
        let module_plan = plan_modules(&self.modules, grid_area);
        let grid_rows = ((self.modules.slots.len() as f32 / 5.0).ceil()).max(1.0);
        for c in module_plan.cmds {
            p.push(c);
        }
        // advance past the grid for the side columns.
        let after_grid_x = area.x + 5.0 * widgets::designer::SLOT_W + 24.0;
        let mut uy = grid_top;

        // 4) upgrade tracks (right column).
        if !self.upgrades.is_empty() {
            p.push(Cmd::Text {
                x: after_grid_x,
                y: uy,
                text: "Upgrades".into(),
                tint: TEXT_TINT,
                clip,
            });
            uy += 18.0;
            for (label, atom) in &self.upgrades {
                p.push(Cmd::Text {
                    x: after_grid_x,
                    y: uy,
                    text: label.clone(),
                    tint: DIM_TINT,
                    clip,
                });
                p.push(Cmd::Text {
                    x: after_grid_x + 160.0,
                    y: uy,
                    text: atom.as_str().into(),
                    tint: atom.tint(TEXT_TINT),
                    clip,
                });
                uy += 16.0;
            }
            uy += 8.0;
        }

        // 5) computed stat panels — the §G hard-limit badge column (all "—").
        if !self.stat_badges.is_empty() {
            p.push(Cmd::Text {
                x: after_grid_x,
                y: uy,
                text: "Computed (unavailable)".into(),
                tint: UNAVAIL_TINT,
                clip,
            });
            uy += 18.0;
            for b in &self.stat_badges {
                p.push(Cmd::Text {
                    x: after_grid_x,
                    y: uy,
                    text: b.label.clone(),
                    tint: DIM_TINT,
                    clip,
                });
                p.push(Cmd::Text {
                    x: after_grid_x + 160.0,
                    y: uy,
                    text: widgets::common::NOT_CAPTURED.into(),
                    tint: UNAVAIL_TINT,
                    clip,
                });
                uy += 16.0;
            }
        }

        let _ = grid_rows; // grid_rows retained for a future fixed-height grid layout
        p
    }
}

/// A one-char fidelity badge for the digest.
fn fid(a: &Atom) -> &'static str {
    match a.fidelity() {
        widgets::common::Fidelity::Exact => "E",
        widgets::common::Fidelity::Derived => "D",
        widgets::common::Fidelity::Unavailable => "U",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use widgets::common::AssetRef;
    use widgets::designer::{Module, Slot};

    fn area() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 1000.0,
            h: 600.0,
        }
    }

    fn model() -> DesignerModel {
        let mut m = DesignerModel::new();
        m.header.push(("Name".into(), Atom::exact("Panzer IV")));
        m.header
            .push(("Production Cost".into(), Atom::unavailable()));
        m.modules = ModuleLayout::new(vec![
            Slot::known(
                "main",
                0,
                0,
                Module::new(AssetRef::Named("GFX_x".into()), Atom::derived("Gun"))
                    .stat("Attack", Atom::derived("12")),
            ),
            Slot::empty("aux", 1, 0),
        ]);
        m.upgrades.push(("Armor".into(), Atom::exact("3")));
        m.stat_badges.push(StatBadge {
            label: "Combat Stats".into(),
        });
        m
    }

    #[test]
    fn digest_is_stable_and_order_sensitive() {
        let m = model();
        assert_eq!(m.digest(area()), m.digest(area()));
        // a different value changes the digest.
        let mut m2 = m.clone();
        m2.header[0].1 = Atom::exact("Tiger");
        assert_ne!(m.digest(area()), m2.digest(area()));
    }

    #[test]
    fn plan_renders_unavailable_as_glyph_not_zero() {
        let p = model().plan(area());
        // the production-cost unavailable header + the combat-stats badge → "—" texts.
        assert!(p.unavailable_text_count() >= 2);
        // the measured upgrade level "3" + module stat "12" are real values, not collapsed.
        assert!(p
            .cmds
            .iter()
            .any(|c| matches!(c, Cmd::Text { text, .. } if text == "3")));
        assert!(p
            .cmds
            .iter()
            .any(|c| matches!(c, Cmd::Text { text, .. } if text == "12")));
    }

    #[test]
    fn plan_is_deterministic() {
        assert_eq!(model().plan(area()).digest(), model().plan(area()).digest());
    }

    #[test]
    fn empty_model_never_panics() {
        let m = DesignerModel::new();
        assert!(!m.digest(area()).is_empty());
        let _ = m.plan(area());
    }
}
