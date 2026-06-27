//! VIEW 5 — ACES (`aces.json`, `acesview`).
//!
//! The grid of flying aces. Binds `countries.<TAG>.extra.aces[]` (§3.11) — the `extra`
//! domain is schema-present but parser-unfed in the save-only demo → ZERO tiles + a section
//! badge (FOLD-IN, never a fabricated ace).

use gui_runtime::scene::Node;
use gui_runtime::GuiFile;

use bind::Fidelity;

use crate::host::context::{BindContext, DisplayAtom, FidelityReport, UnavailableReason};
use crate::host::draw::FrameDraw;
use crate::host::registry::{
    GuiSymbolRef, MenuCluster, MenuRoute, TemplateDescriptor, ViewDescriptor, ViewId,
};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InstanceId, ViewCommand, ViewInstance, ZBand,
};
use crate::{register_template, register_view};

use super::*;

const VIEW_ID: ViewId = ViewId("leaders.aces");
const FILE: &str = "aces.json";
const ROOT: &str = "acesview";

pub struct AcesView;

impl AcesView {
    pub fn new() -> AcesView {
        AcesView
    }
}

impl Default for AcesView {
    fn default() -> Self {
        AcesView::new()
    }
}

struct AcesDrawable {
    roots: Vec<Node>,
    file: GuiFile,
    ace_rows: Vec<DataRow>,
    section: SectionState,
}

impl Drawable for AcesDrawable {
    fn draw_digest(&self) -> String {
        format!(
            "ACES sect={} aces={}",
            self.section.tag(),
            row_digest(&self.ace_rows)
        )
    }
    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        render_rows(
            fd,
            &self.roots,
            &self.file,
            "grid",
            "acesview_entry",
            &self.ace_rows,
        );
    }
}

impl ViewInstance for AcesView {
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

        let (ace_rows, section) = build_ace_rows(ctx, tag, &mut fidelity);

        let roots = build_root(ctx, &file, ROOT, &[]);
        let drawable = AcesDrawable {
            roots: roots.clone(),
            file,
            ace_rows,
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

/// One `acesview_entry` per `extra.aces[]` (FOLD-IN). Unfed in the save-only demo → ZERO
/// rows + a section badge. When folded: name (+ surname + callsign), modifier, kill_type
/// frame, KIA overlay. The current-wing equipment + eligible-plane-types are an ace→wing
/// join / ace-DEF (STATIC DEF) → badged.
fn build_ace_rows(
    ctx: &BindContext,
    tag: &str,
    fidelity: &mut FidelityReport,
) -> (Vec<DataRow>, SectionState) {
    let (aces, state) = country_array(ctx, tag, "extra.aces");
    let arr = match aces {
        Some(a) => a,
        None => {
            state.record(fidelity);
            return (Vec::new(), state);
        }
    };
    let mut rows = Vec::new();
    for ace in arr {
        let mut row = DataRow::new();
        row.push_atom(fidelity, "name", ace_name_atom(ace));
        row.bind(
            fidelity,
            "modifiers",
            elem_field(ace, "modifier"),
            Fidelity::Exact,
            fmt_token,
        );
        // the current-wing equipment / name needs the ace→wing OOB join (unfed) → badged.
        row.badge_unavailable(fidelity, "current_wing_name");
        // the eligible-plane-types are an ace-DEF attribute (STATIC DEF) → badged.
        row.badge_unavailable(fidelity, "current_wing_equipment_icon");
        rows.push(row);
    }
    SectionState::Present.record(fidelity);
    (rows, SectionState::Present)
}

/// Join an ace's name + surname + callsign into one display atom (exact when present;
/// Absent → "—"). Never fabricates a blank — an all-absent ace renders the glyph.
pub(super) fn ace_name_atom(ace: &serde_json::Value) -> DisplayAtom {
    let part = |k: &str| ace.get(k).and_then(|v| v.as_str()).map(|s| s.to_string());
    let mut joined: Vec<String> = Vec::new();
    if let Some(n) = part("name") {
        joined.push(n);
    }
    if let Some(s) = part("surname") {
        joined.push(s);
    }
    let mut base = joined.join(" ");
    if let Some(c) = part("callsign") {
        if !c.is_empty() {
            base = if base.is_empty() {
                format!("\"{c}\"")
            } else {
                format!("{base} \"{c}\"")
            };
        }
    }
    if base.is_empty() {
        DisplayAtom::unavailable_because(UnavailableReason::Absent)
    } else {
        DisplayAtom::text(base, Fidelity::Exact)
    }
}

fn open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(AcesView::new())
}

register_view!(ViewDescriptor {
    id: VIEW_ID,
    gui_root: Some(GuiSymbolRef::root(FILE, ROOT)),
    required_domains: &["countries"],
    route: Some(MenuRoute {
        cluster: MenuCluster::LeadersOfficersTraits,
        label: "Aces",
        order: 5
    }),
    shortcut: None,
    open,
    policy: HostPolicy::window(ZBand::Panel),
});
register_template!(TemplateDescriptor {
    id: ViewId("leaders.aces.entry"),
    gui_template: GuiSymbolRef::template(FILE, "acesview_entry"),
    required_domains: &["countries"],
});
