//! VIEWS 6, 7, 8 — the TRAIT-TREE GRAPH engine (N3).
//!
//! `army_trait_tree.json` (View 6) + `navy_trait_tree.json` (View 7) supply the selected-
//! leader HEADER (portrait / skill / 4 stats / XP) + the tabs + the `drag_scroll` tree
//! CANVASES — but NOT the trait nodes, which the **N3 `widgets::trait_tree`** adapter
//! instantiates. `trait_tree_common.json` (View 8) is the shared PROTOTYPE LIBRARY,
//! registered as TEMPLATES ONLY (no standalone window, no route) and accounted by the
//! element-accounting test.
//!
//! NODE STATE + the demo (Codex no-fab #4). When NO leader is selected (the save-only demo:
//! `commanders` unfed), the window chrome renders + the trait tree carries a single
//! `Unavailable` node — NOT a fabricated `Available` tree and NOT a silently empty tree
//! (which would falsely imply "no trait surface"). When a leader IS selected (a live
//! snapshot), each owned trait token is an `Owned` node; the catalog of pickable/blocked
//! nodes (their `gui_row`/`gui_column` geometry + prereq/XOR edges) is the STATIC-DEF
//! adapter (`common/unit_leader/*.txt`), out of this cluster's scope — those are `Unknown`
//! (honesty-badged), never fabricated `Available`.

use gui_runtime::scene::Node;

use bind::Fidelity;

use widgets::common::{Atom, Canvas};
use widgets::node_graph;
use widgets::trait_tree::{TraitNode, TraitState, TraitTree};

use crate::host::context::{BindContext, Captured, DisplayAtom, FidelityReport, UnavailableReason};
use crate::host::draw::FrameDraw;
use crate::host::registry::{
    GuiSymbolRef, MenuCluster, MenuRoute, TemplateDescriptor, ViewDescriptor, ViewId,
};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InstanceId, ViewCommand, ViewInstance, ZBand,
};
use crate::{register_template, register_view};

use super::*;

// =====================================================================================
// Shared trait-tree machinery (army + navy share the engine; only the stat set differs).
// =====================================================================================

/// Whether this is the army or the navy trait tree (the only delta: the stat set + the
/// selected-leader source array).
#[derive(Clone, Copy)]
enum Branch {
    Army,
    Navy,
}

impl Branch {
    /// The leader-roster source paths (FOLD-IN), first-present wins as the SELECTED leader.
    fn sources(self) -> &'static [&'static str] {
        match self {
            Branch::Army => &["commanders.generals", "commanders.field_marshals"],
            Branch::Navy => &["commanders.admirals"],
        }
    }
    /// The four stat element names + their leader field keys (army vs navy split).
    fn stats(self) -> [(&'static str, &'static str); 4] {
        match self {
            Branch::Army => [
                ("leader_stat_attack", "attack"),
                ("leader_stat_defense", "defense"),
                ("leader_stat_planning", "planning"),
                ("leader_stat_logistics", "logistics"),
            ],
            Branch::Navy => [
                ("leader_stat_attack", "attack"),
                ("leader_stat_defense", "defense"),
                ("leader_stat_maneuvering", "maneuvering"),
                ("leader_stat_coordination", "coordination"),
            ],
        }
    }
}

/// Join a leader's `traits[]` tokens into one display atom (exact when present; Null when
/// the array is present-but-empty; Absent when there is no leader / no traits key).
pub(super) fn traits_join_atom(l: &serde_json::Value) -> DisplayAtom {
    match l.get("traits").and_then(|t| t.as_array()) {
        Some(arr) if !arr.is_empty() => {
            let joined: Vec<String> = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
            if joined.is_empty() {
                DisplayAtom::unavailable_because(UnavailableReason::Null)
            } else {
                DisplayAtom::text(joined.join(", "), Fidelity::Exact)
            }
        }
        Some(_) => DisplayAtom::unavailable_because(UnavailableReason::Null),
        None => DisplayAtom::unavailable_because(UnavailableReason::Absent),
    }
}

