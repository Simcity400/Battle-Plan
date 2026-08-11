//! Sea-zone naval-supremacy tint (plan item **C6**).
//!
//! This module is a legacy diagnostic surface. Production XMAP18 does not use
//! its derived supremacy curve. It consumes exact observer/region 0/128/255
//! control and contested-direction codes; XMAP05 writes them to
//! GradientBorderChannel2.r/a for the shipped `dominance_fx_apply` shader.
//! Capture-gated notes below apply only to this retained diagnostic path.
//!
//! The naval / strategic-navy map mode colours each sea zone (strategic region
//! containing sea provinces) by the side that holds naval supremacy there. This
//! module owns the *data law* for that overlay:
//!
//! 1. **Sea-zone geometry** — parse `map/strategicregions/*.txt` (province sets)
//!    + `map/definition.csv` (province types) into naval regions and their sea
//!    province sets. The sea province set of a zone IS the field geometry that
//!    the GradientBorder (GB) field fills — the same province-set -> pixel
//!    mechanism the political fill rides (`POLITICAL_COLOR_LAW.md`,
//!    `CGradientBorderManager` / `CGradientBorder::CFillBufferThreaded`). This
//!    module produces the per-zone RGBA the GB field is keyed with; the pixel
//!    rasterisation + parity vs a captured supremacy frame is capture-gated and
//!    deferred.
//! 2. **Supremacy -> status -> tint** — given a per-region supremacy snapshot
//!    (controller + supremacy value, shaped exactly like `replay_core`'s
//!    `NavalSupremacyRow`, requirements item 5 / extractor D56) plus the country
//!    colour table, compute each zone's tint. Fails **closed** to open ocean
//!    (no tint) when supremacy data is absent — matching the engine, which draws
//!    no sea lines with no naval data (`REPLAY_DATA_REQUIREMENTS.md` §5).
//!
//! Colours are computed replay-side from inputs, **never recorded** (standing
//! override). Every colour/threshold below is FILE-cited to
//! `common/defines/00_graphics.lua` or fails closed.
//!
//! ## Seam
//!
//! [`ZoneSupremacyInput`] mirrors `replay_core::world::NavalSupremacyRow`
//! (`controller: Leaf<String>`, `supremacy: Leaf<f64>`): a present leaf maps to
//! `Some`, an absent/`Unobtained` leaf to `None`. The `state` (tri-state) leaf
//! of the row is a **POV-relative** overlay (friendly/enemy/contested relative
//! to the viewed country) and is not used by the omniscient-observer tint here —
//! see [`PovNavalRelation`]. This module
//! deliberately does **not** depend on `replay_core`; the live wiring
//! (`NavalSupremacyRow` -> `ZoneSupremacyInput`) belongs at the `replay_app`
//! integration seam, keeping `map_viewer` decoupled from the crate the parallel
//! lane is editing.

use crate::mod_resolver::GameFileResolver;
use crate::Result;
use std::collections::BTreeMap;

/// `00_graphics.lua` — source of every naval-tint colour/threshold below.
pub const NAVAL_SUPREMACY_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
/// Strategic-region province sets (sea-zone geometry join key).
pub const NAVAL_SUPREMACY_STRATEGIC_REGIONS_DIR: &str = "map/strategicregions";
/// Province type table (`sea` classification for the field geometry).
pub const NAVAL_SUPREMACY_DEFINITION_CSV_SOURCE: &str = "map/definition.csv";

// ---------------------------------------------------------------------------
// Defines (FILE law).
// ---------------------------------------------------------------------------

