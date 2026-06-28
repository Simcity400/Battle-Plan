//! `replay_app` — the interactive shell, BATCH-0 host architecture.
//!
//! The map/topbar/state-panel/focus/combat surfaces are now REGISTERED VIEWS driven by
//! the generic window host (`app::host`): the binary seeds the always-on views, then per
//! frame it `rebind_all` → `bind_all` → `compose_offscreen(host, bound, ...)`. There are
//! ZERO per-view draw/route branches here or in `compose.rs`; routing is `ViewCommand`
//! data, hit resolution is the host's, and adding a view never touches this file.
//!
//! What stays here (and only here) is the SHELL glue a headless CI can't drive: the
//! winit/wgpu surface + event loop, the time SCRUBBER transport, the map CAMERA + mode
//! keys, the F/C/ESC overlay toggles, and the map PROVINCE PICK on a host `Miss`.
//!
//! READ-ONLY on assets + the snapshot store; paths via resolver/CLI/env (no personal
//! path in source). `--smoke`/`--shot` run one headless frame through the SAME host
//! compose path so a non-interactive box proves the surface + the architecture.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use app::camera::MapCamera;
use app::combat::CombatShell;
use app::compose::{self, FrameChrome, MenubarChrome};
use app::focus::FocusShell;
use app::host::context::Stage;
use app::host::input::{PointerButton, ViewInput, ViewKey};
use app::host::view::{InstanceId, RebindContext};
use app::host::window_host::HostHit;
use app::host::{BindContext, ViewCommand, ViewId, ViewRegistry, WindowHost};
use app::menubar::{MenuBar, MenuClick};
use app::provenance::Provenance;
use app::scrubber::{ScrubIntent, Scrubber, TrackRect};
use app::views::clusters::chrome_outliner_maps::{
    find_view_button_rects_from_roots, find_view_panel_rect_from_roots,
    find_view_query_rect_from_roots, mapmode_button_targets_from_roots,
    mapmodes_all_button_rect_from_roots, mapmodes_all_grid_rect_from_roots, FindView, MapModesView,
    MinimapView, FIND_VIEW_ID, MAPMODES_VIEW_ID, MINIMAP_VIEW_ID,
};
use app::views::clusters::leaders_officers_traits::{
    trait_tree_viewport_from_roots, ArmyTraitTreeView, NavyTraitTreeView, ARMY_TRAIT_TREE_VIEW_ID,
    NAVY_TRAIT_TREE_VIEW_ID,
};
use app::views::combat::{CombatView, COMBAT_VIEW_ID};
use app::views::focus::{FocusView, FOCUS_VIEW_ID};
use app::views::map::{MapData, MapHover, MapView, MAP_VIEW_ID};
use app::views::state_panel::{StatePanelView, STATE_PANEL_VIEW_ID};
use app::views::topbar::{
    topbar_drill_targets_from_roots, topbar_inert_targets_from_roots, TopbarView, TOPBAR_VIEW_ID,
};

use assets::resolver::GameResolver;
use assets::SpriteManifest;
use gui_runtime::draw::TextureCache;
use gui_runtime::fonts::{FontCache, FontMap};
use gui_runtime::{Loc, Rect};
use render::map::{pick_at, CountryColors, MapIds, PoliticalMap, ProvinceRaster, StateOwners};
use render::modes::MapMode;
use scrub::{Dir, Timeline};
use snapshot::{Snapshot, Store};

mod shot_harness;
mod view;
use view::GpuView;

// ===========================================================================
// CLI / path resolution.
// ===========================================================================
fn arg_value(flag: &str) -> Option<String> {
    let mut it = std::env::args();
    while let Some(a) = it.next() {
        if a == flag {
            return it.next();
        }
        if let Some(v) = a.strip_prefix(&format!("{flag}=")) {
            return Some(v.to_string());
        }
    }
    None
}
fn has_flag(flag: &str) -> bool {
    std::env::args().any(|a| a == flag)
}

fn resolve_shot_view(
    registry: &ViewRegistry,
    requested: Option<&str>,
) -> Result<Option<ViewId>, String> {
    let Some(raw) = requested else {
        return Ok(None);
    };
    if raw.trim().is_empty() {
        return Err("--shot-view requires a registered view id".to_string());
    }
    if raw.trim() != raw {
        return Err(
            "--shot-view requires an exact registered view id without surrounding whitespace"
                .to_string(),
        );
    }
    let id = raw;
    registry
        .iter_views()
        .find(|d| d.id.as_str() == id)
        .map(|d| Some(d.id))
        .ok_or_else(|| format!("unknown --shot-view '{id}' (not a registered view id)"))
}

fn resolve_shot_state(requested: Option<&str>) -> Result<Option<u32>, String> {
    let Some(raw) = requested else {
        return Ok(None);
    };
    let value = raw.trim();
    if value.is_empty() || value != raw {
        return Err("--shot-state requires a positive numeric state id".to_string());
    }
    let state = value
        .parse::<u32>()
        .map_err(|_| format!("invalid --shot-state '{value}' (expected a positive integer)"))?;
    if state == 0 {
        return Err("--shot-state requires a positive numeric state id".to_string());
    }
    Ok(Some(state))
}

fn resolve_map_mode(requested: Option<&str>) -> Result<MapMode, String> {
    let Some(raw) = requested else {
        return Ok(MapMode::Political);
    };
    let value = raw.trim();
    if value.is_empty() || value != raw {
        return Err("--map-mode requires a supported map-mode slug".to_string());
    }
    mode_from_slug(value)
        .ok_or_else(|| format!("invalid --map-mode '{value}' (expected a supported map-mode slug)"))
}

fn reject_shot_all_views_mixed_flags(
    shot_path: Option<&str>,
    shot_view: Option<&str>,
    shot_state: Option<&str>,
    map_mode: Option<&str>,
    smoke: bool,
    shot_focus: bool,
    shot_combat: bool,
) -> Result<(), String> {
    let mut flags = Vec::new();
    if shot_path.is_some() {
        flags.push("--shot");
    }
    if shot_view.is_some() {
        flags.push("--shot-view");
    }
    if shot_state.is_some() {
        flags.push("--shot-state");
    }
    if map_mode.is_some() {
        flags.push("--map-mode");
    }
    if smoke {
        flags.push("--smoke");
    }
    if shot_focus {
        flags.push("--shot-focus");
    }
    if shot_combat {
        flags.push("--shot-combat");
    }
    if flags.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "--shot-all-views cannot be combined with {}",
            flags.join(", ")
        ))
    }
}

fn validate_shot_all_views_batch_flags(
    shot_all_views_dir: Option<&str>,
    run_native: bool,
    command: Option<&str>,
) -> Result<(), String> {
    if run_native && shot_all_views_dir.is_none() {
        return Err("--shot-all-views-run-native requires --shot-all-views=<dir>".to_string());
    }
    if let Some(command) = command {
        if command.trim().is_empty() {
            return Err("--shot-all-views-command requires a command path".to_string());
        }
        if !run_native {
            return Err(
                "--shot-all-views-command requires --shot-all-views-run-native".to_string(),
            );
        }
    }
    Ok(())
}

fn child_path_arg(base: &Path, value: &str) -> String {
    let path = PathBuf::from(value);
    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };
    path.to_string_lossy().into_owned()
}

#[allow(clippy::too_many_arguments)]
fn shot_all_views_child_common_args(
    base: &Path,
    country: &str,
    game: &str,
    vanilla: Option<&str>,
    mod_root: Option<&str>,
    bundle: Option<&str>,
    assets: Option<&str>,
    map: Option<&str>,
    store: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "--country".to_string(),
        country.to_string(),
        "--game".to_string(),
        game.to_string(),
    ];
    for (flag, value) in [
        ("--vanilla", vanilla),
        ("--mod", mod_root),
        ("--bundle", bundle),
        ("--assets", assets),
        ("--map", map),
        ("--store", store),
    ] {
        if let Some(value) = value {
            args.push(flag.to_string());
            args.push(child_path_arg(base, value));
        }
    }
    args
}

