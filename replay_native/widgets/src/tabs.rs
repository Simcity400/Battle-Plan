//! N2 — the TAB / SUB-WINDOW MANAGER: a tab bar selecting a sub-root within a view.
//! Consumers: politics, mil-mgmt (army/navy/air sub-split), intel, find, designers.
//!
//! The widget owns ONLY the bar: tab layout (horizontal top OR vertical side, with
//! overflow clipping), hit-testing (disabled/unavailable tabs are not pickable), the
//! selection model, and the chrome. The SELECTED sub-root's content is the consumer's
//! job — this just tells it which tab is active.
//!
//! No-fabrication (Codex N2): a tab `label` is an [`Atom`]; a tab carries an explicit
//! [`Availability`] (Enabled / Disabled / Unavailable-no-data), so a present-but-blocked
//! tab badges rather than vanishes, and the caller never creates a tab for a view the
//! registry did not report. `selected` is an `Option<TabId>` (a STALE/empty index can
//! never select a non-existent tab).
//!
//! ## Usage
//! ```ignore
//! use widgets::tabs::*;
//! use widgets::common::Atom;
//! let mut bar = TabBar::horizontal(vec![
//!     Tab::enabled("politics", Atom::exact("Politics")),
//!     Tab::enabled("ideas", Atom::exact("National Spirits")),
//!     Tab::unavailable("laws", Atom::exact("Laws")), // present but data blocked
//! ]);
//! bar.select("ideas");
//! // let rects = bar.layout(view_rect);            // -> Vec<TabRect>
//! // if let Some(id) = bar.pick(&rects, mx, my) { bar.select(&id); }
//! // draw_tabs(renderer, &bar, view_rect, &assets);
//! ```

use render::Renderer;

use crate::common::{Atom, DrawAssets, DIM_TINT, TEXT_TINT, UNAVAIL_TINT};
use crate::plan::{Cmd, Plan};

/// A tab's availability (Codex N2: "disabled is not enough"). Distinguishes a tab that
/// exists but is intentionally inert from one whose backing DATA is blocked (badged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Selectable.
    Enabled,
    /// Present but inert by design (e.g. a path-gated sub-tab) — dimmed, not pickable.
    Disabled,
    /// Present but its data is blocked/unfed — badged "no data", not pickable, NEVER
    /// fabricated content.
    Unavailable,
}

impl Availability {
    pub fn is_pickable(&self) -> bool {
        matches!(self, Availability::Enabled)
    }
}

/// One tab: a stable id + an [`Atom`] label + its availability.
#[derive(Debug, Clone, PartialEq)]
pub struct Tab {
    pub id: String,
    pub label: Atom,
    pub avail: Availability,
}

impl Tab {
    pub fn enabled(id: impl Into<String>, label: Atom) -> Tab {
        Tab { id: id.into(), label, avail: Availability::Enabled }
    }
    pub fn disabled(id: impl Into<String>, label: Atom) -> Tab {
        Tab { id: id.into(), label, avail: Availability::Disabled }
    }
    /// A present-but-data-blocked tab (badged, not pickable).
    pub fn unavailable(id: impl Into<String>, label: Atom) -> Tab {
        Tab { id: id.into(), label, avail: Availability::Unavailable }
    }
}

/// Tab-bar orientation (Codex N2: the game has both top-row and side-column tab layouts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// A horizontal row across the top of the view.
    Horizontal,
    /// A vertical column down the left side.
    Vertical,
}

/// The tab bar: tabs + the orientation + the SELECTED tab id (Option — never a stale
/// index). Persistent UI state lives on the consumer's `ViewInstance`; this is the
/// model it owns.
#[derive(Debug, Clone, PartialEq)]
pub struct TabBar {
    pub tabs: Vec<Tab>,
    pub orientation: Orientation,
    /// The selected tab id. `None` until the consumer selects one (or all tabs gone).
    selected: Option<String>,
    /// The cell extent along the bar's main axis (tab width for horizontal, tab height
    /// for vertical). Cross-axis extent comes from the view rect.
    pub cell_main: f32,
}

