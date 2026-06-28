//! CORE SURFACE 2 (migrated) — the TOPBAR, a Chrome-band full-screen pure-frame
//! window. `bind` rebuilds the scene-graph fresh from the `.gui` ROOT, runs
//! `bind::apply_topbar` (the no-fabrication topbar spec), lays it out, and returns a
//! `Drawable` that draws the nodes + the player flag sprite (resolved by tag+ideology,
//! the Codex §4 cache-key concern). Byte-identical to the old topbar path: the topbar
//! does NOT block the map (its empty regions fall through), only its real controls
//! capture (`HitModel` over the laid-out roots).

use bind::{apply_topbar, BindReport, FlagResolver, TopbarSpec};
use gui_runtime::scene::Node;
use gui_runtime::{build_scene, layout, ManifestContent, Rect};

use crate::host::context::{BindContext, Captured};
use crate::host::draw::FrameDraw;
use crate::host::input::{PointerButton, ViewInput};
use crate::host::registry::{ViewDescriptor, ViewId};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InputOutcome, InstanceId, ViewCommand, ViewInstance,
    ZBand,
};
use crate::host::FidelityReport;
use crate::register_view;
use widgets::{plan_tooltip, Atom as WAtom, Reason as WReason, Tooltip, TooltipPlan};

pub const TOPBAR_VIEW_ID: ViewId = ViewId("core.topbar");
const TOPBAR_FILE: &str = "topbar.json";

/// The topbar view instance carries only the observed country tag, passive hover state,
/// and the latest `.gui` button rects synced by the app shell before routing input.
pub struct TopbarView {
    pub country: String,
    drill_targets: Vec<TopbarDrillTarget>,
    inert_targets: Vec<TopbarInertTarget>,
    hover: Option<TopbarHover>,
}

impl TopbarView {
    pub fn new(country: impl Into<String>) -> TopbarView {
        TopbarView {
            country: country.into(),
            drill_targets: Vec::new(),
            inert_targets: Vec::new(),
            hover: None,
        }
    }

    pub fn set_drill_targets(&mut self, targets: Vec<TopbarDrillTarget>) {
        self.drill_targets = targets;
    }

    pub fn set_inert_targets(&mut self, targets: Vec<TopbarInertTarget>) {
        self.inert_targets = targets;
    }

    fn hover_at(&self, x: f32, y: f32) -> Option<TopbarHover> {
        if self
            .drill_targets
            .iter()
            .any(|target| is_threat_leaf(target.leaf) && target.rect.contains(x, y))
        {
            return Some(TopbarHover {
                target: TopbarHoverTarget::Threat,
                cursor: (x, y),
            });
        }
        self.inert_targets
            .iter()
            .any(|target| target.leaf == "faction_button" && target.rect.contains(x, y))
            .then_some(TopbarHover {
                target: TopbarHoverTarget::Faction,
                cursor: (x, y),
            })
    }

    fn drill_command_at(&self, x: f32, y: f32) -> Option<ViewCommand> {
        self.drill_targets
            .iter()
            .find(|target| target.rect.contains(x, y))
            .map(|target| ViewCommand::OpenView {
                view_id: target.view_id,
                arg: None,
            })
    }

    fn has_drill_target_at(&self, x: f32, y: f32) -> bool {
        self.drill_targets
            .iter()
            .any(|target| target.rect.contains(x, y))
    }

