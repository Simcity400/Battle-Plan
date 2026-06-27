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

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;

use bind::Fidelity;
use serde_json::Value;

use widgets::common::{Atom, Canvas, DrawAssets, Reason};
use widgets::node_graph;
use widgets::trait_tree::{TraitNode, TraitState, TraitTree};
use widgets::{plan_tooltip, Tooltip, TooltipPlan};

use crate::host::context::{BindContext, Captured, DisplayAtom, FidelityReport, UnavailableReason};
use crate::host::draw::FrameDraw;
use crate::host::input::{PointerButton, ViewInput};
use crate::host::registry::{
    GuiSymbolRef, MenuCluster, MenuRoute, TemplateDescriptor, ViewDescriptor, ViewId,
};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InputOutcome, InstanceId, RebindContext,
    ViewCommand, ViewInstance, ZBand,
};
use crate::{register_template, register_view};

use super::*;

// =====================================================================================
// Shared trait-tree machinery (army + navy share the engine; only the stat set differs).
// =====================================================================================

#[derive(Debug, Clone, Default)]
struct TraitDef {
    kind: Option<String>,
    trait_type: Option<String>,
    cost: Option<String>,
    prerequisite_count: Option<usize>,
    mutually_exclusive_count: Option<usize>,
    modifier_count: Option<usize>,
    source_file: Option<String>,
}

#[derive(Debug, Default)]
struct TraitDefCatalog {
    traits: HashMap<String, TraitDef>,
}

static TRAIT_DEFS: OnceLock<TraitDefCatalog> = OnceLock::new();

impl TraitDefCatalog {
    fn get() -> &'static TraitDefCatalog {
        TRAIT_DEFS.get_or_init(TraitDefCatalog::load)
    }

    fn load() -> TraitDefCatalog {
        Self::catalog_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|txt| Self::parse(&txt))
            .unwrap_or_else(|| TraitDefCatalog {
                traits: HashMap::new(),
            })
    }

    fn catalog_path() -> Option<PathBuf> {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let replay_native = crate_dir.parent()?;
        let repo = replay_native.parent()?;
        Some(repo.join("game_defs").join("out").join("catalogs.json"))
    }

    fn parse(txt: &str) -> Option<TraitDefCatalog> {
        let v: Value = serde_json::from_str(txt).ok()?;
        let traits = v
            .get("catalogs")?
            .get("leader_traits")?
            .as_object()?
            .iter()
            .map(|(token, def)| (token.clone(), parse_trait_def(def)))
            .collect();
        Some(TraitDefCatalog { traits })
    }

    fn trait_def(&self, token: &str) -> Option<&TraitDef> {
        self.traits.get(token)
    }
}

fn parse_trait_def(def: &Value) -> TraitDef {
    TraitDef {
        kind: scalar_text(def.get("type")),
        trait_type: scalar_text(def.get("trait_type")),
        cost: scalar_text(def.get("cost")),
        prerequisite_count: array_len(def.get("prerequisites")),
        mutually_exclusive_count: array_len(def.get("mutually_exclusive")),
        modifier_count: sum_object_lens(def, &["modifier", "non_shared_modifier"]),
        source_file: scalar_text(def.get("source_file")),
    }
}

fn scalar_text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        Some(Value::Bool(b)) => Some(b.to_string()),
        _ => None,
    }
}

fn object_len(value: Option<&Value>) -> Option<usize> {
    match value {
        Some(value) => value.as_object().map(|obj| obj.len()),
        None => None,
    }
}

fn array_len(value: Option<&Value>) -> Option<usize> {
    match value {
        Some(value) => value.as_array().map(|arr| arr.len()),
        None => None,
    }
}

fn sum_object_lens(def: &Value, fields: &[&str]) -> Option<usize> {
    let mut total = 0usize;
    let mut found = false;
    for field in fields {
        if let Some(len) = object_len(def.get(*field)) {
            total += len;
            found = true;
        }
    }
    if found {
        Some(total)
    } else {
        None
    }
}

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

fn selected_leader_rebind<'a>(
    ctx: &'a RebindContext,
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

fn selectable_trait_node(node: &TraitNode) -> bool {
    matches!(node.state, TraitState::Owned) && node.id != "no_leader" && node.id != "no_traits"
}