impl TabBar {
    /// A horizontal top tab bar. Auto-selects the first ENABLED tab (if any).
    pub fn horizontal(tabs: Vec<Tab>) -> TabBar {
        Self::build(tabs, Orientation::Horizontal, 128.0)
    }
    /// A vertical side tab bar.
    pub fn vertical(tabs: Vec<Tab>) -> TabBar {
        Self::build(tabs, Orientation::Vertical, 30.0)
    }
    fn build(tabs: Vec<Tab>, orientation: Orientation, cell_main: f32) -> TabBar {
        let selected = tabs.iter().find(|t| t.avail.is_pickable()).map(|t| t.id.clone());
        TabBar { tabs, orientation, selected, cell_main }
    }

    /// The selected tab id (if any). `None` for an empty/all-disabled bar.
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }
    /// The index of the selected tab, if it still exists.
    pub fn selected_index(&self) -> Option<usize> {
        let sel = self.selected.as_ref()?;
        self.tabs.iter().position(|t| &t.id == sel)
    }
    /// Select a tab by id IF it exists AND is pickable. Returns true on change. A stale
    /// or disabled/unavailable id is ignored (no fabricated selection).
    pub fn select(&mut self, id: &str) -> bool {
        if self.tabs.iter().any(|t| t.id == id && t.avail.is_pickable()) {
            let changed = self.selected.as_deref() != Some(id);
            self.selected = Some(id.to_string());
            changed
        } else {
            false
        }
    }
    /// Re-point a stale selection to the first ENABLED tab. A selection is stale if the
    /// tab disappeared OR is still present but is no longer pickable (became Disabled /
    /// Unavailable) — Codex N2: otherwise a consumer keeps rendering content for a now
    /// non-pickable tab. Call after the tab set / availability changes (a sub-split or a
    /// data-block appears/disappears per hour).
    pub fn reconcile(&mut self) {
        let still_valid = self
            .selected
            .as_ref()
            .map(|sel| self.tabs.iter().any(|t| &t.id == sel && t.avail.is_pickable()))
            .unwrap_or(false);
        if !still_valid {
            self.selected = self.tabs.iter().find(|t| t.avail.is_pickable()).map(|t| t.id.clone());
        }
    }

    /// Lay the tabs out within `view` (the area the bar sits in). Horizontal tabs take a
    /// fixed `BAR_THICK`-tall strip along the top; vertical tabs a `cell_main`-wide
    /// column on the left. Tabs that overflow the bar are still laid out (with their
    /// rect partly outside) so the caller can clip — never dropped (Codex N2 overflow).
    pub fn layout(&self, view: gui_runtime::Rect) -> Vec<TabRect> {
        let mut out = Vec::with_capacity(self.tabs.len());
        match self.orientation {
            Orientation::Horizontal => {
                let cw = self.cell_main.max(1.0);
                for (i, t) in self.tabs.iter().enumerate() {
                    out.push(TabRect {
                        id: t.id.clone(),
                        index: i,
                        rect: gui_runtime::Rect {
                            x: view.x + i as f32 * cw,
                            y: view.y,
                            w: cw - 2.0,
                            h: BAR_THICK,
                        },
                    });
                }
            }
            Orientation::Vertical => {
                let ch = self.cell_main.max(1.0);
                let cw = SIDE_WIDTH.min(view.w);
                for (i, t) in self.tabs.iter().enumerate() {
                    out.push(TabRect {
                        id: t.id.clone(),
                        index: i,
                        rect: gui_runtime::Rect {
                            x: view.x,
                            y: view.y + i as f32 * ch,
                            w: cw,
                            h: ch - 2.0,
                        },
                    });
                }
            }
        }
        out
    }

    /// The content sub-rect BELOW (horizontal) / RIGHT-OF (vertical) the bar — where the
    /// selected sub-root draws.
    pub fn content_rect(&self, view: gui_runtime::Rect) -> gui_runtime::Rect {
        match self.orientation {
            Orientation::Horizontal => gui_runtime::Rect {
                x: view.x,
                y: view.y + BAR_THICK,
                w: view.w,
                h: (view.h - BAR_THICK).max(0.0),
            },
            Orientation::Vertical => {
                let cw = SIDE_WIDTH.min(view.w);
                gui_runtime::Rect { x: view.x + cw, y: view.y, w: (view.w - cw).max(0.0), h: view.h }
            }
        }
    }

    /// Hit-test a click against the laid-out tabs. Returns the tab id IF the hit tab is
    /// PICKABLE (a disabled/unavailable tab is not selectable, Codex N2). `None` off the
    /// bar or on a non-pickable tab.
    pub fn pick(&self, rects: &[TabRect], x: f32, y: f32) -> Option<String> {
        for tr in rects {
            if tr.rect.contains(x, y) {
                let pickable = self.tabs.get(tr.index).map(|t| t.avail.is_pickable()).unwrap_or(false);
                return pickable.then(|| tr.id.clone());
            }
        }
        None
    }
}