/// Build the N3 trait tree for the SELECTED leader (or the no-leader badge).
///
/// * `Some(leader)` → one `Owned` node per token in `leader.traits[]` (membership exact);
///   an empty/absent `traits[]` → a single `Unavailable` glyph node (the leader has no
///   surfaced traits — not a fabricated `Available`).
/// * `None` (no leader selected — the demo) → a single `Unavailable` glyph node so the tree
///   is structurally present + honestly badged, never empty and never fabricated.
///
/// The pickable/blocked CATALOG nodes (geometry + prereq/XOR edges from
/// `common/unit_leader/*.txt`) are the STATIC-DEF adapter, out of scope here → not invented
/// (Codex no-fab #4: "no selected leader is not a leader with empty traits").
fn build_trait_tree(leader: Option<&serde_json::Value>) -> TraitTree {
    match leader {
        None => TraitTree::new(vec![TraitNode::cell(
            "no_leader",
            0,
            0,
            Atom::unavailable_because(widgets::common::Reason::Blocked),
            TraitState::Unavailable,
        )]),
        Some(l) => {
            let mut nodes = Vec::new();
            if let Some(arr) = l.get("traits").and_then(|t| t.as_array()) {
                for (i, t) in arr.iter().enumerate() {
                    if let Some(tok) = t.as_str() {
                        nodes.push(TraitNode::cell(
                            tok,
                            i as i32,
                            0,
                            Atom::exact(tok.to_string()),
                            TraitState::Owned,
                        ));
                    }
                }
            }
            if nodes.is_empty() {
                // a folded leader with NO surfaced traits → an honest unavailable glyph,
                // never a fabricated available tree.
                nodes.push(TraitNode::cell(
                    "no_traits",
                    0,
                    0,
                    Atom::unavailable_because(widgets::common::Reason::Blocked),
                    TraitState::Unavailable,
                ));
            }
            TraitTree::new(nodes)
        }
    }
}

/// The selected leader for a branch: the FIRST present source array's FIRST element (the
/// demo has none → `None`). Never fabricates a leader.
fn selected_leader<'a>(
    ctx: &'a BindContext,
    tag: &str,
    branch: Branch,
) -> Option<&'a serde_json::Value> {
    for path in branch.sources() {
        if let Captured::Measured(v) = ctx.country_field(tag, path) {
            if let Some(first) = v.as_array().and_then(|a| a.first()) {
                return Some(first);
            }
        }
    }
    None
}

struct TraitTreeDrawable {
    roots: Vec<Node>,
    /// the N3 trait tree (selected leader's traits[] or the no-leader badge).
    tree: TraitTree,
    label: &'static str,
}

impl Drawable for TraitTreeDrawable {
    fn draw_digest(&self) -> String {
        let g = self.tree.to_graph();
        let cam = Canvas::fit(g.bounds(), [0.0, 0.0, 800.0, 600.0], 16.0);
        let (plan, stats) = node_graph::plan_graph(&cam, &g);
        format!(
            "{} nodes={} edges={} dangling={} PLAN {}",
            self.label,
            stats.nodes_drawn,
            stats.edges_drawn,
            self.tree.dangling_edge_count(),
            plan.digest()
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        // draw the N3 trait tree into the regular-traits canvas (the real `drag_scroll`).
        let viewport = find_rect(&self.roots, "regular_traits")
            .or_else(|| find_rect(&self.roots, "tree"))
            .map(|r| [r.x, r.y, r.w.max(200.0), r.h.max(120.0)])
            .unwrap_or([320.0, 200.0, 600.0, 400.0]);
        let g = self.tree.to_graph();
        let cam = Canvas::fit(g.bounds(), viewport, 16.0);
        let white = mint_white(fd);
        let assets = widget_assets(&white);
        node_graph::draw_graph(fd.renderer, &cam, &g, &assets);
    }
}

/// Build a trait-tree view (army or navy — the engine is shared).
fn bind_trait_tree(
    ctx: &BindContext,
    branch: Branch,
    file_name: &str,
    root_name: &str,
    view_id: ViewId,
    label: &'static str,
    policy: HostPolicy,
) -> BoundView {
    let tag = ctx.country;
    let mut fidelity = FidelityReport::default();
    let file = load_gui(ctx, file_name);

    let leader = selected_leader(ctx, tag, branch);

    let mut summary: Vec<Cell> = Vec::new();
    let mut summary_sprites: Vec<SpriteCell> = Vec::new();
    push_leader_header(
        ctx,
        leader,
        branch,
        &mut fidelity,
        &mut summary,
        &mut summary_sprites,
    );

    let tree = build_trait_tree(leader);

    let roots = build_root_with_sprites(ctx, &file, root_name, &summary, &summary_sprites);
    let drawable = TraitTreeDrawable {
        roots: roots.clone(),
        tree,
        label,
    };
    BoundView {
        instance: InstanceId(0),
        view_id,
        policy,
        hit: HitModel::from_roots(roots),
        drawable: Box::new(drawable),
        fidelity,
        commands: Vec::<ViewCommand>::new(),
    }
}

/// Bind the selected-leader header into the trait-tree window chrome. FOLD-IN: every field
/// is "—" in the save-only demo (no leader); exact once a live snapshot folds `commanders`.
fn push_leader_header(
    ctx: &BindContext,
    leader: Option<&serde_json::Value>,
    branch: Branch,
    fidelity: &mut FidelityReport,
    summary: &mut Vec<Cell>,
    summary_sprites: &mut Vec<SpriteCell>,
) {
    let field = |k: &str| -> Captured<&serde_json::Value> {
        match leader {
            Some(l) => crate::host::context::classify_value(l.get(k)),
            None => Captured::Absent,
        }
    };
    let mut push = |element: &str, k: &str, fmt: fn(&serde_json::Value) -> String| {
        let atom = field(k).display_with_fidelity(Fidelity::Exact, fmt);
        fidelity.record(&atom);
        summary.push(Cell {
            element: element.to_string(),
            atom,
        });
    };
    push("name", "name", fmt_token);
    push("title", "role", fmt_token);
    push("skill", "level", fmt_int);
    for (element, key) in branch.stats() {
        push(element, key, fmt_int);
    }
    let portrait = match leader {
        Some(l) => portrait_asset(ctx, l),
        None => assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName),
    };
    push_sprite(
        fidelity,
        summary_sprites,
        "unit_leader",
        portrait,
        Fidelity::Exact,
    );
    // the XP-progress denominator is a rules-DEF constant (derive); badged here.
    let _ = ctx; // (the header reads only the leader record + DEF; ctx kept for symmetry)
    push_unavailable(fidelity, summary, "skill_level_too_low");
}