fn selectable_trait_key(tree: &TraitTree, key: &str) -> bool {
    tree.nodes
        .iter()
        .any(|node| node.id == key && selectable_trait_node(node))
}

pub fn trait_tree_viewport_from_roots(roots: &[Node]) -> [f32; 4] {
    find_rect(roots, "regular_traits")
        .or_else(|| find_rect(roots, "tree"))
        .map(|r| [r.x, r.y, r.w.max(200.0), r.h.max(120.0)])
        .unwrap_or([320.0, 200.0, 600.0, 400.0])
}

fn trait_tree_canvas(tree: &TraitTree, viewport: [f32; 4]) -> Canvas {
    let graph = tree.to_graph();
    Canvas::fit(graph.bounds(), viewport, 16.0)
}

#[derive(Clone, Debug)]
struct TraitPickModel {
    tree: TraitTree,
    canvas: Canvas,
}

#[derive(Default)]
struct TraitTreeState {
    selected_key: Option<String>,
    click_candidate: Option<String>,
    drag_moved: bool,
    hover_cursor: Option<(f32, f32)>,
    pick_model: Option<TraitPickModel>,
}

impl TraitTreeState {
    fn rebind(&mut self, leader: Option<&serde_json::Value>, viewport: [f32; 4]) {
        let tree = build_trait_tree(leader);
        let canvas = trait_tree_canvas(&tree, viewport);
        self.pick_model = Some(TraitPickModel { tree, canvas });
    }

    fn set_viewport(&mut self, viewport: [f32; 4]) {
        let Some(model) = &mut self.pick_model else {
            return;
        };
        model.canvas = trait_tree_canvas(&model.tree, viewport);
    }

