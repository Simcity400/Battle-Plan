//! m4_demo — the M4 GPU political map, end to end against REAL data.
//!
//!   * loads `province_ids.png` as an integer (R32Uint) texture,
//!   * builds the per-province colour LUT from a real snapshot
//!     (province → state → owner → country colour), no-fabrication,
//!   * renders the full-world political map to `out/m4_political_map.png`,
//!   * renders a zoomed-in view (pan/zoom uniforms) to `out/m4_political_map_zoom.png`,
//!   * CPU-picks several ground-truth coordinates → province → state → owner / name
//!     and verifies them against the snapshot (NO GPU readback),
//!   * prints the hover state name for each picked province.
//!
//! Paths come from CLI flags / the documented defaults — no personal literal in
//! committed source. READ-ONLY on the map assets and the snapshot store.

use std::path::{Path, PathBuf};

use render::map::{
    build_lut, pick_at, CountryColors, MapIds, PoliticalMap, ProvinceRaster, StateOwners,
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

/// The repo root is two levels up from this crate (replay_native/render -> repo).
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent() // replay_native
        .and_then(|p| p.parent()) // repo root (battle-plan)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn out_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("out")
}

fn main() {
    // --- Resolve inputs (overridable, never a personal literal). -------------
    let map_dir = arg_value("--map")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("map_pipeline").join("assets"));
    let store_root = arg_value("--store")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("replay").join("snapshots"));
    let game = arg_value("--game").unwrap_or_else(|| "campaign".to_string());

    eprintln!("[m4] map_dir = {}", map_dir.display());
    eprintln!("[m4] store   = {} (game {})", store_root.display(), game);

    if !map_dir.join("province_ids.png").is_file() {
        eprintln!(
            "[m4] BLOCKER: province_ids.png not found under {}. Pass --map <dir>.",
            map_dir.display()
        );
        std::process::exit(2);
    }

    // --- Load the integer province-id raster. --------------------------------
    let raster = match ProvinceRaster::load_png(&map_dir.join("province_ids.png")) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[m4] BLOCKER: failed to load province_ids.png: {e}");
            std::process::exit(2);
        }
    };
    eprintln!(
        "[m4] raster {}x{}  max_province_id={}  (LUT width {})",
        raster.width,
        raster.height,
        raster.max_id,
        raster.max_id + 1
    );

    // --- Load the typed map-id lookups + country colours. --------------------
    let ids = MapIds::load(&map_dir).unwrap_or_else(|e| {
        eprintln!("[m4] BLOCKER: MapIds::load: {e}");
        std::process::exit(2);
    });
    let colors = CountryColors::load(&map_dir).unwrap_or_else(|e| {
        eprintln!("[m4] BLOCKER: CountryColors::load: {e}");
        std::process::exit(2);
    });
    eprintln!("[m4] land provinces = {}", ids.province_count());

    // --- Load one real snapshot and derive owners. ---------------------------
    let store = snapshot::Store::open(&store_root);
    let index = store.index(&game).unwrap_or_else(|e| {
        eprintln!("[m4] BLOCKER: cannot read index for game {game}: {e}");
        std::process::exit(2);
    });
    let entry = index
        .snapshots
        .first()
        .cloned()
        .expect("BLOCKER: no snapshots in index");
    let snap = store.load_file(&game, &entry.file).unwrap_or_else(|e| {
        eprintln!("[m4] BLOCKER: cannot load snapshot {}: {e}", entry.file);
        std::process::exit(2);
    });
    let states = snap
        .states()
        .expect("BLOCKER: snapshot has no states domain");
    let owners = StateOwners::from_states(states);
    eprintln!(
        "[m4] snapshot {} ({}): {} states",
        entry.file,
        snap.date.as_deref().unwrap_or("?"),
        states.len()
    );

    // --- Build the colour LUT (no-fabrication). ------------------------------
    let lut = build_lut(raster.max_id, &ids, &owners, &colors);
    // Report the LUT composition so the no-fabrication split is visible.
    let (mut terrain, mut grey, mut country) = (0usize, 0usize, 0usize);
    for px in lut.chunks_exact(4) {
        if px == render::map::NEUTRAL_TERRAIN {
            terrain += 1;
        } else if px == render::map::NEUTRAL_GREY {
            grey += 1;
        } else {
            country += 1;
        }
    }
    eprintln!(
        "[m4] LUT: {country} country-coloured, {grey} neutral-grey (land/unowned), {terrain} neutral-terrain (sea/unassigned)"
    );

    // --- GPU: build the pass, upload the LUT, render two views. --------------
    // Output target: a 1126x410 canvas (map fills it). Half-res of the raster's
    // 2.75:1 aspect keeps the demo PNG a sane size while proving the shader.
    let (w, h) = (1126u32, 410u32);
    let r = match Renderer::new(w, h) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[m4] BLOCKER: renderer init failed: {e}");
            std::process::exit(2);
        }
    };
    eprintln!(
        "[m4] wgpu backend={:?} adapter={} ({:?})",
        r.adapter_info.backend, r.adapter_info.name, r.adapter_info.device_type
    );
    let pmap = PoliticalMap::new(&r, &raster);
    pmap.update_lut(&r, &lut);

    // (5a) Full-world political map.
    let full = pmap.render(
        &r,
        [0.0, 0.0, w as f32, h as f32],
        [0.0, 0.0], // no pan
        [1.0, 1.0], // no zoom
        [0.05, 0.06, 0.08, 1.0],
    );
    let full_path = out_dir().join("m4_political_map.png");
    save_png(&full_path, w, h, &full).expect("write full map png");

    // (5b) Zoomed-in view — Europe-ish. Centre ~ (0.55, 0.45) in map UV, zoom 4x.
    // map_uv = out_uv/scale + offset; to centre the region at (cx,cy) with scale s:
    //   offset = centre - 0.5/s.
    let scale = [4.0f32, 4.0];
    let centre = [0.555f32, 0.46];
    let offset = [centre[0] - 0.5 / scale[0], centre[1] - 0.5 / scale[1]];
    let zoom = pmap.render(
        &r,
        [0.0, 0.0, w as f32, h as f32],
        offset,
        scale,
        [0.05, 0.06, 0.08, 1.0],
    );
    let zoom_path = out_dir().join("m4_political_map_zoom.png");
    save_png(&zoom_path, w, h, &zoom).expect("write zoom map png");

    // Sanity: both images must have plenty of country-coloured pixels.
    let nonblank = |buf: &[u8]| {
        buf.chunks_exact(4)
            .filter(|p| {
                let q = [p[0], p[1], p[2], p[3]];
                q != render::map::NEUTRAL_TERRAIN && !(q[0] < 16 && q[1] < 16 && q[2] < 24)
                // not the clear colour
            })
            .count()
    };
    eprintln!(
        "[m4] wrote {} ({} land-ish px)",
        full_path.display(),
        nonblank(&full)
    );
    eprintln!(
        "[m4] wrote {} ({} land-ish px)",
        zoom_path.display(),
        nonblank(&zoom)
    );

    // --- (6) CPU pick verification at GROUND-TRUTH coordinates. --------------
    // These (x,y) were pre-verified against the raster + this snapshot in Python.
    // The pick must reproduce the exact province / state / owner chain.
    struct Truth {
        x: u32,
        y: u32,
        pid: u32,
        sid: u32,
        owner: &'static str,
        name: &'static str,
    }
    let truths = [
        Truth {
            x: 1100,
            y: 258,
            pid: 9669,
            sid: 469,
            owner: "CAN",
            name: "STATE_469",
        },
        Truth {
            x: 1719,
            y: 384,
            pid: 12221,
            sid: 357,
            owner: "USA",
            name: "STATE_357",
        },
        Truth {
            x: 3996,
            y: 116,
            pid: 3415,
            sid: 824,
            owner: "SOV",
            name: "STATE_824",
        },
        Truth {
            x: 3193,
            y: 1772,
            pid: 12963,
            sid: 542,
            owner: "ENG",
            name: "STATE_542",
        },
        Truth {
            x: 3122,
            y: 887,
            pid: 4120,
            sid: 450,
            owner: "ITA",
            name: "STATE_450",
        },
        Truth {
            x: 4322,
            y: 908,
            pid: 5102,
            sid: 601,
            owner: "JAP",
            name: "STATE_601",
        },
        Truth {
            x: 2831,
            y: 945,
            pid: 12082,
            sid: 552,
            owner: "WST",
            name: "STATE_552",
        },
        Truth {
            x: 3934,
            y: 994,
            pid: 12886,
            sid: 433,
            owner: "RAJ",
            name: "STATE_433",
        },
    ];
    let mut pick_fail = 0;
    eprintln!("[m4] --- province pick (cursor -> province -> state -> owner / hover) ---");
    for t in &truths {
        match pick_at(&raster, &ids, &owners, t.x, t.y) {
            Some(p) => {
                let ok = p.province_id == t.pid
                    && p.state_id == Some(t.sid)
                    && p.owner.as_deref() == Some(t.owner)
                    && p.state_name_key.as_deref() == Some(t.name);
                eprintln!(
                    "[m4]   ({:>4},{:>4}) -> province {:>5} state {:>4} owner {:<3} hover='{}'  {}",
                    t.x,
                    t.y,
                    p.province_id,
                    p.state_id.map(|s| s as i64).unwrap_or(-1),
                    p.owner.as_deref().unwrap_or("—"),
                    p.state_name_key.as_deref().unwrap_or("—"),
                    if ok { "OK" } else { "MISMATCH" }
                );
                if !ok {
                    pick_fail += 1;
                }
            }
            None => {
                eprintln!("[m4]   ({},{}) -> pick returned None (MISMATCH)", t.x, t.y);
                pick_fail += 1;
            }
        }
    }

    // A sea pick must resolve to NO state / NO owner (never fabricated). Find one:
    // scan a row for a province id absent from the land map.
    let mut sea_demo = None;
    'outer: for y in (0..raster.height).step_by(13) {
        for x in (0..raster.width).step_by(13) {
            let pid = raster.province_at(x, y).unwrap();
            if ids.state_of_province(pid).is_none() {
                sea_demo = Some((x, y, pid));
                break 'outer;
            }
        }
    }
    if let Some((x, y, pid)) = sea_demo {
        let p = pick_at(&raster, &ids, &owners, x, y).unwrap();
        let honest = p.state_id.is_none() && p.owner.is_none();
        eprintln!(
            "[m4]   SEA ({x},{y}) -> province {pid} state {:?} owner {:?}  {}",
            p.state_id,
            p.owner,
            if honest {
                "OK (neutral, not fabricated)"
            } else {
                "MISMATCH"
            }
        );
        if !honest {
            pick_fail += 1;
        }
    }

    if pick_fail > 0 {
        eprintln!("[m4] BLOCKER: {pick_fail} pick verification(s) failed");
        std::process::exit(3);
    }
    if nonblank(&full) < 100_000 {
        eprintln!("[m4] WARNING: full map has suspiciously few land pixels");
        std::process::exit(3);
    }

    println!("OK {} {}", full_path.display(), zoom_path.display());
}
