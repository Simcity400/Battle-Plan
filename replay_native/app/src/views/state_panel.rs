//! CORE SURFACE 3 (migrated) — the STATE PANEL (`province_state.json` →
//! `countrystateview` ROOT), a Panel-band window opened on a province pick.
//!
//! It owns the selected `state_id` + resolved name (its persistent state). `bind`
//! rebuilds the scene-graph fresh, binds via `bind::bind_state`, lays it out at the
//! fixed panel placement, and returns a `Drawable` that draws the nodes + the owner/
//! controller flags + the buildings gridbox + scrollbar + the occupation bars. ALL the
//! old `compose.rs` special draws for this surface move HERE (Codex (c)) — no compose
//! branch remains. The Codex §3 block-rect guard is declared so a click on the panel's
//! empty interior is consumed, not a map re-pick.

use bind::{bind_state, FlagResolver, StateBindReport};
use gui_runtime::draw::{DrawStats, TextureCache};
use gui_runtime::fonts::FontCache;
use gui_runtime::gridbox::{self, GridSpec};
use gui_runtime::scene::{ElementKind, Node};
use gui_runtime::{build_scene, layout, GuiFile, Loc, ManifestContent, Rect};

use crate::host::context::BindContext;
use crate::host::draw::FrameDraw;
use crate::host::registry::{ViewDescriptor, ViewId};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InstanceId, ViewCommand, ViewInstance, ZBand,
};
use crate::host::FidelityReport;
use crate::register_view;

pub const STATE_PANEL_VIEW_ID: ViewId = ViewId("core.state_panel");
const PANEL_FILE: &str = "province_state.json";
const PANEL_X: f32 = 40.0;
const PANEL_Y: f32 = 110.0;

/// The state panel instance: the selected state + its display name (persistent state).
pub struct StatePanelView {
    pub state_id: u32,
    pub state_name: String,
}

impl StatePanelView {
    pub fn new(state_id: u32, state_name: impl Into<String>) -> StatePanelView {
        StatePanelView {
            state_id,
            state_name: state_name.into(),
        }
    }
}

/// The state panel's `Drawable`: nodes + flags + buildings grid + occupation bars.
pub struct StatePanelDrawable {
    roots: Vec<Node>,
    report: StateBindReport,
    panel_file: GuiFile,
}

impl Drawable for StatePanelDrawable {
    /// The state panel's extra (non-node) pixel inputs: the owner/controller flag tags,
    /// the building rows (key+level+frame), and the occupation bar fractions. These are
    /// custom draws OUTSIDE the node digest, so digesting them here makes the panel's
    /// flags/gridbox/bars byte-comparable across a scrub (Codex (c)).
    fn draw_digest(&self) -> String {
        let mut s = format!(
            "STATE_PANEL owner={:?} ctrl={:?} buildings=[",
            self.report.owner_tag, self.report.controller_tag
        );
        for b in self
            .report
            .state_buildings
            .iter()
            .chain(self.report.province_buildings.iter())
        {
            s.push_str(&format!("{}={}@{};", b.key, b.level, b.frame));
        }
        s.push(']');
        match &self.report.occupation {
            Some(o) => s.push_str(&format!(" occ=r{:.2}/c{:.2}", o.resistance, o.compliance)),
            None => s.push_str(" occ=none"),
        }
        s
    }

    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        paint_state_flags(fd, &self.roots, &self.report);
        render_buildings_grid(fd, &self.roots, &self.panel_file, &self.report);
        render_occupation_bars(fd, &self.roots, &self.report);
    }
}

