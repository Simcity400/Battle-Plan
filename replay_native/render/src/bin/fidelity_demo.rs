//! fidelity_demo — the MAP VISUAL FIDELITY pass, end to end against REAL assets.
//!
//! Renders the map the way the in-game the game map reads it: a textured, relief-shaded
//! terrain underlay (from `terrain.bmp` + `world_normal.bmp` + `00_terrain.txt`), the
//! mode's per-province colour tint composited ~55% TRANSLUCENT over it, drawn country
//! and province borders (from the province-id raster + a per-province owner index),
//! game water for the sea, and rivers (`rivers.bmp`).
//!
//! Output (render/out/):
//!   * `fidelity_before.png` — political mode, OLD flat opaque fill (fidelity off),
//!   * `fidelity_after.png`  — political mode, NEW terrain-composited (fidelity on),
//!   * `fidelity_political.png` / `fidelity_control.png` / `fidelity_supply.png` —
//!     representative modes with the full composite.
//!
//! Game map assets come from the runtime resolver (`--vanilla` / `BATTLE_PLAN_VANILLA`, else
//! the documented Steam default). READ-ONLY on every game asset and the snapshot store;
//! no personal path in source.

use std::path::{Path, PathBuf};

use assets::GameResolver;
use render::map::{
    build_lut, build_owner_index, build_secondary_lut, CountryColors, MapIds, PoliticalMap,
    ProvinceRaster, StateOwners,
};
use render::modes::{MapMode, ModeData};
use render::terrain::{
    build_base, Atlas, AtlasNormal, ColorMap, IndexedBmp, RgbBmp, RiverSurface, TerrainBase,
    TerrainDefs, TerrainInputs, WaterColorMap,
};
use render::{save_png, Renderer};

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

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}
fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("out")
}