fn shot_all_views_command_path(base: &Path, command: Option<&str>) -> Result<PathBuf, String> {
    let path = match command {
        Some(raw) => {
            if raw.trim().is_empty() {
                return Err("--shot-all-views-command requires a command path".to_string());
            }
            PathBuf::from(raw)
        }
        None => std::env::current_exe()
            .map_err(|e| format!("resolve current replay_app executable failed: {e}"))?,
    };
    if path.as_os_str().is_empty() {
        return Err("--shot-all-views-command requires a command path".to_string());
    }
    Ok(if path.is_absolute() {
        path
    } else {
        base.join(path)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use app::host::{
        BoundView, Drawable, FidelityReport, HitModel, HostPolicy, InputOutcome, ViewDescriptor,
        ViewInstance, ZBand,
    };
    use app::views::digest::bound_digest;
    use gui_runtime::GuiFile;
    use std::collections::HashMap;

    const TEST_ALPHA: ViewId = ViewId("test.alpha");
    const TEST_BETA: ViewId = ViewId("test.beta");

    fn unused_open(_arg: Option<i64>) -> Box<dyn ViewInstance> {
        panic!("resolver tests must not instantiate views")
    }

    static ALPHA_DESC: ViewDescriptor = ViewDescriptor {
        id: TEST_ALPHA,
        gui_root: None,
        required_domains: &[],
        route: None,
        shortcut: None,
        open: unused_open,
        policy: HostPolicy::window(ZBand::Panel),
    };

    static BETA_DESC: ViewDescriptor = ViewDescriptor {
        id: TEST_BETA,
        gui_root: None,
        required_domains: &[],
        route: None,
        shortcut: None,
        open: unused_open,
        policy: HostPolicy::window(ZBand::Panel),
    };

    fn registry() -> ViewRegistry {
        let mut registry = ViewRegistry::empty();
        registry.insert_view(&BETA_DESC);
        registry.insert_view(&ALPHA_DESC);
        registry
    }

    fn synthetic_topbar_gui() -> GuiFile {
        let doc = serde_json::json!({
            "view": "topbar",
            "windows": [{
                "type": "containerWindowType",
                "name": "topbar",
                "position": { "x": 0, "y": 0 },
                "size": { "width": "100%%", "height": "100%%" },
                "children": [
                    {
                        "type": "buttonType",
                        "name": "construction_button",
                        "position": { "x": 20, "y": 20 },
                        "size": { "width": 40, "height": 40 },
                        "orientation": "UPPER_LEFT",
                        "sprite": "GFX_test_topbar_button"
                    },
                    {
                        "type": "buttonType",
                        "name": "button_speedup",
                        "position": { "x": 80, "y": 20 },
                        "size": { "width": 40, "height": 40 },
                        "orientation": "UPPER_LEFT",
                        "sprite": "GFX_test_topbar_button"
                    },
                    {
                        "type": "buttonType",
                        "name": "menu_button",
                        "position": { "x": 140, "y": 20 },
                        "size": { "width": 40, "height": 40 },
                        "orientation": "UPPER_LEFT",
                        "sprite": "GFX_test_topbar_button"
                    },
                    {
                        "type": "buttonType",
                        "name": "musicplayer_button",
                        "position": { "x": 200, "y": 20 },
                        "size": { "width": 40, "height": 40 },
                        "orientation": "UPPER_LEFT",
                        "sprite": "GFX_test_topbar_button"
                    }
                ]
            }]
        });
        GuiFile::parse(&doc.to_string()).unwrap()
    }

    fn synthetic_state_panel_gui() -> GuiFile {
        let doc = serde_json::json!({
            "view": "province_state",
            "windows": [{
                "type": "containerWindowType",
                "name": "countrystateview",
                "position": { "x": 0, "y": 0 },
                "size": { "width": 436, "height": 496 },
                "children": [{
                    "type": "buttonType",
                    "name": "close_button",
                    "position": { "x": 390, "y": 8 },
                    "size": { "width": 28, "height": 28 },
                    "orientation": "UPPER_LEFT",
                    "sprite": "GFX_closebutton_small"
                }]
            }]
        });
        GuiFile::parse(&doc.to_string()).unwrap()
    }

    fn stage_with_gui(gui_files: HashMap<String, GuiFile>) -> Stage {
        Stage {
            resolver: assets::resolver::GameResolver::new(None, None),
            manifest: assets::SpriteManifest {
                sprites: HashMap::new(),
            },
            loc: Loc::new(),
            gui_files,
            map_dir: std::path::PathBuf::from("."),
            assets_dir: std::path::PathBuf::from("."),
        }
    }

    fn stage_with_topbar() -> Stage {
        let mut gui_files = HashMap::new();
        gui_files.insert("topbar.json".to_string(), synthetic_topbar_gui());
        stage_with_gui(gui_files)
    }

    fn snap_with_construction() -> Snapshot {
        serde_json::from_value(serde_json::json!({
            "date": "1940.1.1.12",
            "hour_index": 100,
            "countries": {
                "GER": {
                    "politics": {
                        "political_power": 10.0,
                        "stability": 0.5,
                        "ruling_party": "fascism"
                    },
                    "economy": {
                        "factories": { "civ": 12, "civ_from_states": 10 },
                        "resources": {}
                    },
                    "production": {
                        "construction_queue": [{
                            "line_id": "cmd_open_line",
                            "template_id": "arms_factory",
                            "active_factories": 7,
                            "produced": 50.0,
                            "cost": 100.0
                        }]
                    }
                }
            }
        }))
        .unwrap()
    }

    struct NoopDrawable;

    impl Drawable for NoopDrawable {
        fn draw(&self, _fd: &mut app::host::FrameDraw) {}
    }

    struct BlockingPanel {
        rect: Rect,
    }

    impl ViewInstance for BlockingPanel {
        fn view_id(&self) -> ViewId {
            ViewId("test.blocking_panel")
        }

        fn policy(&self) -> HostPolicy {
            HostPolicy::window(ZBand::Panel)
        }

        fn on_input(&mut self, _ev: &ViewInput) -> InputOutcome {
            InputOutcome::ignored()
        }

        fn bind(&self, _ctx: &BindContext) -> BoundView {
            BoundView {
                instance: InstanceId(0),
                view_id: self.view_id(),
                policy: self.policy(),
                hit: HitModel::none().with_block_rect(self.rect),
                drawable: Box::new(NoopDrawable),
                fidelity: FidelityReport::default(),
                commands: Vec::new(),
            }
        }

        app::impl_view_any!();
    }

    fn topbar_construction_center(
        host: &WindowHost,
        stage: &'static Stage,
        snap: &Snapshot,
    ) -> (f32, f32) {
        let bound = bind_host(
            host,
            stage,
            snap,
            "GER",
            (0.0, 0.0),
            MapCamera::new(1600, 900, 5632, 2048),
            1600,
            900,
        );
        let topbar = bound
            .iter()
            .find(|bv| bv.view_id == TOPBAR_VIEW_ID)
            .expect("topbar is bound");
        let target = topbar_drill_targets_from_roots(&topbar.hit.roots)
            .into_iter()
            .find(|target| target.leaf == "construction_button")
            .expect("synthetic topbar exposes construction button");
        (
            target.rect.x + target.rect.w * 0.5,
            target.rect.y + target.rect.h * 0.5,
        )
    }

    fn topbar_inert_center(
        host: &WindowHost,
        stage: &'static Stage,
        snap: &Snapshot,
        leaf: &str,
    ) -> (f32, f32) {
        let bound = bind_host(
            host,
            stage,
            snap,
            "GER",
            (0.0, 0.0),
            MapCamera::new(1600, 900, 5632, 2048),
            1600,
            900,
        );
        let topbar = bound
            .iter()
            .find(|bv| bv.view_id == TOPBAR_VIEW_ID)
            .expect("topbar is bound");
        let target = topbar_inert_targets_from_roots(&topbar.hit.roots)
            .into_iter()
            .find(|target| target.leaf == leaf)
            .unwrap_or_else(|| panic!("synthetic topbar exposes inert target {leaf}"));
        (
            target.rect.x + target.rect.w * 0.5,
            target.rect.y + target.rect.h * 0.5,
        )
    }

    fn topbar_speedup_center(
        host: &WindowHost,
        stage: &'static Stage,
        snap: &Snapshot,
    ) -> (f32, f32) {
        topbar_inert_center(host, stage, snap, "button_speedup")
    }

    fn node_rect(roots: &[gui_runtime::scene::Node], name: &str) -> Option<Rect> {
        fn walk(node: &gui_runtime::scene::Node, name: &str) -> Option<Rect> {
            if node.name == name {
                return Some(node.rect);
            }
            node.children.iter().find_map(|child| walk(child, name))
        }
        roots.iter().find_map(|root| walk(root, name))
    }

    #[test]
    fn shot_view_resolver_returns_none_when_flag_absent() {
        assert_eq!(resolve_shot_view(&registry(), None), Ok(None));
    }

    #[test]
    fn shot_view_resolver_returns_registered_static_id() {
        assert_eq!(
            resolve_shot_view(&registry(), Some("test.alpha")),
            Ok(Some(TEST_ALPHA))
        );
    }

    #[test]
    fn shot_view_resolver_fails_closed_for_unknown_id() {
        let err = resolve_shot_view(&registry(), Some("test.missing")).unwrap_err();
        assert!(err.contains("unknown --shot-view 'test.missing'"));
        assert!(err.contains("not a registered view id"));
    }

    #[test]
    fn shot_view_resolver_fails_closed_for_empty_id() {
        let err = resolve_shot_view(&registry(), Some("  ")).unwrap_err();
        assert_eq!(err, "--shot-view requires a registered view id");
    }

    #[test]
    fn shot_view_resolver_fails_closed_for_padded_id() {
        let err = resolve_shot_view(&registry(), Some(" test.alpha ")).unwrap_err();
        assert!(err.contains("without surrounding whitespace"));
    }

    #[test]
    fn shot_state_resolver_returns_positive_state_id() {
        assert_eq!(resolve_shot_state(Some("64")), Ok(Some(64)));
    }

    #[test]
    fn shot_state_resolver_returns_none_when_absent() {
        assert_eq!(resolve_shot_state(None), Ok(None));
    }

    #[test]
    fn shot_state_resolver_fails_closed_for_invalid_state_id() {
        let err = resolve_shot_state(Some("abc")).unwrap_err();
        assert!(err.contains("invalid --shot-state 'abc'"));
        let err = resolve_shot_state(Some("0")).unwrap_err();
        assert_eq!(err, "--shot-state requires a positive numeric state id");
        let err = resolve_shot_state(Some(" 64 ")).unwrap_err();
        assert_eq!(err, "--shot-state requires a positive numeric state id");
    }

    #[test]
    fn map_mode_resolver_fails_closed_for_unknown_or_padded_slug() {
        assert_eq!(resolve_map_mode(None), Ok(MapMode::Political));
        assert_eq!(resolve_map_mode(Some("control")), Ok(MapMode::Control));

        let err = resolve_map_mode(Some("unknown")).unwrap_err();
        assert!(err.contains("invalid --map-mode 'unknown'"));
        let err = resolve_map_mode(Some(" control ")).unwrap_err();
        assert_eq!(err, "--map-mode requires a supported map-mode slug");
    }

    #[test]
    fn shot_all_views_rejects_ambiguous_shot_flags() {
        let err = reject_shot_all_views_mixed_flags(
            Some("one.png"),
            Some("test.alpha"),
            Some("64"),
            Some("control"),
            true,
            true,
            true,
        )
        .unwrap_err();
        assert!(err.contains("--shot-all-views cannot be combined"));
        assert!(err.contains("--shot"));
        assert!(err.contains("--shot-view"));
        assert!(err.contains("--shot-state"));
        assert!(err.contains("--map-mode"));
        assert!(err.contains("--smoke"));
        assert!(err.contains("--shot-focus"));
        assert!(err.contains("--shot-combat"));
    }

    #[test]
    fn shot_all_views_native_batch_flags_require_manifest_mode() {
        assert!(
            validate_shot_all_views_batch_flags(Some("shots"), true, Some("replay_app")).is_ok()
        );

        let err = validate_shot_all_views_batch_flags(None, true, None).unwrap_err();
        assert_eq!(
            err,
            "--shot-all-views-run-native requires --shot-all-views=<dir>"
        );

        let err = validate_shot_all_views_batch_flags(Some("shots"), false, Some("replay_app"))
            .unwrap_err();
        assert_eq!(
            err,
            "--shot-all-views-command requires --shot-all-views-run-native"
        );
    }

    #[test]
    fn shot_all_views_child_args_forward_runtime_config_without_batch_flags() {
        let base = std::env::current_dir().unwrap();
        let args = shot_all_views_child_common_args(
            &base,
            "USA",
            "mp_campaign",
            Some("vanilla"),
            Some("mod_root"),
            Some("bundle"),
            Some("game_assets"),
            Some("map_assets"),
            Some("snapshots"),
        );

        assert_eq!(
            args,
            vec![
                "--country".to_string(),
                "USA".to_string(),
                "--game".to_string(),
                "mp_campaign".to_string(),
                "--vanilla".to_string(),
                child_path_arg(&base, "vanilla"),
                "--mod".to_string(),
                child_path_arg(&base, "mod_root"),
                "--bundle".to_string(),
                child_path_arg(&base, "bundle"),
                "--assets".to_string(),
                child_path_arg(&base, "game_assets"),
                "--map".to_string(),
                child_path_arg(&base, "map_assets"),
                "--store".to_string(),
                child_path_arg(&base, "snapshots"),
            ]
        );
        assert!(!args.iter().any(|arg| matches!(
            arg.as_str(),
            "--shot-all-views" | "--shot-all-views-run-native" | "--shot-all-views-command"
        )));
    }

    #[test]
    fn map_hover_transport_blocks_active_scrubber_drag_even_off_track() {
        let menu = MenuBar::new();
        let registry = registry();
        let track = TrackRect {
            x: 60.0,
            y: 760.0,
            w: 680.0,
            h: 16.0,
        };
        assert!(
            map_hover_blocked_by_transport(true, track, (300.0, 220.0), &menu, &registry, 800),
            "active scrubber drag owns the pointer even after the cursor leaves the track"
        );
    }

    #[test]
    fn map_hover_transport_blocks_track_and_menubar_but_not_exposed_map() {
        let menu = MenuBar::new();
        let registry = registry();
        let track = TrackRect {
            x: 60.0,
            y: 760.0,
            w: 680.0,
            h: 16.0,
        };
        assert!(map_hover_blocked_by_transport(
            false,
            track,
            (100.0, 765.0),
            &menu,
            &registry,
            800
        ));
        assert!(map_hover_blocked_by_transport(
            false,
            track,
            (12.0, 12.0),
            &menu,
            &registry,
            800
        ));
        assert!(
            !map_hover_blocked_by_transport(false, track, (300.0, 220.0), &menu, &registry, 800),
            "exposed map away from transport chrome can show province hover"
        );
    }

    #[test]
    fn topbar_sync_does_not_arm_targets_under_higher_z_panel() {
        let stage = Box::leak(Box::new(stage_with_topbar()));
        let snap = snap_with_construction();
        let mut host = WindowHost::new();
        host.open_instance(Box::new(TopbarView::new("GER")));
        host.rebind_all(&RebindContext {
            snapshot: &snap,
            country: "GER",
            screen: (1600, 900),
            refit: true,
        });
        let (x, y) = topbar_construction_center(&host, stage, &snap);
        host.open_instance(Box::new(BlockingPanel {
            rect: Rect {
                x: x - 5.0,
                y: y - 5.0,
                w: 10.0,
                h: 10.0,
            },
        }));

        sync_topbar_input_targets(
            &mut host,
            stage,
            &snap,
            "GER",
            (x, y),
            MapCamera::new(1600, 900, 5632, 2048),
            1600,
            900,
        );
        let routed = host.route_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });

        assert!(
            routed.commands.is_empty(),
            "a panel above the chrome must prevent topbar OpenView commands"
        );
        assert!(
            !routed.consumed,
            "the shell's later hit/block pass, not the hidden topbar, should consume this click"
        );
    }

    #[test]
    fn topbar_sync_does_not_arm_inert_time_controls_under_higher_z_panel() {
        let stage = Box::leak(Box::new(stage_with_topbar()));
        let snap = snap_with_construction();
        let mut host = WindowHost::new();
        host.open_instance(Box::new(TopbarView::new("GER")));
        host.rebind_all(&RebindContext {
            snapshot: &snap,
            country: "GER",
            screen: (1600, 900),
            refit: true,
        });
        let (x, y) = topbar_speedup_center(&host, stage, &snap);
        host.open_instance(Box::new(BlockingPanel {
            rect: Rect {
                x: x - 5.0,
                y: y - 5.0,
                w: 10.0,
                h: 10.0,
            },
        }));

        sync_topbar_input_targets(
            &mut host,
            stage,
            &snap,
            "GER",
            (x, y),
            MapCamera::new(1600, 900, 5632, 2048),
            1600,
            900,
        );
        let routed = host.route_input(&ViewInput::PointerDown {
            x,
            y,
            button: PointerButton::Left,
        });

        assert!(
            routed.commands.is_empty(),
            "a panel above the chrome must prevent topbar time-control commands"
        );
        assert!(
            !routed.consumed,
            "the shell's later hit/block pass, not the hidden topbar, should consume this time-control click"
        );
    }

    #[test]
    fn topbar_sync_does_not_arm_inert_chrome_controls_under_higher_z_panel() {
        let stage = Box::leak(Box::new(stage_with_topbar()));
        let snap = snap_with_construction();

        for leaf in ["menu_button", "musicplayer_button"] {
            let mut host = WindowHost::new();
            host.open_instance(Box::new(TopbarView::new("GER")));
            host.rebind_all(&RebindContext {
                snapshot: &snap,
                country: "GER",
                screen: (1600, 900),
                refit: true,
            });
            let (x, y) = topbar_inert_center(&host, stage, &snap, leaf);
            host.open_instance(Box::new(BlockingPanel {
                rect: Rect {
                    x: x - 5.0,
                    y: y - 5.0,
                    w: 10.0,
                    h: 10.0,
                },
            }));

            sync_topbar_input_targets(
                &mut host,
                stage,
                &snap,
                "GER",
                (x, y),
                MapCamera::new(1600, 900, 5632, 2048),
                1600,
                900,
            );
            let routed = host.route_input(&ViewInput::PointerDown {
                x,
                y,
                button: PointerButton::Left,
            });

            assert!(
                routed.commands.is_empty(),
                "{leaf} under a panel must not emit topbar commands"
            );
            assert!(
                !routed.consumed,
                "{leaf} under a panel must not be consumed by the hidden topbar target cache"
            );
        }
    }

    #[test]
    fn state_panel_real_close_leaf_closes_through_shell_capture_path() {
        let mut gui_files = HashMap::new();
        gui_files.insert(
            "province_state.json".to_string(),
            synthetic_state_panel_gui(),
        );
        let stage = Box::leak(Box::new(stage_with_gui(gui_files)));
        let snap = snap_with_construction();
        let mut host = WindowHost::new();
        host.open_instance(Box::new(StatePanelView::new(64, "STATE_64")));
        host.rebind_all(&RebindContext {
            snapshot: &snap,
            country: "GER",
            screen: (1600, 900),
            refit: true,
        });

        let bound = bind_host(
            &host,
            stage,
            &snap,
            "GER",
            (0.0, 0.0),
            MapCamera::new(1600, 900, 5632, 2048),
            1600,
            900,
        );
        let panel = bound
            .iter()
            .find(|bv| bv.view_id == STATE_PANEL_VIEW_ID)
            .expect("state panel bound");
        let close = node_rect(&panel.hit.roots, "close_button")
            .expect("state panel exposes close_button leaf");
        let hit = host.resolve_click(&bound, close.x + close.w * 0.5, close.y + close.h * 0.5);
        let mut needs_redraw = false;
        match hit {
            HostHit::Captured { view_id, leaf, .. } => {
                assert_eq!(view_id, STATE_PANEL_VIEW_ID);
                assert_eq!(leaf.as_deref(), Some("close_button"));
                handle_captured(&mut host, view_id, leaf.as_deref(), &mut needs_redraw);
            }
            other => panic!("state panel close leaf must capture, got {other:?}"),
        }

        assert!(needs_redraw, "closing a panel must request redraw");
        assert!(
            !host.is_open(STATE_PANEL_VIEW_ID),
            "shell close handling must close the captured state panel"
        );
    }

    #[test]
    fn structural_open_commands_rebind_new_view_before_input() {
        let stage = Box::leak(Box::new(stage_with_gui(HashMap::new())));
        let snap = snap_with_construction();
        let registry = ViewRegistry::from_inventory();
        let mut host = WindowHost::new();

        let effects = execute_view_commands(
            &mut host,
            &registry,
            None,
            vec![ViewCommand::OpenView {
                view_id: ViewId("production.construction"),
                arg: None,
            }],
        );
        assert!(effects.opened_or_closed_view);
        rebind_after_structural_commands(&mut host, effects, &snap, "GER", (1600, 900));

        let click = host.route_input(&ViewInput::PointerDown {
            x: 1000.0,
            y: 217.0,
            button: PointerButton::Left,
        });
        assert!(
            click.consumed && click.needs_redraw,
            "the command-opened construction view must have queue keys from rebind before row input"
        );

        let bound = bind_host(
            &host,
            stage,
            &snap,
            "GER",
            (1000.0, 217.0),
            MapCamera::new(1600, 900, 5632, 2048),
            1600,
            900,
        );
        let construction = bound
            .iter()
            .find(|bv| bv.view_id == ViewId("production.construction"))
            .expect("construction view opened");
        let digest = bound_digest(construction);
        assert!(
            digest.contains("selected_key=construction:line:cmd_open_line"),
            "row input after command-open must select the measured construction line, got: {digest}"
        );
    }
}

