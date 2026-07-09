//! C5 (files-first half) — STATIC railway + supply-hub map GEOMETRY and render
//! law, prepared as a pure function of MOD-RESOLVED shipped installed content.
//!
//! The raw parse of `map/railways.txt` / `map/supply_nodes.txt` already lives in
//! [`crate::map_static_data`] (`RailwaySegment`, `SupplyNode`,
//! `railway_segments_from_resolver`, `supply_nodes_from_resolver`). This module
//! does NOT re-parse; it consumes those and adds the geometry (world-space
//! centerline polylines + hub markers) and the FILE/CAPTURE/UNKNOWN render-law
//! disposition the replay overlay needs, with full referential-integrity and
//! bounds gates on the shipped files.
//!
//! BUILT-rail state over time (which segments are constructed, damaged, on
//! cooldown) is RECORDING data (EXTRACTION_NEEDS A8, extractor-side, stays OPEN);
//! nothing here derives game state.
//!
//! # File formats (verified against the shipped vanilla files)
//!
//! * `map/railways.txt` — one railway per line:
//!   `<level> <province_count> <province_id …×province_count>`. Verified `909`
//!   lines; column 2 equals the trailing-id count on every line; levels observed
//!   `1..=4` (histogram `1:467 2:406 3:29 4:7`). The engine data model allows
//!   level up to 5 (`RAILWAY_MAP_ARROW_THICK_LEVEL_THRESHOLD = 5`).
//! * `map/supply_nodes.txt` — one hub per line: `<flag> <province_id>`. Verified
//!   `727` lines; the leading column (`SupplyNode.level`) is always `1` in
//!   vanilla — an enabled flag whose full domain is engine-side.
//!
//! # Render chain the engine uses (all cited, FILE-derived)
//!
//! The default-map rail overlay is a TEXTURED RIBBON drawn by
//! `gfx/models/supply/railroad.shader` (Effect `railroad`), NOT a flat-colour
//! line:
//! * VS: `Out.vPosition = mul( ViewProjectionMatrix, v.vPosition )` — the ribbon
//!   vertices arrive already in WORLD space, so the engine builds the ribbon VB
//!   on the CPU from the province path. The width-scaling branch in the shipped
//!   VS is commented out ("disabled for now. doesn't look good anyway"): ribbon
//!   width is baked into the VB, not a shader uniform.
//! * VS input `float4 vPosition : POSITION; float2 vUV : TEXCOORD0`; PS samples
//!   `RailTexture` (= `gfx/models/supply/railroad.dds`, 64×320, `AddressU = Wrap`
//!   along track, `AddressV = Wrap` across width) and discards `vUV.y < 0 ||
//!   vUV.y >= 1`. A damaged variant `railroad_damaged.dds` ships alongside
//!   (recording state selects it — A8).
//! * PS `ConstantBuffer(1,32)`: `ShadowMapTextureMatrix`, `vTimeGlobal`,
//!   `vRailwayAlpha`; blend `src_alpha / inv_src_alpha`, `WriteMask RGB`,
//!   `DepthEnable = no`. `vRailwayAlpha` is the camera-cutoff fade.
//! * Supply hubs are the on-map sprite `GFX_supply_hub_icon_strip`
//!   (`interface/supply.gfx`; `…/supply_hub_icon_strip.dds`, `180×36` = 5 frames
//!   of `36×36`).
//!
//! # Level / cutoff semantics (from `common/defines/00_graphics.lua`)
//!
//! Read through the resolver (mod override wins):
//! * `RAILWAY_Y_OFFSET = 0.9` — world-space vertical offset of the ribbon above
//!   the map plane; applied to every polyline point's `y`.
//! * `RAILWAY_CAMERA_CUTOFF = 300.0` / `RAILWAY_CAMERA_CUTOFF_SPEED = 3.0` —
//!   ribbon fades out above camera height 300 (feeds `vRailwayAlpha`).
//! * `RAILWAY_ICON_CUTOFF = 900.0` — supply-hub icon cutoff height.
//! * `RAILWAY_MAP_ARROW_THIN/MEDIUM/THICK_LEVEL_THRESHOLD = 1 / 3 / 5` — bucket
//!   rail level into thin (L1) / medium (L2–3) / thick (L4–5), but the shipped
//!   comment scopes these to the SUPPLY MAP MODE map-arrow renderer, NOT the
//!   default-view `railroad` ribbon. They are FILE evidence of the level→width
//!   intent; the default ribbon's per-level world width is UNKNOWN.
//!
//! # Coordinate convention
//!
//! Identical to `map_furniture` / `weather_law`: world XZ = province-raster pixel
//! space, `x` right in `[0, 5632]`, `z` down in `[0, 2048]`, 1:1 for the vanilla
//! `provinces.bmp`; `y` is height (here `RAILWAY_Y_OFFSET`). Each polyline point
//! is `[x, RAILWAY_Y_OFFSET, z]`.
//!
//! # Honesty / fail-closed
//!
//! * A railway whose path references a province absent from the raster yields NO
//!   polyline (reported as a gap), never a guessed point.
//! * The per-province anchor is the pixel CENTROID — a DERIVED_APPROXIMATION of
//!   where the engine threads the rail through the province; the exact entry/exit
//!   point and any inter-centroid curve are UNKNOWN, so the polyline is emitted
//!   as straight centroid-to-centroid segments, labelled.
//! * Ribbon width, UV tiling rate, and the supply-hub icon frame index are all
//!   UNKNOWN (engine-side) and carried as such with named closure paths.