/// The `00_graphics.lua` constants that drive the strategic-navy region map.
///
/// FILE citations (`common/defines/00_graphics.lua`):
/// - `NAVAL_REGION_TRANSPARENCY_RATIO_MIN` / `_MAX` — the alpha (opacity) range
///   of the tint; min at low naval dominance, max at high dominance.
/// - `NAVAL_REGION_TRANSPARENCY_RANGE` — the naval-dominance span over which the
///   ratio interpolates ("range of naval dominance to be rendered").
/// - `STRATEGIC_MODE_{ENEMY,OUR,ALLY}_STRIPES_COLOR` — the POV-relative stripe
///   overlay colours (E6-gated; vanilla alpha is 0.0 = disabled).
/// - `STRATEGIC_MODE_COUNTRY_COLOR_STRIPES_TRANSP` — the country-colour stripe
///   transparency (0.0 in vanilla).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavalTintDefines {
    /// `NAVAL_REGION_TRANSPARENCY_RATIO_MIN` (FILE).
    pub transparency_ratio_min: f32,
    /// `NAVAL_REGION_TRANSPARENCY_RATIO_MAX` (FILE).
    pub transparency_ratio_max: f32,
    /// `NAVAL_REGION_TRANSPARENCY_RANGE` (FILE).
    pub transparency_range: f32,
    /// `STRATEGIC_MODE_ENEMY_STRIPES_COLOR` (FILE).
    pub enemy_stripes_color: [f32; 4],
    /// `STRATEGIC_MODE_OUR_STRIPES_COLOR` (FILE).
    pub our_stripes_color: [f32; 4],
    /// `STRATEGIC_MODE_ALLY_STRIPES_COLOR` (FILE).
    pub ally_stripes_color: [f32; 4],
    /// `STRATEGIC_MODE_COUNTRY_COLOR_STRIPES_TRANSP` (FILE).
    pub country_color_stripes_transp: f32,
}

/// The vanilla `00_graphics.lua` values, verbatim (FILE, cited above). Used as
/// the offline gate reference; the resolver path re-parses the shipped file so
/// mods override correctly.
pub const VANILLA_NAVAL_TINT_DEFINES: NavalTintDefines = NavalTintDefines {
    transparency_ratio_min: 0.5,
    transparency_ratio_max: 0.8,
    transparency_range: 2000.0,
    enemy_stripes_color: [0.827, 0.172, 0.184, 0.0],
    our_stripes_color: [0.427, 0.619, 0.858, 0.0],
    ally_stripes_color: [0.427, 0.619, 0.858, 0.0],
    country_color_stripes_transp: 0.0,
};

fn parse_color4(source: &str, key: &str) -> Result<[f32; 4]> {
    let values = crate::map_furniture::parse_lua_number_list(source, key).ok_or_else(|| {
        format!("{NAVAL_SUPREMACY_GRAPHICS_DEFINES_SOURCE}: missing colour define {key}")
    })?;
    if values.len() != 4 {
        return Err(format!(
            "{NAVAL_SUPREMACY_GRAPHICS_DEFINES_SOURCE}: {key} expected 4 components, got {}",
            values.len()
        )
        .into());
    }
    Ok([values[0], values[1], values[2], values[3]])
}

/// Parse the naval-tint defines out of `00_graphics.lua` source text.
pub fn naval_tint_defines_from_source(source: &str) -> Result<NavalTintDefines> {
    Ok(NavalTintDefines {
        transparency_ratio_min: crate::parse_lua_number_define(
            source,
            "NAVAL_REGION_TRANSPARENCY_RATIO_MIN",
        )?,
        transparency_ratio_max: crate::parse_lua_number_define(
            source,
            "NAVAL_REGION_TRANSPARENCY_RATIO_MAX",
        )?,
        transparency_range: crate::parse_lua_number_define(
            source,
            "NAVAL_REGION_TRANSPARENCY_RANGE",
        )?,
        enemy_stripes_color: parse_color4(source, "STRATEGIC_MODE_ENEMY_STRIPES_COLOR")?,
        our_stripes_color: parse_color4(source, "STRATEGIC_MODE_OUR_STRIPES_COLOR")?,
        ally_stripes_color: parse_color4(source, "STRATEGIC_MODE_ALLY_STRIPES_COLOR")?,
        country_color_stripes_transp: crate::parse_lua_number_define(
            source,
            "STRATEGIC_MODE_COUNTRY_COLOR_STRIPES_TRANSP",
        )?,
    })
}