fn mode_from_slug(slug: &str) -> Option<MapMode> {
    use render::modes::BuildingKind::*;
    let m = match slug {
        "political" => MapMode::Political,
        "control" => MapMode::Control,
        "resistance" => MapMode::Resistance,
        "compliance" => MapMode::Compliance,
        "victory_points" => MapMode::VictoryPoints,
        "resources" => MapMode::Resources,
        "weather" => MapMode::Weather,
        "front_lines" => MapMode::FrontLines,
        "diplomacy" => MapMode::Diplomacy,
        "intel" => MapMode::Intel,
        "air" => MapMode::Air,
        "naval" => MapMode::Naval,
        "supply" => MapMode::Supply,
        "buildings_infrastructure" => MapMode::Buildings(Infrastructure),
        "buildings_industrial_complex" => MapMode::Buildings(CivilianFactory),
        "buildings_arms_factory" => MapMode::Buildings(MilitaryFactory),
        "buildings_dockyard" => MapMode::Buildings(Dockyard),
        "buildings_air_base" => MapMode::Buildings(AirBase),
        "buildings_anti_air_building" => MapMode::Buildings(AntiAir),
        _ => return None,
    };
    Some(m)
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn find_assets_dir() -> Option<PathBuf> {
    if let Some(a) = arg_value("--assets").or_else(|| std::env::var("REPLAY_ASSETS").ok()) {
        return Some(PathBuf::from(a));
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let cand = dir.join("game_assets");
        if cand.join("gui").join("topbar.json").is_file() {
            return Some(cand);
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn loc_dirs(resolver: &GameResolver) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(m) = &resolver.mod_root {
        v.push(m.join("localisation").join("english"));
        v.push(m.join("localisation").join("replace").join("english"));
    }
    v.push(resolver.vanilla.join("localisation").join("english"));
    v
}

fn main() {
    let country = arg_value("--country").unwrap_or_else(|| "GER".to_string());
    let game = arg_value("--game").unwrap_or_else(|| "campaign".to_string());
    let vanilla = arg_value("--vanilla");
    let mod_root = arg_value("--mod");
    let bundle_arg = arg_value("--bundle");
    let smoke = has_flag("--smoke");
    let shot_path = arg_value("--shot");
    let shot_view_arg = arg_value("--shot-view");
    let shot_all_views_dir = arg_value("--shot-all-views");
    let shot_all_views_run_native = has_flag("--shot-all-views-run-native");
    let shot_all_views_command_arg = arg_value("--shot-all-views-command");
    let shot_state_arg = arg_value("--shot-state");
    let map_mode_arg = arg_value("--map-mode");
    let shot_state: Option<u32> =
        resolve_shot_state(shot_state_arg.as_deref()).unwrap_or_else(|e| {
            eprintln!("[app] SHOT BLOCKER: {e}");
            std::process::exit(2);
        });
    let shot_focus = has_flag("--shot-focus");
    let shot_combat = has_flag("--shot-combat");

    // The registry is pure inventory data, so the manifest-only harness can fail closed
    // before game assets, snapshots, or GPU setup are required.
    let registry = ViewRegistry::from_inventory();
    validate_shot_all_views_batch_flags(
        shot_all_views_dir.as_deref(),
        shot_all_views_run_native,
        shot_all_views_command_arg.as_deref(),
    )
    .unwrap_or_else(|e| {
        eprintln!("[app] SHOT BLOCKER: {e}");
        std::process::exit(2);
    });
    if let Some(dir) = shot_all_views_dir.as_deref() {
        reject_shot_all_views_mixed_flags(
            shot_path.as_deref(),
            shot_view_arg.as_deref(),
            shot_state_arg.as_deref(),
            map_mode_arg.as_deref(),
            smoke,
            shot_focus,
            shot_combat,
        )
        .unwrap_or_else(|e| {
            eprintln!("[app] SHOT BLOCKER: {e}");
            std::process::exit(2);
        });
        match shot_harness::write_registry_shot_manifest(&registry, Path::new(dir)) {
            Ok(write) => {
                if shot_all_views_run_native {
                    let base = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                    let command =
                        shot_all_views_command_path(&base, shot_all_views_command_arg.as_deref())
                            .unwrap_or_else(|e| {
                                eprintln!("[app] SHOT BLOCKER: {e}");
                                std::process::exit(2);
                            });
                    let common_args = shot_all_views_child_common_args(
                        &base,
                        &country,
                        &game,
                        vanilla.as_deref(),
                        mod_root.as_deref(),
                        bundle_arg.as_deref(),
                        arg_value("--assets").as_deref(),
                        arg_value("--map").as_deref(),
                        arg_value("--store").as_deref(),
                    );
                    let config = shot_harness::NativeShotBatchConfig {
                        command,
                        common_args,
                    };
                    let run =
                        shot_harness::run_native_shot_batch_from_manifest(&write.path, &config)
                            .unwrap_or_else(|e| {
                                eprintln!("[app] SHOT BLOCKER: {e}");
                                std::process::exit(3);
                            });
                    eprintln!(
                        "[app] SHOT: wrote {} native screenshots from {} (the game references and diffs pending)",
                        run.captured_count,
                        run.manifest_path.display()
                    );
                    println!(
                        "OK shot-all-views manifest={} native_captured={} hoi4_reference_pending={} diff_pending={}",
                        run.manifest_path.display(),
                        run.captured_count,
                        run.pending_hoi4_reference_count,
                        run.pending_diff_count
                    );
                } else {
                    eprintln!(
                        "[app] SHOT: wrote registry-derived manifest {} ({} native targets planned; run --shot-all-views-run-native to emit native shots)",
                        write.path.display(),
                        write.target_count
                    );
                    println!(
                        "OK shot-all-views manifest={} targets={} status=native_targets_planned",
                        write.path.display(),
                        write.target_count
                    );
                }
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("[app] SHOT BLOCKER: {e}");
                std::process::exit(2);
            }
        }
    }

    let shot_view = resolve_shot_view(&registry, shot_view_arg.as_deref()).unwrap_or_else(|e| {
        eprintln!("[app] SHOT BLOCKER: {e}");
        std::process::exit(2);
    });
    if shot_view == Some(STATE_PANEL_VIEW_ID) && shot_state.is_none() {
        eprintln!(
            "[app] SHOT BLOCKER: --shot-view={} requires --shot-state=<positive state id>",
            STATE_PANEL_VIEW_ID
        );
        std::process::exit(2);
    }
    let requested_map_mode = resolve_map_mode(map_mode_arg.as_deref()).unwrap_or_else(|e| {
        eprintln!("[app] SHOT BLOCKER: {e}");
        std::process::exit(2);
    });

    let resolver = GameResolver::new(vanilla.as_deref(), mod_root.as_deref());

    let map_dir = arg_value("--map")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("map_pipeline").join("assets"));
    let store_root = arg_value("--store")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("replay").join("snapshots"));

    eprintln!("[app] country={country} game={game}");
    eprintln!("[app] vanilla = {}", resolver.vanilla.display());
    if !resolver.vanilla_exists() {
        eprintln!(
            "[app] BLOCKER: game install not found. Pass --vanilla <path> or set BATTLE_PLAN_VANILLA."
        );
        std::process::exit(2);
    }
    let assets_dir = find_assets_dir().unwrap_or_else(|| {
        eprintln!("[app] BLOCKER: game_assets/gui/topbar.json not found (pass --assets).");
        std::process::exit(2);
    });
    eprintln!("[app] assets = {}", assets_dir.display());
    eprintln!("[app] map    = {}", map_dir.display());
    eprintln!("[app] store  = {} (game {game})", store_root.display());

    // -- the snapshot timeline --
    let store = Store::open(&store_root);
    let mut timeline = Timeline::open(store, game.clone(), 6).unwrap_or_else(|e| {
        eprintln!("[app] BLOCKER: open timeline for game {game}: {e}");
        std::process::exit(2);
    });
    if timeline.is_empty() {
        eprintln!("[app] BLOCKER: game {game} has no snapshots.");
        std::process::exit(2);
    }
    eprintln!(
        "[app] timeline: {} snapshots: {:?}",
        timeline.len(),
        timeline.hours()
    );

    let first_frame = timeline.seek_to(0).unwrap_or_else(|e| {
        eprintln!("[app] BLOCKER: seek first snapshot: {e}");
        std::process::exit(2);
    });
    let provenance = Provenance::resolve(bundle_arg.as_deref(), &assets_dir, &first_frame.snapshot)
        .unwrap_or_else(|e| {
            eprintln!("[app] BLOCKER: resolve bundle/provenance: {e}");
            std::process::exit(2);
        });
    eprintln!("[app] {}", provenance.banner());
    if let Some(w) = provenance.warning() {
        eprintln!("[app] WARNING: {w}");
    }

    // -- the static stage (loaded once) --
    let manifest =
        SpriteManifest::load(&assets_dir.join("sprites.json")).expect("BLOCKER: load sprites.json");
    let mut loc = Loc::new();
    for base in loc_dirs(&resolver) {
        let _ = loc.add_dir(&base);
    }
    // the view registry (built once from the inventory of all registered views). Built
    // HERE — before the Stage is leaked — so the binary can pre-load EXACTLY the parsed
    // `.gui` set every registered view needs into `Stage.gui_files` (the BATCH-1 .gui-set
    // loader). A cluster then fetches its real game layout via `BindContext::gui_symbol`
    // and never loads a `.gui` itself.
    eprintln!(
        "[app] registry: {} views, {} templates",
        registry.len(),
        registry.template_count()
    );

    // pre-parse the `.gui` SET: every file any registered view ROOT/TEMPLATE references,
    // PLUS the two migrated-surface files (their views use a custom root, so they are not
    // in `required_gui_files`). Deduped; a file that fails to parse is skipped + warned.
    let gui_dir = assets_dir.join("gui");
    let mut required: Vec<&str> = registry.required_gui_files();
    for f in ["topbar.json", "province_state.json"] {
        if !required.contains(&f) {
            required.push(f);
        }
    }
    let (gui_files, gui_failed) = Stage::load_gui_set(&gui_dir, &required);
    eprintln!(
        "[app] stage: loc keys={} sprites={} gui_files={}/{} ({} failed)",
        loc.len(),
        manifest.sprites.len(),
        gui_files.len(),
        required.len(),
        gui_failed.len()
    );
    let _ = gui_failed;

    // -- map data --
    let raster = ProvinceRaster::load_png(&map_dir.join("province_ids.png"))
        .expect("BLOCKER: load province_ids.png");
    let ids = MapIds::load(&map_dir).expect("BLOCKER: MapIds::load");
    let colors = CountryColors::load(&map_dir).expect("BLOCKER: CountryColors::load");
    eprintln!(
        "[app] map raster {}x{}  land provinces {}",
        raster.width,
        raster.height,
        ids.province_count()
    );

    // -- window + surface --
    let event_loop = winit::event_loop::EventLoop::new().expect("BLOCKER: create event loop");
    let init_w = 1600u32;
    let init_h = 900u32;
    let window = Arc::new(
        winit::window::WindowBuilder::new()
            .with_title("Battle Plan — interactive shell (host architecture)")
            .with_inner_size(winit::dpi::PhysicalSize::new(init_w, init_h))
            .build(&event_loop)
            .expect("BLOCKER: build window"),
    );
    let mut gpu = GpuView::new(window.clone(), init_w, init_h).unwrap_or_else(|e| {
        eprintln!("[app] BLOCKER: GPU/surface init failed: {e}");
        std::process::exit(2);
    });
    eprintln!(
        "[app] wgpu surface format = {:?}  size = {:?}",
        gpu.surface_format(),
        gpu.size()
    );

    // the GPU map pass over the shared device, then leak the static singletons (once
    // per process) so the host + caches borrow them for the 'static event loop.
    let pmap = PoliticalMap::new(&gpu.renderer, &raster);
    // The province→strategic-region join + sea-province set for the FROZEN-SEAS dynamic branch
    // (mod over vanilla, parsed once). On failure we run WITHOUT frozen seas (snow/mud still
    // work) rather than fabricate — log the reason.
    let strategic_regions = match render::strategic_regions::StrategicRegions::load(&resolver) {
        Ok(sr) => {
            eprintln!(
                "[app] strategic regions: {} provinces, {} regions, {} sea provinces",
                sr.province_count(),
                sr.region_count(),
                sr.sea_count()
            );
            Some(sr)
        }
        Err(e) => {
            eprintln!("[app] WARNING: frozen-seas join unavailable ({e}); snow/mud still active");
            None
        }
    };
    let map_data: &'static MapData = Box::leak(Box::new(MapData {
        raster,
        ids,
        colors,
        pmap,
        strategic_regions,
    }));

    let stage: &'static Stage = Box::leak(Box::new(Stage {
        resolver: GameResolver::new(
            Some(resolver.vanilla.to_str().unwrap_or("")),
            resolver.mod_root.as_deref().and_then(|p| p.to_str()),
        ),
        manifest,
        loc,
        gui_files,
        map_dir: map_dir.clone(),
        assets_dir: assets_dir.clone(),
    }));

    let font_map: &'static FontMap = Box::leak(Box::new(FontMap::from_core_gfx(&stage.resolver)));
    let mut fonts = FontCache::new(font_map, &stage.resolver);
    let mut textures = TextureCache::new(&stage.manifest, &stage.resolver);

    // the scrubber transport.
    let mut scrubber = Scrubber::new(timeline.len(), 0.45);
    fn scrubber_track(w: u32, h: u32) -> TrackRect {
        TrackRect {
            x: 60.0,
            y: h as f32 - 40.0,
            w: w as f32 - 120.0,
            h: 16.0,
        }
    }
    scrubber.set_track(scrubber_track(init_w, init_h));

    // the active map mode (the camera + mode live in the seeded MapView instance).
    let init_mode: MapMode = requested_map_mode;
    let init_camera = MapCamera::new(
        init_w,
        init_h,
        map_data.raster.width,
        map_data.raster.height,
    );

    // the focus + combat shells (loaded once; their views are seeded on toggle).
    let focus_white: &'static render::Texture =
        Box::leak(Box::new(focus_tree::view::white_texture(&gpu.renderer)));
    let combat_white: &'static render::Texture =
        Box::leak(Box::new(combat_panel::view::white_texture(&gpu.renderer)));
    // the BATCH-1 MENUBAR (host chrome): a leaked white pixel it tints + its state.
    let menubar_white: &'static render::Texture =
        Box::leak(Box::new(widgets::common::white_texture(&gpu.renderer)));
    let mut menubar = MenuBar::new();
    let combat_centroids = combat_panel::bubbles::ProvinceCentroids::from_raster(&map_data.raster);
    let combat_names = combat_panel::ProvinceNames::load(&stage.map_dir, &stage.loc);

    // the current snapshot.
    let mut current: Arc<Snapshot> = first_frame.snapshot.clone();

    // -- seed the host: the always-on map + topbar views --
    let mut host = WindowHost::new();
    host.open_instance(Box::new(MapView::new(init_camera, init_mode, map_data)));
    host.open_instance(Box::new(TopbarView::new(country.clone())));
    // bind once so the first frame is correct.
    host.rebind_all(&RebindContext {
        snapshot: &current,
        country: &country,
        screen: gpu.size(),
        refit: true,
    });

    // interaction state owned by the shell (transport + map controls).
    let mut cursor = (0.0f32, 0.0f32);
    let mut last_tick = Instant::now();
    let mut left_down = false;
    let mut right_down = false;
    let mut map_drag: Option<(f32, f32)> = None; // right-drag pan
    let mut combat_active = false;
    let mut needs_redraw = true;

    // ----------------------------------------------------------------------
    // SHOT / SMOKE: one headless frame through the SAME host compose path.
    // ----------------------------------------------------------------------
    if shot_path.is_some() || smoke {
        let mut seed_focus = shot_focus;
        let mut seed_combat = shot_combat;
        if let Some(vid) = shot_view {
            if host.is_open(vid) {
                eprintln!("[app] SHOT: view {vid} already open; using existing host instance");
            } else if vid == FOCUS_VIEW_ID {
                seed_focus = true;
            } else if vid == COMBAT_VIEW_ID {
                seed_combat = true;
            } else if vid == STATE_PANEL_VIEW_ID {
                open_state_panel(
                    &mut host,
                    stage,
                    &current,
                    shot_state.expect("state-panel shot state was validated"),
                );
            } else if host.open_view(&registry, vid, None).is_some() {
                eprintln!("[app] SHOT: opened registered view {vid}");
            } else {
                eprintln!("[app] SHOT BLOCKER: registered view {vid} could not be opened");
                std::process::exit(2);
            }
        }
        if let Some(sid) = shot_state {
            if shot_view != Some(STATE_PANEL_VIEW_ID) {
                open_state_panel(&mut host, stage, &current, sid);
            }
        }
        if seed_focus {
            let shell = FocusShell::load(&stage.resolver);
            host.open_instance(Box::new(FocusView::new(
                shell,
                focus_white,
                &country,
                &current,
                gpu.size(),
            )));
        }
        if seed_combat {
            let mut shell = CombatShell::new(Some(combat_centroids.clone()), combat_names.clone());
            shell.rebind(&current);
            shell.open_first();
            host.open_instance(Box::new(CombatView::new(
                shell,
                combat_white,
                current_camera(&host),
                gpu.size(),
            )));
        }
        host.rebind_all(&RebindContext {
            snapshot: &current,
            country: &country,
            screen: gpu.size(),
            refit: true,
        });

        let (w, h) = gpu.size();
        let bound = bind_host(
            &host,
            stage,
            &current,
            &country,
            cursor,
            current_camera(&host),
            w,
            h,
        );
        let chrome = FrameChrome {
            scrubber_track: Some(rect_of(scrubber.track())),
            scrubber_thumb_x: scrubber.thumb_x(),
            menubar: Some(build_menubar_chrome(
                &menubar,
                &registry,
                menubar_white,
                cursor,
                w,
            )),
        };
        compose::compose_offscreen(
            &mut gpu.renderer,
            stage,
            &mut fonts,
            &mut textures,
            &host,
            &bound,
            &chrome,
            w,
            h,
        );

        if let Some(path) = shot_path {
            let pixels = gpu.renderer.readback_public();
            match render::save_png(Path::new(&path), w, h, &pixels) {
                Ok(()) => {
                    let drawn = pixels
                        .chunks_exact(4)
                        .filter(|p| !(p[0] < 20 && p[1] < 24 && p[2] < 28))
                        .count();
                    gpu.wait_idle();
                    eprintln!("[app] SHOT: wrote {path} ({drawn} non-bg px)");
                    println!("OK shot {path} size={w}x{h} drawn={drawn}");
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("[app] SHOT BLOCKER: write {path} failed: {e}");
                    std::process::exit(3);
                }
            }
        } else {
            match gpu.present() {
                Ok(()) => {
                    gpu.wait_idle();
                    eprintln!(
                        "[app] SMOKE: rendered + presented one frame OK (surface path proven)."
                    );
                    println!(
                        "OK smoke surface_format={:?} size={:?}",
                        gpu.surface_format(),
                        gpu.size()
                    );
                    std::process::exit(0);
                }
                Err(e) => {
                    eprintln!("[app] SMOKE BLOCKER: surface present failed: {e}");
                    std::process::exit(3);
                }
            }
        }
    }

    eprintln!("[app] controls: drag scrubber (bottom) | scroll=zoom | right-drag=pan | left-click=select state | F=focus tree | C=combat overlay | SPACE=play/pause | ←/→=step | M or ]/[ = map mode | ESC=close overlay/panel/quit");

    // ----------------------------------------------------------------------
    // The interactive event loop.
    // ----------------------------------------------------------------------
    use winit::event::{ElementState, Event, MouseButton, MouseScrollDelta, WindowEvent};
    use winit::keyboard::{KeyCode, PhysicalKey};

    let _ = event_loop.run(move |event, elwt| {
        elwt.set_control_flow(winit::event_loop::ControlFlow::Poll);

        macro_rules! refresh_hover {
            () => {{
                let (w, h) = gpu.size();
                refresh_map_hover(
                    &mut host,
                    stage,
                    &current,
                    &country,
                    map_data,
                    &menubar,
                    &registry,
                    cursor,
                    scrubber.is_dragging(),
                    scrubber.track(),
                    w,
                    h,
                )
            }};
        }

        // Apply a time change: seek + rebind EVERY open registered window (deliverable 7).
        macro_rules! apply_seek {
            ($pos:expr) => {{
                if let Ok(f) = timeline.seek_to($pos) {
                    current = f.snapshot.clone();
                    scrubber.sync_pos(timeline.pos());
                    let screen = gpu.size();
                    host.rebind_all(&RebindContext {
                        snapshot: &current,
                        country: &country,
                        screen,
                        refit: false,
                    });
                    let _ = refresh_hover!();
                    needs_redraw = true;
                }
            }};
        }

        macro_rules! cycle_mode {
            ($dir:ident) => {{
                if let Some(mv) = map_view_mut(&mut host) {
                    mv.mode = mv.mode.$dir();
                    eprintln!("[app] map mode -> {}", mv.mode.label());
                }
                sync_mapmodes_active_mode(&mut host);
                needs_redraw = true;
            }};
        }

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => elwt.exit(),

                WindowEvent::Resized(size) => {
                    gpu.resize(size.width, size.height);
                    if let Some(mv) = map_view_mut(&mut host) {
                        mv.camera.set_viewport(size.width, size.height);
                    }
                    sync_combat_camera(&mut host);
                    scrubber.set_track(scrubber_track(size.width, size.height));
                    host.rebind_all(&RebindContext {
                        snapshot: &current,
                        country: &country,
                        screen: (size.width, size.height),
                        refit: false,
                    });
                    let _ = refresh_hover!();
                    needs_redraw = true;
                }

                WindowEvent::CursorMoved { position, .. } => {
                    cursor = (position.x as f32, position.y as f32);
                    if scrubber.is_dragging() {
                        if let ScrubIntent::Seek(idx) = scrubber.drag_to(cursor.0) {
                            apply_seek!(idx);
                        }
                    }
                    // route a move (with drag flag) to the host overlays first.
                    let drag = left_down;
                    sync_combat_camera(&mut host);
                    if host.is_open(MAPMODES_VIEW_ID) {
                        let (w, h) = gpu.size();
                        let map_camera = current_camera(&host);
                        sync_mapmodes_input_panel(
                            &mut host, stage, &current, &country, cursor, map_camera, w, h,
                        );
                    }
                    let r = host.route_input(&ViewInput::PointerMove {
                        x: cursor.0,
                        y: cursor.1,
                        drag,
                    });
                    let emitter = r.emitter;
                    if !r.commands.is_empty() {
                        let effects =
                            execute_view_commands(&mut host, &registry, emitter, r.commands);
                        rebind_after_structural_commands(
                            &mut host,
                            effects,
                            &current,
                            &country,
                            gpu.size(),
                        );
                        if effects.changed {
                            needs_redraw = true;
                        }
                    }
                    if r.needs_redraw {
                        needs_redraw = true;
                    }
                    if !r.consumed {
                        if let Some((ax, ay)) = map_drag {
                            if let Some(mv) = map_view_mut(&mut host) {
                                mv.camera.pan_by_pixels(cursor.0 - ax, cursor.1 - ay);
                            }
                            sync_combat_camera(&mut host);
                            map_drag = Some(cursor);
                            needs_redraw = true;
                        }
                    }
                    if refresh_hover!() {
                        needs_redraw = true;
                    }
                }

                WindowEvent::MouseWheel { delta, .. } => {
                    let dy = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => (p.y as f32) / 40.0,
                    };
                    sync_combat_camera(&mut host);
                    let (w, h) = gpu.size();
                    let map_camera = current_camera(&host);
                    sync_find_input_panel(
                        &mut host, stage, &current, &country, cursor, map_camera, w, h,
                    );
                    let r = host.route_input(&ViewInput::Wheel {
                        x: cursor.0,
                        y: cursor.1,
                        dy,
                    });
                    if !r.consumed {
                        if let Some(mv) = map_view_mut(&mut host) {
                            mv.camera.zoom_at(dy, cursor.0, cursor.1, 16.0);
                        }
                        sync_combat_camera(&mut host);
                    }
                    let _ = refresh_hover!();
                    needs_redraw = true;
                }

                WindowEvent::MouseInput { state, button, .. } => match (state, button) {
                    (ElementState::Pressed, MouseButton::Left) => {
                        left_down = true;
                        // 0) the MENUBAR (top chrome) is checked FIRST — it sits above
                        //    every view + the map, and routes a click → OpenView.
                        let menu_layout = menubar.layout(&registry, gpu.size().0 as f32);
                        let menu_click = menubar.pick(&menu_layout, cursor.0, cursor.1);
                        if menu_click != MenuClick::Miss {
                            blur_find_query(&mut host, &mut needs_redraw);
                            if let Some(vid) = menubar.apply(&menu_click) {
                                if host.open_view(&registry, vid, None).is_some() {
                                    host.rebind_all(&RebindContext {
                                        snapshot: &current,
                                        country: &country,
                                        screen: gpu.size(),
                                        refit: true,
                                    });
                                }
                            }
                            needs_redraw = true;
                        }
                        // 1) scrubber owns the bottom strip.
                        else if let ScrubIntent::Seek(idx) = scrubber.press(cursor.0, cursor.1) {
                            blur_find_query(&mut host, &mut needs_redraw);
                            apply_seek!(idx);
                        } else {
                            // 2) route the click to the host (overlays + windows).
                            sync_combat_camera(&mut host);
                            let (w, h) = gpu.size();
                            let map_camera = current_camera(&host);
                            sync_minimap_input_panel(
                                &mut host, stage, &current, &country, cursor, map_camera, w, h,
                            );
                            sync_topbar_input_targets(
                                &mut host, stage, &current, &country, cursor, map_camera, w, h,
                            );
                            sync_find_input_panel(
                                &mut host, stage, &current, &country, cursor, map_camera, w, h,
                            );
                            sync_mapmodes_input_panel(
                                &mut host, stage, &current, &country, cursor, map_camera, w, h,
                            );
                            sync_trait_tree_input_panels(
                                &mut host, stage, &current, &country, cursor, map_camera, w, h,
                            );
                            let r = host.route_input(&ViewInput::PointerDown {
                                x: cursor.0,
                                y: cursor.1,
                                button: PointerButton::Left,
                            });
                            // run any commands the views emitted (open/close).
                            let emitter = r.emitter;
                            if !r.commands.is_empty() {
                                let effects = execute_view_commands(
                                    &mut host, &registry, emitter, r.commands,
                                );
                                rebind_after_structural_commands(
                                    &mut host,
                                    effects,
                                    &current,
                                    &country,
                                    gpu.size(),
                                );
                                if effects.changed {
                                    needs_redraw = true;
                                }
                            }
                            if r.needs_redraw {
                                needs_redraw = true;
                            }
                            if !r.consumed {
                                // 3) GUI hit-test (front-to-back) over the bound windows.
                                let (w, h) = gpu.size();
                                let bound = bind_host(
                                    &host,
                                    stage,
                                    &current,
                                    &country,
                                    cursor,
                                    current_camera(&host),
                                    w,
                                    h,
                                );
                                match host.resolve_click(&bound, cursor.0, cursor.1) {
                                    HostHit::Captured { view_id, leaf, .. } => {
                                        handle_captured(
                                            &mut host,
                                            view_id,
                                            leaf.as_deref(),
                                            &mut needs_redraw,
                                        );
                                    }
                                    HostHit::Blocked { .. } => { /* consumed; no map pick */ }
                                    HostHit::Miss => {
                                        // 4) MAP pick (the bottom layer).
                                        if let Some(sid) = map_pick(&host, map_data, cursor) {
                                            open_state_panel(&mut host, stage, &current, sid);
                                            needs_redraw = true;
                                        }
                                    }
                                }
                            }
                        }
                        if refresh_hover!() {
                            needs_redraw = true;
                        }
                    }
                    (ElementState::Released, MouseButton::Left) => {
                        left_down = false;
                        scrubber.release();
                        sync_combat_camera(&mut host);
                        let r = host.route_input(&ViewInput::PointerUp {
                            x: cursor.0,
                            y: cursor.1,
                            button: PointerButton::Left,
                        });
                        let emitter = r.emitter;
                        let effects =
                            execute_view_commands(&mut host, &registry, emitter, r.commands);
                        rebind_after_structural_commands(
                            &mut host,
                            effects,
                            &current,
                            &country,
                            gpu.size(),
                        );
                        if r.needs_redraw || effects.changed {
                            needs_redraw = true;
                        }
                        if refresh_hover!() {
                            needs_redraw = true;
                        }
                    }
                    (ElementState::Pressed, MouseButton::Right) => {
                        right_down = true;
                        map_drag = Some(cursor);
                        if refresh_hover!() {
                            needs_redraw = true;
                        }
                    }
                    (ElementState::Released, MouseButton::Right) => {
                        right_down = false;
                        map_drag = None;
                        if refresh_hover!() {
                            needs_redraw = true;
                        }
                    }
                    _ => {}
                },

                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state == ElementState::Pressed {
                        let (w, h) = gpu.size();
                        let map_camera = current_camera(&host);
                        sync_find_input_panel(
                            &mut host, stage, &current, &country, cursor, map_camera, w, h,
                        );

                        let mut key_consumed = false;
                        match event.physical_key {
                            PhysicalKey::Code(KeyCode::Backspace) => {
                                key_consumed = route_view_key(
                                    &mut host,
                                    &registry,
                                    ViewKey::Backspace,
                                    &current,
                                    &country,
                                    gpu.size(),
                                    &mut needs_redraw,
                                );
                            }
                            PhysicalKey::Code(KeyCode::Enter) => {
                                key_consumed = route_view_key(
                                    &mut host,
                                    &registry,
                                    ViewKey::Enter,
                                    &current,
                                    &country,
                                    gpu.size(),
                                    &mut needs_redraw,
                                );
                            }
                            _ => {}
                        }
                        if let Some(text) = event.text.as_ref() {
                            for ch in text.chars().filter(|ch| !ch.is_control()) {
                                key_consumed |= route_view_key(
                                    &mut host,
                                    &registry,
                                    ViewKey::Char(ch),
                                    &current,
                                    &country,
                                    gpu.size(),
                                    &mut needs_redraw,
                                );
                            }
                        }

                        if !key_consumed {
                            match event.physical_key {
                                PhysicalKey::Code(KeyCode::Space) => {
                                    scrubber.toggle_play();
                                    last_tick = Instant::now();
                                }
                                PhysicalKey::Code(KeyCode::Escape) => {
                                    // ESC: close the front-most closeable overlay/panel, else quit.
                                    if let Some(vid) = host.front_closeable() {
                                        let (w, h) = gpu.size();
                                        let emitter = front_instance_id(
                                            &host,
                                            vid,
                                            stage,
                                            &current,
                                            &country,
                                            cursor,
                                            current_camera(&host),
                                            w,
                                            h,
                                        );
                                        let (consumed, route_redraw, commands) =
                                            if let Some(view) = host.instance_mut(vid) {
                                                let out = view.on_input(&ViewInput::Key {
                                                    key: ViewKey::Escape,
                                                });
                                                (out.consumed, out.needs_redraw, out.commands)
                                            } else {
                                                (false, false, Vec::new())
                                            };
                                        let effects = execute_view_commands(
                                            &mut host, &registry, emitter, commands,
                                        );
                                        rebind_after_structural_commands(
                                            &mut host,
                                            effects,
                                            &current,
                                            &country,
                                            gpu.size(),
                                        );
                                        if !consumed {
                                            host.close_view(vid);
                                        }
                                        if route_redraw || effects.changed || !consumed {
                                            needs_redraw = true;
                                        }
                                    } else {
                                        elwt.exit();
                                    }
                                }
                                PhysicalKey::Code(KeyCode::KeyF) => {
                                    toggle_focus(
                                        &mut host,
                                        stage,
                                        focus_white,
                                        &current,
                                        &country,
                                        gpu.size(),
                                    );
                                    needs_redraw = true;
                                }
                                PhysicalKey::Code(KeyCode::KeyC) => {
                                    combat_active = !combat_active;
                                    if combat_active {
                                        let mut shell = CombatShell::new(
                                            Some(combat_centroids.clone()),
                                            combat_names.clone(),
                                        );
                                        shell.rebind(&current);
                                        let cam = current_camera(&host);
                                        host.open_instance(Box::new(CombatView::new(
                                            shell,
                                            combat_white,
                                            cam,
                                            gpu.size(),
                                        )));
                                        host.rebind_all(&RebindContext {
                                            snapshot: &current,
                                            country: &country,
                                            screen: gpu.size(),
                                            refit: false,
                                        });
                                    } else {
                                        host.close_view(COMBAT_VIEW_ID);
                                    }
                                    eprintln!(
                                        "[app] combat overlay {}",
                                        if combat_active { "ON" } else { "off" }
                                    );
                                    needs_redraw = true;
                                }
                                PhysicalKey::Code(KeyCode::ArrowRight) => {
                                    let _ = timeline.step(Dir::Forward);
                                    apply_seek!(timeline.pos());
                                }
                                PhysicalKey::Code(KeyCode::ArrowLeft) => {
                                    let _ = timeline.step(Dir::Backward);
                                    apply_seek!(timeline.pos());
                                }
                                PhysicalKey::Code(KeyCode::KeyM)
                                | PhysicalKey::Code(KeyCode::BracketRight) => {
                                    cycle_mode!(next);
                                }
                                PhysicalKey::Code(KeyCode::BracketLeft) => {
                                    cycle_mode!(prev);
                                }
                                // GENERIC descriptor-driven shortcuts (Codex (b)/(d)): a
                                // cluster view that binds `shortcut: Some(Letter('X'))` gets
                                // toggled here with NO per-view branch — adding a hotkey
                                // never edits main.rs. (The Core F/C overlays are seeded glue
                                // above; they declare `shortcut: None`.)
                                PhysicalKey::Code(code) => {
                                    if let Some(letter) = keycode_letter(code) {
                                        if let Some(vid) = registry.shortcut_view(letter) {
                                            if host.is_open(vid) {
                                                host.close_view(vid);
                                            } else if host.open_view(&registry, vid, None).is_some()
                                            {
                                                host.rebind_all(&RebindContext {
                                                    snapshot: &current,
                                                    country: &country,
                                                    screen: gpu.size(),
                                                    refit: true,
                                                });
                                            }
                                            needs_redraw = true;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                        if refresh_hover!() {
                            needs_redraw = true;
                        }
                    }
                }

                WindowEvent::RedrawRequested => {
                    let (w, h) = gpu.size();
                    sync_mapmodes_active_mode(&mut host);
                    let bound = bind_host(
                        &host,
                        stage,
                        &current,
                        &country,
                        cursor,
                        current_camera(&host),
                        w,
                        h,
                    );
                    let chrome = FrameChrome {
                        scrubber_track: Some(rect_of(scrubber.track())),
                        scrubber_thumb_x: scrubber.thumb_x(),
                        menubar: Some(build_menubar_chrome(
                            &menubar,
                            &registry,
                            menubar_white,
                            cursor,
                            w,
                        )),
                    };
                    compose::compose_offscreen(
                        &mut gpu.renderer,
                        stage,
                        &mut fonts,
                        &mut textures,
                        &host,
                        &bound,
                        &chrome,
                        w,
                        h,
                    );
                    match gpu.present() {
                        Ok(()) => {}
                        Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                            let (w, h) = gpu.size();
                            gpu.resize(w.max(1), h.max(1));
                            needs_redraw = true;
                        }
                        Err(e) => eprintln!("[app] surface present error: {e}"),
                    }
                }
                _ => {}
            },

            Event::AboutToWait => {
                if scrubber.is_playing() {
                    let now = Instant::now();
                    let dt = now.duration_since(last_tick).as_secs_f32();
                    last_tick = now;
                    if let ScrubIntent::StepForward = scrubber.tick(dt) {
                        let _ = timeline.step(Dir::Forward);
                        apply_seek!(timeline.pos());
                    }
                }
                let _ = right_down;
                if needs_redraw {
                    needs_redraw = false;
                    window.request_redraw();
                }
            }
            _ => {}
        }
    });
}

