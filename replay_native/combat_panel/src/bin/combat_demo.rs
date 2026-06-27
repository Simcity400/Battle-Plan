//! STAGE 4 deliverable — render a REAL combat panel (+ the map bubbles) to an
//! offscreen PNG showing the roster + location + duration EXACT/DERIVED and the
//! battle-detail as "—" UNAVAILABLE. READ-ONLY: reads the snapshot store, the
//! province-id raster + map artifacts, the loc + flag assets only; never writes
//! the game or a save.
//!
//! Output: replay_native/render/out/combat_panel.png
//! Paths via CLI/env/documented defaults — no personal literal in source.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use assets::resolver::GameResolver;
use combat_panel::bubbles::{layout_bubbles, MapCamera, ProvinceCentroids};
use combat_panel::view::{draw_bubbles, draw_panel, white_texture, DrawAssets};
use combat_panel::{aggregate_combats, flag_path, CombatDomain, ProvinceNames};
use gui_runtime::fonts::{FontCache, FontMap};
use gui_runtime::loc::Loc;
use render::map::ProvinceRaster;
use render::{save_png, Renderer, Texture};
use snapshot::Store;

fn arg(flag: &str) -> Option<String> {
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

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("render")
        .join("out")
}

fn main() {
    let game = arg("--game").unwrap_or_else(|| "replay".to_string());
    let width: u32 = arg("--width").and_then(|s| s.parse().ok()).unwrap_or(1280);
    let height: u32 = arg("--height").and_then(|s| s.parse().ok()).unwrap_or(900);

    let vanilla = arg("--vanilla");
    let mod_root = arg("--mod").or_else(|| {
        let p = repo_root().parent()?.join("test");
        p.is_dir().then(|| p.to_string_lossy().into_owned())
    });
    let resolver = GameResolver::new(vanilla.as_deref(), mod_root.as_deref());
    // The map_pipeline artifacts (province_ids.png + province_to_state/state_meta
    // json) — same convention as the m4/app demos.
    let map_dir = arg("--map")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("map_pipeline").join("assets"));

    // 1) Load the snapshot + extract the combat domain.
    let store_root = arg("--store")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("replay").join("snapshots"));
    let store = Store::open(&store_root);
    let idx = match store.index(&game) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("[combat_demo] BLOCKER: cannot read index for game `{game}`: {e}");
            std::process::exit(2);
        }
    };
    let hour = arg("--hour")
        .and_then(|s| s.parse::<i64>().ok())
        .or_else(|| idx.snapshots.iter().find_map(|e| e.hour_index));
    let hour = match hour {
        Some(h) => h,
        None => {
            eprintln!("[combat_demo] BLOCKER: no snapshots in game `{game}`.");
            std::process::exit(2);
        }
    };
    let snap = match store.load(&game, hour) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[combat_demo] BLOCKER: cannot load snapshot {hour}: {e}");
            std::process::exit(2);
        }
    };
    let date = snap.date.clone().unwrap_or_default();
    // Assemble just the domains the combat reader needs into a Value (Snapshot's
    // payload domains are raw `serde_json::Value`s — no full re-serialize needed).
    let mut obj = serde_json::Map::new();
    for key in ["combats", "map", "provinces"] {
        if let Some(v) = snap.domain(key) {
            obj.insert(key.to_string(), v.clone());
        }
    }
    let snap_value = serde_json::Value::Object(obj);
    let domain = CombatDomain::from_snapshot_value(&snap_value);
    eprintln!(
        "[combat_demo] game={game} date={date} hour={hour}: {} combats, {} controllers",
        domain.combats.len(),
        domain.controller.len()
    );

    // 2) Province names (derived) from map artifacts + loc.
    let mut loc = Loc::new();
    let _ = loc.add_dir(&resolver.vanilla.join("localisation").join("english"));
    if let Some(m) = &resolver.mod_root {
        let _ = loc.add_dir(&m.join("localisation").join("english"));
    }
    let names = ProvinceNames::load(&map_dir, &loc);
    eprintln!("[combat_demo] province names resolved: {}", names.len());

    // 3) Aggregate the land combats into the roster view-model.
    let controller_of = |p: i64| domain.controller_of(p);
    let province_name = |p: i64| names.get(p);
    let combats = aggregate_combats(&snap_value.get("combats").cloned().unwrap_or(serde_json::Value::Null),
        Some("land"), &controller_of, &province_name);
    if combats.is_empty() {
        eprintln!("[combat_demo] BLOCKER: no land combats to render (combats[] empty for this frame).");
        std::process::exit(2);
    }
    eprintln!("[combat_demo] aggregated {} land combats", combats.len());

    // 4) Province centroids + a fit camera for the bubbles (best-effort: the PNG
    //    still renders the panel even if the raster is unavailable).
    let raster = ProvinceRaster::load_png(&map_dir.join("province_ids.png")).ok();
    let centroids = raster.as_ref().map(ProvinceCentroids::from_raster);

    // 5) Renderer + assets.
    let mut renderer = match Renderer::new(width, height) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[combat_demo] BLOCKER: GPU init failed: {e}");
            std::process::exit(3);
        }
    };
    let white = white_texture(&renderer);
    let font_map = FontMap::from_core_gfx(&resolver);
    let mut fonts = FontCache::new(&font_map, &resolver);
    let loaded = fonts.get(&renderer, "hoi_16mbs");
    let (font_ref, pages_ref): (Option<&assets::BmFont>, &[Texture]) = match &loaded {
        Some(l) => (Some(&l.font), &l.pages),
        None => {
            eprintln!("[combat_demo] note: font unavailable; rendering chrome only.");
            (None, &[])
        }
    };

    // Resolve the DERIVED holding-tag flags only (never the unavailable attacker).
    // The holding tag's ruling-party ideology picks the authentic ideology flag.
    let ideology_of = |tag: &str| -> Option<String> {
        snap.country(tag)
            .and_then(|c| c.get("politics"))
            .and_then(|p| p.get("ruling_party"))
            .and_then(|r| r.as_str())
            .map(|s| s.to_string())
    };
    let extracted_flags = repo_root().join("game_assets").join("assets").join("flags");
    let extracted_flags = extracted_flags.is_dir().then_some(extracted_flags);
    let mut flags: HashMap<String, Texture> = HashMap::new();
    for vm in &combats {
        if let combat_panel::SideTag::HoldingTagDerived(t) = &vm.defender.tag {
            if !flags.contains_key(t) {
                let ideo = ideology_of(t);
                if let Some(p) = flag_path(&resolver, extracted_flags.as_deref(), t, ideo.as_deref()) {
                    if let Ok(tex) = renderer.load_image(&p) {
                        flags.insert(t.clone(), tex);
                    }
                }
            }
        }
    }
    eprintln!("[combat_demo] loaded {} holding-tag flags", flags.len());

    let assets = DrawAssets {
        white: &white,
        font: font_ref,
        font_pages: pages_ref,
        flags: &flags,
    };

    // background
    let full = [0.0, 0.0, width as f32, height as f32];
    renderer.draw_sprite_frame_clipped(&white, 0.0, 0.0, width as f32, height as f32, 0, 1, [0.06, 0.07, 0.09, 1.0], Some(full));

    // 6) Bubbles laid out over a left "map" strip (fit the combat provinces into it).
    let mut bubbles_drawn = 0;
    let selected = &combats[0]; // the panel we open
    let map_view = [0.0, 36.0, width as f32 * 0.46, height as f32 - 36.0];
    if let Some(cent) = &centroids {
        // fit the combat-province centroids into the map strip.
        let pts: Vec<(f32, f32)> = combats
            .iter()
            .filter(|v| v.location_province >= 0)
            .filter_map(|v| cent.get(v.location_province as u32))
            .collect();
        if !pts.is_empty() {
            let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
            for (x, y) in &pts {
                minx = minx.min(*x); miny = miny.min(*y); maxx = maxx.max(*x); maxy = maxy.max(*y);
            }
            let bw = (maxx - minx).max(1.0);
            let bh = (maxy - miny).max(1.0);
            let margin = 60.0;
            let zoom = ((map_view[2] - 2.0 * margin) / bw).min((map_view[3] - 2.0 * margin) / bh).max(0.001);
            let cam = MapCamera::new((minx, miny), zoom, (map_view[0] + margin, map_view[1] + margin));
            let bubbles = layout_bubbles(&combats, cent, &cam);
            // a faint map-strip backdrop + label
            renderer.draw_sprite_frame_clipped(&white, map_view[0], map_view[1], map_view[2], map_view[3], 0, 1, [0.09, 0.12, 0.14, 1.0], Some(map_view));
            bubbles_drawn = draw_bubbles(&mut renderer, &white, &bubbles, Some(&selected.combat_id), map_view);
            if let Some(f) = font_ref {
                renderer.draw_text_clipped(f, pages_ref, &format!("COMBAT BUBBLES  ({} land battles)", bubbles.len()), map_view[0] + 12.0, map_view[1] + 8.0, [0.9, 0.9, 0.85, 1.0], Some(map_view));
            }
        }
    }
    if bubbles_drawn == 0 {
        eprintln!("[combat_demo] note: province raster unavailable; bubbles not placed (panel still rendered).");
    }

    // 7) The stacked combat panel on the right (the opened bubble).
    let pw = width as f32 * 0.50;
    let ph = (height as f32 - 72.0).min(620.0);
    let pxp = width as f32 - pw - 24.0;
    let pyp = 48.0;
    let (stats, _close) = draw_panel(&mut renderer, selected, &assets, pxp, pyp, pw, ph);

    // top title bar
    if let Some(f) = font_ref {
        renderer.draw_sprite_frame_clipped(&white, 0.0, 0.0, width as f32, 36.0, 0, 1, [0.04, 0.05, 0.07, 1.0], Some(full));
        let t = format!("STAGE 4 COMBAT PANEL   game={game}  {date}   bubble->panel  (read-only)");
        renderer.draw_text_clipped(f, pages_ref, &t, 12.0, 9.0, [0.95, 0.95, 0.92, 1.0], Some(full));
    }

    let rgba = renderer.finish([0.06, 0.07, 0.09, 1.0]);
    let out = out_dir();
    let _ = std::fs::create_dir_all(&out);
    let path = out.join("combat_panel.png");
    save_png(&path, width, height, &rgba).expect("save png");

    eprintln!(
        "[combat_demo] wrote {} ({}x{}) | bubbles={} panel={} | rows: exact={} derived={} UNAVAILABLE={}",
        path.display(), width, height, bubbles_drawn, stats.panel_drawn,
        stats.exact_rows, stats.derived_rows, stats.unavailable_rows
    );
    eprintln!(
        "[combat_demo] opened combat {} | attacker refs={} (TAG {}) | defender refs={} (TAG {})",
        selected.combat_id,
        selected.attacker.ref_count(), selected.attacker.tag.display(),
        selected.defender.ref_count(), selected.defender.tag.display(),
    );
}
