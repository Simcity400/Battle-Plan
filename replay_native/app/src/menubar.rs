//! The MENUBAR widget — BATCH 1's one app-side piece. BATCH 0 left the menubar MODEL
//! (`ViewRegistry::menubar()` → clusters + their routed items) and the open PATH
//! (`WindowHost::open_view`); this builds the on-screen BAR that draws the cluster menu
//! and routes a click → `OpenView`.
//!
//! It is HOST CHROME (like the scrubber transport), NOT a registered `ViewInstance`
//! (Codex: making it a view is circular — you'd need the view system to open the thing
//! that opens views; it routes views, it isn't content, and it must be hit-tested before
//! the window host). So it lives here in the binary's lib, fed by `registry.menubar()`,
//! and the binary checks it FIRST on a click (above the scrubber + the host).
//!
//! The logic (layout + hit-test + the draw plan) is PURE + headlessly unit-tested; the
//! draw executes the plan through the shared `widgets` substrate.
//!
//! No-fabrication: the bar only ever shows clusters/items the registry REPORTED — it
//! never invents a menu entry. An EMPTY cluster (no registered routed views yet, the
//! pre-BATCH-2 state) is shown DISABLED (a visible "(empty)" affordance), never hidden
//! and never faked with a dead item.

use render::Renderer;
use widgets::common::{self, Atom, DrawAssets};
use widgets::plan::{Cmd, Plan};

use crate::host::{MenuCluster, ViewId, ViewRegistry};

/// The menubar's transient open state: which cluster's dropdown is open (if any). This
/// is the only mutable UI state — persisted on the binary across frames.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MenuBar {
    /// The cluster whose dropdown is currently open, or `None` (bar collapsed).
    open: Option<MenuCluster>,
}

impl MenuBar {
    pub fn new() -> MenuBar {
        MenuBar { open: None }
    }
    pub fn open_cluster(&self) -> Option<MenuCluster> {
        self.open
    }
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }
    /// Close any open dropdown.
    pub fn close(&mut self) {
        self.open = None;
    }
    /// Toggle a cluster's dropdown (clicking the open one closes it).
    pub fn toggle(&mut self, cluster: MenuCluster) {
        self.open = if self.open == Some(cluster) {
            None
        } else {
            Some(cluster)
        };
    }
}

/// The bar height (the top strip) + the dropdown item height.
pub const BAR_H: f32 = 26.0;
pub const ITEM_H: f32 = 22.0;
pub const CLUSTER_W: f32 = 96.0;
pub const DROPDOWN_W: f32 = 220.0;

/// One laid-out top-level cluster button.
#[derive(Debug, Clone, PartialEq)]
pub struct ClusterRect {
    pub cluster: MenuCluster,
    pub rect: gui_runtime::Rect,
    /// True iff the cluster has at least one routed item (else shown disabled).
    pub enabled: bool,
}

/// One laid-out dropdown item.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemRect {
    pub view_id: ViewId,
    pub label: &'static str,
    pub rect: gui_runtime::Rect,
}

/// The fully laid-out bar for one frame: the cluster buttons + (if open) the dropdown
/// items. Built fresh each frame from the registry's menubar model (which is immutable),
/// so the bar always reflects the registered views with NO per-frame mutation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MenuLayout {
    pub clusters: Vec<ClusterRect>,
    pub items: Vec<ItemRect>,
}

/// What a click on the bar resolved to.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuClick {
    /// The click toggled a cluster's dropdown (open/close).
    ToggleCluster(MenuCluster),
    /// The click chose an item → open this view.
    OpenView(ViewId),
    /// The click hit the bar/dropdown but no actionable target (consumed; close menu).
    Consumed,
    /// The click missed the bar entirely (the binary routes it onward).
    Miss,
}