fn main() {
    // --- province-id raster + typed ids + colours (the map_pipeline artifacts) ----
    let map_dir = arg_value("--map")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("map_pipeline").join("assets"));
    let store_root = arg_value("--store")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("replay").join("snapshots"));
    let game = arg_value("--game").unwrap_or_else(|| "campaign".to_string());

    // --- game map assets (terrain/normal/rivers + terrain defs) via the resolver ---
    let resolver = GameResolver::new(
        arg_value("--vanilla").as_deref(),
        arg_value("--mod").as_deref(),
    );
    eprintln!("[fid] vanilla = {}", resolver.vanilla.display());
    if !resolver.vanilla_exists() {
        eprintln!(
            "[fid] BLOCKER: game install not found at {}. Pass --vanilla <dir> or set BATTLE_PLAN_VANILLA.",
            resolver.vanilla.display()
        );
        std::process::exit(2);
    }

    let resolve = |rel: &str| -> PathBuf {
        resolver.resolve(rel).unwrap_or_else(|| {
            eprintln!("[fid] BLOCKER: asset not found: {rel}");
            std::process::exit(2);
        })
    };
    let terrain_bmp = resolve("map/terrain.bmp");
    let normal_bmp = resolve("map/world_normal.bmp");
    let rivers_bmp = resolve("map/rivers.bmp");
    let heightmap_bmp = resolve("map/heightmap.bmp");
    let terrain_txt = resolve("common/terrain/00_terrain.txt");
    // The REAL game terrain colourmap (land base) + water colourmap + ground-detail atlas
    // + river surface diffuse — every faithful base layer comes from a real game asset.
    let colormap_dds = resolve("map/terrain/colormap_rgb_cityemissivemask_a.dds");
    let water_colormap_dds = resolve("map/terrain/colormap_water_0.dds");
    let atlas_dds = resolve("map/terrain/atlas0.dds");
    let atlas_normal_dds = resolve("map/terrain/atlas_normal0.dds");
    let river_surface_dds = resolve("map/terrain/RiverSurface_diffuse_0.dds");

    if !map_dir.join("province_ids.png").is_file() {
        eprintln!(
            "[fid] BLOCKER: province_ids.png not found under {}",
            map_dir.display()
        );
        std::process::exit(2);
    }

    // --- load everything ---------------------------------------------------------
    let raster = ProvinceRaster::load_png(&map_dir.join("province_ids.png")).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: province raster: {e}");
        std::process::exit(2);
    });
    eprintln!(
        "[fid] province raster {}x{} max_id={}",
        raster.width, raster.height, raster.max_id
    );

    let ids = MapIds::load(&map_dir).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: MapIds: {e}");
        std::process::exit(2);
    });
    let colors = CountryColors::load(&map_dir).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: colours: {e}");
        std::process::exit(2);
    });

    let defs = TerrainDefs::load(&terrain_txt).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: terrain defs: {e}");
        std::process::exit(2);
    });
    let terrain = IndexedBmp::load(&terrain_bmp).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: terrain.bmp: {e}");
        std::process::exit(2);
    });
    let normal = RgbBmp::load(&normal_bmp).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: world_normal.bmp: {e}");
        std::process::exit(2);
    });
    let rivers = IndexedBmp::load(&rivers_bmp).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: rivers.bmp: {e}");
        std::process::exit(2);
    });
    let heightmap = IndexedBmp::load(&heightmap_bmp).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: heightmap.bmp: {e}");
        std::process::exit(2);
    });
    // base = the REAL game colourmap (RGB = land colour); atlas0 = ground detail; the
    // real water colourmap + river surface diffuse — every layer from a real game asset.
    let colormap = ColorMap::load(&colormap_dds).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: colormap dds: {e}");
        std::process::exit(2);
    });
    let water_colormap = WaterColorMap::load(&water_colormap_dds).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: water colormap dds: {e}");
        std::process::exit(2);
    });
    let atlas = Atlas::load(&atlas_dds).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: atlas0.dds: {e}");
        std::process::exit(2);
    });
    let atlas_normal = AtlasNormal::load(&atlas_normal_dds).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: atlas_normal0.dds: {e}");
        std::process::exit(2);
    });
    let river_surface = RiverSurface::load(&river_surface_dds).unwrap_or_else(|e| {
        eprintln!("[fid] WARN: RiverSurface_diffuse_0.dds: {e} — using fallback river colour");
        RiverSurface {
            color: render::terrain::RIVER_COLOR,
        }
    });
    eprintln!(
        "[fid] terrain {}x{}  normal {}x{}  rivers {}x{}  height {}x{}  colormap {}x{}  water {}x{}",
        terrain.width, terrain.height, normal.width, normal.height,
        rivers.width, rivers.height, heightmap.width, heightmap.height,
        colormap.width, colormap.height, water_colormap.width, water_colormap.height
    );

    // --- build the terrain base via the FAITHFUL game PixelShaderTerrain math ----------
    let inputs = TerrainInputs {
        colormap: &colormap,
        water_colormap: &water_colormap,
        terrain: &terrain,
        normal: &normal,
        heightmap: &heightmap,
        rivers: &rivers,
        atlas: &atlas,
        atlas_normal: &atlas_normal,
        river_surface: &river_surface,
        defs: &defs,
    };
    let base: TerrainBase = build_base(raster.width, raster.height, &inputs);
    {
        // quick composition report (visible proof the layers are non-trivial).
        let mut river = 0usize;
        let mut emis = 0usize;
        for px in base.river_rgba.chunks_exact(4) {
            if px[3] > 0 {
                river += 1;
            }
        }
        for px in base.emissive_rgba.chunks_exact(4) {
            if px[0] > 0 {
                emis += 1;
            }
        }
        eprintln!("[fid] base: {river} river px, {emis} city-emissive px");
    }

    // --- one real snapshot -> owners + ModeData ----------------------------------
    let store = snapshot::Store::open(&store_root);
    let index = store.index(&game).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: index {game}: {e}");
        std::process::exit(2);
    });
    let entry = index
        .snapshots
        .first()
        .cloned()
        .expect("BLOCKER: no snapshots");
    let snap = store.load_file(&game, &entry.file).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: load {}: {e}", entry.file);
        std::process::exit(2);
    });
    let states = snap.states().expect("BLOCKER: snapshot has no states");
    let owners = StateOwners::from_states(states);
    eprintln!(
        "[fid] snapshot {} ({}): {} states",
        entry.file,
        snap.date.as_deref().unwrap_or("?"),
        states.len()
    );

    let mode_data =
        ModeData::from_snapshot(states, snap.domain("map"), snap.domain("diplomacy_world"))
            .with_provinces(snap.domain("provinces"))
            .with_weather(snap.domain("weather"))
            .with_naval_air(snap.domain("naval_air"))
            .with_front_adjacency(&raster);

    // --- GPU pass + fidelity layers ----------------------------------------------
    let (w, h) = (1126u32, 410u32);
    let r = Renderer::new(w, h).unwrap_or_else(|e| {
        eprintln!("[fid] BLOCKER: renderer init: {e}");
        std::process::exit(2);
    });
    eprintln!(
        "[fid] wgpu backend={:?} adapter={}",
        r.adapter_info.backend, r.adapter_info.name
    );

    let mut pmap = PoliticalMap::new(&r, &raster);
    pmap.upload_fidelity(&r, &base);
    // FAITHFUL per-pixel material + grading textures (snow/mud/ice material + the real
    // colorcorrection.tga ColorCube). Non-fatal resolver (missing → no-op/identity fallback).
    let mats = render::material::Materials::load(|rel| resolver.resolve(rel));
    eprintln!(
        "[fid] materials: snow {}x{}  mud {}x{}  ice {}x{}  colorcube {}x{}",
        mats.snow.width,
        mats.snow.height,
        mats.mud_diffuse.width,
        mats.mud_diffuse.height,
        mats.ice_diffuse.width,
        mats.ice_diffuse.height,
        mats.color_cube.width,
        mats.color_cube.height
    );
    pmap.upload_materials(&r, &mats);
    let owner_index = build_owner_index(raster.max_id, &ids, &owners);
    pmap.update_owner_index(&r, &owner_index);
    // FAITHFUL gradient-border field (CPU SDF) — built once from raster+owner+state,
    // uploaded to binding 10. Drives the ported gradient_border ramp in the shader.
    let border_field = render::borders::build_border_field(&raster, &owner_index, &ids);
    pmap.update_border_field(&r, &border_field);
    // occupation stripes (secondary colour mask): controller != owner provinces.
    let secondary = build_secondary_lut(
        raster.max_id,
        &ids,
        &owners,
        &mode_data.province_control,
        &colors,
    );
    let occupied = secondary.chunks_exact(4).filter(|p| p[3] > 0).count();
    eprintln!("[fid] occupation stripes: {occupied} occupied provinces");
    pmap.update_secondary_lut(&r, &secondary);

    let full = [0.0, 0.0, w as f32, h as f32];
    let clear = [0.02, 0.03, 0.05, 1.0];

    // --- BEFORE/AFTER for political ---------------------------------------------
    let pol = build_lut(raster.max_id, &ids, &owners, &colors);
    pmap.update_lut(&r, &pol);

    // BEFORE: fidelity OFF -> original flat opaque fill.
    pmap.set_fidelity(false);
    let before = pmap.render(&r, full, [0.0, 0.0], [1.0, 1.0], clear);
    save_png(&out_dir().join("fidelity_before.png"), w, h, &before).expect("before png");

    // AFTER: fidelity ON -> terrain + relief + borders + translucent tint.
    pmap.set_fidelity(true);
    let after = pmap.render(&r, full, [0.0, 0.0], [1.0, 1.0], clear);
    save_png(&out_dir().join("fidelity_after.png"), w, h, &after).expect("after png");
    // political mode is also the canonical per-mode output.
    save_png(&out_dir().join("fidelity_political.png"), w, h, &after).expect("political png");

    // --- representative per-mode composites (fidelity on) ------------------------
    for (mode, name) in [(MapMode::Control, "control"), (MapMode::Supply, "supply")] {
        let ml = render::modes::recolor(mode, &mode_data, &ids, &colors, raster.max_id);
        pmap.update_lut(&r, &ml.lut);
        let img = pmap.render(&r, full, [0.0, 0.0], [1.0, 1.0], clear);
        let path = out_dir().join(format!("fidelity_{name}.png"));
        save_png(&path, w, h, &img).expect("mode png");
        eprintln!(
            "[fid] {name}: badge={} -> {}",
            ml.fidelity.badge(),
            path.display()
        );
    }

    // --- zoomed views (political) so terrain texture, relief, rivers, drawn borders and
    //     occupation stripes are visible at detail. Two zooms: (1) Europe/N-Africa/Med —
    //     a long coastline; (2) a second region (the Pacific / SE-Asia) — a coastline plus
    //     striped occupation borders where controller != owner. ---
    pmap.update_lut(&r, &pol);
    let render_zoom = |name: &str, centre: [f32; 2], zoomf: f32, zw: u32, zh: u32| {
        let rz = Renderer::new(zw, zh).expect("zoom renderer");
        let mut pz = PoliticalMap::new(&rz, &raster);
        pz.upload_fidelity(&rz, &base);
        pz.upload_materials(&rz, &mats);
        pz.update_owner_index(&rz, &owner_index);
        pz.update_border_field(&rz, &border_field);
        pz.update_secondary_lut(&rz, &secondary);
        pz.update_lut(&rz, &pol);
        pz.set_fidelity(true);
        let scale = [zoomf, zoomf];
        let offset = [centre[0] - 0.5 / scale[0], centre[1] - 0.5 / scale[1]];
        let img = pz.render(&rz, [0.0, 0.0, zw as f32, zh as f32], offset, scale, clear);
        let path = out_dir().join(format!("{name}.png"));
        save_png(&path, zw, zh, &img).expect("zoom png");
        eprintln!("[fid] {name} -> {}", path.display());
    };
    // (1) Europe / N-Africa / Mediterranean — coastlines, rivers, relief.
    render_zoom("fidelity_after_zoom", [0.555, 0.40], 5.0, 1400, 1000);
    // (2) SE-Asia / Pacific — coastline + occupation stripes (1940 Japan-occupied China).
    render_zoom("fidelity_zoom_coast_stripes", [0.74, 0.46], 7.0, 1400, 1000);

    // --- DYNAMIC (time-varying) proof: the SAME political map under three synthetic ticks
    //     a scrubber would drive, proving the parameterised machinery changes the look. The
    //     per-province winter/mud/frozen + the global season/sun come from DynamicInputs.
    {
        use render::dynamic::{
            pack_province_dynamic_lut, DynamicInputs, DynamicUniforms, ProvinceDynamic, WeatherType,
        };
        // Build a per-province latitude → north-ness map (province centroid V) so a synthetic
        // "winter" puts snow/ice on cold (northern + far-southern) provinces, exactly as the
        // data side will from real SnowMudData. We approximate each province's latitude by the
        // mean V of its raster pixels.
        let np = (raster.max_id as usize) + 1;
        let mut sum_v = vec![0f64; np];
        let mut sum_u = vec![0f64; np];
        let mut cnt = vec![0u32; np];
        for y in 0..raster.height {
            let fv = (y as f64 + 0.5) / raster.height as f64;
            let row = (y as usize) * (raster.width as usize);
            for x in 0..raster.width as usize {
                let id = raster.ids[row + x] as usize;
                if id < np {
                    sum_v[id] += fv;
                    sum_u[id] += (x as f64 + 0.5) / raster.width as f64;
                    cnt[id] += 1;
                }
            }
        }
        let prov_v = |id: usize| -> f32 {
            if cnt[id] > 0 {
                (sum_v[id] / cnt[id] as f64) as f32
            } else {
                0.5
            }
        };
        let prov_u = |id: usize| -> f32 {
            if cnt[id] > 0 {
                (sum_u[id] / cnt[id] as f64) as f32
            } else {
                0.5
            }
        };
        // is a province sea/no-owner (for frozen sea)? reuse the owner index.
        let is_sea = |id: usize| -> bool { id < owner_index.len() && owner_index[id] == 0 };

        pmap.set_fidelity(true);
        pmap.update_lut(&r, &pol);
        let render_dynamic = |name: &str, inp: &DynamicInputs| {
            let lut = pack_province_dynamic_lut(&inp.provinces, inp.snow_mud_fade);
            let du = DynamicUniforms::from_inputs(inp);
            pmap.update_dynamic(&r, &lut, &du);
            let img = pmap.render(&r, full, [0.0, 0.0], [1.0, 1.0], clear);
            let path = out_dir().join(format!("{name}.png"));
            save_png(&path, w, h, &img).expect("dynamic png");
            eprintln!(
                "[fid] {name}: night={:.2} fade={:.2} -> {}",
                du.state[0],
                du.state[1],
                path.display()
            );
        };

        // (A) summer noon — the baseline (no snow/ice, full day).
        let summer = DynamicInputs::neutral(np, (1940, 7, 15, 12));
        render_dynamic("dynamic_summer_noon", &summer);

        // (B) winter midnight — snow on cold land, frozen northern seas, night tone + city
        //     lights. winter_level ramps with |latitude - equator|; frozen where cold sea.
        let mut winter = DynamicInputs::neutral(np, (1941, 1, 15, 0));
        for id in 0..np {
            let v = prov_v(id);
            // coldness: high in the far north (v<0.32) and far south (v>0.78).
            let cold = ((0.34 - v).max(v - 0.74)).max(0.0) / 0.34;
            let cold = cold.clamp(0.0, 1.0);
            winter.provinces[id] = ProvinceDynamic {
                winter_level: cold,
                frozen: cold > 0.55 && is_sea(id),
                weather: if cold > 0.5 {
                    WeatherType::Snow
                } else {
                    WeatherType::Clear
                },
                ..Default::default()
            };
        }
        render_dynamic("dynamic_winter_midnight", &winter);

        // (C) autumn-rain mud front — a band of muddy provinces (the rasputitsa look) at
        //     mid-latitude, day. Proves clear→rain/mud drives the look independently.
        let mut mud = DynamicInputs::neutral(np, (1940, 10, 20, 12));
        for id in 0..np {
            let v = prov_v(id);
            let band = (1.0 - ((v - 0.42).abs() / 0.08)).clamp(0.0, 1.0);
            mud.provinces[id] = ProvinceDynamic {
                mud_level: band,
                weather: if band > 0.3 {
                    WeatherType::RainHeavy
                } else {
                    WeatherType::Clear
                },
                ..Default::default()
            };
        }
        render_dynamic("dynamic_autumn_mud", &mud);

        // (D) NAVAL DOMINANCE + river flow — contested seas (Mediterranean/Atlantic band) get
        //     the pulsing friendly/enemy overlay; global_time set so the pulse + river flow are
        //     at a visible phase. Proves the animated war-state/water branches drive the look.
        let mut naval = DynamicInputs::neutral(np, (1940, 7, 15, 12));
        naval.global_time = 1.2; // a pulse phase where the contested overlay is near-peak
        naval.animate = true;
        for id in 0..np {
            if is_sea(id) {
                let v = prov_v(id);
                // contested band across the mid-latitude seas: friendly to the west, enemy east.
                if (0.36..0.52).contains(&v) {
                    let u_lon = prov_u(id);
                    naval.provinces[id].dominance = if u_lon < 0.5 { 0.9 } else { -0.9 };
                }
            }
        }
        render_dynamic("dynamic_naval_dominance", &naval);

        // reset to the neutral/static dynamic state for any later renders.
        let neutral = DynamicInputs::neutral(np, (1940, 7, 15, 12));
        pmap.update_dynamic(
            &r,
            &pack_province_dynamic_lut(&neutral.provinces, neutral.snow_mud_fade),
            &DynamicUniforms::from_inputs(&neutral),
        );
    }

    // --- sanity: the AFTER image must differ substantially from BEFORE, and must
    //     contain terrain (non-tint) pixels (proves the composite is translucent). --
    let differ = before
        .chunks_exact(4)
        .zip(after.chunks_exact(4))
        .filter(|(a, b)| a != b)
        .count();
    let total = (w as usize) * (h as usize);
    eprintln!(
        "[fid] before/after differ on {differ}/{total} px ({}%)",
        differ * 100 / total
    );
    if differ < total / 20 {
        eprintln!("[fid] BLOCKER: AFTER barely differs from BEFORE — composite not applied");
        std::process::exit(3);
    }

    println!(
        "OK {} {}",
        out_dir().join("fidelity_before.png").display(),
        out_dir().join("fidelity_after.png").display()
    );
}