use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const DEFINITION_CSV_SOURCE: &str = "map/definition.csv";
pub const PROVINCES_BMP_SOURCE: &str = "map/provinces.bmp";
/// `gfx/models/supply/railroad.shader` — Effect `railroad`, the textured ribbon.
pub const RAIL_SHADER_SOURCE: &str = "gfx/models/supply/railroad.shader";
pub const RAIL_SHADER_EFFECT: &str = "railroad";
/// `RailTexture` sampler source (64×320, Wrap/Wrap).
pub const RAIL_TEXTURE_SOURCE: &str = "gfx/models/supply/railroad.dds";
/// Damaged-segment variant (recording state selects it — A8).
pub const RAIL_TEXTURE_DAMAGED_SOURCE: &str = "gfx/models/supply/railroad_damaged.dds";
/// `interface/supply.gfx` — declares `GFX_supply_hub_icon_strip`.
pub const SUPPLY_GFX_SOURCE: &str = "interface/supply.gfx";
pub const SUPPLY_HUB_ICON_SPRITE: &str = "GFX_supply_hub_icon_strip";
/// `gfx/interface/mapicons/supply/supply_hub_icon_strip.dds` (180×36, 5 frames).
pub const SUPPLY_HUB_ICON_TEXTURE: &str = "gfx/interface/mapicons/supply/supply_hub_icon_strip.dds";
pub const SUPPLY_HUB_ICON_FRAMES: u32 = 5;

pub const GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";

/// Engine data model allows rail level up to `RAILWAY_MAP_ARROW_THICK_LEVEL_THRESHOLD`.
pub const RAIL_LEVEL_MAX: u32 = 5;

/// Vanilla province-raster extent (`provinces.bmp`), the world XZ bounds.
pub const MAP_WIDTH: usize = 5632;
pub const MAP_HEIGHT: usize = 2048;

/// FILE-derived rail geometry/cutoff defines (`00_graphics.lua`), resolver-read.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RailGeometryDefines {
    /// `RAILWAY_Y_OFFSET` — ribbon height above the map plane.
    pub y_offset: f32,
    /// `RAILWAY_CAMERA_CUTOFF` — camera height above which the ribbon fades out.
    pub camera_cutoff: f32,
    /// `RAILWAY_CAMERA_CUTOFF_SPEED` — fade in/out speed.
    pub camera_cutoff_speed: f32,
    /// `RAILWAY_ICON_CUTOFF` — supply-hub icon cutoff height.
    pub icon_cutoff: f32,
    /// `RAILWAY_MAP_ARROW_THIN_LEVEL_THRESHOLD` (supply-map-mode arrow only).
    pub arrow_thin_threshold: f32,
    /// `RAILWAY_MAP_ARROW_MEDIUM_LEVEL_THRESHOLD`.
    pub arrow_medium_threshold: f32,
    /// `RAILWAY_MAP_ARROW_THICK_LEVEL_THRESHOLD`.
    pub arrow_thick_threshold: f32,
}