/// Load + parse the naval-tint defines via the mod-aware resolver.
pub fn naval_tint_defines_from_resolver(resolver: &GameFileResolver) -> Result<NavalTintDefines> {
    let path = resolver.resolve_required(
        NAVAL_SUPREMACY_GRAPHICS_DEFINES_SOURCE,
        "naval tint graphics defines",
    )?;
    let source = crate::read_latin1(&path)?;
    naval_tint_defines_from_source(&source)
}

// ---------------------------------------------------------------------------
// Sea-zone geometry (field the GB fill is keyed with).
// ---------------------------------------------------------------------------

/// One naval region: a strategic region that contains at least one sea province.
/// `sea_provinces` is the GB-field geometry the tint is painted into; interior
/// land provinces (islands) keep their political colour and are excluded.
#[derive(Clone, Debug, PartialEq)]
pub struct SeaZone {
    pub region_id: u32,
    pub name: String,
    /// Sea provinces only, sorted — the field geometry.
    pub sea_provinces: Vec<u32>,
    /// Non-sea (island) province count inside the region, kept for reporting.
    pub land_province_count: usize,
}

/// The full naval-region set derived from the shipped map files, plus the
/// classification counts used by the vanilla parse gate.
#[derive(Clone, Debug, PartialEq)]
pub struct NavalRegionSet {
    /// region id -> sea zone (only regions with >= 1 sea province).
    pub zones: BTreeMap<u32, SeaZone>,
    /// sea province -> owning region id.
    pub province_to_zone: BTreeMap<u32, u32>,
    /// Total strategic regions parsed (naval + land).
    pub total_strategic_regions: usize,
    /// Regions whose provinces are *all* sea.
    pub pure_sea_region_count: usize,
    /// Total sea provinces covered by naval regions.
    pub sea_province_count: usize,
}

/// A province is naval-tintable water iff its `definition.csv` type is `sea`.
/// Lakes are excluded — they are inland water, not naval (strategic) regions,
/// and never appear in `map/strategicregions` sea zones.
pub fn province_is_sea(province_type: &str) -> bool {
    province_type == "sea"
}

/// Classify strategic regions into naval regions from the parsed province sets
/// and a province-type lookup. Pure: no I/O, deterministic.
///
/// `region_to_provinces` is the raw `map/strategicregions` join
/// (`game_data::load_strategic_regions`); `region_names` supplies the region
/// display name; `province_type` returns the `definition.csv` type for a
/// province id (or `None` if the id is unknown, treated as non-sea).
pub fn classify_naval_regions<'a>(
    region_to_provinces: &BTreeMap<u32, Vec<u32>>,
    region_names: impl Fn(u32) -> String,
    province_type: impl Fn(u32) -> Option<&'a str>,
) -> NavalRegionSet {
    let mut zones = BTreeMap::new();
    let mut province_to_zone = BTreeMap::new();
    let mut pure_sea_region_count = 0usize;
    let mut sea_province_count = 0usize;

    for (&region_id, provinces) in region_to_provinces {
        let mut sea_provinces = Vec::new();
        let mut land_province_count = 0usize;
        for &province in provinces {
            if province_type(province)
                .map(province_is_sea)
                .unwrap_or(false)
            {
                sea_provinces.push(province);
            } else {
                land_province_count += 1;
            }
        }
        if sea_provinces.is_empty() {
            continue; // pure-land strategic region: not a naval region.
        }
        sea_provinces.sort_unstable();
        sea_provinces.dedup();
        if land_province_count == 0 {
            pure_sea_region_count += 1;
        }
        sea_province_count += sea_provinces.len();
        for &province in &sea_provinces {
            province_to_zone.insert(province, region_id);
        }
        zones.insert(
            region_id,
            SeaZone {
                region_id,
                name: region_names(region_id),
                sea_provinces,
                land_province_count,
            },
        );
    }

    NavalRegionSet {
        zones,
        province_to_zone,
        total_strategic_regions: region_to_provinces.len(),
        pure_sea_region_count,
        sea_province_count,
    }
}