// ===========================================================================
// Helpers (host wiring; no per-view branches in the loop).
// ===========================================================================

fn rect_of(t: TrackRect) -> Rect {
    Rect {
        x: t.x,
        y: t.y,
        w: t.w,
        h: t.h,
    }
}

/// Build the menubar chrome for one frame: lay it out from the registry model + build
/// its render plan (pure), wrapped with the leaked white + label font. Host chrome —
/// drawn above all views, hit-tested before the host.
fn build_menubar_chrome(
    menubar: &MenuBar,
    registry: &ViewRegistry,
    white: &'static render::Texture,
    cursor: (f32, f32),
    screen_w: u32,
) -> MenubarChrome {
    let layout = menubar.layout(registry, screen_w as f32);
    let plan = app::menubar::plan_menubar(menubar, &layout, screen_w as f32, Some(cursor));
    MenubarChrome {
        plan,
        white,
        font: "hoi_16mbs",
    }
}

/// Map a winit `KeyCode` to its A–Z letter (for the generic descriptor-driven shortcut
/// dispatch). Returns `None` for non-letter keys.
fn keycode_letter(code: winit::keyboard::KeyCode) -> Option<char> {
    use winit::keyboard::KeyCode::*;
    let c = match code {
        KeyA => 'A',
        KeyB => 'B',
        KeyD => 'D',
        KeyE => 'E',
        KeyG => 'G',
        KeyH => 'H',
        KeyI => 'I',
        KeyJ => 'J',
        KeyK => 'K',
        KeyL => 'L',
        KeyN => 'N',
        KeyO => 'O',
        KeyP => 'P',
        KeyQ => 'Q',
        KeyR => 'R',
        KeyS => 'S',
        KeyT => 'T',
        KeyU => 'U',
        KeyV => 'V',
        KeyW => 'W',
        KeyX => 'X',
        KeyY => 'Y',
        KeyZ => 'Z',
        // C/F/M are Core-reserved (combat / focus / map-mode), handled explicitly above.
        _ => return None,
    };
    Some(c)
}