/// A single railway's world-space centerline. `points[i]` corresponds to
/// `provinces[i]`'s pixel centroid, lifted to `y = defines.y_offset`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RailPolyline {
    pub level: u32,
    /// The province path (all present in the raster; fail-closed otherwise).
    pub provinces: Vec<u32>,
    /// World `[x, y_offset, z]` points, one per province.
    pub points: Vec<[f32; 3]>,
    /// 1-based index of the source railway segment (file order).
    pub segment_index: usize,
}

/// A supply-hub marker at a node province's world centroid.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SupplyHubMarker {
    pub province: u32,
    pub pos: [f32; 3],
    /// 1-based index of the source supply node (file order).
    pub node_index: usize,
}

/// Pixel centroid of a province in the raster (a DERIVED_APPROXIMATION anchor).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProvinceCentroid {
    pub x: f32,
    pub z: f32,
    pub pixel_count: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RailFileDerivedStats {
    pub schema: String,
    pub railway_count: usize,
    /// level -> number of railways at that level.
    pub level_histogram: BTreeMap<u32, usize>,
    pub supply_node_count: usize,
    pub distinct_rail_provinces: usize,
    pub distinct_supply_provinces: usize,
    /// Segments whose level is outside the shipped `1..=RAIL_LEVEL_MAX` range,
    /// as `(segment_index, level)`. Empty on vanilla.
    pub invalid_levels: Vec<(usize, u32)>,
    /// Segments with fewer than 2 path provinces (cannot form a segment), by
    /// 1-based index. Empty on vanilla.
    pub degenerate_paths: Vec<usize>,
    /// Rail/supply provinces referenced by the files but absent from
    /// `definition.csv` (should be empty on vanilla).
    pub provinces_missing_from_definition: Vec<u32>,
    /// Rail/supply provinces defined but painted on zero raster pixels (no
    /// centroid; their geometry is fail-closed dropped).
    pub provinces_absent_from_raster: Vec<u32>,
    pub rail_polyline_count: usize,
    /// Railways dropped because a path province had no centroid.
    pub rail_polylines_dropped: usize,
    pub supply_hub_marker_count: usize,
    pub defines: RailGeometryDefines,
    /// FILE/CAPTURE/UNKNOWN render-law disposition, machine-readable.
    pub render_law: Vec<RailLawRow>,
    pub gaps: Vec<String>,
}

/// One render-law parameter with its class and citation/closure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RailLawRow {
    pub parameter: String,
    /// `FILE`, `CAPTURE`, or `UNKNOWN`.
    pub class: String,
    pub evidence_or_closure: String,
}

#[derive(Clone, Debug)]
pub struct RailFileDerivedLayer {
    pub railways: Vec<RailwaySegment>,
    pub supply_nodes: Vec<SupplyNode>,
    pub polylines: Vec<RailPolyline>,
    pub hub_markers: Vec<SupplyHubMarker>,
    pub stats: RailFileDerivedStats,
}

/// Read the rail defines from `00_graphics.lua` (resolver-routed, mod override
/// wins). Fails closed if any define is missing or non-numeric.
pub fn rail_geometry_defines_from_resolver(
    resolver: &GameFileResolver,
) -> Result<RailGeometryDefines> {
    let path = resolver.resolve_required(GRAPHICS_DEFINES_SOURCE, "graphics defines")?;
    parse_rail_geometry_defines(&read_latin1(&path)?)
}

pub fn parse_rail_geometry_defines(source: &str) -> Result<RailGeometryDefines> {
    Ok(RailGeometryDefines {
        y_offset: parse_lua_number_define(source, "RAILWAY_Y_OFFSET")?,
        camera_cutoff: parse_lua_number_define(source, "RAILWAY_CAMERA_CUTOFF")?,
        camera_cutoff_speed: parse_lua_number_define(source, "RAILWAY_CAMERA_CUTOFF_SPEED")?,
        icon_cutoff: parse_lua_number_define(source, "RAILWAY_ICON_CUTOFF")?,
        arrow_thin_threshold: parse_lua_number_define(
            source,
            "RAILWAY_MAP_ARROW_THIN_LEVEL_THRESHOLD",
        )?,
        arrow_medium_threshold: parse_lua_number_define(
            source,
            "RAILWAY_MAP_ARROW_MEDIUM_LEVEL_THRESHOLD",
        )?,
        arrow_thick_threshold: parse_lua_number_define(
            source,
            "RAILWAY_MAP_ARROW_THICK_LEVEL_THRESHOLD",
        )?,
    })
}