    fn has_inert_target_at(&self, x: f32, y: f32) -> bool {
        self.inert_targets
            .iter()
            .any(|target| target.rect.contains(x, y))
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TopbarDrillTarget {
    pub leaf: &'static str,
    pub view_id: ViewId,
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy)]
pub struct TopbarInertTarget {
    pub leaf: &'static str,
    pub rect: Rect,
}

#[derive(Debug, Clone, PartialEq)]
struct TopbarHover {
    target: TopbarHoverTarget,
    cursor: (f32, f32),
}

#[derive(Debug, Clone, PartialEq)]
enum TopbarHoverTarget {
    Faction,
    Threat,
}

const TOPBAR_DRILL_TARGETS: &[(&str, ViewId)] = &[
    ("threat_button", ViewId("intel.world_tension_popup")),
    ("threat_value_button", ViewId("intel.world_tension_popup")),
    ("decisionview_button", ViewId("production.decisions")),
    ("intel_agency_button", ViewId("intel.intel_agency")),
    ("technology_button", ViewId("research.research")),
    ("diplomacy_button", ViewId("intel.diplomacy")),
    ("trade_button", ViewId("intel.trade")),
    ("construction_button", ViewId("production.construction")),
    ("production_button", ViewId("production.production")),
    ("deployment_button", ViewId("military.deployment")),
    ("logistics_button", ViewId("logistics.logistics")),
    ("officer_corp_button", ViewId("leaders.officer_corps")),
    ("army_button", ViewId("military.army")),
    ("navy_button", ViewId("military.navies")),
    ("air_button", ViewId("military.air_group")),
];

const TOPBAR_INERT_TARGETS: &[&str] = &[
    "pause_button_date",
    "pause_button_date_paused",
    "button_speeddown",
    "button_speedstep1",
    "button_speedstep2",
    "button_speedstep3",
    "button_speedstep4",
    "button_speedstep5",
    "button_speedup",
    "reopen_ingame_lobby",
    "menu_button",
    "help_button",
    "achievements_button",
    "dismissed_alerts_button",
    "faction_button",
    "playlist_button",
    "music_pause_button",
    "music_next_button",
    "musicplayer_button",
];

pub fn topbar_drill_targets_from_roots(roots: &[Node]) -> Vec<TopbarDrillTarget> {
    TOPBAR_DRILL_TARGETS
        .iter()
        .filter_map(|(leaf, view_id)| {
            find_rect(roots, leaf).map(|rect| TopbarDrillTarget {
                leaf,
                view_id: *view_id,
                rect,
            })
        })
        .collect()
}

pub fn topbar_inert_targets_from_roots(roots: &[Node]) -> Vec<TopbarInertTarget> {
    TOPBAR_INERT_TARGETS
        .iter()
        .filter_map(|leaf| find_rect(roots, leaf).map(|rect| TopbarInertTarget { leaf, rect }))
        .collect()
}

fn consumed_no_redraw(commands: Vec<ViewCommand>) -> InputOutcome {
    InputOutcome {
        consumed: true,
        commands,
        needs_redraw: false,
    }
}

/// The topbar's `Drawable`: the laid-out nodes + the player flag sprite.
pub struct TopbarDrawable {
    roots: Vec<Node>,
    /// (rect, tag, ideology) for the flag sprite (resolved per-frame by tag+ideology).
    flag: Option<(Rect, String, Option<String>)>,
    tooltip: Option<TooltipPlan>,
}

impl Drawable for TopbarDrawable {
    /// The topbar's extra (non-node) pixel input is the FLAG sprite identity, which
    /// depends on TAG + ruling-party ideology (the Codex §4 cache-key trap). Digesting
    /// it here means a stale flag carried across a scrub would change the digest — the
    /// node digest alone (which has no flag) would miss it.
    fn draw_digest(&self) -> String {
        match &self.flag {
            Some((rect, tag, ideo)) => format!(
                "TOPBAR_FLAG {tag}/{ideo:?}@{:.1},{:.1},{:.1},{:.1}",
                rect.x, rect.y, rect.w, rect.h
            ),
            None => "TOPBAR_FLAG none".into(),
        }
    }

    fn draw(&self, fd: &mut FrameDraw) {
        fd.draw_nodes(&self.roots);
        if let Some((rect, tag, ideology)) = &self.flag {
            let flag_res = FlagResolver::new(&fd.stage.resolver);
            if let Some(rf) = flag_res.resolve(tag, ideology.as_deref()) {
                if let Ok(tex) = fd.renderer.load_image(&rf.path) {
                    fd.renderer
                        .draw_sprite(&tex, rect.x, rect.y, rect.w, rect.h);
                }
            }
        }
    }

    fn tooltip_plan(&self) -> Option<&TooltipPlan> {
        self.tooltip.as_ref()
    }
}

impl ViewInstance for TopbarView {
    fn view_id(&self) -> ViewId {
        TOPBAR_VIEW_ID
    }
    fn policy(&self) -> HostPolicy {
        HostPolicy::window(ZBand::Chrome)
    }

    fn on_input(&mut self, ev: &ViewInput) -> InputOutcome {
        match *ev {
            ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            } => match self.drill_command_at(x, y) {
                Some(command) => consumed_no_redraw(vec![command]),
                None if self.has_inert_target_at(x, y) => consumed_no_redraw(Vec::new()),
                None => InputOutcome::ignored(),
            },
            ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            } if self.has_drill_target_at(x, y) || self.has_inert_target_at(x, y) => {
                consumed_no_redraw(Vec::new())
            }
            ViewInput::PointerMove { x, y, drag: false } => {
                let before = self.hover.clone();
                self.hover = self.hover_at(x, y);
                InputOutcome {
                    consumed: false,
                    commands: Vec::new(),
                    needs_redraw: self.hover != before,
                }
            }
            _ => InputOutcome::ignored(),
        }
    }

    fn bind(&self, ctx: &BindContext) -> BoundView {
        let stage = ctx.stage;
        let screen = ctx.screen;
        let cm = ManifestContent {
            manifest: &stage.manifest,
            measure: |_t, _f| None,
        };

        // fresh scene-graph each tick (no stale rows across a scrub).
        let mut roots: Vec<Node> = match stage.gui_file(TOPBAR_FILE) {
            Some(f) => build_scene(f, &stage.manifest, &stage.loc),
            None => Vec::new(),
        };
        let spec = TopbarSpec::topbar();
        let report: BindReport = apply_topbar(&mut roots, &spec, ctx.snapshot(), &self.country);
        for root in roots.iter_mut() {
            layout(root, screen, None, &cm);
        }

        let flag = flag_rect(&roots, &report, &self.country);
        let tooltip = topbar_tooltip_for_hover(
            ctx,
            &self.country,
            self.hover.as_ref(),
            &self.drill_targets,
            &self.inert_targets,
        );
        let fidelity = topbar_fidelity(&report);

        BoundView {
            instance: InstanceId(0),
            view_id: TOPBAR_VIEW_ID,
            policy: self.policy(),
            // the topbar is a pure full-screen frame: NO block rect — its empty regions
            // fall through to the map; only its real controls capture (via the roots).
            hit: HitModel::from_roots(roots.clone()),
            drawable: Box::new(TopbarDrawable {
                roots,
                flag,
                tooltip,
            }),
            fidelity,
            commands: Vec::<ViewCommand>::new(),
        }
    }

    crate::impl_view_any!();
}