/// Build the `BindContext` + bind every open view (the per-frame host bind).
#[allow(clippy::too_many_arguments)]
fn bind_host(
    host: &WindowHost,
    stage: &'static Stage,
    snap: &Snapshot,
    country: &str,
    cursor: (f32, f32),
    map_camera: MapCamera,
    w: u32,
    h: u32,
) -> Vec<app::host::BoundView> {
    let date = snap.date.clone().unwrap_or_default();
    let ctx = BindContext {
        stage,
        snapshot: snap,
        country,
        screen: Rect {
            x: 0.0,
            y: 0.0,
            w: w as f32,
            h: h as f32,
        },
        date: &date,
        cursor,
        map_camera: Some(map_camera),
    };
    host.bind_all(&ctx)
}

#[allow(clippy::too_many_arguments)]
fn front_instance_id(
    host: &WindowHost,
    view_id: ViewId,
    stage: &'static Stage,
    snap: &Snapshot,
    country: &str,
    cursor: (f32, f32),
    map_camera: MapCamera,
    w: u32,
    h: u32,
) -> Option<InstanceId> {
    bind_host(host, stage, snap, country, cursor, map_camera, w, h)
        .into_iter()
        .rev()
        .find(|bv| bv.view_id == view_id)
        .map(|bv| bv.instance)
}