impl MenuBar {
    /// Lay the bar out across the top of a `screen_w`-wide window. The cluster buttons
    /// come from the registry's menubar model (only registered, routed clusters); the
    /// dropdown items appear under the OPEN cluster. Pure — no GPU.
    pub fn layout(&self, registry: &ViewRegistry, screen_w: f32) -> MenuLayout {
        let model = registry.menubar();
        let mut clusters = Vec::with_capacity(model.len());
        let mut items = Vec::new();
        let mut x = 0.0;
        for group in &model {
            let rect = gui_runtime::Rect {
                x,
                y: 0.0,
                w: CLUSTER_W,
                h: BAR_H,
            };
            let enabled = !group.items.is_empty();
            clusters.push(ClusterRect {
                cluster: group.cluster,
                rect,
                enabled,
            });
            // if this cluster is the open one, lay its items as a dropdown beneath it.
            if self.open == Some(group.cluster) {
                let dx = x;
                let mut iy = BAR_H;
                for it in &group.items {
                    items.push(ItemRect {
                        view_id: it.view_id,
                        label: it.label,
                        rect: gui_runtime::Rect {
                            x: dx,
                            y: iy,
                            w: DROPDOWN_W,
                            h: ITEM_H,
                        },
                    });
                    iy += ITEM_H;
                }
            }
            x += CLUSTER_W;
            if x > screen_w {
                break; // overflow: stop laying out clusters past the window edge
            }
        }
        MenuLayout { clusters, items }
    }

    /// Hit-test a click. A dropdown ITEM wins (it sits above the bar); else a cluster
    /// BUTTON toggles; else a click inside the bar strip is consumed (and closes the
    /// menu); else a Miss (routed onward by the binary). Disabled clusters are not
    /// toggleable (consumed, no-op).
    pub fn pick(&self, layout: &MenuLayout, x: f32, y: f32) -> MenuClick {
        // 1) an open dropdown item (drawn above the bar) is checked first.
        for it in &layout.items {
            if it.rect.contains(x, y) {
                return MenuClick::OpenView(it.view_id);
            }
        }
        // 2) a top-level cluster button.
        for c in &layout.clusters {
            if c.rect.contains(x, y) {
                return if c.enabled {
                    MenuClick::ToggleCluster(c.cluster)
                } else {
                    MenuClick::Consumed // disabled cluster: consume, no toggle
                };
            }
        }
        // 3) a click in the bar strip but off a button (close the menu).
        if y <= BAR_H && x >= 0.0 {
            return MenuClick::Consumed;
        }
        MenuClick::Miss
    }

    /// Apply a click outcome to the bar's open-state + return a `ViewId` to OPEN, if any.
    /// (The binary calls `host.open_view(registry, id, None)` on `Some`.)
    pub fn apply(&mut self, click: &MenuClick) -> Option<ViewId> {
        match click {
            MenuClick::ToggleCluster(c) => {
                self.toggle(*c);
                None
            }
            MenuClick::OpenView(id) => {
                self.close(); // selecting an item closes the dropdown
                Some(*id)
            }
            MenuClick::Consumed => {
                self.close();
                None
            }
            MenuClick::Miss => None,
        }
    }
}

const BAR_BG: [f32; 4] = [0.08, 0.09, 0.11, 1.0];
const CLUSTER_BG: [f32; 4] = [0.12, 0.13, 0.16, 1.0];
const CLUSTER_OPEN: [f32; 4] = [0.22, 0.30, 0.42, 1.0];
const CLUSTER_DISABLED: [f32; 4] = [0.09, 0.09, 0.11, 1.0];
const DROPDOWN_BG: [f32; 4] = [0.10, 0.11, 0.14, 0.99];
const ITEM_HOVER: [f32; 4] = [0.18, 0.22, 0.30, 1.0];