/// The player-flag rect + (tag, ideology) from the bound report (mirrors the old
/// `topbar_flag_rect`).
fn flag_rect(
    roots: &[Node],
    report: &BindReport,
    country: &str,
) -> Option<(Rect, String, Option<String>)> {
    let rect = find_rect(roots, "player_flag")?;
    Some((rect, country.to_string(), report.flag_ideology.clone()))
}

/// Tally the topbar's bound fields into a fidelity report (so the per-view accounting
/// is visible). The `apply_topbar` report's `not_captured` flag = an unavailable badge.
fn topbar_fidelity(report: &BindReport) -> FidelityReport {
    let mut f = FidelityReport::default();
    for field in &report.fields {
        if field.not_captured {
            f.unavailable += 1;
        } else {
            match field.fidelity {
                bind::Fidelity::Exact => f.exact += 1,
                bind::Fidelity::Derived => f.derived += 1,
                bind::Fidelity::Unavailable => f.unavailable += 1,
            }
        }
    }
    f
}

fn topbar_tooltip_for_hover(
    ctx: &BindContext,
    country: &str,
    hover: Option<&TopbarHover>,
    drill_targets: &[TopbarDrillTarget],
    inert_targets: &[TopbarInertTarget],
) -> Option<TooltipPlan> {
    let hover = hover?;
    let valid_target = match hover.target {
        TopbarHoverTarget::Faction => inert_targets.iter().any(|target| {
            target.leaf == "faction_button" && target.rect.contains(hover.cursor.0, hover.cursor.1)
        }),
        TopbarHoverTarget::Threat => drill_targets.iter().any(|target| {
            is_threat_leaf(target.leaf) && target.rect.contains(hover.cursor.0, hover.cursor.1)
        }),
    };
    if !valid_target {
        return None;
    }
    match hover.target {
        TopbarHoverTarget::Faction => {
            let model = faction_tooltip_model(ctx, country);
            Some(plan_tooltip(&model, hover.cursor, ctx.screen))
        }
        TopbarHoverTarget::Threat => {
            let model = threat_tooltip_model(ctx);
            Some(plan_tooltip(&model, hover.cursor, ctx.screen))
        }
    }
}

fn is_threat_leaf(leaf: &str) -> bool {
    matches!(leaf, "threat_button" | "threat_value_button")
}

fn faction_tooltip_model(ctx: &BindContext, country: &str) -> Tooltip {
    Tooltip::domain_composer(WAtom::exact("Faction"))
        .exact_row("Country", country.to_string())
        .row(
            "Name",
            captured_scalar_atom(ctx.country_field(country, "politics.faction.name")),
        )
        .row(
            "Leader",
            captured_bool_atom(ctx.country_field(country, "politics.faction.is_leader")),
        )
        .build()
}

fn captured_scalar_atom(value: Captured<&serde_json::Value>) -> WAtom {
    match value {
        Captured::Measured(v) => measured_scalar(v)
            .map(WAtom::exact)
            .unwrap_or_else(|| WAtom::unavailable_because(WReason::Blocked)),
        Captured::Null => WAtom::unavailable_because(WReason::Null),
        Captured::Absent => WAtom::unavailable_because(WReason::Absent),
    }
}

fn captured_bool_atom(value: Captured<&serde_json::Value>) -> WAtom {
    match value {
        Captured::Measured(v) => match v.as_bool() {
            Some(true) => WAtom::exact("yes"),
            Some(false) => WAtom::exact("no"),
            None => WAtom::unavailable_because(WReason::Blocked),
        },
        Captured::Null => WAtom::unavailable_because(WReason::Null),
        Captured::Absent => WAtom::unavailable_because(WReason::Absent),
    }
}

fn threat_tooltip_model(ctx: &BindContext) -> Tooltip {
    Tooltip::new(WAtom::exact("World tension"))
        .with_row("Aggregate", WAtom::unavailable_because(WReason::Blocked))
        .with_row("Threat entries", threat_entry_count_atom(ctx))
}

fn threat_entry_count_atom(ctx: &BindContext) -> WAtom {
    match ctx.world_events() {
        Captured::Measured(v) => match v.get("threat") {
            Some(serde_json::Value::Array(rows)) => WAtom::exact(rows.len().to_string()),
            Some(serde_json::Value::Null) => WAtom::unavailable_because(WReason::Null),
            Some(_) => WAtom::unavailable_because(WReason::Blocked),
            None => WAtom::unavailable_because(WReason::Absent),
        },
        Captured::Null => WAtom::unavailable_because(WReason::Null),
        Captured::Absent => WAtom::unavailable_because(WReason::Absent),
    }
}