/// One raster pass computing pixel centroids for exactly the `wanted` provinces.
/// `province_ids` is row-major `width*height`, `0` = no province.
pub fn compute_province_centroids(
    province_ids: &[u32],
    width: usize,
    height: usize,
    wanted: &BTreeSet<u32>,
) -> BTreeMap<u32, ProvinceCentroid> {
    let mut acc: BTreeMap<u32, (f64, f64, u64)> = BTreeMap::new();
    if province_ids.len() != width.saturating_mul(height) {
        return BTreeMap::new();
    }
    for y in 0..height {
        let row = y * width;
        for x in 0..width {
            let province = province_ids[row + x];
            if province == 0 || !wanted.contains(&province) {
                continue;
            }
            let entry = acc.entry(province).or_insert((0.0, 0.0, 0));
            entry.0 += x as f64;
            entry.1 += y as f64;
            entry.2 += 1;
        }
    }
    acc.into_iter()
        .map(|(id, (sx, sz, count))| {
            (
                id,
                ProvinceCentroid {
                    x: (sx / count as f64) as f32,
                    z: (sz / count as f64) as f32,
                    pixel_count: count,
                },
            )
        })
        .collect()
}

/// Build one world-space polyline per railway. A railway whose path references a
/// province with no centroid is dropped (fail-closed) and reported.
pub fn rail_polylines(
    railways: &[RailwaySegment],
    centroids: &BTreeMap<u32, ProvinceCentroid>,
    defines: &RailGeometryDefines,
) -> (Vec<RailPolyline>, Vec<String>) {
    let mut polylines = Vec::new();
    let mut gaps = Vec::new();
    for (idx, railway) in railways.iter().enumerate() {
        let segment_index = idx + 1;
        if railway.provinces.len() < 2 {
            // A single-province "path" cannot form a ribbon segment; skip it
            // fail-closed rather than emit a zero-length polyline.
            gaps.push(format!(
                "railway #{segment_index} (level {}) has {} province(s); needs >=2 for a segment",
                railway.level,
                railway.provinces.len()
            ));
            continue;
        }
        let mut points = Vec::with_capacity(railway.provinces.len());
        let mut missing = Vec::new();
        for province in &railway.provinces {
            match centroids.get(province) {
                Some(c) => points.push([c.x, defines.y_offset, c.z]),
                None => missing.push(*province),
            }
        }
        if !missing.is_empty() {
            gaps.push(format!(
                "railway #{segment_index} (level {}) fail-closed: path provinces absent from raster: {:?}",
                railway.level, missing
            ));
            continue;
        }
        polylines.push(RailPolyline {
            level: railway.level,
            provinces: railway.provinces.clone(),
            points,
            segment_index,
        });
    }
    (polylines, gaps)
}

/// Build one hub marker per supply node with a centroid; drop (report) the rest.
pub fn supply_hub_markers(
    supply_nodes: &[SupplyNode],
    centroids: &BTreeMap<u32, ProvinceCentroid>,
    defines: &RailGeometryDefines,
) -> (Vec<SupplyHubMarker>, Vec<String>) {
    let mut markers = Vec::new();
    let mut gaps = Vec::new();
    for (idx, node) in supply_nodes.iter().enumerate() {
        let node_index = idx + 1;
        match centroids.get(&node.province_id) {
            Some(c) => markers.push(SupplyHubMarker {
                province: node.province_id,
                pos: [c.x, defines.y_offset, c.z],
                node_index,
            }),
            None => gaps.push(format!(
                "supply node #{node_index} fail-closed: province {} absent from raster",
                node.province_id
            )),
        }
    }
    (markers, gaps)
}