/// Build the render plan for the menubar (pure; headlessly testable). `hover` is the
/// cursor for item-hover highlight (optional).
pub fn plan_menubar(
    bar: &MenuBar,
    layout: &MenuLayout,
    screen_w: f32,
    hover: Option<(f32, f32)>,
) -> Plan {
    let mut p = Plan::new();
    let clip = Some([0.0, 0.0, screen_w, 2000.0]);
    // the bar strip.
    p.push(Cmd::Rect {
        x: 0.0,
        y: 0.0,
        w: screen_w,
        h: BAR_H,
        tint: BAR_BG,
        clip,
    });
    for c in &layout.clusters {
        let r = c.rect;
        let bg = if bar.open == Some(c.cluster) {
            CLUSTER_OPEN
        } else if c.enabled {
            CLUSTER_BG
        } else {
            CLUSTER_DISABLED
        };
        p.push(Cmd::Rect {
            x: r.x,
            y: r.y,
            w: r.w - 1.0,
            h: r.h,
            tint: bg,
            clip,
        });
        let label = if c.enabled {
            Atom::exact(c.cluster.label())
        } else {
            // a disabled cluster shows "(empty)" — honest, never a fake item.
            Atom::exact(format!("{} (empty)", c.cluster.label()))
        };
        let normal = if c.enabled {
            common::TEXT_TINT
        } else {
            common::DIM_TINT
        };
        let cell_clip = Some([r.x, r.y, r.w, r.h]);
        p.push(Cmd::Text {
            x: r.x + 6.0,
            y: r.y + 6.0,
            text: label.as_str().into(),
            tint: label.tint(normal),
            clip: cell_clip,
        });
    }
    // the open dropdown.
    if !layout.items.is_empty() {
        let first = &layout.items[0].rect;
        let h = layout.items.len() as f32 * ITEM_H;
        p.push(Cmd::Rect {
            x: first.x,
            y: first.y,
            w: DROPDOWN_W,
            h,
            tint: DROPDOWN_BG,
            clip,
        });
        for it in &layout.items {
            let r = it.rect;
            let hovered = hover.map(|(hx, hy)| r.contains(hx, hy)).unwrap_or(false);
            if hovered {
                p.push(Cmd::Rect {
                    x: r.x,
                    y: r.y,
                    w: r.w,
                    h: r.h,
                    tint: ITEM_HOVER,
                    clip,
                });
            }
            let cell_clip = Some([r.x, r.y, r.w, r.h]);
            p.push(Cmd::Text {
                x: r.x + 8.0,
                y: r.y + 4.0,
                text: it.label.into(),
                tint: common::TEXT_TINT,
                clip: cell_clip,
            });
        }
    }
    p
}