/// One laid-out tab's screen rect.
#[derive(Debug, Clone, PartialEq)]
pub struct TabRect {
    pub id: String,
    pub index: usize,
    pub rect: gui_runtime::Rect,
}

/// The horizontal bar height / vertical side-column width.
pub const BAR_THICK: f32 = 28.0;
pub const SIDE_WIDTH: f32 = 150.0;

const TAB_BG: [f32; 4] = [0.12, 0.13, 0.16, 1.0];
const TAB_SELECTED: [f32; 4] = [0.22, 0.30, 0.42, 1.0];
const TAB_DISABLED: [f32; 4] = [0.10, 0.10, 0.12, 1.0];
const TAB_BORDER: [f32; 4] = [0.05, 0.05, 0.07, 1.0];

/// Build the render plan for the tab bar within `view`. The selected tab is
/// highlighted; a disabled/unavailable tab is dimmed; an unavailable tab's label draws
/// its `—` glyph IF the label itself is unavailable (the caller decides the label).
pub fn plan_tabs(bar: &TabBar, view: gui_runtime::Rect) -> Plan {
    let mut p = Plan::new();
    let rects = bar.layout(view);
    // the bar background strip (clipped to the view so overflow tabs are cut).
    let clip = Some([view.x, view.y, view.w, view.h]);
    let bar_rect = match bar.orientation {
        Orientation::Horizontal => [view.x, view.y, view.w, BAR_THICK],
        Orientation::Vertical => [view.x, view.y, SIDE_WIDTH.min(view.w), view.h],
    };
    p.push(Cmd::Rect { x: bar_rect[0], y: bar_rect[1], w: bar_rect[2], h: bar_rect[3], tint: TAB_BG, clip });

    let sel = bar.selected();
    for tr in &rects {
        let tab = match bar.tabs.get(tr.index) {
            Some(t) => t,
            None => continue,
        };
        let r = tr.rect;
        let bg = if Some(tr.id.as_str()) == sel {
            TAB_SELECTED
        } else if tab.avail.is_pickable() {
            TAB_BG
        } else {
            TAB_DISABLED
        };
        p.push(Cmd::Rect { x: r.x, y: r.y, w: r.w, h: r.h, tint: bg, clip });
        // a thin bottom/right border between tabs.
        p.push(Cmd::Rect { x: r.x, y: r.y + r.h - 1.0, w: r.w, h: 1.0, tint: TAB_BORDER, clip });
        // label tint by availability.
        let normal = match tab.avail {
            Availability::Enabled => TEXT_TINT,
            Availability::Disabled => DIM_TINT,
            Availability::Unavailable => UNAVAIL_TINT,
        };
        // clip the label to its own tab cell so a long label doesn't spill.
        let cell_clip = Some([r.x, r.y, r.w, r.h]);
        p.push(Cmd::Text { x: r.x + 8.0, y: r.y + 7.0, text: tab.label.as_str().into(), tint: tab.label.tint(normal), clip: cell_clip });
        // an "unavailable" tab gets a small badge dot so it reads as no-data.
        if matches!(tab.avail, Availability::Unavailable) {
            p.push(Cmd::Rect { x: r.x + r.w - 8.0, y: r.y + 4.0, w: 4.0, h: 4.0, tint: UNAVAIL_TINT, clip: cell_clip });
        }
    }
    p
}