/// The static FILE/CAPTURE/UNKNOWN render-law disposition, shared by tooling and
/// any future replay overlay so both cite one law.
pub fn rail_render_law(defines: &RailGeometryDefines) -> Vec<RailLawRow> {
    let f = |p: &str, c: &str, e: String| RailLawRow {
        parameter: p.to_string(),
        class: c.to_string(),
        evidence_or_closure: e,
    };
    vec![
        f(
            "railway path + level",
            "FILE",
            "map/railways.txt: <level> <count> <province…> (parsed by map_static_data::railway_segments_*)".to_string(),
        ),
        f(
            "supply hub provinces",
            "FILE",
            "map/supply_nodes.txt: <flag> <province> (parsed by map_static_data::supply_nodes_*)".to_string(),
        ),
        f(
            "ribbon draw shader/texture",
            "FILE",
            format!("{RAIL_SHADER_SOURCE} (Effect {RAIL_SHADER_EFFECT}) samples RailTexture={RAIL_TEXTURE_SOURCE}; damaged={RAIL_TEXTURE_DAMAGED_SOURCE}"),
        ),
        f(
            "ribbon vertical offset (y)",
            "FILE",
            format!("RAILWAY_Y_OFFSET={} ({GRAPHICS_DEFINES_SOURCE})", defines.y_offset),
        ),
        f(
            "ribbon camera-cutoff fade",
            "FILE",
            format!(
                "RAILWAY_CAMERA_CUTOFF={} / RAILWAY_CAMERA_CUTOFF_SPEED={} -> vRailwayAlpha ({GRAPHICS_DEFINES_SOURCE})",
                defines.camera_cutoff, defines.camera_cutoff_speed
            ),
        ),
        f(
            "supply-hub icon sprite",
            "FILE",
            format!("{SUPPLY_HUB_ICON_SPRITE} ({SUPPLY_GFX_SOURCE}); {SUPPLY_HUB_ICON_TEXTURE}, {SUPPLY_HUB_ICON_FRAMES} frames; icon cutoff RAILWAY_ICON_CUTOFF={}", defines.icon_cutoff),
        ),
        f(
            "level->arrow width thresholds",
            "FILE",
            format!(
                "RAILWAY_MAP_ARROW_THIN/MEDIUM/THICK_LEVEL_THRESHOLD={}/{}/{} — SUPPLY MAP MODE arrows only, NOT the default ribbon",
                defines.arrow_thin_threshold, defines.arrow_medium_threshold, defines.arrow_thick_threshold
            ),
        ),
        f(
            "per-province rail anchor",
            "UNKNOWN",
            "polyline uses pixel CENTROID = DERIVED_APPROXIMATION; engine's exact per-province rail entry/exit point untraced. Close: capture rail VB or binary RE of the ribbon builder (RE target: railroad-ribbon CPU VB build in hoi4.exe).".to_string(),
        ),
        f(
            "default-ribbon world width per level",
            "UNKNOWN",
            "railroad.shader width-scale branch is disabled; ribbon width is baked into the engine-built VB, not in files. Close: capture the rail draw VB (plan CAP9 cities/rails family) or RE the ribbon builder.".to_string(),
        ),
        f(
            "inter-centroid curve/smoothing",
            "UNKNOWN",
            "polyline emitted as straight centroid-to-centroid segments; any engine spline/curve untraced. Close: capture rail VB / RE.".to_string(),
        ),
        f(
            "RailTexture UV tiling rate along track",
            "UNKNOWN",
            "vUV.x wrap rate is baked into the VB; not in files. Close: capture rail VB / RE.".to_string(),
        ),
        f(
            "supply-hub icon frame index (0..4)",
            "UNKNOWN",
            "which of the 5 strip frames a hub shows is engine state (hub level/status). Close: RE or a labelled supply-map capture.".to_string(),
        ),
        f(
            "built-rail state over time (constructed/damaged/cooldown)",
            "UNKNOWN",
            "RECORDING data (EXTRACTION_NEEDS A8, extractor-side) — not derivable from files; out of this lane's scope.".to_string(),
        ),
    ]
}