/// Load the naval-region set from the shipped map files via the mod-aware
/// resolver (`map/strategicregions` + `map/definition.csv`).
pub fn naval_region_set_from_resolver(resolver: &GameFileResolver) -> Result<NavalRegionSet> {
    let region_files =
        resolver.resolve_dir_files_recursive(NAVAL_SUPREMACY_STRATEGIC_REGIONS_DIR, Some("txt"))?;
    let (region_to_provinces, region_meta, _province_to_region) =
        crate::load_strategic_regions(&region_files)?;
    let definition_csv = resolver.resolve_required(
        NAVAL_SUPREMACY_DEFINITION_CSV_SOURCE,
        "naval supremacy definition.csv",
    )?;
    let (definitions, _color_to_id) = crate::load_definition_csv(&definition_csv)?;
    Ok(classify_naval_regions(
        &region_to_provinces,
        |id| {
            region_meta
                .get(&id)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| format!("STRATEGICREGION_{id}"))
        },
        |province| definitions.get(&province).map(|d| d.province_type.as_str()),
    ))
}

// ---------------------------------------------------------------------------
// Supremacy -> status -> tint.
// ---------------------------------------------------------------------------

/// Per-region supremacy snapshot, shaped like `replay_core::NavalSupremacyRow`.
/// `None` = the leaf is absent/`Unobtained` in the recording (fail-closed to
/// open ocean).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneSupremacyInput {
    /// Controlling side tag (the country whose colour tints the zone).
    pub controller: Option<String>,
    /// Naval supremacy / dominance magnitude for that controller.
    pub supremacy: Option<f64>,
}

/// The omniscient-observer status of a sea zone.
#[derive(Clone, Debug, PartialEq)]
pub enum ZoneSupremacyStatus {
    /// No usable supremacy data -> engine draws no sea lines (degrade).
    OpenOcean,
    /// Held by `controller` with the given supremacy magnitude.
    Controlled { controller: String, supremacy: f64 },
}

/// The computed tint for one sea zone.
#[derive(Clone, Debug, PartialEq)]
pub struct ZoneTint {
    pub region_id: u32,
    pub status: ZoneSupremacyStatus,
    /// The RGBA the GB field is keyed with (`None` = open ocean / no tint, i.e.
    /// controller colour missing or no supremacy data — fail-closed).
    pub fill_rgba: Option<[f32; 4]>,
}

/// POV-relative naval relationship (E6 observer-country mode). The stripe colour
/// overlay is keyed on this; in the default omniscient view it is not applied
/// (and vanilla stripe alpha is 0.0 regardless).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PovNavalRelation {
    /// The viewed country itself controls the zone.
    Own,
    /// An ally / same-faction country controls the zone.
    Ally,
    /// An enemy controls the zone.
    Enemy,
}

/// The POV stripe overlay colour for a relationship (FILE:
/// `STRATEGIC_MODE_{OUR,ALLY,ENEMY}_STRIPES_COLOR`). E6-gated; not applied in the
/// omniscient view. Vanilla alpha is 0.0 (disabled).
pub fn pov_stripe_color(relation: PovNavalRelation, defines: &NavalTintDefines) -> [f32; 4] {
    match relation {
        PovNavalRelation::Own => defines.our_stripes_color,
        PovNavalRelation::Ally => defines.ally_stripes_color,
        PovNavalRelation::Enemy => defines.enemy_stripes_color,
    }
}

/// Classify a supremacy input into a zone status (fail-closed to open ocean).
///
/// Requires **both** a non-empty controller tag and a supremacy value: the tint
/// needs the controller for the fill colour and the supremacy for the alpha, so
/// a half-populated row degrades to open ocean rather than inventing either.
pub fn zone_supremacy_status(input: &ZoneSupremacyInput) -> ZoneSupremacyStatus {
    match (&input.controller, input.supremacy) {
        (Some(controller), Some(supremacy)) if !controller.is_empty() => {
            ZoneSupremacyStatus::Controlled {
                controller: controller.clone(),
                supremacy,
            }
        }
        _ => ZoneSupremacyStatus::OpenOcean,
    }
}

