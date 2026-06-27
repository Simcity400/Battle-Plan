//! VIEW 4 — LEADER-GROUPS MAP PORTRAITS (`leader_groups.json`,
//! `leader_groups_portrait_window`).
//!
//! The map-overlay command portraits floating next to selected army/army-group leaders.
//! Binds the §12 `order_of_battle.armies[]`/`army_groups[]` `leader` join + member counts.
//! Unfed in the save-only demo → portraits/counts badged FOLD-IN; the RUNTIME-cached
//! battle-plan power/eval bars are badged PROXY (non-authoritative even once folded — Codex
//! note E in the binding map).

use gui_runtime::scene::Node;
use gui_runtime::GuiFile;

use bind::Fidelity;

use crate::host::context::{BindContext, FidelityReport};
use crate::host::draw::FrameDraw;
use crate::host::registry::{
    GuiSymbolRef, MenuCluster, MenuRoute, TemplateDescriptor, ViewDescriptor, ViewId,
};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InstanceId, ViewCommand, ViewInstance, ZBand,
};
use crate::{register_template, register_view};

use super::*;

const VIEW_ID: ViewId = ViewId("leaders.leader_groups");
const FILE: &str = "leader_groups.json";
const ROOT: &str = "leader_groups_portrait_window";

pub struct LeaderGroupsView;

impl LeaderGroupsView {
    pub fn new() -> LeaderGroupsView {
        LeaderGroupsView
    }
}

impl Default for LeaderGroupsView {
    fn default() -> Self {
        LeaderGroupsView::new()
    }
}

struct LeaderGroupsDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    summary_sprites: Vec<SpriteCell>,
    /// the attached-unit pip rows (air wings / railway guns under the portrait).
    attached_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for LeaderGroupsDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "LEADERGRP sect={} sprites={} attached={}",
            self.section.tag(),
            sprites_digest(&self.summary_sprites),
            row_digest(&self.attached_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "attached_units",
            "group_wing_type_box",
            &self.attached_rows,
        );
    }
}

impl ViewInstance for LeaderGroupsView {
    fn view_id(&self) -> ViewId {
        VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, FILE);

        let mut summary: Vec<Cell> = Vec::new();
        let mut summary_sprites: Vec<SpriteCell> = Vec::new();
        push_sprite(
            &mut fidelity,
            &mut summary_sprites,
            "leader_icon",
            land_oob_portrait_asset(ctx, ctx.country),
            Fidelity::Exact,
        );
        // The army's commanding general's divisions count (OOB join, unfed) → FOLD-IN.
        push_unavailable(&mut fidelity, &mut summary, "divisions_count");
        // The RUNTIME-cached battle-plan bars: PROXY even once folded (badged unavailable).
        push_unavailable(&mut fidelity, &mut summary, "plan_evaluation");

        let (attached_rows, section) = build_attached_rows(ctx, &mut fidelity);

        let roots = build_root_with_sprites(ctx, &file, ROOT, &summary, &summary_sprites);
        let drawable = LeaderGroupsDrawable {
            roots: roots.clone(),
            file,
            summary_sprites,
            attached_rows,
            section,
        };
        BoundView {
            instance: InstanceId(0),
            view_id: VIEW_ID,
            policy: self.policy(),
            hit: HitModel::from_roots(roots),
            drawable: Box::new(drawable),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The attached-unit pips: one per army-group / army that has attached air-wings or railway
/// guns (§12 join). Unfed in the demo → ZERO rows + a section badge. (When folded, each row
/// binds the attached `wing_count` / `railway_gun_count`.)
fn build_attached_rows(
    ctx: &BindContext,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let oob = ctx.order_of_battle();
    let (armies, state) = domain_array(&oob, "/armies");
    let arr = match armies {
        Some(a) => a,
        None => {
            state.record(fidelity);
            return (Vec::new(), state);
        }
    };
    let mut rows = Vec::new();
    for army in arr {
        // an attached air-wing / railway-gun count, if the army record carries it.
        let wing = match army.pointer("/attached_air_wings") {
            Some(serde_json::Value::Array(a)) => Some(a.len()),
            _ => None,
        };
        if let Some(n) = wing {
            let mut row = DataRow::new();
            let v = serde_json::Value::from(n);
            row.bind(
                fidelity,
                "wing_count_box",
                crate::host::context::Captured::Measured(&v),
                Fidelity::Exact,
                fmt_int,
            );
            rows.push(row);
        }
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(LeaderGroupsView::new())
}

register_view!(ViewDescriptor {
    id: VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(FILE, ROOT)),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LeadersOfficersTraits,
        label: "Command Groups",
        order: 4
    }),
    shortcut: None,
    open,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.leader_groups.wing_box"),
    gui_template: GuiSymbolRef::template(FILE, "group_wing_type_box"),
    required_domains: &["order_of_battle"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.leader_groups.railway_box"),
    gui_template: GuiSymbolRef::template(FILE, "group_railway_guns_box"),
    required_domains: &["order_of_battle"],
});
