//! VIEW 3 — NAVY-LEADER FLEET STRIP (`navy_leader.json`, `navy_leader_window`).
//!
//! The per-fleet admiral portrait strip. Binds `order_of_battle.fleets[]` (§12, a TOP-LEVEL
//! domain read via `ctx.order_of_battle()`), each fleet → its `leader` admiral. Unfed in the
//! save-only demo → ZERO tiles + a section badge (never a fabricated fleet).

use gui_runtime::scene::Node;
use gui_runtime::GuiFile;

use bind::Fidelity;

use crate::host::context::{BindContext, Captured, FidelityReport};
use crate::host::draw::FrameDraw;
use crate::host::registry::{
    GuiSymbolRef, MenuCluster, MenuRoute, TemplateDescriptor, ViewDescriptor, ViewId,
};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InstanceId, ViewCommand, ViewInstance, ZBand,
};
use crate::{register_template, register_view};

use super::*;

const VIEW_ID: ViewId = ViewId("leaders.navy_leader");
const FILE: &str = "navy_leader.json";
const ROOT: &str = "navy_leader_window";

pub struct NavyLeaderView;

impl NavyLeaderView {
    pub fn new() -> NavyLeaderView {
        NavyLeaderView
    }
}

impl Default for NavyLeaderView {
    fn default() -> Self {
        NavyLeaderView::new()
    }
}

struct NavyLeaderDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    fleet_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for NavyLeaderDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "NAVYLEADER sect={} fleets={}",
            self.section.tag(),
            row_digest(&self.fleet_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "fleets_box",
            "new_fleet_window",
            &self.fleet_rows,
        );
    }
}

impl ViewInstance for NavyLeaderView {
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

        let (fleet_rows, section) = build_fleet_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, ROOT, &[]);
        let drawable = NavyLeaderDrawable {
            roots: roots.clone(),
            file,
            fleet_rows,
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

/// One tile per `order_of_battle.fleets[]` owned by `<TAG>`, each → its `leader` admiral
/// (FOLD-IN). The OOB domain is unfed in the save-only demo → ZERO tiles + a section badge.
/// `taskforce_count` (per-fleet task-force count) is bound when the fleet record carries it.
fn build_fleet_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let oob = ctx.order_of_battle();
    let (fleets, state) = domain_array(&oob, "/fleets");
    let arr = match fleets {
        Some(a) => a,
        None => {
            state.record(fidelity);
            return (Vec::new(), state);
        }
    };
    let mut rows = Vec::new();
    for fleet in arr {
        // ONLY this country's fleets — the owner must EXPLICITLY be `<TAG>`. An owner-absent
        // fleet is AMBIGUOUS (the OOB list may carry shared/global fleets with no owner tag),
        // so it is SKIPPED rather than leaked as the observed country's (Codex (d) fix). We
        // never fabricate an owner to keep it.
        match elem_field(fleet, "owner") {
            Captured::Measured(o) if o.as_str() == Some(tag) => {}
            _ => continue,
        }
        let mut row = DataRow::new();
        let mut portrait = portrait_asset(ctx, fleet);
        if matches!(
            portrait,
            assets::SpriteAsset::Unavailable(assets::SpriteMiss::NoSpriteName)
        ) {
            portrait =
                commander_portrait_for_ref(ctx, tag, fleet.get("leader"), &["commanders.admirals"]);
        }
        row.bind_sprite(fidelity, "portrait_btn", portrait, Fidelity::Exact);
        // task-force count under this fleet's admiral.
        let tf = match fleet.pointer("/task_forces") {
            Some(serde_json::Value::Array(a)) => {
                crate::host::context::Captured::Measured(serde_json::Value::from(a.len()))
            }
            _ => crate::host::context::Captured::Absent,
        };
        let tf_atom = match tf {
            crate::host::context::Captured::Measured(v) => {
                crate::host::context::DisplayAtom::text(fmt_int(&v), Fidelity::Exact)
            }
            _ => crate::host::context::DisplayAtom::unavailable(),
        };
        row.push_atom(fidelity, "taskforce_count", tf_atom);
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(NavyLeaderView::new())
}

register_view!(ViewDescriptor {
    id: VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(FILE, ROOT)),
    required_domains: &["order_of_battle"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LeadersOfficersTraits,
        label: "Admirals",
        order: 3
    }),
    shortcut: None,
    open,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.navy_leader.fleet_tile"),
    gui_template: GuiSymbolRef::template(FILE, "new_fleet_window"),
    required_domains: &["order_of_battle"],
});