fn measured_scalar(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
        serde_json::Value::Number(n) => Some(n.to_string()),
        serde_json::Value::Bool(b) => Some(b.to_string()),
        _ => None,
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

/// Seeded by the binary with the observed country; not menu-opened (it is always on).
fn open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
    Box::new(TopbarView::new("GER"))
}

register_view!(ViewDescriptor {
    id: TOPBAR_VIEW_ID,
    gui_root: Some(crate::host::registry::GuiSymbolRef::root(
        TOPBAR_FILE,
        "topbar"
    )),
    required_domains: &["countries"],
    route: None, // always-on chrome, seeded at startup
    shortcut: None,
    open,
    policy: HostPolicy::window(ZBand::Chrome),
});

pub fn link() {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::context::{BindContext, Stage};
    use crate::views::digest::bound_digest;
    use gui_runtime::{GuiFile, Loc};
    use std::collections::{BTreeSet, HashMap};
    use std::path::Path;

    fn synthetic_topbar_gui() -> GuiFile {
        let mut buttons: Vec<serde_json::Value> = TOPBAR_DRILL_TARGETS
            .iter()
            .enumerate()
            .map(|(idx, (leaf, _))| {
                serde_json::json!({
                    "type": "buttonType",
                    "name": leaf,
                    "position": { "x": 10 + (idx as i64 * 34), "y": 10 },
                    "size": { "width": 30, "height": 30 },
                    "orientation": "UPPER_LEFT",
                    "sprite": "GFX_test_topbar_button"
                })
            })
            .collect();
        buttons.extend(TOPBAR_INERT_TARGETS.iter().enumerate().map(|(idx, leaf)| {
            serde_json::json!({
                "type": "buttonType",
                "name": leaf,
                "position": { "x": 10 + (idx as i64 * 34), "y": 60 },
                "size": { "width": 30, "height": 30 },
                "orientation": "UPPER_LEFT",
                "sprite": "GFX_test_topbar_button"
            })
        }));
        let doc = serde_json::json!({
            "view": "topbar",
            "windows": [{
                "type": "containerWindowType",
                "name": "topbar",
                "position": { "x": 0, "y": 0 },
                "size": { "width": "100%%", "height": "100%%" },
                "children": buttons
            }]
        });
        GuiFile::parse(&doc.to_string()).unwrap()
    }

    fn stage_with_topbar(gui: GuiFile, sprites: HashMap<String, assets::SpriteEntry>) -> Stage {
        let mut gui_files = HashMap::new();
        gui_files.insert(TOPBAR_FILE.to_string(), gui);
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest { sprites },
            loc: Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        }
    }

    fn snap() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "hour_index": 100,
            "countries": {
                "GER": {
                    "politics": {
                        "political_power": 10.0,
                        "stability": 0.5,
                        "ruling_party": "fascism",
                        "faction": {
                            "name": "axis",
                            "is_leader": true
                        }
                    }
                }
            }
        }))
        .unwrap()
    }

    fn snap_without_faction() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "hour_index": 100,
            "countries": {
                "GER": {
                    "politics": {
                        "political_power": 10.0,
                        "stability": 0.5,
                        "ruling_party": "fascism"
                    }
                }
            }
        }))
        .unwrap()
    }

    fn snap_with_null_faction() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "hour_index": 100,
            "countries": {
                "GER": {
                    "politics": {
                        "political_power": 10.0,
                        "stability": 0.5,
                        "ruling_party": "fascism",
                        "faction": {
                            "name": null,
                            "is_leader": null
                        }
                    }
                }
            }
        }))
        .unwrap()
    }

    fn snap_with_threat_events() -> snapshot::Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "hour_index": 100,
            "countries": {
                "GER": {
                    "politics": {
                        "political_power": 10.0,
                        "stability": 0.5,
                        "ruling_party": "fascism",
                        "faction": {
                            "name": "axis",
                            "is_leader": true
                        }
                    }
                }
            },
            "world_events": {
                "threat": [
                    {"tag": "GER", "threat": 5.0, "final_threat": 12.0},
                    {"tag": "ITA", "threat": 2.0, "final_threat": 4.0}
                ]
            }
        }))
        .unwrap()
    }

    fn ctx<'a>(stage: &'a Stage, snap: &'a snapshot::Snapshot) -> BindContext<'a> {
        BindContext {
            stage,
            snapshot: snap,
            country: "GER",
            screen: Rect {
                x: 0.0,
                y: 0.0,
                w: 1600.0,
                h: 900.0,
            },
            date: "1940.1.1.12",
            cursor: (0.0, 0.0),
            map_camera: None,
        }
    }

    fn center(rect: Rect) -> (f32, f32) {
        (rect.x + rect.w * 0.5, rect.y + rect.h * 0.5)
    }

    #[test]
    fn topbar_buttons_emit_read_only_open_view_commands() {
        let stage = stage_with_topbar(synthetic_topbar_gui(), HashMap::new());
        let snap = snap();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        let targets = topbar_drill_targets_from_roots(&bv.hit.roots);
        assert_eq!(
            targets.len(),
            TOPBAR_DRILL_TARGETS.len(),
            "synthetic topbar should expose every registered drill target"
        );
        view.set_drill_targets(targets.clone());
        view.set_inert_targets(topbar_inert_targets_from_roots(&bv.hit.roots));

        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        for target in targets {
            let (x, y) = center(target.rect);
            let out = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(out.consumed, "{} click should be captured", target.leaf);
            assert!(
                !out.needs_redraw,
                "{} click emits a host command but does not mutate topbar state",
                target.leaf
            );
            assert_eq!(
                out.commands,
                vec![ViewCommand::OpenView {
                    view_id: target.view_id,
                    arg: None
                }],
                "{} must open its registered read-only replay view",
                target.leaf
            );
        }
        let after = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert_eq!(
            before, after,
            "topbar navigation clicks must not mutate the topbar render state"
        );
    }

    #[test]
    fn topbar_empty_space_still_falls_through() {
        let stage = stage_with_topbar(synthetic_topbar_gui(), HashMap::new());
        let snap = snap();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        view.set_drill_targets(topbar_drill_targets_from_roots(&bv.hit.roots));
        view.set_inert_targets(topbar_inert_targets_from_roots(&bv.hit.roots));

        let out = view.on_input(&ViewInput::PointerDown {
            x: 1200.0,
            y: 800.0,
            button: PointerButton::Left,
        });
        assert!(!out.consumed);
        assert!(out.commands.is_empty());
        assert!(!out.needs_redraw);
    }

    #[test]
    fn topbar_inert_controls_are_replay_disabled_noops() {
        let stage = stage_with_topbar(synthetic_topbar_gui(), HashMap::new());
        let snap = snap();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        let inert_targets = topbar_inert_targets_from_roots(&bv.hit.roots);
        assert_eq!(
            inert_targets.len(),
            TOPBAR_INERT_TARGETS.len(),
            "synthetic topbar should expose every registered inert target"
        );
        view.set_drill_targets(topbar_drill_targets_from_roots(&bv.hit.roots));
        view.set_inert_targets(inert_targets.clone());

        for target in inert_targets {
            let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
            let (x, y) = center(target.rect);
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(
                down.consumed,
                "{} pointer-down must be consumed",
                target.leaf
            );
            assert!(
                down.commands.is_empty(),
                "{} pointer-down must emit no replay command",
                target.leaf
            );
            assert!(!down.needs_redraw, "{} pointer-down is inert", target.leaf);

            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(up.consumed, "{} pointer-up must be consumed", target.leaf);
            assert!(
                up.commands.is_empty(),
                "{} pointer-up must emit no replay command",
                target.leaf
            );
            assert!(!up.needs_redraw, "{} pointer-up is inert", target.leaf);
            assert_eq!(
                before,
                bound_digest(&view.bind(&ctx(&stage, &snap))),
                "{} disabled click must leave the topbar digest unchanged",
                target.leaf
            );
        }
    }

    #[test]
    fn topbar_alert_threat_faction_slice_routes_only_backed_controls() {
        let stage = stage_with_topbar(synthetic_topbar_gui(), HashMap::new());
        let snap = snap();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        let drill_targets = topbar_drill_targets_from_roots(&bv.hit.roots);
        let inert_targets = topbar_inert_targets_from_roots(&bv.hit.roots);
        view.set_drill_targets(drill_targets.clone());
        view.set_inert_targets(inert_targets.clone());

        for (leaf, view_id) in [
            ("threat_button", ViewId("intel.world_tension_popup")),
            ("threat_value_button", ViewId("intel.world_tension_popup")),
        ] {
            let target = drill_targets
                .iter()
                .find(|target| target.leaf == leaf)
                .unwrap_or_else(|| panic!("synthetic topbar should expose {leaf}"));
            let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
            let (x, y) = center(target.rect);
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(down.consumed, "{leaf} click should be captured");
            assert!(!down.needs_redraw, "{leaf} is read-only navigation");
            assert_eq!(
                down.commands,
                vec![ViewCommand::OpenView { view_id, arg: None }],
                "{leaf} must open only its backed read-only replay surface"
            );
            assert_eq!(
                before,
                bound_digest(&view.bind(&ctx(&stage, &snap))),
                "{leaf} click must leave the topbar digest unchanged"
            );
        }

        for leaf in ["dismissed_alerts_button", "faction_button"] {
            let target = inert_targets
                .iter()
                .find(|target| target.leaf == leaf)
                .unwrap_or_else(|| panic!("synthetic topbar should expose inert {leaf}"));
            let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
            let (x, y) = center(target.rect);
            let down = view.on_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(down.consumed, "{leaf} must consume pointer-down");
            assert!(
                down.commands.is_empty(),
                "{leaf} must not emit a replay command"
            );
            assert!(!down.needs_redraw, "{leaf} click is inert");
            let up = view.on_input(&ViewInput::PointerUp {
                x,
                y,
                button: PointerButton::Left,
            });
            assert!(up.consumed, "{leaf} must consume pointer-up");
            assert!(up.commands.is_empty(), "{leaf} pointer-up is inert");
            assert!(!up.needs_redraw, "{leaf} pointer-up needs no redraw");
            assert_eq!(
                before,
                bound_digest(&view.bind(&ctx(&stage, &snap))),
                "{leaf} click must leave the topbar digest unchanged"
            );
        }
    }

    #[test]
    fn topbar_faction_hover_uses_factual_snapshot_data() {
        let stage = stage_with_topbar(synthetic_topbar_gui(), HashMap::new());
        let snap = snap();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        view.set_drill_targets(topbar_drill_targets_from_roots(&bv.hit.roots));
        let inert_targets = topbar_inert_targets_from_roots(&bv.hit.roots);
        view.set_inert_targets(inert_targets.clone());

        let faction = inert_targets
            .iter()
            .find(|target| target.leaf == "faction_button")
            .expect("synthetic topbar should expose faction button");
        let (x, y) = center(faction.rect);
        let out = view.on_input(&ViewInput::PointerMove { x, y, drag: false });
        assert!(!out.consumed);
        assert!(out.commands.is_empty());
        assert!(out.needs_redraw, "new faction hover should redraw tooltip");

        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(digest.contains("TOOLTIP TT"), "{digest}");
        assert!(digest.contains("Faction"), "{digest}");
        assert!(digest.contains("axis"), "{digest}");
        assert!(digest.contains("Leader"), "{digest}");
        assert!(digest.contains("yes"), "{digest}");
    }

    #[test]
    fn topbar_faction_hover_badges_missing_faction_data() {
        let stage = stage_with_topbar(synthetic_topbar_gui(), HashMap::new());
        let snap = snap_without_faction();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        view.set_drill_targets(topbar_drill_targets_from_roots(&bv.hit.roots));
        let inert_targets = topbar_inert_targets_from_roots(&bv.hit.roots);
        view.set_inert_targets(inert_targets.clone());

        let faction = inert_targets
            .iter()
            .find(|target| target.leaf == "faction_button")
            .expect("synthetic topbar should expose faction button");
        let (x, y) = center(faction.rect);
        let out = view.on_input(&ViewInput::PointerMove { x, y, drag: false });
        assert!(!out.consumed);
        assert!(out.commands.is_empty());
        assert!(out.needs_redraw);

        let bound = view.bind(&ctx(&stage, &snap));
        let tooltip = bound
            .tooltip_plan()
            .expect("faction hover should stay visible and badge missing data");
        assert_eq!(tooltip.plan().unavailable_text_count(), 2);
        let digest = bound_digest(&bound);
        assert!(digest.contains("TOOLTIP TT"), "{digest}");
        assert!(digest.contains("Faction"), "{digest}");
        assert!(digest.contains("Name"), "{digest}");
        assert!(digest.contains("Leader"), "{digest}");
        assert!(digest.contains(":\"missing\""), "{digest}");
        assert!(!digest.contains("axis"), "{digest}");
        assert!(!digest.contains(":\"0\""), "{digest}");

        let out = view.on_input(&ViewInput::PointerMove {
            x: 1200.0,
            y: 800.0,
            drag: false,
        });
        assert!(out.needs_redraw, "leaving the stale hover should redraw");
        assert!(bound_digest(&view.bind(&ctx(&stage, &snap))).contains("TOOLTIP -"));
    }

    #[test]
    fn topbar_faction_hover_badges_null_faction_data() {
        let stage = stage_with_topbar(synthetic_topbar_gui(), HashMap::new());
        let snap = snap_with_null_faction();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        view.set_drill_targets(topbar_drill_targets_from_roots(&bv.hit.roots));
        let inert_targets = topbar_inert_targets_from_roots(&bv.hit.roots);
        view.set_inert_targets(inert_targets.clone());

        let faction = inert_targets
            .iter()
            .find(|target| target.leaf == "faction_button")
            .expect("synthetic topbar should expose faction button");
        let (x, y) = center(faction.rect);
        let out = view.on_input(&ViewInput::PointerMove { x, y, drag: false });
        assert!(!out.consumed);
        assert!(out.commands.is_empty());
        assert!(out.needs_redraw);

        let bound = view.bind(&ctx(&stage, &snap));
        let tooltip = bound
            .tooltip_plan()
            .expect("faction hover should stay visible and badge null data");
        assert_eq!(tooltip.plan().unavailable_text_count(), 2);
        let digest = bound_digest(&bound);
        assert!(digest.contains("Faction"), "{digest}");
        assert!(digest.contains("Name"), "{digest}");
        assert!(digest.contains("Leader"), "{digest}");
        assert!(digest.contains(":\"null\""), "{digest}");
        assert!(!digest.contains("axis"), "{digest}");
        assert!(!digest.contains(":\"0\""), "{digest}");
    }

    #[test]
    fn topbar_threat_hover_uses_factual_threat_ledger_count() {
        let stage = stage_with_topbar(synthetic_topbar_gui(), HashMap::new());
        let snap = snap_with_threat_events();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        let drill_targets = topbar_drill_targets_from_roots(&bv.hit.roots);
        view.set_drill_targets(drill_targets.clone());
        view.set_inert_targets(topbar_inert_targets_from_roots(&bv.hit.roots));

        let threat = drill_targets
            .iter()
            .find(|target| target.leaf == "threat_button")
            .expect("synthetic topbar should expose threat_button");
        let (x, y) = center(threat.rect);
        let out = view.on_input(&ViewInput::PointerMove { x, y, drag: false });
        assert!(!out.consumed);
        assert!(out.commands.is_empty());
        assert!(out.needs_redraw, "new threat hover should redraw tooltip");

        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(digest.contains("TOOLTIP TT"), "{digest}");
        assert!(digest.contains("World tension"), "{digest}");
        assert!(digest.contains("Aggregate"), "{digest}");
        assert!(digest.contains("Threat entries"), "{digest}");
        assert!(digest.contains(":\"2\""), "{digest}");
        assert!(
            !digest.contains(":\"0\""),
            "threat hover must not fabricate a zero aggregate/count: {digest}"
        );
    }

    #[test]
    fn topbar_threat_hover_badges_missing_threat_ledger_unavailable() {
        let stage = stage_with_topbar(synthetic_topbar_gui(), HashMap::new());
        let snap = snap();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        let drill_targets = topbar_drill_targets_from_roots(&bv.hit.roots);
        view.set_drill_targets(drill_targets.clone());
        view.set_inert_targets(topbar_inert_targets_from_roots(&bv.hit.roots));

        let threat = drill_targets
            .iter()
            .find(|target| target.leaf == "threat_value_button")
            .expect("synthetic topbar should expose threat_value_button");
        let (x, y) = center(threat.rect);
        let out = view.on_input(&ViewInput::PointerMove { x, y, drag: false });
        assert!(!out.consumed);
        assert!(out.commands.is_empty());
        assert!(out.needs_redraw);

        let digest = bound_digest(&view.bind(&ctx(&stage, &snap)));
        assert!(digest.contains("World tension"), "{digest}");
        assert!(digest.contains("Aggregate"), "{digest}");
        assert!(digest.contains("Threat entries"), "{digest}");
        assert!(digest.contains(":\"unfed\""), "{digest}");
        assert!(digest.contains(":\"missing\""), "{digest}");
        assert!(
            !digest.contains(":\"0\""),
            "missing threat ledger must not render a fabricated zero: {digest}"
        );
    }

    #[test]
    fn topbar_dismissed_alerts_click_is_inert_without_alert_ledger() {
        let stage = stage_with_topbar(synthetic_topbar_gui(), HashMap::new());
        let snap = snap();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        view.set_drill_targets(topbar_drill_targets_from_roots(&bv.hit.roots));
        let inert_targets = topbar_inert_targets_from_roots(&bv.hit.roots);
        view.set_inert_targets(inert_targets.clone());

        let dismissed = inert_targets
            .iter()
            .find(|target| target.leaf == "dismissed_alerts_button")
            .expect("synthetic topbar should expose dismissed alerts button");
        let before = bound_digest(&view.bind(&ctx(&stage, &snap)));
        let (x, y) = center(dismissed.rect);
        let down = view.on_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(down.consumed);
        assert!(down.commands.is_empty());
        assert!(!down.needs_redraw);
        let up = view.on_input(&ViewInput::PointerUp {
            x,
            y,
            button: PointerButton::Left,
        });
        assert!(up.consumed);
        assert!(up.commands.is_empty());
        assert!(!up.needs_redraw);
        assert_eq!(
            before,
            bound_digest(&view.bind(&ctx(&stage, &snap))),
            "dismissed alerts has no replay ledger, so it must stay an inert no-op"
        );
    }

    #[test]
    fn topbar_alert_threat_faction_targets_fail_closed_when_geometry_absent() {
        let doc = serde_json::json!({
            "view": "topbar",
            "windows": [{
                "type": "containerWindowType",
                "name": "topbar",
                "position": { "x": 0, "y": 0 },
                "size": { "width": "100%%", "height": "100%%" },
                "children": [{
                    "type": "buttonType",
                    "name": "construction_button",
                    "position": { "x": 20, "y": 20 },
                    "size": { "width": 30, "height": 30 },
                    "orientation": "UPPER_LEFT",
                    "sprite": "GFX_test_topbar_button"
                }]
            }]
        });
        let stage = stage_with_topbar(GuiFile::parse(&doc.to_string()).unwrap(), HashMap::new());
        let snap = snap();
        let mut view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        let drill_targets = topbar_drill_targets_from_roots(&bv.hit.roots);
        let inert_targets = topbar_inert_targets_from_roots(&bv.hit.roots);

        for leaf in ["threat_button", "threat_value_button"] {
            assert!(
                drill_targets.iter().all(|target| target.leaf != leaf),
                "{leaf} must not be targetable without real GUI geometry"
            );
        }
        for leaf in ["dismissed_alerts_button", "faction_button"] {
            assert!(
                inert_targets.iter().all(|target| target.leaf != leaf),
                "{leaf} must not be targetable without real GUI geometry"
            );
        }
        view.set_drill_targets(drill_targets);
        view.set_inert_targets(inert_targets);

        let out = view.on_input(&ViewInput::PointerDown {
            x: 900.0,
            y: 80.0,
            button: PointerButton::Left,
        });
        assert!(!out.consumed);
        assert!(out.commands.is_empty());
        assert!(!out.needs_redraw);
    }

    fn gui_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("game_assets")
            .join("gui")
    }

    fn load_real_topbar() -> Option<GuiFile> {
        std::fs::read_to_string(gui_dir().join(TOPBAR_FILE))
            .ok()
            .and_then(|s| GuiFile::parse(&s).ok())
    }

    fn sprite_entry_sized(name: &str, width: u32, height: u32) -> assets::SpriteEntry {
        assets::SpriteEntry {
            name: name.to_string(),
            kind: "spriteType".to_string(),
            no_of_frames: 1,
            source_gfx: None,
            texturefile: None,
            category: None,
            width: Some(width),
            height: Some(height),
            nine_slice: None,
            size: None,
            border_size: None,
        }
    }

    fn topbar_button_sprites() -> HashMap<String, assets::SpriteEntry> {
        [
            "GFX_topbar_decisionview_button",
            "GFX_topbar_intelligence",
            "GFX_topbar_technology",
            "GFX_topbar_diplomacy",
            "GFX_topbar_trade_button",
            "GFX_construction_button",
            "GFX_topbar_production",
            "GFX_deployment_button",
            "GFX_ledger_button",
            "GFX_staff_office_button",
            "GFX_armyoverview_button",
            "GFX_navyoverview_button",
            "GFX_airoverview_button",
            "GFX_world_tension_icon_big_strip",
            "GFX_WT_anim_strip",
            "GFX_topbar_faction_empty",
            "GFX_faction_logo_generic_democratic_miniature",
            "GFX_date_pause_button_bg",
            "GFX_date_pause_button",
            "GFX_zoom_out",
            "GFX_zoom_in",
            "GFX_speed_step",
            "GFX_reopen_lobby",
            "GFX_menu_button",
            "GFX_help_button",
            "GFX_achievements_button",
            "GFX_show_dismissed_alerts_icon",
            "GFX_playlist_button",
            "GFX_music_pause_button",
            "GFX_music_next_button",
            "GFX_musicplayer_button",
        ]
        .into_iter()
        .map(|name| (name.to_string(), sprite_entry_sized(name, 40, 40)))
        .collect()
    }

    #[test]
    fn real_topbar_gui_exposes_registered_drill_targets() {
        let Some(gui) = load_real_topbar() else {
            return;
        };
        let stage = stage_with_topbar(gui, topbar_button_sprites());
        let snap = snap();
        let view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        let found: BTreeSet<&str> = topbar_drill_targets_from_roots(&bv.hit.roots)
            .iter()
            .map(|target| target.leaf)
            .collect();
        let missing: Vec<&str> = TOPBAR_DRILL_TARGETS
            .iter()
            .map(|(leaf, _)| *leaf)
            .filter(|leaf| !found.contains(leaf))
            .collect();
        assert!(
            missing.is_empty(),
            "real extracted topbar.json lost registered drill target rects: {missing:?}"
        );
    }

    #[test]
    fn real_topbar_gui_exposes_registered_inert_controls() {
        let Some(gui) = load_real_topbar() else {
            return;
        };
        let stage = stage_with_topbar(gui, topbar_button_sprites());
        let snap = snap();
        let view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));
        let found: BTreeSet<&str> = topbar_inert_targets_from_roots(&bv.hit.roots)
            .iter()
            .map(|target| target.leaf)
            .collect();
        let missing: Vec<&str> = TOPBAR_INERT_TARGETS
            .iter()
            .copied()
            .filter(|leaf| !found.contains(leaf))
            .collect();
        assert!(
            missing.is_empty(),
            "real extracted topbar.json lost registered inert target rects: {missing:?}"
        );
    }

    #[test]
    fn real_topbar_gui_exposes_alert_threat_faction_slice_rects() {
        let Some(gui) = load_real_topbar() else {
            return;
        };
        let stage = stage_with_topbar(gui, topbar_button_sprites());
        let snap = snap();
        let view = TopbarView::new("GER");
        let bv = view.bind(&ctx(&stage, &snap));

        let drill_found: BTreeSet<&str> = topbar_drill_targets_from_roots(&bv.hit.roots)
            .iter()
            .map(|target| target.leaf)
            .collect();
        for leaf in ["threat_button", "threat_value_button"] {
            assert!(
                drill_found.contains(leaf),
                "real extracted topbar.json must expose {leaf} before it can be routed"
            );
        }

        let inert_found: BTreeSet<&str> = topbar_inert_targets_from_roots(&bv.hit.roots)
            .iter()
            .map(|target| target.leaf)
            .collect();
        assert!(
            inert_found.contains("dismissed_alerts_button"),
            "real extracted topbar.json must expose dismissed_alerts_button before it can be consumed"
        );
        assert!(
            inert_found.contains("faction_button"),
            "real extracted topbar.json must expose faction_button before it can be hovered or consumed"
        );
    }
}