impl ViewInstance for StatePanelView {
    fn view_id(&self) -> ViewId {
        STATE_PANEL_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Panel)
    }
    fn bind(&self, ctx: &BindContext) -> BoundView {
        let stage = ctx.stage;
        let report = bind_state(ctx.snapshot(), &self.state_id.to_string(), &self.state_name);
        let panel_file = stage
            .gui_file(PANEL_FILE)
            .cloned()
            .unwrap_or_else(empty_gui);
        let roots = build_panel_roots(stage, &panel_file, &report, ctx.screen);

        let fidelity = panel_fidelity(&report);

        BoundView {
            instance: InstanceId(0),
            view_id: STATE_PANEL_VIEW_ID,
            policy: self.policy(),
            // the Codex §3 guard: the panel root is a pure container (no capturing bg),
            // so without an explicit block rect a click on its empty interior would
            // re-pick a province BEHIND it. Declare the panel's bounds as the block rect.
            hit: HitModel::from_roots(roots.clone()).with_block_rect(panel_block_rect()),
            drawable: Box::new(StatePanelDrawable {
                roots,
                report,
                panel_file,
            }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The panel's map-blocking bounding rect (placement + declared size + chrome margin),
/// identical to the old `panel_block_rect`.
fn panel_block_rect() -> Rect {
    Rect {
        x: PANEL_X,
        y: PANEL_Y,
        w: 436.0 + 8.0,
        h: 496.0 + 8.0,
    }
}

/// Build + lay out the panel scene-graph for the bound report — byte-identical to the
/// old `rebuild_panel_roots` (same filter set + placement + text substitution order).
fn build_panel_roots(
    stage: &crate::host::context::Stage,
    panel_file: &GuiFile,
    report: &StateBindReport,
    screen: Rect,
) -> Vec<Node> {
    let cm = ManifestContent {
        manifest: &stage.manifest,
        measure: |_t, _f| None,
    };
    let mut panel: Vec<Node> = build_scene(panel_file, &stage.manifest, &stage.loc)
        .into_iter()
        .filter(|w| w.name == "countrystateview")
        .collect();
    for root in panel.iter_mut() {
        root.orientation = gui_runtime::Anchor::UpperLeft;
        root.origo = gui_runtime::Anchor::UpperLeft;
        root.pos = (PANEL_X, PANEL_Y);
    }
    for fld in &report.fields {
        if fld.element.contains("status_container")
            || fld.element == "state_owners"
            || fld.element == "controller_flag"
        {
            continue;
        }
        for root in panel.iter_mut() {
            bind::apply::set_element_text(root, &fld.element, &fld.value);
        }
    }
    for root in panel.iter_mut() {
        layout(root, screen, None, &cm);
    }
    panel
}

/// Tally the state-panel bound fields into a fidelity report.
fn panel_fidelity(report: &StateBindReport) -> FidelityReport {
    let mut f = FidelityReport::default();
    for fld in &report.fields {
        if fld.not_captured {
            f.unavailable += 1;
        } else {
            match fld.fidelity {
                bind::Fidelity::Exact => f.exact += 1,
                bind::Fidelity::Derived => f.derived += 1,
                bind::Fidelity::Unavailable => f.unavailable += 1,
            }
        }
    }
    f
}

fn empty_gui() -> GuiFile {
    GuiFile::parse(r#"{"view":"empty","windows":[]}"#).expect("empty gui parses")
}

// --- the special draws, moved out of compose.rs into the owning view ---

fn paint_state_flags(fd: &mut FrameDraw, panel: &[Node], report: &StateBindReport) {
    let flag_res = FlagResolver::new(&fd.stage.resolver);
    if let (Some(tag), Some(rect)) = (&report.owner_tag, find_rect(panel, "state_owners")) {
        if let Some(rf) = flag_res.resolve(tag, None) {
            if let Ok(tex) = fd.renderer.load_image(&rf.path) {
                fd.renderer.draw_sprite(&tex, rect.x, rect.y, 41.0, 26.0);
            }
        }
    }
    if let (Some(tag), Some(rect)) = (&report.controller_tag, find_rect(panel, "controller_flag")) {
        if let Some(rf) = flag_res.resolve(tag, None) {
            if let Ok(tex) = fd.renderer.load_image(&rf.path) {
                fd.renderer.draw_sprite(&tex, rect.x, rect.y, 41.0, 26.0);
            }
        }
    }
}

fn render_buildings_grid(
    fd: &mut FrameDraw,
    panel: &[Node],
    panel_file: &GuiFile,
    report: &StateBindReport,
) {
    let viewport = match find_rect(panel, "state_building_entries") {
        Some(r) => r,
        None => return,
    };
    let template = match find_template(panel_file, "state_building_entry") {
        Some(t) => t,
        None => return,
    };
    let mut rows = report.state_buildings.clone();
    rows.extend(report.province_buildings.clone());
    let n = rows.len();
    if n == 0 {
        return;
    }
    let manifest = &fd.stage.manifest;
    let spec = GridSpec::from_gridbox(
        &stub_named("state_building_entries", viewport),
        &template,
        14.0,
    );
    let content = ManifestContent {
        manifest,
        measure: |_t, _f| None,
    };
    let mut inst = gridbox::instantiate(&template, n, viewport, &spec, 0.0, &content);
    for (i, row) in inst.rows.iter_mut().enumerate() {
        let b = &rows[i];
        bind::apply::set_element_text(row, "level", &b.level);
        gridbox::set_row_icon_frame(row, "building_picture", b.frame);
    }
    let mut stats = DrawStats::default();
    let textures: &mut TextureCache = fd.textures;
    let fonts: &mut FontCache = fd.fonts;
    for row in &inst.rows {
        gui_runtime::draw::draw_window(fd.renderer, row, textures, fonts, &mut stats);
    }
    if let Some(sb) = inst.scrollbar {
        if let Some(t) = textures.get(fd.renderer, "GFX_scrollbar_vertical_bg") {
            let border = manifest
                .get("GFX_scrollbar_vertical_bg")
                .and_then(|e| e.border_size)
                .map(|b| [b[0] as f32, b[1] as f32])
                .unwrap_or([0.0, 52.0]);
            fd.renderer
                .draw_nine_slice(t, sb.track.x, sb.track.y, sb.track.w, sb.track.h, border);
        }
        if let Some(t) = textures.get(fd.renderer, "GFX_generic_box_96") {
            fd.renderer
                .draw_sprite(t, sb.thumb.x, sb.thumb.y, sb.thumb.w, sb.thumb.h);
        }
    }
}

fn render_occupation_bars(fd: &mut FrameDraw, panel: &[Node], report: &StateBindReport) {
    let occ = match &report.occupation {
        Some(o) => *o,
        None => return,
    };
    let base = find_rect(panel, "resistance_container").unwrap_or(Rect {
        x: 60.0,
        y: 760.0,
        w: 519.0,
        h: 100.0,
    });
    let bar_w = 81.0_f32;
    let bar_h = 14.0_f32;
    for (i, (gfx, frac)) in [
        (
            "GFX_occupation_state_resistance_bar",
            (occ.resistance / 100.0) as f32,
        ),
        (
            "GFX_occupation_state_compliance_bar",
            (occ.compliance / 100.0) as f32,
        ),
    ]
    .iter()
    .enumerate()
    {
        let y = base.y + 6.0 + i as f32 * 28.0;
        if let Some(t) = fd.textures.get(fd.renderer, gfx) {
            fd.renderer.draw_progress_bar(
                t,
                base.x + 40.0,
                y,
                bar_w,
                bar_h,
                *frac,
                0,
                1,
                [1.0, 1.0, 1.0, 1.0],
            );
        }
    }
}

fn find_rect(roots: &[Node], name: &str) -> Option<Rect> {
    fn walk(n: &Node, name: &str) -> Option<Rect> {
        if n.name == name && n.rect.w > 0.0 && n.rect.h > 0.0 {
            return Some(n.rect);
        }
        for c in &n.children {
            if let Some(r) = walk(c, name) {
                return Some(r);
            }
        }
        None
    }
    roots.iter().find_map(|r| walk(r, name))
}

fn find_template(file: &GuiFile, name: &str) -> Option<Node> {
    let raw = file
        .windows
        .iter()
        .find(|w| w.name.as_deref() == Some(name))?;
    let loc = Loc::new();
    let manifest = assets::SpriteManifest {
        sprites: std::collections::HashMap::new(),
    };
    // template node uses the file's own kinds; the manifest only promotes 9-slice/
    // progressbar, which the building row doesn't need — an empty manifest matches the
    // old `find_template` (which passed the real manifest but the row has no such kind).
    Some(Node::from_raw(raw, &manifest, &loc))
}

fn stub_named(name: &str, rect: Rect) -> Node {
    Node {
        name: name.to_string(),
        raw_kind: "gridBoxType".into(),
        tooltip: None,
        kind: ElementKind::Container { background: None },
        pos: (0.0, 0.0),
        size_w: None,
        size_h: None,
        orientation: gui_runtime::Anchor::UpperLeft,
        origo: gui_runtime::Anchor::UpperLeft,
        clipping: false,
        alwaystransparent: false,
        rect,
        clip: None,
        children: vec![],
    }
}

/// Opened by a province pick (drill) carrying the state id; the binary resolves the
/// name (via map_dir + loc) before opening — but the registry factory can also build a
/// nameless instance from the arg (the binary prefers the named path).
///
/// NO-FABRICATION (Codex): a missing/invalid arg must NOT fabricate a state. State ids
/// are 1-based in the game, so `arg` absent or `<= 0` means "no real state selected": we
/// build a panel on sentinel id `0` with an EMPTY name, so `bind_state` honestly badges
/// the name (and every snapshot field, since `states."0"` never exists) as not-captured
/// "—" — instead of the old `STATE_0`, a fabricated non-empty name `bind_state` marked
/// EXACT/captured. A real drill always supplies a positive id + resolved name.
fn open(arg: Option<i64>) -> Box<dyn ViewInstance> {
    match arg {
        Some(sid) if sid > 0 => {
            let sid = sid as u32;
            Box::new(StatePanelView::new(sid, format!("STATE_{sid}")))
        }
        // no real selection -> sentinel id 0 + empty (not-captured) name, never a
        // fabricated `STATE_0`.
        _ => Box::new(StatePanelView::new(0, String::new())),
    }
}

register_view!(ViewDescriptor {
    id: STATE_PANEL_VIEW_ID,
    gui_root: Some(crate::host::registry::GuiSymbolRef::root(
        PANEL_FILE,
        "countrystateview"
    )),
    required_domains: &["states"],
    route: None, // opened by province pick, not the menu
    shortcut: None,
    open,
    policy: HostPolicy::window(ZBand::Panel),
});

register_template!(crate::host::registry::TemplateDescriptor {
    id: ViewId("core.state_building_entry"),
    gui_template: crate::host::registry::GuiSymbolRef::template(PANEL_FILE, "state_building_entry"),
    required_domains: &["states"],
});

pub fn link() {}

use crate::register_template;

#[cfg(test)]
mod tests {
    use super::*;

    /// NO-FABRICATION (Codex): the registry factory `open(None)` (the generic
    /// command/registry open path with no drill arg) must NOT fabricate a `STATE_0`
    /// non-existent state. It builds a sentinel id `0` with an EMPTY name, so
    /// `bind_state` honestly badges the name as not-captured (state ids are 1-based, so
    /// `states."0"` never exists and every field resolves to "—"), never a fabricated
    /// captured value.
    #[test]
    fn open_with_no_arg_does_not_fabricate_state_zero() {
        let snap: snapshot::Snapshot = serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "states": { "64": { "owner": "GER", "victory_points_total": 5 } }
        }))
        .unwrap();

        // the factory must not invent a "STATE_0" name.
        let inst = open(None);
        let v = inst
            .as_any()
            .downcast_ref::<StatePanelView>()
            .expect("state panel view");
        assert_eq!(v.state_id, 0);
        assert_eq!(
            v.state_name, "",
            "open(None) must NOT fabricate a STATE_0 name"
        );

        // and binding the sentinel state reports the name as NOT-CAPTURED (the
        // not-captured glyph), never a fabricated exact value.
        let report = bind_state(&snap, &v.state_id.to_string(), &v.state_name);
        let name_field = report
            .fields
            .iter()
            .find(|f| f.element == "state_name")
            .expect("state_name field");
        assert!(
            name_field.not_captured,
            "the empty sentinel name must badge not-captured, not a fabricated STATE_0"
        );
        assert_ne!(name_field.value, "STATE_0");

        // a NEGATIVE / zero arg is treated the same (no real selection).
        let neg = open(Some(-7));
        let nv = neg.as_any().downcast_ref::<StatePanelView>().unwrap();
        assert_eq!(nv.state_name, "");
        assert_eq!(
            open(Some(0))
                .as_any()
                .downcast_ref::<StatePanelView>()
                .unwrap()
                .state_name,
            ""
        );

        // a real positive arg still builds a named instance (the documented fallback).
        let pos = open(Some(64));
        let pv = pos.as_any().downcast_ref::<StatePanelView>().unwrap();
        assert_eq!(pv.state_id, 64);
        assert_eq!(pv.state_name, "STATE_64");
    }
}