#[derive(Debug, Clone, Copy, Default)]
struct CommandEffects {
    changed: bool,
    opened_or_closed_view: bool,
}

fn execute_view_commands(
    host: &mut WindowHost,
    registry: &ViewRegistry,
    emitter: Option<InstanceId>,
    commands: Vec<ViewCommand>,
) -> CommandEffects {
    let mut effects = CommandEffects::default();
    for cmd in commands {
        match cmd {
            ViewCommand::RecenterMap { raster_x, raster_y } => {
                if let Some(mv) = map_view_mut(host) {
                    effects.changed |= mv.camera.recenter_on_raster(raster_x, raster_y);
                }
            }
            ViewCommand::SetMapMode { mode } => {
                let command = ViewCommand::SetMapMode { mode };
                if let Some(mv) = map_view_mut(host) {
                    if mv.apply_command(&command) {
                        eprintln!("[app] map mode -> {}", mv.mode.label());
                        if let Some(mm) = host.downcast_mut::<MapModesView>(MAPMODES_VIEW_ID) {
                            mm.set_active_mode(mode);
                        }
                        effects.changed = true;
                    }
                }
            }
            other => {
                let structural = matches!(
                    other,
                    ViewCommand::OpenView { .. }
                        | ViewCommand::CloseSelf
                        | ViewCommand::CloseView { .. }
                );
                if host.execute_commands(registry, emitter, [other]) {
                    effects.changed = true;
                    effects.opened_or_closed_view |= structural;
                }
            }
        }
    }
    if effects.changed {
        sync_combat_camera(host);
    }
    effects
}

