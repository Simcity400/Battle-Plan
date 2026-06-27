//! modes_demo — Stage 2: render EVERY map mode to an offscreen PNG on a REAL snapshot.
//!
//! For each [`MapMode`] in the switcher cycle (plus every building kind) this:
//!   * extracts the mode's inputs from the snapshot (no-fabrication: Absent/Null/Measured
//!     preserved by [`render::modes::ModeData::from_snapshot`]),
//!   * builds the per-province colour LUT via [`render::modes::recolor`] and reads back
//!     the EFFECTIVE fidelity badge (downgraded to `unavailable` where data is missing),
//!   * uploads the LUT to the SAME province-id GPU pass used by the political map,
//!   * renders the full-world map to `render/out/modes_<slug>.png`,
//!   * prints the active mode + its badge + the LUT composition (country / ramp / grey /
//!     terrain pixel split) so the no-fabrication neutrals are visible.
//!
//! Paths come from CLI flags / documented defaults — no personal literal in source.
//! READ-ONLY on map assets + the snapshot store.

use std::path::{Path, PathBuf};

use render::map::{CountryColors, MapIds, PoliticalMap, ProvinceRaster};
use render::modes::{BuildingKind, MapMode, ModeData};
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
    let map_dir = arg_value("--map")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("map_pipeline").join("assets"));
    let store_root = arg_value("--store")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("replay").join("snapshots"));
    let game = arg_value("--game").unwrap_or_else(|| "campaign".to_string());

    eprintln!("[modes] map_dir = {}", map_dir.display());
    eprintln!("[modes] store   = {} (game {})", store_root.display(), game);

    if !map_dir.join("province_ids.png").is_file() {
        eprintln!(
            "[modes] BLOCKER: province_ids.png not found under {}",
            map_dir.display()
        );
        std::process::exit(2);
    }

    // --- Load the integer province-id raster (retried on transient asset read). ----
    let raster = load_raster_retry(&map_dir.join("province_ids.png"));
    eprintln!(
        "[modes] raster {}x{}  max_province_id={}",
        raster.width, raster.height, raster.max_id
    );

    let ids = MapIds::load(&map_dir).unwrap_or_else(|e| {
        eprintln!("[modes] BLOCKER: MapIds::load: {e}");
        std::process::exit(2);
    });
    let colors = CountryColors::load(&map_dir).unwrap_or_else(|e| {
        eprintln!("[modes] BLOCKER: CountryColors::load: {e}");
        std::process::exit(2);
    });

    // --- Load one real snapshot. -------------------------------------------------
    let store = snapshot::Store::open(&store_root);
    let index = store.index(&game).unwrap_or_else(|e| {
        eprintln!("[modes] BLOCKER: cannot read index for {game}: {e}");
        std::process::exit(2);
    });
    let entry = index
        .snapshots
        .first()
        .cloned()
        .expect("BLOCKER: no snapshots in index");
    let snap = store.load_file(&game, &entry.file).unwrap_or_else(|e| {
        eprintln!("[modes] BLOCKER: cannot load snapshot {}: {e}", entry.file);
        std::process::exit(2);
    });
    let states = snap
        .states()
        .expect("BLOCKER: snapshot has no states domain");
    eprintln!(
        "[modes] snapshot {} ({}) — {} states",
        entry.file,
        snap.date.as_deref().unwrap_or("?"),
        states.len()
    );

    // --- Build the per-snapshot ModeData ONCE (then reuse for every mode). --------
    let mode_data =
        ModeData::from_snapshot(states, snap.domain("map"), snap.domain("diplomacy_world"))
            .with_provinces(snap.domain("provinces"))
            .with_front_adjacency(&raster);

    // --- GPU pass (shared across all modes; only the LUT changes). ---------------
    let (w, h) = (1126u32, 410u32);
    let r = Renderer::new(w, h).unwrap_or_else(|e| {
        eprintln!("[modes] BLOCKER: renderer init failed: {e}");
        std::process::exit(2);
    });
    eprintln!(
        "[modes] wgpu backend={:?} adapter={}",
        r.adapter_info.backend, r.adapter_info.name
    );
    let pmap = PoliticalMap::new(&r, &raster);

    // Every mode in the switcher cycle, PLUS every building kind (so each is rendered).
    let mut modes: Vec<MapMode> = MapMode::CYCLE.to_vec();
    for k in [
        BuildingKind::Infrastructure,
        BuildingKind::CivilianFactory,
        BuildingKind::MilitaryFactory,
        BuildingKind::Dockyard,
        BuildingKind::AirBase,
        BuildingKind::AntiAir,
    ] {
        let m = MapMode::Buildings(k);
        if !modes.iter().any(|x| x.slug() == m.slug()) {
            modes.push(m);
        }
    }

    let mut rendered = 0usize;
    let mut failures = 0usize;
    for mode in modes {
        let ml = render::modes::recolor(mode, &mode_data, &ids, &colors, raster.max_id);
        // LUT composition (the no-fabrication split made visible).
        let (mut terrain, mut grey, mut colored) = (0usize, 0usize, 0usize);
        for px in ml.lut.chunks_exact(4) {
            if px == render::map::NEUTRAL_TERRAIN {
                terrain += 1;
            } else if px == render::map::NEUTRAL_GREY {
                grey += 1;
            } else {
                colored += 1;
            }
        }
        pmap.update_lut(&r, &ml.lut);
        let img = pmap.render(
            &r,
            [0.0, 0.0, w as f32, h as f32],
            [0.0, 0.0],
            [1.0, 1.0],
            [0.05, 0.06, 0.08, 1.0],
        );
        let path = out_dir().join(format!("modes_{}.png", mode.slug()));
        if let Err(e) = save_png(&path, w, h, &img) {
            eprintln!("[modes] BLOCKER: write {}: {e}", path.display());
            failures += 1;
            continue;
        }
        eprintln!(
            "[modes] {:<22} badge={:<11} LUT[country/ramp={colored} grey={grey} terrain={terrain}]  -> {}",
            mode.label(),
            ml.fidelity.badge(),
            path.file_name().unwrap().to_string_lossy()
        );
        rendered += 1;
    }

    if failures > 0 {
        eprintln!("[modes] BLOCKER: {failures} mode render(s) failed");
        std::process::exit(3);
    }
    eprintln!(
        "[modes] OK — {rendered} modes rendered + badged into {}",
        out_dir().display()
    );
    println!("OK {rendered}");
}

/// Load the raster, retrying once on a transient read failure (another agent may be
/// regenerating `game_assets/`; the map_pipeline raster is separate but we honour the
/// retry rule). NEVER edits the asset.
fn load_raster_retry(path: &Path) -> ProvinceRaster {
    match ProvinceRaster::load_png(path) {
        Ok(r) => r,
        Err(e1) => {
            eprintln!("[modes] raster load failed once ({e1}); retrying…");
            std::thread::sleep(std::time::Duration::from_millis(250));
            ProvinceRaster::load_png(path).unwrap_or_else(|e2| {
                eprintln!("[modes] BLOCKER: raster load failed twice: {e2}");
                std::process::exit(2);
            })
        }
    }
}