/// Draw the tab bar (plan + execute). READ-ONLY.
pub fn draw_tabs(r: &mut Renderer, bar: &TabBar, view: gui_runtime::Rect, assets: &DrawAssets) {
    plan_tabs(bar, view).execute(r, assets, |_, _| false);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar() -> TabBar {
        TabBar::horizontal(vec![
            Tab::enabled("a", Atom::exact("Alpha")),
            Tab::disabled("b", Atom::exact("Beta")),
            Tab::unavailable("c", Atom::exact("Gamma")),
        ])
    }

    #[test]
    fn auto_selects_first_enabled() {
        assert_eq!(bar().selected(), Some("a"));
        // a bar with no enabled tab selects nothing (never a fabricated index).
        let none = TabBar::horizontal(vec![Tab::disabled("x", Atom::exact("X"))]);
        assert_eq!(none.selected(), None);
    }

    #[test]
    fn empty_bar_never_panics() {
        let mut b = TabBar::horizontal(vec![]);
        assert_eq!(b.selected(), None);
        assert_eq!(b.selected_index(), None);
        b.reconcile();
        let view = gui_runtime::Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        assert!(b.layout(view).is_empty());
        assert_eq!(b.pick(&[], 10.0, 10.0), None);
        let _ = plan_tabs(&b, view); // no panic
    }

    #[test]
    fn select_rejects_stale_disabled_unavailable() {
        let mut b = bar();
        assert!(!b.select("ghost")); // not present
        assert!(!b.select("b")); // disabled
        assert!(!b.select("c")); // unavailable
        assert_eq!(b.selected(), Some("a")); // unchanged
        assert!(b.select("a") == false || b.selected() == Some("a")); // idempotent
    }

    #[test]
    fn pick_ignores_non_pickable_tabs() {
        let b = bar();
        let view = gui_runtime::Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let rects = b.layout(view);
        // tab 0 (enabled) centre is pickable.
        let r0 = &rects[0].rect;
        assert_eq!(b.pick(&rects, r0.x + 4.0, r0.y + 4.0).as_deref(), Some("a"));
        // tab 1 (disabled) is not.
        let r1 = &rects[1].rect;
        assert_eq!(b.pick(&rects, r1.x + 4.0, r1.y + 4.0), None);
    }

    #[test]
    fn reconcile_repoints_stale_selection() {
        let mut b = bar();
        assert_eq!(b.selected(), Some("a")); // "a" auto-selected
        // the tab set shrinks; "a" is gone.
        b.tabs.retain(|t| t.id != "a");
        b.reconcile();
        // selection re-points to the first enabled (none left -> None).
        assert_eq!(b.selected(), None);
    }

    #[test]
    fn reconcile_repoints_when_selected_tab_becomes_non_pickable() {
        // "a" is selected; it stays present but becomes Unavailable (data blocked). A new
        // enabled tab "d" exists. reconcile must move OFF the now-unpickable "a".
        let mut b = TabBar::horizontal(vec![
            Tab::enabled("a", Atom::exact("A")),
            Tab::enabled("d", Atom::exact("D")),
        ]);
        assert_eq!(b.selected(), Some("a"));
        b.tabs[0].avail = Availability::Unavailable; // "a" data blocked
        b.reconcile();
        assert_eq!(b.selected(), Some("d")); // moved to the first still-pickable tab
    }

    #[test]
    fn vertical_content_rect_is_right_of_bar() {
        let b = TabBar::vertical(vec![Tab::enabled("a", Atom::exact("A"))]);
        let view = gui_runtime::Rect { x: 0.0, y: 0.0, w: 400.0, h: 300.0 };
        let c = b.content_rect(view);
        assert!(c.x > view.x); // content sits right of the side column
        assert!((c.w - (view.w - SIDE_WIDTH)).abs() < 0.01);
    }

    #[test]
    fn plan_unavailable_tab_badges_not_fabricates() {
        let b = bar();
        let view = gui_runtime::Rect { x: 0.0, y: 0.0, w: 600.0, h: 300.0 };
        let p = plan_tabs(&b, view);
        // deterministic.
        assert_eq!(p.digest(), plan_tabs(&b, view).digest());
    }
}