fn rebind_after_structural_commands(
    host: &mut WindowHost,
    effects: CommandEffects,
    snap: &Snapshot,
    country: &str,
    screen: (u32, u32),
) {
    if effects.opened_or_closed_view {
        host.rebind_all(&RebindContext {
            snapshot: snap,
            country,
            screen,
            refit: true,
        });
        sync_combat_camera(host);
    }
}

fn route_view_key(
    host: &mut WindowHost,
    registry: &ViewRegistry,
    key: ViewKey,
    snap: &Snapshot,
    country: &str,
    screen: (u32, u32),
    needs_redraw: &mut bool,
) -> bool {
    let r = host.route_input(&ViewInput::Key { key });
    let effects = execute_view_commands(host, registry, r.emitter, r.commands);
    rebind_after_structural_commands(host, effects, snap, country, screen);
    if r.needs_redraw || effects.changed {
        *needs_redraw = true;
    }
    r.consumed
}

fn blur_find_query(host: &mut WindowHost, needs_redraw: &mut bool) {
    if let Some(find) = host.downcast_mut::<FindView>(FIND_VIEW_ID) {
        if find.clear_query_focus() {
            *needs_redraw = true;
        }
    }
}

/// The current map camera (read from the seeded MapView instance).
fn current_camera(host: &WindowHost) -> MapCamera {
    // the map view is always open; read its camera. Fall back to a 1x1 if somehow not.
    host_map_camera(host).unwrap_or_else(|| MapCamera::new(1, 1, 1, 1))
}