/// The interpolation parameter of the tint alpha: `clamp(supremacy / RANGE, 0, 1)`.
///
/// LAW: `NAVAL_REGION_TRANSPARENCY_RANGE` = FILE; the linear shape/direction
/// (dominance 0 -> min ratio, dominance >= RANGE -> max ratio) is DERIVED from
/// the define comments ("range of naval dominance to be rendered", "min/max
/// transparency ratio") and is capture-gated for the exact curve.
pub fn supremacy_ratio(supremacy: f64, defines: &NavalTintDefines) -> f32 {
    if defines.transparency_range <= 0.0 {
        return 0.0;
    }
    ((supremacy as f32) / defines.transparency_range).clamp(0.0, 1.0)
}

/// The tint alpha (opacity) for a supremacy magnitude:
/// `lerp(RATIO_MIN, RATIO_MAX, supremacy_ratio)`.
pub fn supremacy_alpha(supremacy: f64, defines: &NavalTintDefines) -> f32 {
    let ratio = supremacy_ratio(supremacy, defines);
    defines.transparency_ratio_min
        + (defines.transparency_ratio_max - defines.transparency_ratio_min) * ratio
}

/// Compute the tint for a single sea zone.
///
/// `country_colors` is the resolved tag -> RGB table
/// (`game_data::load_country_colors*`, includes cosmetic-tag overrides). A
/// controller with no colour entry fails closed to open ocean (no invented
/// colour).
pub fn zone_tint(
    region_id: u32,
    input: &ZoneSupremacyInput,
    country_colors: &BTreeMap<String, [u8; 3]>,
    defines: &NavalTintDefines,
) -> ZoneTint {
    let status = zone_supremacy_status(input);
    let fill_rgba = match &status {
        ZoneSupremacyStatus::Controlled {
            controller,
            supremacy,
        } => country_colors.get(controller).map(|rgb| {
            [
                rgb[0] as f32 / 255.0,
                rgb[1] as f32 / 255.0,
                rgb[2] as f32 / 255.0,
                supremacy_alpha(*supremacy, defines),
            ]
        }),
        ZoneSupremacyStatus::OpenOcean => None,
    };
    ZoneTint {
        region_id,
        status,
        fill_rgba,
    }
}

