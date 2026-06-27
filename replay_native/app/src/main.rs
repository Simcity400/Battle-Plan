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
use app::host::view::RebindContext;
use app::host::window_host::HostHit;
use app::host::{BindContext, ViewId, ViewRegistry, WindowHost};
use app::menubar::{MenuBar, MenuClick};
use app::provenance::Provenance;
use app::scrubber::{ScrubIntent, Scrubber, TrackRect};
use app::views::combat::{CombatView, COMBAT_VIEW_ID};
use app::views::focus::{FocusView, FOCUS_VIEW_ID};
use app::views::map::{MapData, MapView, MAP_VIEW_ID};
use app::views::state_panel::StatePanelView;
use app::views::topbar::TopbarView;

use assets::resolver::GameResolver;
use assets::SpriteManifest;
use gui_runtime::draw::TextureCache;
use gui_runtime::fonts::{FontCache, FontMap};
use gui_runtime::{Loc, Rect};
use render::map::{pick_at, CountryColors, MapIds, PoliticalMap, ProvinceRaster, StateOwners};
use render::modes::MapMode;
use scrub::{Dir, Timeline};
use snapshot::{Snapshot, Store};

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
    let shot_state: Option<u32> = arg_value("--shot-state").and_then(|s| s.parse().ok());
    let shot_focus = has_flag("--shot-focus");
    let shot_combat = has_flag("--shot-combat");

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
    let registry = ViewRegistry::from_inventory();
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
    let init_mode: MapMode = arg_value("--map-mode")
        .and_then(|s| mode_from_slug(&s))
        .unwrap_or(MapMode::Political);
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
    let mut combat_active = true;
    let mut needs_redraw = true;

    // ----------------------------------------------------------------------
    // SHOT / SMOKE: one headless frame through the SAME host compose path.
    // ----------------------------------------------------------------------
    if shot_path.is_some() || smoke {
        if let Some(sid) = shot_state {
            let name = compose::resolve_state_name(&stage.map_dir, &stage.loc, sid)
                .unwrap_or_else(|| format!("STATE_{sid}"));
            host.open_instance(Box::new(StatePanelView::new(sid, name)));
        }
        if shot_focus {
            let shell = FocusShell::load(&stage.resolver);
            host.open_instance(Box::new(FocusView::new(
                shell,
                focus_white,
                &country,
                &current,
                gpu.size(),
            )));
        }
        if shot_combat {
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
                    scrubber.set_track(scrubber_track(size.width, size.height));
                    host.rebind_all(&RebindContext {
                        snapshot: &current,
                        country: &country,
                        screen: (size.width, size.height),
                        refit: false,
                    });
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
                    let r = host.route_input(&ViewInput::PointerMove {
                        x: cursor.0,
                        y: cursor.1,
                        drag,
                    });
                    if r.needs_redraw {
                        needs_redraw = true;
                    }
                    if !r.consumed {
                        if let Some((ax, ay)) = map_drag {
                            if let Some(mv) = map_view_mut(&mut host) {
                                mv.camera.pan_by_pixels(cursor.0 - ax, cursor.1 - ay);
                            }
                            map_drag = Some(cursor);
                            needs_redraw = true;
                        }
                    }
                }

                WindowEvent::MouseWheel { delta, .. } => {
                    let dy = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => (p.y as f32) / 40.0,
                    };
                    let r = host.route_input(&ViewInput::Wheel {
                        x: cursor.0,
                        y: cursor.1,
                        dy,
                    });
                    if !r.consumed {
                        if let Some(mv) = map_view_mut(&mut host) {
                            mv.camera.zoom_at(dy, cursor.0, cursor.1, 16.0);
                        }
                    }
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
                            apply_seek!(idx);
                        } else {
                            // 2) route the click to the host (overlays + windows).
                            let r = host.route_input(&ViewInput::PointerDown {
                                x: cursor.0,
                                y: cursor.1,
                                button: PointerButton::Left,
                            });
                            // run any commands the views emitted (open/close).
                            let emitter = r.emitter;
                            if !r.commands.is_empty() {
                                let changed = host.execute_commands(&registry, emitter, r.commands);
                                if changed {
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
                    }
                    (ElementState::Released, MouseButton::Left) => {
                        left_down = false;
                        scrubber.release();
                        let _ = host.route_input(&ViewInput::PointerUp {
                            x: cursor.0,
                            y: cursor.1,
                            button: PointerButton::Left,
                        });
                    }
                    (ElementState::Pressed, MouseButton::Right) => {
                        right_down = true;
                        map_drag = Some(cursor);
                    }
                    (ElementState::Released, MouseButton::Right) => {
                        right_down = false;
                        map_drag = None;
                    }
                    _ => {}
                },

                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state == ElementState::Pressed {
                        match event.physical_key {
                            PhysicalKey::Code(KeyCode::Space) => {
                                scrubber.toggle_play();
                                last_tick = Instant::now();
                            }
                            PhysicalKey::Code(KeyCode::Escape) => {
                                // ESC: close the front-most closeable overlay/panel, else quit.
                                if let Some(vid) = host.front_closeable() {
                                    // let the view react (combat ESC closes its sub-panel
                                    // first), then close the window if still front.
                                    let _ = host.route_input(&ViewInput::Key {
                                        key: ViewKey::Escape,
                                    });
                                    host.close_view(vid);
                                    needs_redraw = true;
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
                                        } else if host.open_view(&registry, vid, None).is_some() {
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
                }

                WindowEvent::RedrawRequested => {
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

/// The current map camera (read from the seeded MapView instance).
fn current_camera(host: &WindowHost) -> MapCamera {
    // the map view is always open; read its camera. Fall back to a 1x1 if somehow not.
    host_map_camera(host).unwrap_or_else(|| MapCamera::new(1, 1, 1, 1))
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

/// Classify a map click into a picked state id (the bottom-layer pick, on a host Miss).
fn map_pick(host: &WindowHost, map_data: &MapData, cursor: (f32, f32)) -> Option<u32> {
    let cam = map_view_ref(host)?.camera;
    let (rx, ry) = cam.screen_to_raster(cursor.0, cursor.1)?;
    let owners = StateOwners::from_states(&serde_json::Map::new());
    let p = pick_at(&map_data.raster, &map_data.ids, &owners, rx, ry)?;
    p.state_id
}
