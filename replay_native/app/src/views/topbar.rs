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

use crate::host::context::BindContext;
use crate::host::draw::FrameDraw;
use crate::host::input::{PointerButton, ViewInput};
use crate::host::registry::{ViewDescriptor, ViewId};
use crate::host::view::{
    BoundView, Drawable, HitModel, HostPolicy, InputOutcome, InstanceId, ViewCommand, ViewInstance,
    ZBand,
};
use crate::host::FidelityReport;
use crate::register_view;

pub const TOPBAR_VIEW_ID: ViewId = ViewId("core.topbar");
const TOPBAR_FILE: &str = "topbar.json";

/// The topbar view instance — stateless across ticks (no scroll/camera). It carries
/// only the observed country tag plus the latest `.gui` button rects synced by the app
/// shell before routing input.
pub struct TopbarView {
    pub country: String,
    drill_targets: Vec<TopbarDrillTarget>,
    inert_targets: Vec<TopbarInertTarget>,
}

impl TopbarView {
    pub fn new(country: impl Into<String>) -> TopbarView {
        TopbarView {
            country: country.into(),
            drill_targets: Vec::new(),
            inert_targets: Vec::new(),
        }
    }

    pub fn set_drill_targets(&mut self, targets: Vec<TopbarDrillTarget>) {
        self.drill_targets = targets;
    }

    pub fn set_inert_targets(&mut self, targets: Vec<TopbarInertTarget>) {
        self.inert_targets = targets;
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

const TOPBAR_DRILL_TARGETS: &[(&str, ViewId)] = &[
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
        let fidelity = topbar_fidelity(&report);

        BoundView {
            instance: InstanceId(0),
            view_id: TOPBAR_VIEW_ID,
            policy: self.policy(),
            // the topbar is a pure full-screen frame: NO block rect — its empty regions
            // fall through to the map; only its real controls capture (via the roots).
            hit: HitModel::from_roots(roots.clone()),
            drawable: Box::new(TopbarDrawable { roots, flag }),
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
                        "ruling_party": "fascism"
                    }
                }
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
            "GFX_date_pause_button_bg",
            "GFX_date_pause_button",
            "GFX_zoom_out",
            "GFX_zoom_in",
            "GFX_speed_step",
            "GFX_reopen_lobby",
            "GFX_menu_button",
            "GFX_help_button",
            "GFX_achievements_button",
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
}