/// Full files-first pipeline: parse both files (via `map_static_data`), load the
/// province raster, compute centroids, build polylines + hub markers, and
/// assemble stats. Deterministic and fail-closed; every referenced province is
/// validated against `definition.csv` and the raster.
pub fn rail_file_derived_layer_from_resolver(
    resolver: &GameFileResolver,
) -> Result<RailFileDerivedLayer> {
    let railways = railway_segments_from_resolver(resolver)?;
    let supply_nodes = supply_nodes_from_resolver(resolver)?;
    let defines = rail_geometry_defines_from_resolver(resolver)?;

    let definition_csv = resolver.resolve_required(DEFINITION_CSV_SOURCE, "definition.csv")?;
    let provinces_bmp = resolver.resolve_required(PROVINCES_BMP_SOURCE, "provinces.bmp")?;
    let (definitions, color_to_id) = load_definition_csv(&definition_csv)?;
    let raster = load_provinces_bmp(&provinces_bmp, &color_to_id)?;

    // Referenced province sets.
    let mut rail_provinces = BTreeSet::new();
    for railway in &railways {
        rail_provinces.extend(railway.provinces.iter().copied());
    }
    let mut supply_provinces = BTreeSet::new();
    for node in &supply_nodes {
        supply_provinces.insert(node.province_id);
    }
    let mut wanted = rail_provinces.clone();
    wanted.extend(supply_provinces.iter().copied());

    // Referential integrity vs definition.csv.
    let provinces_missing_from_definition = wanted
        .iter()
        .copied()
        .filter(|id| !definitions.contains_key(id))
        .collect::<Vec<_>>();

    let centroids = compute_province_centroids(&raster.ids, raster.width, raster.height, &wanted);
    let provinces_absent_from_raster = wanted
        .iter()
        .copied()
        .filter(|id| !centroids.contains_key(id))
        .collect::<Vec<_>>();

    // Level/path validation on the parsed segments.
    let mut invalid_levels = Vec::new();
    let mut degenerate_paths = Vec::new();
    for (idx, railway) in railways.iter().enumerate() {
        if !(1..=RAIL_LEVEL_MAX).contains(&railway.level) {
            invalid_levels.push((idx + 1, railway.level));
        }
        if railway.provinces.len() < 2 {
            degenerate_paths.push(idx + 1);
        }
    }

    let (polylines, mut gaps) = rail_polylines(&railways, &centroids, &defines);
    let (hub_markers, hub_gaps) = supply_hub_markers(&supply_nodes, &centroids, &defines);
    gaps.extend(hub_gaps);
    if !provinces_missing_from_definition.is_empty() {
        gaps.push(format!(
            "{} rail/supply provinces are absent from {DEFINITION_CSV_SOURCE}: {:?}",
            provinces_missing_from_definition.len(),
            provinces_missing_from_definition
        ));
    }
    for (idx, level) in &invalid_levels {
        gaps.push(format!(
            "railway #{idx} level {level} outside shipped range 1..={RAIL_LEVEL_MAX}"
        ));
    }

    let mut level_histogram: BTreeMap<u32, usize> = BTreeMap::new();
    for railway in &railways {
        *level_histogram.entry(railway.level).or_insert(0) += 1;
    }
    let rail_polylines_dropped = railways.len() - polylines.len();

    let stats = RailFileDerivedStats {
        schema: "battle-plan-rail-file-derived-layer-v1".to_string(),
        railway_count: railways.len(),
        level_histogram,
        supply_node_count: supply_nodes.len(),
        distinct_rail_provinces: rail_provinces.len(),
        distinct_supply_provinces: supply_provinces.len(),
        invalid_levels,
        degenerate_paths,
        provinces_missing_from_definition,
        provinces_absent_from_raster,
        rail_polyline_count: polylines.len(),
        rail_polylines_dropped,
        supply_hub_marker_count: hub_markers.len(),
        defines,
        render_law: rail_render_law(&defines),
        gaps,
    };

    Ok(RailFileDerivedLayer {
        railways,
        supply_nodes,
        polylines,
        hub_markers,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const VANILLA_GAME_DIR: &str = r"X:\SteamLibrary\steamapps\common\Hearts of Iron IV";

    fn test_defines() -> RailGeometryDefines {
        RailGeometryDefines {
            y_offset: 0.9,
            camera_cutoff: 300.0,
            camera_cutoff_speed: 3.0,
            icon_cutoff: 900.0,
            arrow_thin_threshold: 1.0,
            arrow_medium_threshold: 3.0,
            arrow_thick_threshold: 5.0,
        }
    }

    #[test]
    fn parses_rail_defines_from_lua() {
        let lua = r#"
            RAILWAY_Y_OFFSET = 0.9,
            RAILWAY_CAMERA_CUTOFF = 300.0, -- comment
            RAILWAY_CAMERA_CUTOFF_SPEED = 3.0,
            RAILWAY_ICON_CUTOFF = 900.0,
            RAILWAY_MAP_ARROW_THIN_LEVEL_THRESHOLD = 1,
            RAILWAY_MAP_ARROW_MEDIUM_LEVEL_THRESHOLD = 3,
            RAILWAY_MAP_ARROW_THICK_LEVEL_THRESHOLD = 5,
        "#;
        let d = parse_rail_geometry_defines(lua).expect("defines");
        assert_eq!(d.y_offset, 0.9);
        assert_eq!(d.camera_cutoff, 300.0);
        assert_eq!(d.camera_cutoff_speed, 3.0);
        assert_eq!(d.icon_cutoff, 900.0);
        assert_eq!(d.arrow_thick_threshold, 5.0);
    }

    #[test]
    fn polyline_lifts_centroids_to_y_offset_and_drops_missing() {
        let defines = test_defines();
        let mut centroids = BTreeMap::new();
        centroids.insert(
            10,
            ProvinceCentroid {
                x: 100.0,
                z: 200.0,
                pixel_count: 4,
            },
        );
        centroids.insert(
            20,
            ProvinceCentroid {
                x: 110.0,
                z: 205.0,
                pixel_count: 4,
            },
        );
        let railways = vec![
            RailwaySegment {
                level: 2,
                provinces: vec![10, 20],
            },
            // references province 30 with no centroid -> dropped.
            RailwaySegment {
                level: 1,
                provinces: vec![10, 30],
            },
        ];
        let (polylines, gaps) = rail_polylines(&railways, &centroids, &defines);
        assert_eq!(polylines.len(), 1);
        assert_eq!(
            polylines[0].points,
            vec![[100.0, 0.9, 200.0], [110.0, 0.9, 205.0]]
        );
        assert_eq!(polylines[0].segment_index, 1);
        assert_eq!(gaps.len(), 1);
        assert!(gaps[0].contains("30"));
    }

    #[test]
    fn single_province_path_is_dropped_fail_closed() {
        let defines = test_defines();
        let mut centroids = BTreeMap::new();
        centroids.insert(
            10,
            ProvinceCentroid {
                x: 1.0,
                z: 1.0,
                pixel_count: 1,
            },
        );
        let railways = vec![RailwaySegment {
            level: 1,
            provinces: vec![10],
        }];
        let (polylines, gaps) = rail_polylines(&railways, &centroids, &defines);
        assert!(polylines.is_empty());
        assert_eq!(gaps.len(), 1);
    }

    #[test]
    fn supply_marker_lifts_to_y_offset() {
        let defines = test_defines();
        let mut centroids = BTreeMap::new();
        centroids.insert(
            67,
            ProvinceCentroid {
                x: 50.0,
                z: 60.0,
                pixel_count: 9,
            },
        );
        let nodes = vec![
            SupplyNode {
                level: 1,
                province_id: 67,
            },
            SupplyNode {
                level: 1,
                province_id: 999,
            },
        ];
        let (markers, gaps) = supply_hub_markers(&nodes, &centroids, &defines);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].pos, [50.0, 0.9, 60.0]);
        assert_eq!(gaps.len(), 1);
    }

    #[test]
    fn centroid_pass_averages_only_wanted_provinces() {
        // 4x2 raster: province 10 fills the left 2 columns, 20 the right 2.
        let width = 4;
        let height = 2;
        let ids = vec![10, 10, 20, 20, 10, 10, 20, 20];
        let wanted = BTreeSet::from([10u32, 20u32]);
        let c = compute_province_centroids(&ids, width, height, &wanted);
        assert_eq!(c[&10].x, 0.5);
        assert_eq!(c[&10].z, 0.5);
        assert_eq!(c[&10].pixel_count, 4);
        assert_eq!(c[&20].x, 2.5);
    }

    #[test]
    fn render_law_labels_engine_side_residuals_unknown() {
        let law = rail_render_law(&test_defines());
        let unknowns = law.iter().filter(|r| r.class == "UNKNOWN").count();
        assert!(
            unknowns >= 5,
            "engine-side residuals must be labelled UNKNOWN"
        );
        assert!(law
            .iter()
            .any(|r| r.class == "FILE" && r.parameter.contains("ribbon draw")));
    }

    // ---- Vanilla FILE gates (exact counts on the shipped files) ----

    #[test]
    fn gate_vanilla_rail_defines_are_shipped_values() {
        let game_dir = PathBuf::from(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping vanilla rail defines gate: {VANILLA_GAME_DIR} is absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let d = rail_geometry_defines_from_resolver(&resolver).expect("defines");
        assert_eq!(d.y_offset, 0.9);
        assert_eq!(d.camera_cutoff, 300.0);
        assert_eq!(d.camera_cutoff_speed, 3.0);
        assert_eq!(d.icon_cutoff, 900.0);
        assert_eq!(d.arrow_thin_threshold, 1.0);
        assert_eq!(d.arrow_medium_threshold, 3.0);
        assert_eq!(d.arrow_thick_threshold, 5.0);
    }

    #[test]
    fn gate_vanilla_rail_geometry_and_referential_integrity() {
        let game_dir = PathBuf::from(VANILLA_GAME_DIR);
        if !game_dir.is_dir() {
            eprintln!("skipping vanilla rail geometry gate: {VANILLA_GAME_DIR} is absent");
            return;
        }
        let resolver = GameFileResolver::vanilla(game_dir).expect("vanilla resolver");
        let layer = rail_file_derived_layer_from_resolver(&resolver).expect("layer");
        let s = &layer.stats;
        println!(
            "gate_rail_geometry: {} railways, {} supply nodes, {} distinct rail provinces, {} distinct supply provinces, {} polylines ({} dropped), {} hub markers, {} missing-from-def, {} absent-from-raster",
            s.railway_count,
            s.supply_node_count,
            s.distinct_rail_provinces,
            s.distinct_supply_provinces,
            s.rail_polyline_count,
            s.rail_polylines_dropped,
            s.supply_hub_marker_count,
            s.provinces_missing_from_definition.len(),
            s.provinces_absent_from_raster.len(),
        );

        assert_eq!(s.railway_count, 909);
        assert_eq!(s.supply_node_count, 727);
        assert_eq!(s.distinct_rail_provinces, 3126);
        assert_eq!(s.distinct_supply_provinces, 727);

        // Level histogram is exactly the shipped distribution.
        assert_eq!(s.level_histogram.get(&1).copied(), Some(467));
        assert_eq!(s.level_histogram.get(&2).copied(), Some(406));
        assert_eq!(s.level_histogram.get(&3).copied(), Some(29));
        assert_eq!(s.level_histogram.get(&4).copied(), Some(7));
        assert_eq!(s.level_histogram.get(&5).copied(), None);
        assert!(s.invalid_levels.is_empty());
        assert!(s.degenerate_paths.is_empty());

        // Every supply node flag is the enabled flag in vanilla.
        assert!(layer.supply_nodes.iter().all(|n| n.level == 1));

        // Referential integrity: every referenced province is defined and painted.
        assert!(
            s.provinces_missing_from_definition.is_empty(),
            "rail/supply provinces absent from definition.csv: {:?}",
            s.provinces_missing_from_definition
        );
        assert!(
            s.provinces_absent_from_raster.is_empty(),
            "rail/supply provinces painted on zero raster pixels: {:?}",
            s.provinces_absent_from_raster
        );

        // With full referential integrity, no geometry is dropped.
        assert_eq!(s.rail_polylines_dropped, 0);
        assert_eq!(s.rail_polyline_count, 909);
        assert_eq!(s.supply_hub_marker_count, 727);

        // Every polyline point is inside the world XZ bounds and has one point
        // per path province (path continuity: no province silently dropped).
        for (poly, railway) in layer.polylines.iter().zip(layer.railways.iter()) {
            assert_eq!(poly.points.len(), railway.provinces.len());
            for p in &poly.points {
                assert!(
                    p[0] >= 0.0 && p[0] <= MAP_WIDTH as f32,
                    "rail x {} out of bounds on segment {}",
                    p[0],
                    poly.segment_index
                );
                assert!(
                    p[2] >= 0.0 && p[2] <= MAP_HEIGHT as f32,
                    "rail z {} out of bounds on segment {}",
                    p[2],
                    poly.segment_index
                );
                assert_eq!(
                    p[1], layer.stats.defines.y_offset,
                    "rail y == RAILWAY_Y_OFFSET"
                );
            }
        }
        for m in &layer.hub_markers {
            assert!(m.pos[0] >= 0.0 && m.pos[0] <= MAP_WIDTH as f32);
            assert!(m.pos[2] >= 0.0 && m.pos[2] <= MAP_HEIGHT as f32);
        }

        // The render law carries the UNKNOWN residuals explicitly.
        let unknowns = s
            .render_law
            .iter()
            .filter(|row| row.class == "UNKNOWN")
            .count();
        assert!(
            unknowns >= 5,
            "expected the engine-side residuals labelled UNKNOWN"
        );
    }
}