/// Compute tints for every naval region: regions with a supremacy input get
/// their computed tint; naval regions absent from `inputs` degrade to open
/// ocean. Non-naval (pure-land) regions are never tinted.
pub fn naval_supremacy_tints(
    regions: &NavalRegionSet,
    inputs: &BTreeMap<u32, ZoneSupremacyInput>,
    country_colors: &BTreeMap<String, [u8; 3]>,
    defines: &NavalTintDefines,
) -> BTreeMap<u32, ZoneTint> {
    let mut out = BTreeMap::new();
    for &region_id in regions.zones.keys() {
        let input = inputs.get(&region_id).cloned().unwrap_or_default();
        out.insert(
            region_id,
            zone_tint(region_id, &input, country_colors, defines),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPHICS_SNIPPET: &str = r#"
NDefines = {
	NGraphics = {
		STRATEGIC_MODE_COUNTRY_COLOR_STRIPES_TRANSP = 0.0,
		STRATEGIC_MODE_ENEMY_STRIPES_COLOR = { 0.827, 0.172, 0.184, 0.0 },
		STRATEGIC_MODE_OUR_STRIPES_COLOR = { 0.427, 0.619, 0.858, 0.0 },
		STRATEGIC_MODE_ALLY_STRIPES_COLOR = { 0.427, 0.619, 0.858, 0.0 },
		NAVAL_REGION_ACCESS_AVOID_COLOR = { 1, 1, 0, 0.35 },
		NAVAL_REGION_TRANSPARENCY_RATIO_MAX = 0.8,
		NAVAL_REGION_TRANSPARENCY_RATIO_MIN = 0.5,
		NAVAL_REGION_TRANSPARENCY_RANGE = 2000.0,
	}
}
"#;

    #[test]
    fn defines_parse_matches_vanilla_constant() {
        let parsed = naval_tint_defines_from_source(GRAPHICS_SNIPPET).expect("parse defines");
        assert_eq!(parsed, VANILLA_NAVAL_TINT_DEFINES);
    }

    #[test]
    fn classify_splits_sea_land_and_islands() {
        // region 10: pure sea (2 sea). region 20: sea + 1 island land.
        // region 30: pure land (not a naval region).
        let mut r2p = BTreeMap::new();
        r2p.insert(10u32, vec![100u32, 101]);
        r2p.insert(20u32, vec![200u32, 201, 202]);
        r2p.insert(30u32, vec![300u32, 301]);
        let types: BTreeMap<u32, &str> = BTreeMap::from([
            (100, "sea"),
            (101, "sea"),
            (200, "sea"),
            (201, "sea"),
            (202, "land"), // island
            (300, "land"),
            (301, "land"),
        ]);
        let set = classify_naval_regions(
            &r2p,
            |id| format!("STRATEGICREGION_{id}"),
            |p| types.get(&p).copied(),
        );
        assert_eq!(set.total_strategic_regions, 3);
        assert_eq!(set.zones.len(), 2, "only regions with sea provinces");
        assert!(!set.zones.contains_key(&30), "pure-land region excluded");
        assert_eq!(set.pure_sea_region_count, 1, "only region 10 is all-sea");
        assert_eq!(set.sea_province_count, 4);
        // Field geometry = sea provinces only; the island (202) is excluded.
        assert_eq!(set.zones[&20].sea_provinces, vec![200, 201]);
        assert_eq!(set.zones[&20].land_province_count, 1);
        assert_eq!(set.province_to_zone.get(&202), None);
        assert_eq!(set.province_to_zone.get(&200), Some(&20));
    }

    #[test]
    fn supremacy_alpha_law() {
        let d = VANILLA_NAVAL_TINT_DEFINES;
        // 0 dominance -> ratio min.
        assert!((supremacy_alpha(0.0, &d) - 0.5).abs() < 1e-6);
        // >= RANGE -> ratio max, clamped.
        assert!((supremacy_alpha(2000.0, &d) - 0.8).abs() < 1e-6);
        assert!((supremacy_alpha(5000.0, &d) - 0.8).abs() < 1e-6);
        // midpoint.
        assert!((supremacy_alpha(1000.0, &d) - 0.65).abs() < 1e-6);
    }

    #[test]
    fn fail_closed_open_ocean() {
        let d = VANILLA_NAVAL_TINT_DEFINES;
        let colors: BTreeMap<String, [u8; 3]> = BTreeMap::from([("ENG".to_string(), [1u8, 2, 3])]);
        // No controller -> open ocean, no fill.
        let t = zone_tint(1, &ZoneSupremacyInput::default(), &colors, &d);
        assert_eq!(t.status, ZoneSupremacyStatus::OpenOcean);
        assert_eq!(t.fill_rgba, None);
        // Controller present but no supremacy -> open ocean.
        let t = zone_tint(
            1,
            &ZoneSupremacyInput {
                controller: Some("ENG".to_string()),
                supremacy: None,
            },
            &colors,
            &d,
        );
        assert_eq!(t.status, ZoneSupremacyStatus::OpenOcean);
        // Controller with no colour entry -> fail closed to no fill.
        let t = zone_tint(
            1,
            &ZoneSupremacyInput {
                controller: Some("ZZZ".to_string()),
                supremacy: Some(1000.0),
            },
            &colors,
            &d,
        );
        assert!(matches!(t.status, ZoneSupremacyStatus::Controlled { .. }));
        assert_eq!(t.fill_rgba, None, "unknown controller colour fails closed");
    }

    /// fx_naval_air_activity gate. Expectations derived from the fixture
    /// generator source (`replay_fixture_gen/src/main.rs::fx_naval_air_activity`),
    /// NOT by running the code under test:
    ///   region_control["301"] = [{controller, tri_state, supremacy}]
    ///   controller = "ENG" if tick < 5 else "GER"
    ///   supremacy  = 0.55 + tick * 0.02   (SNAPSHOT_COUNT = 12, ticks 0..=11)
    #[test]
    fn fx_naval_air_activity_zone_statuses() {
        let d = VANILLA_NAVAL_TINT_DEFINES;
        // Synthetic colour table (production resolves these from country files;
        // that path is covered by `gate_vanilla_naval_region_parse`). Here we
        // only assert the tint law wires colour + alpha correctly.
        let colors: BTreeMap<String, [u8; 3]> = BTreeMap::from([
            ("ENG".to_string(), [200u8, 40, 40]),
            ("GER".to_string(), [80u8, 80, 90]),
        ]);
        // The generator's exact per-tick outputs.
        let expected = |tick: usize| -> (&'static str, f64) {
            let controller = if tick < 5 { "ENG" } else { "GER" };
            let supremacy = 0.55 + tick as f64 * 0.02;
            (controller, supremacy)
        };
        for tick in [0usize, 4, 5, 7, 11] {
            let (controller, supremacy) = expected(tick);
            let input = ZoneSupremacyInput {
                controller: Some(controller.to_string()),
                supremacy: Some(supremacy),
            };
            let t = zone_tint(301, &input, &colors, &d);
            assert_eq!(
                t.status,
                ZoneSupremacyStatus::Controlled {
                    controller: controller.to_string(),
                    supremacy,
                },
                "tick {tick} controller/supremacy"
            );
            let rgb = colors[controller];
            let alpha = supremacy_alpha(supremacy, &d);
            let fill = t.fill_rgba.expect("controlled zone has a fill");
            assert!((fill[0] - rgb[0] as f32 / 255.0).abs() < 1e-6);
            assert!((fill[1] - rgb[1] as f32 / 255.0).abs() < 1e-6);
            assert!((fill[2] - rgb[2] as f32 / 255.0).abs() < 1e-6);
            assert!((fill[3] - alpha).abs() < 1e-6, "tick {tick} alpha");
        }
        // Controller flips ENG (tick 4) -> GER (tick 5).
        assert_eq!(expected(4).0, "ENG");
        assert_eq!(expected(5).0, "GER");

        // A naval region with no supremacy input degrades to open ocean.
        let regions = NavalRegionSet {
            zones: BTreeMap::from([(
                301,
                SeaZone {
                    region_id: 301,
                    name: "STRATEGICREGION_301".to_string(),
                    sea_provinces: vec![1, 2],
                    land_province_count: 0,
                },
            )]),
            province_to_zone: BTreeMap::from([(1, 301), (2, 301)]),
            total_strategic_regions: 1,
            pure_sea_region_count: 1,
            sea_province_count: 2,
        };
        let tints = naval_supremacy_tints(&regions, &BTreeMap::new(), &colors, &d);
        assert_eq!(tints[&301].status, ZoneSupremacyStatus::OpenOcean);
        assert_eq!(tints[&301].fill_rgba, None);
    }

    /// Vanilla parse gate (skips without `BATTLE_PLAN_GAME_DIR`). Asserts the sea-zone
    /// count against the shipped files.
    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_naval_region_parse() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let set = naval_region_set_from_resolver(&resolver).expect("naval region set");
        assert_eq!(
            set.total_strategic_regions, 304,
            "vanilla strategic regions"
        );
        assert_eq!(
            set.zones.len(),
            98,
            "vanilla naval regions (>=1 sea province)"
        );
        assert_eq!(set.pure_sea_region_count, 57, "vanilla all-sea regions");
        assert_eq!(
            set.sea_province_count, 3133,
            "vanilla sea provinces in regions"
        );
        // Region 301 (South Island) is pure land -> not a naval region.
        assert!(!set.zones.contains_key(&301), "301 is a land region");
        // The defines also parse from the shipped file.
        let defines = naval_tint_defines_from_resolver(&resolver).expect("defines");
        assert_eq!(defines, VANILLA_NAVAL_TINT_DEFINES);
    }
}