// =====================================================================================
// VIEW 6 — ARMY TRAIT TREE
// =====================================================================================

const ARMY_VIEW_ID: ViewId = ViewId("leaders.army_trait_tree");
const ARMY_FILE: &str = "army_trait_tree.json";
const ARMY_ROOT: &str = "unitleader_trait_tree_window";

pub struct ArmyTraitTreeView;

impl ArmyTraitTreeView {
    pub fn new() -> ArmyTraitTreeView {
        ArmyTraitTreeView
    }
}

impl Default for ArmyTraitTreeView {
    fn default() -> Self {
        ArmyTraitTreeView::new()
    }
}

impl ViewInstance for ArmyTraitTreeView {
    fn view_id(&self) -> ViewId {
        ARMY_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        bind_trait_tree(
            ctx,
            Branch::Army,
            ARMY_FILE,
            ARMY_ROOT,
            ARMY_VIEW_ID,
            "ARMYTRAIT",
            self.policy(),
        )
    }
    crate::impl_view_any!();
}

fn open_army(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(ArmyTraitTreeView::new())
}

register_view!(ViewDescriptor {
    id: ARMY_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(ARMY_FILE, ARMY_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LeadersOfficersTraits,
        label: "Army Trait Tree",
        order: 6
    }),
    shortcut: None,
    open: open_army,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 7 — NAVY TRAIT TREE
// =====================================================================================

const NAVY_VIEW_ID: ViewId = ViewId("leaders.navy_trait_tree");
const NAVY_FILE: &str = "navy_trait_tree.json";
const NAVY_ROOT: &str = "navyleader_trait_tree_window";

pub struct NavyTraitTreeView;

impl NavyTraitTreeView {
    pub fn new() -> NavyTraitTreeView {
        NavyTraitTreeView
    }
}

impl Default for NavyTraitTreeView {
    fn default() -> Self {
        NavyTraitTreeView::new()
    }
}

impl ViewInstance for NavyTraitTreeView {
    fn view_id(&self) -> ViewId {
        NAVY_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        bind_trait_tree(
            ctx,
            Branch::Navy,
            NAVY_FILE,
            NAVY_ROOT,
            NAVY_VIEW_ID,
            "NAVYTRAIT",
            self.policy(),
        )
    }
    crate::impl_view_any!();
}

fn open_navy(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(NavyTraitTreeView::new())
}

register_view!(ViewDescriptor {
    id: NAVY_VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(NAVY_FILE, NAVY_ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LeadersOfficersTraits,
        label: "Navy Trait Tree",
        order: 7
    }),
    shortcut: None,
    open: open_navy,
    policy: HostPolicy::window(ZBand::Panel),
});

// =====================================================================================
// VIEW 8 — TRAIT-TREE COMMON PROTOTYPES (templates only — no window, no route).
// =====================================================================================
//
// The shared prototype library the army/navy adapters clone per node + per edge. There is
// NO standalone window to route — these are registered as first-class TEMPLATES (reachable
// + accounted, never orphaned) without editing any shared file (Codex no-fab #5).

const COMMON_FILE: &str = "trait_tree_common.json";

register_template!(TemplateDescriptor {
    id: ViewId("leaders.trait_common.unit_leader_trait"),
    gui_template: GuiSymbolRef::template(COMMON_FILE, "unit_leader_trait"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.trait_common.picked_trait"),
    gui_template: GuiSymbolRef::template(COMMON_FILE, "picked_trait"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.trait_common.pickable_trait"),
    gui_template: GuiSymbolRef::template(COMMON_FILE, "pickable_trait"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.trait_common.disabled_trait"),
    gui_template: GuiSymbolRef::template(COMMON_FILE, "disabled_trait"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.trait_common.active_trait"),
    gui_template: GuiSymbolRef::template(COMMON_FILE, "active_trait"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.trait_common.advisor_trait"),
    gui_template: GuiSymbolRef::template(COMMON_FILE, "advisor_trait"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.trait_common.exile_trait"),
    gui_template: GuiSymbolRef::template(COMMON_FILE, "exile_trait"),
    required_domains: &["countries"],
});