/// Draw the menubar (plan + execute) through the shared widgets substrate.
pub fn draw_menubar(
    r: &mut Renderer,
    bar: &MenuBar,
    layout: &MenuLayout,
    screen_w: f32,
    hover: Option<(f32, f32)>,
    assets: &DrawAssets,
) {
    plan_menubar(bar, layout, screen_w, hover).execute(r, assets, |_, _| false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::registry::{MenuRoute, ViewDescriptor};
    use crate::host::view::{HostPolicy, ViewInstance, ZBand};

    fn open_stub(_a: Option<i64>) -> Box<dyn ViewInstance> {
        unreachable!("test stub never opened")
    }

    static V_POL: ViewDescriptor = ViewDescriptor {
        id: ViewId("politics.politics"),
        gui_root: None,
        required_domains: &[],
        route: Some(MenuRoute {
            cluster: MenuCluster::PoliticsGovernment,
            label: "Politics",
            order: 1,
        }),
        shortcut: None,
        open: open_stub,
        policy: HostPolicy::window(ZBand::Panel),
    };
    static V_IDEAS: ViewDescriptor = ViewDescriptor {
        id: ViewId("politics.ideas"),
        gui_root: None,
        required_domains: &[],
        route: Some(MenuRoute {
            cluster: MenuCluster::PoliticsGovernment,
            label: "National Spirits",
            order: 2,
        }),
        shortcut: None,
        open: open_stub,
        policy: HostPolicy::window(ZBand::Panel),
    };

    fn registry() -> ViewRegistry {
        let mut reg = ViewRegistry::empty();
        reg.insert_view(&V_POL);
        reg.insert_view(&V_IDEAS);
        reg
    }

    #[test]
    fn layout_lists_only_registered_clusters_and_items() {
        let bar = MenuBar::new();
        let reg = registry();
        // closed bar: cluster buttons but no items.
        let l = bar.layout(&reg, 1600.0);
        assert_eq!(l.clusters.len(), MenuCluster::ALL.len());
        assert!(l.items.is_empty());
        // the Politics cluster is ENABLED (2 items); the others are disabled (empty).
        let pol = l
            .clusters
            .iter()
            .find(|c| c.cluster == MenuCluster::PoliticsGovernment)
            .unwrap();
        assert!(pol.enabled);
        let prod = l
            .clusters
            .iter()
            .find(|c| c.cluster == MenuCluster::ProductionConstructionDecisions)
            .unwrap();
        assert!(!prod.enabled); // no registered views -> disabled, not hidden
    }

    #[test]
    fn open_cluster_shows_its_items_only() {
        let mut bar = MenuBar::new();
        bar.toggle(MenuCluster::PoliticsGovernment);
        let l = bar.layout(&registry(), 1600.0);
        assert_eq!(l.items.len(), 2); // the 2 politics items
                                      // items are ordered by their route order.
        assert_eq!(l.items[0].label, "Politics");
        assert_eq!(l.items[1].label, "National Spirits");
    }

    #[test]
    fn click_item_opens_view_and_closes_menu() {
        let mut bar = MenuBar::new();
        bar.toggle(MenuCluster::PoliticsGovernment);
        let reg = registry();
        let l = bar.layout(&reg, 1600.0);
        let it = &l.items[1].rect;
        let click = bar.pick(&l, it.x + 4.0, it.y + 4.0);
        assert_eq!(click, MenuClick::OpenView(ViewId("politics.ideas")));
        let opened = bar.apply(&click);
        assert_eq!(opened, Some(ViewId("politics.ideas")));
        assert!(!bar.is_open()); // selecting an item closed the dropdown
    }

    #[test]
    fn click_cluster_button_toggles() {
        let mut bar = MenuBar::new();
        let reg = registry();
        let l = bar.layout(&reg, 1600.0);
        let pol = l
            .clusters
            .iter()
            .find(|c| c.cluster == MenuCluster::PoliticsGovernment)
            .unwrap();
        let click = bar.pick(&l, pol.rect.x + 4.0, pol.rect.y + 4.0);
        assert_eq!(
            click,
            MenuClick::ToggleCluster(MenuCluster::PoliticsGovernment)
        );
        assert_eq!(bar.apply(&click), None);
        assert_eq!(bar.open_cluster(), Some(MenuCluster::PoliticsGovernment));
    }

    #[test]
    fn disabled_cluster_does_not_toggle() {
        let bar = MenuBar::new();
        let reg = registry();
        let l = bar.layout(&reg, 1600.0);
        let prod = l
            .clusters
            .iter()
            .find(|c| c.cluster == MenuCluster::ProductionConstructionDecisions)
            .unwrap();
        let click = bar.pick(&l, prod.rect.x + 4.0, prod.rect.y + 4.0);
        assert_eq!(click, MenuClick::Consumed); // disabled -> consumed, no open
    }

    #[test]
    fn click_below_bar_off_dropdown_misses() {
        let bar = MenuBar::new();
        let reg = registry();
        let l = bar.layout(&reg, 1600.0);
        // a click well below the (closed) bar is a Miss -> routed onward to the map/host.
        assert_eq!(bar.pick(&l, 400.0, 500.0), MenuClick::Miss);
    }

    #[test]
    fn empty_registry_bar_never_panics() {
        let bar = MenuBar::new();
        let reg = ViewRegistry::empty();
        let l = bar.layout(&reg, 1600.0);
        // every cluster is disabled; no items; pick anywhere is safe.
        assert!(l.clusters.iter().all(|c| !c.enabled));
        assert!(l.items.is_empty());
        let _ = plan_menubar(&bar, &l, 1600.0, None);
        assert_eq!(bar.pick(&l, 9999.0, 9999.0), MenuClick::Miss);
    }

    #[test]
    fn plan_is_deterministic() {
        let mut bar = MenuBar::new();
        bar.toggle(MenuCluster::PoliticsGovernment);
        let reg = registry();
        let l = bar.layout(&reg, 1600.0);
        assert_eq!(
            plan_menubar(&bar, &l, 1600.0, None).digest(),
            plan_menubar(&bar, &l, 1600.0, None).digest()
        );
    }
}