    fn pick(&self, x: f32, y: f32) -> Option<String> {
        let model = self.pick_model.as_ref()?;
        let key = model.tree.to_graph().pick(&model.canvas, x, y)?;
        selectable_trait_key(&model.tree, &key).then_some(key)
    }

    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => {
                let Some(model) = &self.pick_model else {
                    self.click_candidate = None;
                    self.drag_moved = false;
                    return InputOutcome::ignored();
                };
                if !model.canvas.contains(x, y) {
                    self.click_candidate = None;
                    self.drag_moved = false;
                    self.hover_cursor = None;
                    return InputOutcome::ignored();
                }
                self.click_candidate = self.pick(x, y);
                self.drag_moved = false;
                self.hover_cursor = self.click_candidate.as_ref().map(|_| (x, y));
                if self.click_candidate.is_some() {
                    InputOutcome::consumed()
                } else {
                    InputOutcome::ignored()
                }
            }
            ViewInput::PointerMove { x: _, y: _, drag } if drag => {
                if self.click_candidate.is_some() {
                    self.drag_moved = true;
                    self.click_candidate = None;
                    self.hover_cursor = None;
                    InputOutcome::consumed()
                } else {
                    InputOutcome::ignored()
                }
            }
            ViewInput::PointerMove { x, y, drag: false } => {
                let next = self.pick(x, y).map(|_| (x, y));
                let changed = self.hover_cursor != next;
                self.hover_cursor = next;
                if changed {
                    InputOutcome {
                        consumed: false,
                        commands: Vec::new(),
                        needs_redraw: true,
                    }
                } else {
                    InputOutcome::ignored()
                }
            }
            ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } => {
                let candidate = if !self.drag_moved {
                    match (self.click_candidate.take(), self.pick(x, y)) {
                        (Some(down), Some(up)) if down == up => Some(down),
                        _ => None,
                    }
                } else {
                    self.click_candidate = None;
                    None
                };
                self.drag_moved = false;
                self.hover_cursor = self.pick(x, y).map(|_| (x, y));
                match candidate {
                    Some(key) => {
                        let changed = self.selected_key.as_deref() != Some(key.as_str());
                        self.selected_key = Some(key);
                        InputOutcome {
                            consumed: true,
                            commands: Vec::new(),
                            needs_redraw: changed,
                        }
                    }
                    None => InputOutcome::ignored(),
                }
            }
            _ => InputOutcome::ignored(),
        }
    }

    #[cfg(test)]
    fn node_center(&self, key: &str) -> Option<(f32, f32)> {
        let model = self.pick_model.as_ref()?;
        let graph = model.tree.to_graph();
        let node = graph.node(key)?;
        let (cx, cy) = node.center();
        Some(model.canvas.to_screen(cx, cy))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TraitNodeDetail {
    title: Atom,
    rows: Vec<TraitNodeDetailRow>,
}

impl TraitNodeDetail {
    fn new(title: Atom) -> TraitNodeDetail {
        TraitNodeDetail {
            title,
            rows: Vec::new(),
        }
    }

    fn with_row(mut self, label: &'static str, value: Atom) -> TraitNodeDetail {
        self.rows.push(TraitNodeDetailRow { label, value });
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TraitNodeDetailRow {
    label: &'static str,
    value: Atom,
}

fn selected_trait_detail(
    tree: &TraitTree,
    selected: Option<&str>,
    leader: Option<&serde_json::Value>,
    branch: Branch,
) -> Option<TraitNodeDetail> {
    let key = selected?;
    let node = tree
        .nodes
        .iter()
        .find(|node| node.id == key && selectable_trait_node(node))?;
    Some(
        TraitNodeDetail::new(node.label.clone())
            .with_row("Trait ID", Atom::exact(node.id.clone()))
            .with_row("State", Atom::exact("owned"))
            .with_row(
                "Branch",
                Atom::exact(match branch {
                    Branch::Army => "army",
                    Branch::Navy => "navy",
                }),
            )
            .with_row("Leader name", leader_atom(leader, "name"))
            .with_row("Leader key", leader_key_atom(leader))
            .with_trait_def_rows(&node.id),
    )
}

fn trait_tooltip_plan(
    ctx: &BindContext,
    tree: &TraitTree,
    viewport: [f32; 4],
    hover_cursor: Option<(f32, f32)>,
    leader: Option<&serde_json::Value>,
    branch: Branch,
) -> Option<TooltipPlan> {
    let cursor = hover_cursor?;
    let canvas = trait_tree_canvas(tree, viewport);
    let key = tree.to_graph().pick(&canvas, cursor.0, cursor.1)?;
    let tooltip = trait_tooltip(tree, Some(key.as_str()), leader, branch)?;
    Some(plan_tooltip(&tooltip, cursor, ctx.screen))
}

fn trait_tooltip(
    tree: &TraitTree,
    key: Option<&str>,
    leader: Option<&serde_json::Value>,
    branch: Branch,
) -> Option<Tooltip> {
    let key = key?;
    let node = tree
        .nodes
        .iter()
        .find(|node| node.id == key && selectable_trait_node(node))?;
    Some(
        Tooltip::new(node.label.clone())
            .with_row("Trait ID", Atom::exact(node.id.clone()))
            .with_row("State", Atom::exact("owned"))
            .with_row(
                "Branch",
                Atom::exact(match branch {
                    Branch::Army => "army",
                    Branch::Navy => "navy",
                }),
            )
            .with_row("Leader name", leader_atom(leader, "name"))
            .with_row("Leader key", leader_key_atom(leader))
            .with_trait_def_rows(&node.id),
    )
}

trait TraitDefRows: Sized {
    fn with_trait_def_rows(self, token: &str) -> Self;
}

impl TraitDefRows for TraitNodeDetail {
    fn with_trait_def_rows(mut self, token: &str) -> TraitNodeDetail {
        for (label, atom) in trait_def_rows(token) {
            self = self.with_row(label, atom);
        }
        self
    }
}

impl TraitDefRows for Tooltip {
    fn with_trait_def_rows(mut self, token: &str) -> Tooltip {
        for (label, atom) in trait_def_rows(token) {
            self = self.with_row(label, atom);
        }
        self
    }
}

fn trait_def_rows(token: &str) -> Vec<(&'static str, Atom)> {
    let Some(def) = TraitDefCatalog::get().trait_def(token) else {
        return vec![
            ("Catalog type", Atom::unavailable_because(Reason::Absent)),
            ("Trait type", Atom::unavailable_because(Reason::Absent)),
            ("Cost", Atom::unavailable_because(Reason::Absent)),
            ("Prereq count", Atom::unavailable_because(Reason::Absent)),
            (
                "Mutual exclusions",
                Atom::unavailable_because(Reason::Absent),
            ),
            ("Modifier rows", Atom::unavailable_because(Reason::Absent)),
            ("Source file", Atom::unavailable_because(Reason::Absent)),
        ];
    };

    vec![
        ("Catalog type", optional_derived_atom(def.kind.as_deref())),
        (
            "Trait type",
            optional_derived_atom(def.trait_type.as_deref()),
        ),
        ("Cost", optional_derived_atom(def.cost.as_deref())),
        (
            "Prereq count",
            optional_derived_count_atom(def.prerequisite_count),
        ),
        (
            "Mutual exclusions",
            optional_derived_count_atom(def.mutually_exclusive_count),
        ),
        (
            "Modifier rows",
            optional_derived_count_atom(def.modifier_count),
        ),
        (
            "Source file",
            optional_derived_atom(def.source_file.as_deref()),
        ),
    ]
}

fn optional_derived_atom(value: Option<&str>) -> Atom {
    match value {
        Some(value) if !value.is_empty() => Atom::derived(value),
        _ => Atom::unavailable_because(Reason::Null),
    }
}

fn optional_derived_count_atom(value: Option<usize>) -> Atom {
    match value {
        Some(value) => Atom::derived(value.to_string()),
        None => Atom::unavailable_because(Reason::Null),
    }
}

fn leader_atom(leader: Option<&serde_json::Value>, field: &str) -> Atom {
    match leader.and_then(|l| l.get(field)) {
        Some(serde_json::Value::Null) => Atom::unavailable_because(Reason::Null),
        Some(v) => Atom::exact(fmt_token(v)),
        None => Atom::unavailable_because(Reason::Absent),
    }
}

fn leader_key_atom(leader: Option<&serde_json::Value>) -> Atom {
    let Some(leader) = leader else {
        return Atom::unavailable_because(Reason::Absent);
    };
    for field in ["unit_leader_id", "char_id", "token"] {
        match leader.get(field) {
            Some(serde_json::Value::Null) => return Atom::unavailable_because(Reason::Null),
            Some(v) => return Atom::exact(fmt_token(v)),
            None => {}
        }
    }
    Atom::unavailable_because(Reason::Absent)
}

fn trait_selection_state(tree: &TraitTree, selected: Option<&str>) -> &'static str {
    match selected {
        None => "none",
        Some(key) if selectable_trait_key(tree, key) => "selected",
        Some(_) => "missing_selected",
    }
}

fn trait_detail_digest(detail: Option<&TraitNodeDetail>) -> String {
    match detail {
        None => "none".to_string(),
        Some(detail) => {
            let mut s = format!("title={}", atom_digest(&detail.title));
            for row in &detail.rows {
                s.push_str(&format!(";{}={}", row.label, atom_digest(&row.value)));
            }
            s
        }
    }
}

fn atom_digest(atom: &Atom) -> String {
    format!(
        "text={:?},fidelity={:?},reason={:?}",
        atom.as_str(),
        atom.fidelity(),
        atom.reason()
    )
}

fn record_trait_detail_fidelity(fid: &mut FidelityReport, detail: Option<&TraitNodeDetail>) {
    let Some(detail) = detail else {
        return;
    };
    record_trait_atom_fidelity(fid, &detail.title);
    for row in &detail.rows {
        record_trait_atom_fidelity(fid, &row.value);
    }
}

fn record_trait_tooltip_fidelity(fid: &mut FidelityReport, tooltip: Option<&TooltipPlan>) {
    if tooltip.is_some() {
        fid.derived += 1;
    }
}

fn record_trait_atom_fidelity(fid: &mut FidelityReport, atom: &Atom) {
    match atom.fidelity() {
        widgets::common::Fidelity::Exact => fid.exact += 1,
        widgets::common::Fidelity::Derived => fid.derived += 1,
        widgets::common::Fidelity::Unavailable => fid.unavailable += 1,
    }
}

struct TraitTreeDrawable {
    roots: Vec<Node>,
    /// the N3 trait tree (selected leader's traits[] or the no-leader badge).
    tree: TraitTree,
    label: &'static str,
    branch: Branch,
    viewport: [f32; 4],
    tooltip: Option<TooltipPlan>,
    selected_key: Option<String>,
    selected_detail: Option<TraitNodeDetail>,
}

impl Drawable for TraitTreeDrawable {
    fn tooltip_plan(&self) -> Option<&TooltipPlan> {
        self.tooltip.as_ref()
    }

    fn draw_digest(&self) -> String {
        let g = self.tree.to_graph();
        let cam = Canvas::fit(g.bounds(), self.viewport, 16.0);
        let (plan, stats) = node_graph::plan_graph(&cam, &g);
        format!(
            "{} branch={} nodes={} edges={} dangling={} selected={} selected_key={} selected_detail={} PLAN {}",
            self.label,
            match self.branch {
                Branch::Army => "army",
                Branch::Navy => "navy",
            },
            stats.nodes_drawn,
            stats.edges_drawn,
            self.tree.dangling_edge_count(),
            trait_selection_state(&self.tree, self.selected_key.as_deref()),
            self.selected_key.as_deref().unwrap_or("none"),
            trait_detail_digest(self.selected_detail.as_ref()),
            plan.digest()
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        // draw the N3 trait tree into the regular-traits canvas (the real `drag_scroll`).
        let g = self.tree.to_graph();
        let cam = Canvas::fit(g.bounds(), self.viewport, 16.0);
        let white = mint_white(fd);
        let assets = widget_assets(&white);
        node_graph::draw_graph(fd.renderer, &cam, &g, &assets);
        if let Some(node) = self
            .selected_key
            .as_deref()
            .and_then(|key| g.node(key))
            .filter(|node| selectable_trait_key(&self.tree, &node.id))
        {
            let (cx, cy) = node.center();
            let (sx, sy) = cam.to_screen(cx, cy);
            let w = (node.w * cam.scale).max(1.0) + 6.0;
            let h = (node.h * cam.scale).max(1.0) + 6.0;
            widgets::common::stroke_rect(
                fd.renderer,
                &white,
                sx - w / 2.0,
                sy - h / 2.0,
                w,
                h,
                3.0,
                [0.92, 0.86, 0.46, 0.95],
                Some(self.viewport),
            );
        }
        if let Some(detail) = &self.selected_detail {
            draw_trait_detail_panel(fd.renderer, &assets, self.viewport, detail);
        }
    }
}

fn draw_trait_detail_panel(
    renderer: &mut render::Renderer,
    assets: &DrawAssets,
    viewport: [f32; 4],
    detail: &TraitNodeDetail,
) {
    let [vx, vy, vw, vh] = viewport;
    if !(vx.is_finite() && vy.is_finite() && vw.is_finite() && vh.is_finite()) {
        return;
    }
    if vw < 120.0 || vh < 96.0 {
        return;
    }
    let panel_w = (vw - 24.0).max(120.0).min(330.0);
    let desired_h = 46.0 + detail.rows.len() as f32 * 22.0;
    let panel_h = desired_h.min((vh - 24.0).max(72.0));
    let x = vx + vw - panel_w - 12.0;
    let y = vy + 12.0;
    let clip = Some([x, y, panel_w, panel_h]);

    widgets::common::fill_rect(
        renderer,
        assets.white,
        x,
        y,
        panel_w,
        panel_h,
        [0.07, 0.08, 0.09, 0.88],
        Some(viewport),
    );
    widgets::common::stroke_rect(
        renderer,
        assets.white,
        x,
        y,
        panel_w,
        panel_h,
        1.0,
        [0.55, 0.50, 0.38, 0.95],
        Some(viewport),
    );
    widgets::common::draw_atom(
        renderer,
        assets,
        &detail.title,
        x + 12.0,
        y + 12.0,
        widgets::common::TEXT_TINT,
        clip,
    );

    let label_x = x + 12.0;
    let value_x = x + 118.0;
    let mut row_y = y + 38.0;
    for row in &detail.rows {
        if row_y + 16.0 > y + panel_h {
            break;
        }
        widgets::common::draw_text(
            renderer,
            assets,
            row.label,
            label_x,
            row_y,
            widgets::common::DIM_TINT,
            clip,
        );
        widgets::common::draw_atom(
            renderer,
            assets,
            &row.value,
            value_x,
            row_y,
            widgets::common::TEXT_TINT,
            clip,
        );
        row_y += 22.0;
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
    selected_key: Option<&str>,
    hover_cursor: Option<(f32, f32)>,
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
    let viewport = trait_tree_viewport_from_roots(&roots);
    let tooltip = trait_tooltip_plan(ctx, &tree, viewport, hover_cursor, leader, branch);
    let selected_detail = selected_trait_detail(&tree, selected_key, leader, branch);
    record_trait_tooltip_fidelity(&mut fidelity, tooltip.as_ref());
    record_trait_detail_fidelity(&mut fidelity, selected_detail.as_ref());
    let drawable = TraitTreeDrawable {
        roots: roots.clone(),
        tree,
        label,
        branch,
        viewport,
        tooltip,
        selected_key: selected_key.map(str::to_string),
        selected_detail,
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

pub const ARMY_TRAIT_TREE_VIEW_ID: ViewId = ViewId("leaders.army_trait_tree");
const ARMY_FILE: &str = "army_trait_tree.json";
const ARMY_ROOT: &str = "unitleader_trait_tree_window";

#[derive(Default)]
pub struct ArmyTraitTreeView {
    state: TraitTreeState,
}

impl ArmyTraitTreeView {
    pub fn new() -> ArmyTraitTreeView {
        ArmyTraitTreeView::default()
    }

    pub fn set_input_viewport(&mut self, viewport: [f32; 4]) {
        self.state.set_viewport(viewport);
    }

    #[cfg(test)]
    pub(crate) fn debug_node_center(&self, key: &str) -> Option<(f32, f32)> {
        self.state.node_center(key)
    }

    #[cfg(test)]
    pub(crate) fn debug_set_selected_key(&mut self, key: impl Into<String>) {
        self.state.selected_key = Some(key.into());
    }
}

impl ViewInstance for ArmyTraitTreeView {
    fn view_id(&self) -> ViewId {
        ARMY_TRAIT_TREE_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn rebind(&mut self, ctx: &RebindContext) {
        let tag = if ctx.country.is_empty() {
            "GER"
        } else {
            ctx.country
        };
        let leader = selected_leader_rebind(ctx, tag, Branch::Army);
        self.state.rebind(leader, [320.0, 200.0, 600.0, 400.0]);
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        self.state.on_input(ev)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        bind_trait_tree(
            ctx,
            Branch::Army,
            ARMY_FILE,
            ARMY_ROOT,
            ARMY_TRAIT_TREE_VIEW_ID,
            "ARMYTRAIT",
            self.state.selected_key.as_deref(),
            self.state.hover_cursor,
            self.policy(),
        )
    }
    crate::impl_view_any!();
}

fn open_army(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(ArmyTraitTreeView::new())
}

register_view!(ViewDescriptor {
    id: ARMY_TRAIT_TREE_VIEW_ID,
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

pub const NAVY_TRAIT_TREE_VIEW_ID: ViewId = ViewId("leaders.navy_trait_tree");
const NAVY_FILE: &str = "navy_trait_tree.json";
const NAVY_ROOT: &str = "navyleader_trait_tree_window";

#[derive(Default)]
pub struct NavyTraitTreeView {
    state: TraitTreeState,
}

impl NavyTraitTreeView {
    pub fn new() -> NavyTraitTreeView {
        NavyTraitTreeView::default()
    }

    pub fn set_input_viewport(&mut self, viewport: [f32; 4]) {
        self.state.set_viewport(viewport);
    }

    #[cfg(test)]
    pub(crate) fn debug_node_center(&self, key: &str) -> Option<(f32, f32)> {
        self.state.node_center(key)
    }
}

impl ViewInstance for NavyTraitTreeView {
    fn view_id(&self) -> ViewId {
        NAVY_TRAIT_TREE_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn rebind(&mut self, ctx: &RebindContext) {
        let tag = if ctx.country.is_empty() {
            "GER"
        } else {
            ctx.country
        };
        let leader = selected_leader_rebind(ctx, tag, Branch::Navy);
        self.state.rebind(leader, [320.0, 200.0, 600.0, 400.0]);
    }
    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        self.state.on_input(ev)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        bind_trait_tree(
            ctx,
            Branch::Navy,
            NAVY_FILE,
            NAVY_ROOT,
            NAVY_TRAIT_TREE_VIEW_ID,
            "NAVYTRAIT",
            self.state.selected_key.as_deref(),
            self.state.hover_cursor,
            self.policy(),
        )
    }
    crate::impl_view_any!();
}

fn open_navy(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(NavyTraitTreeView::new())
}

register_view!(ViewDescriptor {
    id: NAVY_TRAIT_TREE_VIEW_ID,
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
