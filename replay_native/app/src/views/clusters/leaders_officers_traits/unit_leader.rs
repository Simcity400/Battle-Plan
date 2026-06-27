//! VIEW 2 — UNIT-LEADER ROSTER + DETAIL (`unit_leader.json`, `armyleaderwindow`).
//!
//! The leader-selection roster for the army high-command (the load-bearing context the
//! brief assigns: general / field-marshal detail). Binds `commanders.generals[]` +
//! `commanders.field_marshals[]` (FOLD-IN → "—" in the save-only demo; ZERO rows, NOT a
//! fabricated empty-leader struct). The navy/captured/operative/division/faction roots are
//! row-template-classified in the bundle manifest.

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

const VIEW_ID: ViewId = ViewId("leaders.unit_leader");
const FILE: &str = "unit_leader.json";
const ROOT: &str = "armyleaderwindow";

pub struct UnitLeaderView;

impl UnitLeaderView {
    pub fn new() -> UnitLeaderView {
        UnitLeaderView
    }
}

impl Default for UnitLeaderView {
    fn default() -> Self {
        UnitLeaderView::new()
    }
}

struct UnitLeaderDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    leader_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for UnitLeaderDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "UNITLEADER sect={} leaders={}",
            self.section.tag(),
            row_digest(&self.leader_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "leaders_grid",
            "armyleaderentry",
            &self.leader_rows,
        );
    }
}

impl ViewInstance for UnitLeaderView {
    fn view_id(&self) -> ViewId {
        VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let tag = ctx.country;
        let mut fidelity = FidelityReport::default();
        let file = load_gui(ctx, FILE);

        let (leader_rows, section) = build_leader_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, ROOT, &[]);
        let drawable = UnitLeaderDrawable {
            roots: roots.clone(),
            file,
            leader_rows,
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

/// One `armyleaderentry` per army leader from `commanders.generals[] + field_marshals[]`
/// (FOLD-IN). In the save-only demo `commanders` is unfed → ZERO rows + a section badge
/// (Codex no-fab #3: an unfed roster is NOT a fabricated empty-leader with skill 0). When a
/// live snapshot folds the domain, each row binds name/role/skill/stats/traits exact.
fn build_leader_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let mut leaders: Vec<&serde_json::Value> = Vec::new();
    let mut section = SectionState::Absent;
    let mut any_present = false;
    for path in ["commanders.generals", "commanders.field_marshals"] {
        let (arr, st) = country_array(ctx, tag, path);
        match arr {
            Some(a) => {
                any_present = true;
                section = SectionState::Present;
                leaders.extend(a.iter());
            }
            None => {
                // remember the FIRST not-present reason for the badge.
                if !any_present && section == SectionState::Absent {
                    section = st;
                }
            }
        }
    }
    if !any_present {
        section.record(fidelity);
        return (Vec::new(), section);
    }
    let mut rows = Vec::new();
    for l in leaders {
        let mut row = DataRow::new();
        row.bind_sprite(
            fidelity,
            "leader_button",
            portrait_asset(ctx, l),
            Fidelity::Exact,
        );
        row.bind(
            fidelity,
            "name",
            elem_field(l, "name"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "title",
            elem_field(l, "role"),
            Fidelity::Exact,
            fmt_token,
        );
        row.bind(
            fidelity,
            "skill",
            elem_field(l, "level"),
            Fidelity::Exact,
            fmt_int,
        );
        row.bind(
            fidelity,
            "leader_stat_attack",
            elem_field(l, "attack"),
            Fidelity::Exact,
            fmt_int,
        );
        row.bind(
            fidelity,
            "leader_stat_defense",
            elem_field(l, "defense"),
            Fidelity::Exact,
            fmt_int,
        );
        row.bind(
            fidelity,
            "leader_stat_planning",
            elem_field(l, "planning"),
            Fidelity::Exact,
            fmt_int,
        );
        row.bind(
            fidelity,
            "leader_stat_logistics",
            elem_field(l, "logistics"),
            Fidelity::Exact,
            fmt_int,
        );
        row.push_atom(
            fidelity,
            "traits_grid",
            super::trait_trees::traits_join_atom(l),
        );
        // the divisions-count is an OOB join (§12, unfed) → FOLD-IN.
        row.badge_unavailable(fidelity, "divisions_count");
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(UnitLeaderView::new())
}

register_view!(ViewDescriptor {
    id: VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(FILE, ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LeadersOfficersTraits,
        label: "Generals",
        order: 2
    }),
    shortcut: None,
    open,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.unit_leader.army_entry"),
    gui_template: GuiSymbolRef::template(FILE, "armyleaderentry"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.unit_leader.navy_entry"),
    gui_template: GuiSymbolRef::template(FILE, "navyleaderentry"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.unit_leader.captured_entry"),
    gui_template: GuiSymbolRef::template(FILE, "capturedarmyleaderentry"),
    required_domains: &["countries"],
});