fn sync_combat_camera(host: &mut WindowHost) {
    let cam = current_camera(host);
    if let Some(cv) = host.downcast_mut::<CombatView>(COMBAT_VIEW_ID) {
        cv.set_camera(cam);
    }
    let map_cam = host_map_camera(host);
    if let Some(mm) = host.downcast_mut::<MinimapView>(MINIMAP_VIEW_ID) {
        mm.set_camera(map_cam);
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_topbar_input_targets(
    host: &mut WindowHost,
    stage: &'static Stage,
    snap: &Snapshot,
    country: &str,
    cursor: (f32, f32),
    map_camera: MapCamera,
    w: u32,
    h: u32,
) {
    let bound = bind_host(host, stage, snap, country, cursor, map_camera, w, h);
    let topbar_is_front_hit = matches!(
        host.resolve_click(&bound, cursor.0, cursor.1),
        HostHit::Captured { view_id, .. } if view_id == TOPBAR_VIEW_ID
    );
    let (drill_targets, inert_targets) = if topbar_is_front_hit {
        let topbar_bound = bound
            .iter()
            .find(|bv| bv.view_id == TOPBAR_VIEW_ID)
            .map(|bv| bv.hit.roots.as_slice());
        (
            topbar_bound
                .map(topbar_drill_targets_from_roots)
                .unwrap_or_default(),
            topbar_bound
                .map(topbar_inert_targets_from_roots)
                .unwrap_or_default(),
        )
    } else {
        (Vec::new(), Vec::new())
    };
    if let Some(topbar) = host.downcast_mut::<TopbarView>(TOPBAR_VIEW_ID) {
        topbar.set_drill_targets(drill_targets);
        topbar.set_inert_targets(inert_targets);
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_minimap_input_panel(
    host: &mut WindowHost,
    stage: &'static Stage,
    snap: &Snapshot,
    country: &str,
    cursor: (f32, f32),
    map_camera: MapCamera,
    w: u32,
    h: u32,
) {
    let panel = bind_host(host, stage, snap, country, cursor, map_camera, w, h)
        .into_iter()
        .find(|bv| bv.view_id == MINIMAP_VIEW_ID)
        .and_then(|bv| bv.hit.block_rect);
    if let (Some(panel), Some(mm)) = (panel, host.downcast_mut::<MinimapView>(MINIMAP_VIEW_ID)) {
        mm.set_input_panel(panel);
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_find_input_panel(
    host: &mut WindowHost,
    stage: &'static Stage,
    snap: &Snapshot,
    country: &str,
    cursor: (f32, f32),
    map_camera: MapCamera,
    w: u32,
    h: u32,
) {
    let bound = bind_host(host, stage, snap, country, cursor, map_camera, w, h)
        .into_iter()
        .find(|bv| bv.view_id == FIND_VIEW_ID);
    if let (Some(bound), Some(find)) = (bound, host.downcast_mut::<FindView>(FIND_VIEW_ID)) {
        let panel = find_view_panel_rect_from_roots(
            Rect {
                x: 0.0,
                y: 0.0,
                w: w as f32,
                h: h as f32,
            },
            &bound.hit.roots,
        );
        let query_rect = find_view_query_rect_from_roots(
            Rect {
                x: 0.0,
                y: 0.0,
                w: w as f32,
                h: h as f32,
            },
            &bound.hit.roots,
        );
        let (find_button_rect, home_button_rect) = find_view_button_rects_from_roots(
            Rect {
                x: 0.0,
                y: 0.0,
                w: w as f32,
                h: h as f32,
            },
            &bound.hit.roots,
        );
        find.set_input_regions(panel, query_rect, find_button_rect, home_button_rect);
        find.set_navigation_geometry(&stage.map_dir, map_camera);
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_mapmodes_input_panel(
    host: &mut WindowHost,
    stage: &'static Stage,
    snap: &Snapshot,
    country: &str,
    cursor: (f32, f32),
    map_camera: MapCamera,
    w: u32,
    h: u32,
) {
    let bound = bind_host(host, stage, snap, country, cursor, map_camera, w, h)
        .into_iter()
        .find(|bv| bv.view_id == MAPMODES_VIEW_ID);
    if let (Some(bound), Some(mm)) = (bound, host.downcast_mut::<MapModesView>(MAPMODES_VIEW_ID)) {
        let targets = mapmode_button_targets_from_roots(&bound.hit.roots);
        let all_button = mapmodes_all_button_rect_from_roots(&bound.hit.roots);
        let all_grid = mapmodes_all_grid_rect_from_roots(&bound.hit.roots);
        if targets.is_empty() {
            if let Some(panel) = bound.hit.block_rect {
                mm.set_input_panel(panel);
            }
        } else {
            mm.set_input_targets_with_all(targets, all_button, all_grid);
        }
    }
    sync_mapmodes_active_mode(host);
}

fn sync_mapmodes_active_mode(host: &mut WindowHost) {
    let active_mode = map_view_ref(host).map(|mv| mv.mode);
    if let (Some(mode), Some(mm)) = (
        active_mode,
        host.downcast_mut::<MapModesView>(MAPMODES_VIEW_ID),
    ) {
        mm.set_active_mode(mode);
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_trait_tree_input_panels(
    host: &mut WindowHost,
    stage: &'static Stage,
    snap: &Snapshot,
    country: &str,
    cursor: (f32, f32),
    map_camera: MapCamera,
    w: u32,
    h: u32,
) {
    if !host.is_open(ARMY_TRAIT_TREE_VIEW_ID) && !host.is_open(NAVY_TRAIT_TREE_VIEW_ID) {
        return;
    }
    let bound = bind_host(host, stage, snap, country, cursor, map_camera, w, h);
    let army_viewport = bound
        .iter()
        .find(|bv| bv.view_id == ARMY_TRAIT_TREE_VIEW_ID)
        .map(|bv| trait_tree_viewport_from_roots(&bv.hit.roots));
    let navy_viewport = bound
        .iter()
        .find(|bv| bv.view_id == NAVY_TRAIT_TREE_VIEW_ID)
        .map(|bv| trait_tree_viewport_from_roots(&bv.hit.roots));

    if let (Some(viewport), Some(view)) = (
        army_viewport,
        host.downcast_mut::<ArmyTraitTreeView>(ARMY_TRAIT_TREE_VIEW_ID),
    ) {
        view.set_input_viewport(viewport);
    }
    if let (Some(viewport), Some(view)) = (
        navy_viewport,
        host.downcast_mut::<NavyTraitTreeView>(NAVY_TRAIT_TREE_VIEW_ID),
    ) {
        view.set_input_viewport(viewport);
    }
}

fn host_map_camera(host: &WindowHost) -> Option<MapCamera> {
    // we can't downcast a &dyn here; instead the binary keeps the camera in the MapView,
    // accessed via map_view_ref. The host stores Box<dyn ViewInstance>; we use the
    // downcast helper below.
    map_view_ref(host).map(|mv| mv.camera)
}

/// Downcast the open map view (immutable). The host holds `Box<dyn ViewInstance>`; we
/// know the Background-band singleton is a `MapView`.
fn map_view_ref(host: &WindowHost) -> Option<&MapView> {
    host.downcast_ref::<MapView>(MAP_VIEW_ID)
}
fn map_view_mut(host: &mut WindowHost) -> Option<&mut MapView> {
    host.downcast_mut::<MapView>(MAP_VIEW_ID)
}

/// Handle a host `Captured` hit: a close control on a panel/overlay closes it.
fn handle_captured(
    host: &mut WindowHost,
    view_id: ViewId,
    leaf: Option<&str>,
    needs_redraw: &mut bool,
) {
    const CLOSE_NAMES: &[&str] = &["close_button", "close", "exit", "X"];
    if let Some(name) = leaf {
        if CLOSE_NAMES.iter().any(|n| *n == name) {
            host.close_view(view_id);
            *needs_redraw = true;
        }
    }
    // other captured controls are inert chrome (no drill) — same as before.
}

/// Open the state panel for `sid` through the host (resolving the display name).
fn open_state_panel(host: &mut WindowHost, stage: &Stage, _snap: &Snapshot, sid: u32) {
    let name = compose::resolve_state_name(&stage.map_dir, &stage.loc, sid)
        .unwrap_or_else(|| format!("STATE_{sid}"));
    host.open_instance(Box::new(StatePanelView::new(sid, name)));
}

/// Toggle the focus overlay: open a seeded FocusView, or close it if open.
fn toggle_focus(
    host: &mut WindowHost,
    stage: &Stage,
    white: &'static render::Texture,
    snap: &Snapshot,
    country: &str,
    screen: (u32, u32),
) {
    if host.is_open(FOCUS_VIEW_ID) {
        host.close_view(FOCUS_VIEW_ID);
        return;
    }
    let shell = FocusShell::load(&stage.resolver);
    if !shell.has_catalog() {
        eprintln!("[app] focus catalog not loaded; overlay disabled.");
        return;
    }
    host.open_instance(Box::new(FocusView::new(
        shell, white, country, snap, screen,
    )));
    host.rebind_all(&RebindContext {
        snapshot: snap,
        country,
        screen,
        refit: true,
    });
    eprintln!("[app] focus overlay OPEN");
}

#[allow(clippy::too_many_arguments)]
fn refresh_map_hover(
    host: &mut WindowHost,
    stage: &'static Stage,
    snap: &Snapshot,
    country: &str,
    map_data: &MapData,
    menubar: &MenuBar,
    registry: &ViewRegistry,
    cursor: (f32, f32),
    scrubber_dragging: bool,
    scrubber_track: TrackRect,
    w: u32,
    h: u32,
) -> bool {
    let hover = map_hover_at(
        host,
        stage,
        snap,
        country,
        map_data,
        menubar,
        registry,
        cursor,
        scrubber_dragging,
        scrubber_track,
        w,
        h,
    );
    map_view_mut(host)
        .map(|mv| mv.set_hover(hover))
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
fn map_hover_at(
    host: &WindowHost,
    stage: &'static Stage,
    snap: &Snapshot,
    country: &str,
    map_data: &MapData,
    menubar: &MenuBar,
    registry: &ViewRegistry,
    cursor: (f32, f32),
    scrubber_dragging: bool,
    scrubber_track: TrackRect,
    w: u32,
    h: u32,
) -> Option<MapHover> {
    if map_hover_blocked_by_transport(
        scrubber_dragging,
        scrubber_track,
        cursor,
        menubar,
        registry,
        w,
    ) {
        return None;
    }
    let bound = bind_host(
        host,
        stage,
        snap,
        country,
        cursor,
        current_camera(host),
        w,
        h,
    );
    if !matches!(
        host.resolve_click(&bound, cursor.0, cursor.1),
        HostHit::Miss
    ) {
        return None;
    }
    map_hover_pick(host, map_data, snap, cursor)
}

fn map_hover_blocked_by_transport(
    scrubber_dragging: bool,
    scrubber_track: TrackRect,
    cursor: (f32, f32),
    menubar: &MenuBar,
    registry: &ViewRegistry,
    screen_w: u32,
) -> bool {
    if scrubber_dragging || scrubber_track.contains(cursor.0, cursor.1) {
        return true;
    }
    let menu_layout = menubar.layout(registry, screen_w as f32);
    !matches!(
        menubar.pick(&menu_layout, cursor.0, cursor.1),
        MenuClick::Miss
    )
}

fn map_hover_pick(
    host: &WindowHost,
    map_data: &MapData,
    snap: &Snapshot,
    cursor: (f32, f32),
) -> Option<MapHover> {
    let cam = map_view_ref(host)?.camera;
    let (rx, ry) = cam.screen_to_raster(cursor.0, cursor.1)?;
    let empty = serde_json::Map::new();
    let states = snap.states().unwrap_or(&empty);
    let owners = StateOwners::from_states(states);
    let pick = pick_at(&map_data.raster, &map_data.ids, &owners, rx, ry)?;
    Some(MapHover::from_pick(pick, cursor, (rx, ry)))
}

/// Classify a map click into a picked state id (the bottom-layer pick, on a host Miss).
fn map_pick(host: &WindowHost, map_data: &MapData, cursor: (f32, f32)) -> Option<u32> {
    let cam = map_view_ref(host)?.camera;
    let (rx, ry) = cam.screen_to_raster(cursor.0, cursor.1)?;
    let owners = StateOwners::from_states(&serde_json::Map::new());
    let p = pick_at(&map_data.raster, &map_data.ids, &owners, rx, ry)?;
    p.state_id
}
