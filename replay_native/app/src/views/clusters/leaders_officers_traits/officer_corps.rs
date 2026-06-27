//! VIEW 1 — OFFICER CORPS (`officer_corps.json`, `countryofficercorpview`).
//!
//! The high-command / advisor / doctrine-spirit hub. The advisor chief grids
//! (`commanders.advisors[]`, FOLD-IN) render "—" in the save-only demo; the officer-corps
//! SPIRIT tiles are DERIVED NOW from `politics.ideas_other[]` (folded). Captured-generals,
//! named-HQ-templates and doctrine-track readouts are badged (FOLD-IN / TRUE-GAP).

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

const VIEW_ID: ViewId = ViewId("leaders.officer_corps");
const FILE: &str = "officer_corps.json";
const ROOT: &str = "countryofficercorpview";

pub struct OfficerCorpsView;

impl OfficerCorpsView {
    pub fn new() -> OfficerCorpsView {
        OfficerCorpsView
    }
}

impl Default for OfficerCorpsView {
    fn default() -> Self {
        OfficerCorpsView::new()
    }
}

struct OfficerCorpsDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    /// the per-branch chief/theorist advisor rows (FOLD-IN; 0 rows + section badge in demo).
    chief_rows: Vec<DataRow>,
    chief_section: SectionState,
    /// the officer-corps spirit tiles DERIVED from `politics.ideas_other[]` (present).
    spirit_rows: Vec<DataRow>,
    spirit_section: SectionState,
}

impl Drawable for OfficerCorpsDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "OFFICER chief_sect={} chiefs={} spirit_sect={} spirits={}",
            self.chief_section.tag(),
            row_digest(&self.chief_rows),
            self.spirit_section.tag(),
            row_digest(&self.spirit_rows),
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "high_command_advisors",
            "high_command_entry",
            &self.chief_rows,
        );
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "army_spirits_view",
            "army_spirit_window_item_grid",
            &self.spirit_rows,
        );
    }
}

impl ViewInstance for OfficerCorpsView {
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

        let mut summary: Vec<Cell> = Vec::new();
        // The country-leader context (officer-corps header). FOLD-IN → "—" in demo.
        push_country(
            ctx,
            tag,
            &mut fidelity,
            &mut summary,
            "army_title",
            "commanders.leader.name",
            Fidelity::Exact,
            fmt_token,
        );
        // Captured-generals counter (abilities domain, unfed) → FOLD-IN.
        push_unavailable(&mut fidelity, &mut summary, "captured_generals_amount");
        // The HQ deployed counter (OOB HQ join, unfed) → FOLD-IN.
        push_unavailable(&mut fidelity, &mut summary, "deployed_counter");

        let (chief_rows, chief_section) = build_chief_rows(ctx, tag, &mut fidelity);
        let (spirit_rows, spirit_section) = build_spirit_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, ROOT, &summary);
        let drawable = OfficerCorpsDrawable {
            roots: roots.clone(),
            file,
            chief_rows,
            chief_section,
            spirit_rows,
            spirit_section,
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

/// The chief/theorist advisor rows: one `high_command_entry` per `commanders.advisors[]`
/// (FOLD-IN). In the save-only demo `commanders` is unfed → ZERO rows + a section badge
/// (NOT a fabricated "empty advisor" with skill 0 — Codex no-fab #3).
fn build_chief_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let (advisors, state) = country_array(ctx, tag, "commanders.advisors");
    let arr = match advisors {
        Some(a) => a,
        None => {
            state.record(fidelity);
            return (Vec::new(), state);
        }
    };
    let mut rows = Vec::new();
    for adv in arr {
        let mut row = DataRow::new();
        row.bind_sprite(
            fidelity,
            "button",
            portrait_asset(ctx, adv),
            Fidelity::Exact,
        );
        row.bind(
            fidelity,
            "leader_text",
            elem_field(adv, "name"),
            Fidelity::Exact,
            fmt_token,
        );
        // advisor portrait + trait/type frames are asset / idea-DB steps → badged.
        row.badge_unavailable(fidelity, "advisor_type_icon");
        row.badge_unavailable(fidelity, "idea_traits");
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

/// The officer-corps SPIRIT tiles: one `army_spirit_window_item_grid`-style tile per active
/// spirit token. DERIVED NOW from `politics.ideas_other[]` (the ordered active-ideas set —
/// folded in this demo snapshot). The spirit NAME is a loc of the token (derived); the
/// modifier blurb / cost are idea-DEF (STATIC DEF) → badged. The ledger PARTITION (which
/// branch a spirit belongs to) is the trait-DEF reach, badged here, so we list the WHOLE
/// active-ideas set as candidate officer-corps spirits (membership exact; partition gap).
fn build_spirit_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let (ideas, state) = country_array(ctx, tag, "politics.ideas_other");
    let arr = match ideas {
        Some(a) => a,
        None => {
            state.record(fidelity);
            return (Vec::new(), state);
        }
    };
    let mut rows = Vec::new();
    for idea in arr {
        let mut row = DataRow::new();
        // spirit token → name (loc of the active token; derived). The token may be a string
        // or an object {token: ...}; take the string form, else the `token` key.
        let token_cap = if idea.is_string() {
            crate::host::context::classify_value(Some(idea))
        } else {
            elem_field(idea, "token")
        };
        row.bind(fidelity, "name", token_cap, Fidelity::Derived, fmt_token);
        // the spirit modifier blurb + cost are idea-DEF (STATIC DEF, not country state).
        row.badge_unavailable(fidelity, "traits");
        row.badge_unavailable(fidelity, "cost");
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

fn open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(OfficerCorpsView::new())
}

register_view!(ViewDescriptor {
    id: VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(FILE, ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LeadersOfficersTraits,
        label: "Officer Corps",
        order: 1
    }),
    shortcut: None,
    open,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.officer_corps.high_command_entry"),
    gui_template: GuiSymbolRef::template(FILE, "high_command_entry"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.officer_corps.army_chief_entry"),
    gui_template: GuiSymbolRef::template(FILE, "army_chief_entry"),
    required_domains: &["countries"],
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.officer_corps.army_spirit_grid"),
    gui_template: GuiSymbolRef::template(FILE, "army_spirit_window_item_grid"),
    required_domains: &["countries"],
});
