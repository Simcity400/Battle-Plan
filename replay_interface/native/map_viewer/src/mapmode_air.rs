//! Strategic-air map mode.
//!
//! This module is a diagnostic surface. Production XMAP18 does not use
//! its threshold-derived classifier: it consumes exact sparse raw friendly and
//! enemy superiority score lanes for every observer/strategic-region cell,
//! applies the exact-a729 ratio thresholds, and materializes the result through
//! XMAP05. The UNKNOWN notes below therefore describe only this retained
//! diagnostic path, not a production map-mode residual.
//!
//! The strategic-air map mode (`ui_mapmode_strategic_air`, F3) tints each
//! **strategic region** by the air-superiority contest happening in it. This
//! module owns the replay-side *data law* for that overlay:
//!
//! 1. **Air-region geometry** — an air region IS a strategic region, covering
//!    ALL its provinces (land + sea), unlike the naval mode which fills only the
//!    sea provinces of a region. We reuse the same `map/strategicregions/*.txt`
//!    join the naval module rides (`crate::load_strategic_regions`, the loader
//!    `naval_supremacy_tint` calls under the hood) but keep the full province set
//!    per region rather than the sea-only subset. No edit to that module.
//! 2. **Air-superiority → status → tint** — given a per-region air-superiority
//!    snapshot shaped exactly like `replay_core`'s [`AirSuperiorityRow`]
//!    (`controller` / `mission_mask` / `wing_count` / `plane_count`, requirements
//!    item 8, landed extractor-side) plus the country-colour table, classify each
//!    region and compute its fill. Fails **closed** to no air activity (the
//!    NEUTRAL colour) when the controller leaf is absent/empty or no planes are
//!    present — the engine likewise shows the neutral tone where no wing operates.
//!
//! Colours are computed replay-side from recorded inputs, **never recorded**
//! (standing override). Every colour below is FILE-cited to
//! `common/defines/00_graphics.lua` or fails closed; the module does **not**
//! depend on `replay_core` (the live `AirSuperiorityRow -> AirActivityInput`
//! wiring belongs at the `replay_app` seam, keeping `map_viewer` decoupled from
//! the crate the parallel lane edits).
//!
//! ## Honesty — what is FILE vs UNKNOWN here
//!
//! - The four strategic-air status colours (`STRATEGIC_AIR_COLOR_{BAD,GOOD,
//!   AVERAGE,NEUTRAL}`) are **FILE** and ship alpha 1.0 (the air overlay is
//!   opaque). The NEUTRAL (no-activity) fill is fully file-derived.
//! - The vanilla air map mode paints those good/average/bad colours **relative
//!   to the viewed country's** air-superiority ratio in the region. The exact
//!   ratio thresholds (what ratio reads GOOD vs AVERAGE vs BAD) are engine
//!   runtime, NOT in the defines → **UNKNOWN / capture-gated**. Only
//!   `AIR_REGION_SUPERIORITY_PIXEL_SCALE=0.04` relates
//!   the raw superiority to on-map pixels, not a status cutoff.
//! - The **default omniscient observer** has no viewed country, so good/bad is
//!   undefined. For it we fill an active region by the air-superiority
//!   **controller's country colour** (a DERIVED observer choice, the same one the
//!   C6 naval tint makes for sea zones — labelled, not a claim that vanilla's air
//!   mode colours by controller). The POV good/average/bad path
//!   ([`pov_air_status_color`]) is exposed for the E6 observe-as-country mode and
//!   is threshold-UNKNOWN.

use crate::mod_resolver::GameFileResolver;
use crate::Result;
use std::collections::BTreeMap;

/// `00_graphics.lua` — source of every strategic-air status colour below.
pub const AIR_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
/// Strategic-region province sets (air-region geometry join key). Same file set
/// the naval mode reads; air keeps the full province set (land + sea).
pub const AIR_STRATEGIC_REGIONS_DIR: &str = "map/strategicregions";

// ---------------------------------------------------------------------------
// Status colours (FILE law).
// ---------------------------------------------------------------------------

/// The four strategic-air map-mode region colours (`00_graphics.lua`,
/// `STRATEGIC_AIR_COLOR_*`), each RGBA. In vanilla all four ship **alpha 1.0**
/// (the air overlay is opaque); [`air_overlay_alpha`] reads that back so the
/// controller-colour omniscient fill stays tied to the file convention.
///
/// FILE citations (`common/defines/00_graphics.lua`):
/// - `STRATEGIC_AIR_COLOR_BAD` (`:906`) — enemy dominates (red).
/// - `STRATEGIC_AIR_COLOR_GOOD` (`:907`) — viewed country dominates (green).
/// - `STRATEGIC_AIR_COLOR_AVERAGE` (`:908`) — contested (yellow).
/// - `STRATEGIC_AIR_COLOR_NEUTRAL` (`:909`) — no air activity (grey).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrategicAirColors {
    pub bad: [f32; 4],
    pub good: [f32; 4],
    pub average: [f32; 4],
    pub neutral: [f32; 4],
}

/// The vanilla `00_graphics.lua` values, verbatim (FILE, cited above). Offline
/// gate reference; the resolver path re-parses the shipped file so mods override.
pub const VANILLA_STRATEGIC_AIR_COLORS: StrategicAirColors = StrategicAirColors {
    bad: [0.8, 0.0, 0.0, 1.0],
    good: [0.0, 0.8, 0.0, 1.0],
    average: [0.8, 0.8, 0.0, 1.0],
    neutral: [140.0 / 255.0, 131.0 / 255.0, 119.0 / 255.0, 1.0],
};

/// Evaluate one Lua colour-list token, handling the `a/b` division form the
/// NEUTRAL colour ships (`140.0/255`). The shared `parse_lua_number_list` cannot
/// (it `parse::<f32>()`s each token whole), so the air module carries its own
/// tiny evaluator.
fn eval_number_token(token: &str) -> Option<f32> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    let mut parts = token.split('/');
    let mut acc = parts.next()?.trim().parse::<f32>().ok()?;
    for p in parts {
        acc /= p.trim().parse::<f32>().ok()?;
    }
    Some(acc)
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Parse `KEY = { a, b, c, d }` (division tokens allowed), with identifier
/// boundaries so `STRATEGIC_AIR_COLOR_NEUTRAL` does not match
/// `STRATEGIC_AIR_COLOR_NEUTRAL_WHILE_HIGHLIGHTING_HOLD`.
fn parse_color4(source: &str, key: &str) -> Result<[f32; 4]> {
    for line in source.lines() {
        let code = line.split("--").next().unwrap_or(line);
        let bytes = code.as_bytes();
        let Some(pos) = code.find(key) else {
            continue;
        };
        let end = pos + key.len();
        let before_ok = pos == 0 || !is_ident_byte(bytes[pos - 1]);
        let after_ok = end >= bytes.len() || !is_ident_byte(bytes[end]);
        if !before_ok || !after_ok {
            continue;
        }
        let rest = code[end..].trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('{') else {
            continue;
        };
        let Some(close) = rest.find('}') else {
            continue;
        };
        let values = rest[..close]
            .split(',')
            .filter_map(eval_number_token)
            .collect::<Vec<_>>();
        if values.len() != 4 {
            return Err(format!(
                "{AIR_GRAPHICS_DEFINES_SOURCE}: {key} expected 4 components, got {}",
                values.len()
            )
            .into());
        }
        return Ok([values[0], values[1], values[2], values[3]]);
    }
    Err(format!("{AIR_GRAPHICS_DEFINES_SOURCE}: missing colour define {key}").into())
}

/// Parse the strategic-air status colours out of `00_graphics.lua` source text.
pub fn strategic_air_colors_from_source(source: &str) -> Result<StrategicAirColors> {
    Ok(StrategicAirColors {
        bad: parse_color4(source, "STRATEGIC_AIR_COLOR_BAD")?,
        good: parse_color4(source, "STRATEGIC_AIR_COLOR_GOOD")?,
        average: parse_color4(source, "STRATEGIC_AIR_COLOR_AVERAGE")?,
        neutral: parse_color4(source, "STRATEGIC_AIR_COLOR_NEUTRAL")?,
    })
}

/// Load + parse the strategic-air colours via the mod-aware resolver.
pub fn strategic_air_colors_from_resolver(
    resolver: &GameFileResolver,
) -> Result<StrategicAirColors> {
    let path =
        resolver.resolve_required(AIR_GRAPHICS_DEFINES_SOURCE, "strategic air status colours")?;
    let source = crate::read_latin1(&path)?;
    strategic_air_colors_from_source(&source)
}

/// The air-overlay alpha (opacity), read from the file colours' 4th component.
/// All four `STRATEGIC_AIR_COLOR_*` ship alpha 1.0 in vanilla, so the overlay is
/// opaque; this returns the NEUTRAL alpha so the value stays file-tied even under
/// a mod that changed it. FILE.
pub fn air_overlay_alpha(colors: &StrategicAirColors) -> f32 {
    colors.neutral[3]
}

// ---------------------------------------------------------------------------
// Air-region geometry (the field the tint is painted into).
// ---------------------------------------------------------------------------

/// One air region: a strategic region and ALL of its provinces (land + sea).
/// Distinct from `naval_supremacy_tint::SeaZone`, which keeps sea provinces only.
#[derive(Clone, Debug, PartialEq)]
pub struct AirRegion {
    pub region_id: u32,
    pub name: String,
    /// Every province in the region, sorted — the full field geometry.
    pub provinces: Vec<u32>,
}

/// The full air-region set derived from the shipped strategic-region files.
#[derive(Clone, Debug, PartialEq)]
pub struct AirRegionSet {
    /// region id -> air region (every strategic region is an air region).
    pub regions: BTreeMap<u32, AirRegion>,
    /// province -> owning region id.
    pub province_to_region: BTreeMap<u32, u32>,
    /// Total strategic regions parsed (== air regions; nothing is filtered out).
    pub total_regions: usize,
}

/// Build the air-region set from a raw strategic-region join. Pure, deterministic.
///
/// `region_to_provinces` is the raw `map/strategicregions` join
/// (`crate::load_strategic_regions`); `region_names` supplies the display name.
/// Unlike the naval classifier, NO province is dropped — an air region spans its
/// whole strategic region.
pub fn build_air_regions(
    region_to_provinces: &BTreeMap<u32, Vec<u32>>,
    region_names: impl Fn(u32) -> String,
) -> AirRegionSet {
    let mut regions = BTreeMap::new();
    let mut province_to_region = BTreeMap::new();
    for (&region_id, provinces) in region_to_provinces {
        let mut provinces = provinces.clone();
        provinces.sort_unstable();
        provinces.dedup();
        for &province in &provinces {
            province_to_region.insert(province, region_id);
        }
        regions.insert(
            region_id,
            AirRegion {
                region_id,
                name: region_names(region_id),
                provinces,
            },
        );
    }
    AirRegionSet {
        total_regions: region_to_provinces.len(),
        regions,
        province_to_region,
    }
}

/// Load the air-region set from the shipped map files via the mod-aware resolver
/// (`map/strategicregions`). Reuses the exact loader the naval tint rides.
pub fn air_region_set_from_resolver(resolver: &GameFileResolver) -> Result<AirRegionSet> {
    let region_files = resolver.resolve_dir_files(AIR_STRATEGIC_REGIONS_DIR, Some("txt"))?;
    let (region_to_provinces, region_meta, _province_to_region) =
        crate::load_strategic_regions(&region_files)?;
    Ok(build_air_regions(&region_to_provinces, |id| {
        region_meta
            .get(&id)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| format!("STRATEGICREGION_{id}"))
    }))
}

// ---------------------------------------------------------------------------
// Air superiority -> status -> tint.
// ---------------------------------------------------------------------------

/// Per-region air-superiority snapshot, shaped like
/// `replay_core::AirSuperiorityRow`. `None` = the leaf is absent/`Unobtained` in
/// the recording (fail-closed to no air activity).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AirActivityInput {
    /// The side holding air superiority in the region.
    pub controller: Option<String>,
    /// Bitmask of active air missions in the region (opaque; carried through).
    pub mission_mask: Option<i64>,
    /// Number of wings operating in the region.
    pub wing_count: Option<i64>,
    /// Number of planes operating in the region.
    pub plane_count: Option<i64>,
}

/// The omniscient-observer status of an air region.
#[derive(Clone, Debug, PartialEq)]
pub enum AirRegionStatus {
    /// No usable air activity -> engine shows the neutral tone.
    NoActivity,
    /// Air superiority held by `controller`, with the operating force carried
    /// through for the reader (magnitudes verbatim, no rescale).
    Contested {
        controller: String,
        plane_count: Option<i64>,
        wing_count: Option<i64>,
        mission_mask: Option<i64>,
    },
}

/// Classify an air-superiority input into a region status (fail-closed).
///
/// Requires a non-empty controller tag AND at least one plane: a controller with
/// zero planes (or an absent plane count) is treated as no air activity rather
/// than inventing a contest. `wing_count`/`mission_mask` are carried but do not
/// gate the classification.
pub fn air_region_status(input: &AirActivityInput) -> AirRegionStatus {
    match (&input.controller, input.plane_count) {
        (Some(controller), Some(planes)) if !controller.is_empty() && planes > 0 => {
            AirRegionStatus::Contested {
                controller: controller.clone(),
                plane_count: input.plane_count,
                wing_count: input.wing_count,
                mission_mask: input.mission_mask,
            }
        }
        _ => AirRegionStatus::NoActivity,
    }
}

/// POV-relative air relationship (E6 observe-as-country mode). The good/average/
/// bad status colour is keyed on this; in the default omniscient view it is not
/// applied (there is no viewed country to be relative to).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PovAirRelation {
    /// The viewed country holds air superiority here.
    Friendly,
    /// Air superiority is contested / mixed relative to the viewed country.
    Contested,
    /// An enemy holds air superiority here.
    Enemy,
}

/// The POV status colour for a relationship (FILE: `STRATEGIC_AIR_COLOR_{GOOD,
/// AVERAGE,BAD}`). E6-gated; the *classification* of a region into
/// Friendly/Contested/Enemy needs the viewed country's diplomacy + the
/// superiority-ratio thresholds, which are **UNKNOWN / capture-gated** (see
/// module docs). This function only maps an already-decided relation to its file
/// colour.
pub fn pov_air_status_color(relation: PovAirRelation, colors: &StrategicAirColors) -> [f32; 4] {
    match relation {
        PovAirRelation::Friendly => colors.good,
        PovAirRelation::Contested => colors.average,
        PovAirRelation::Enemy => colors.bad,
    }
}

/// The computed tint for one air region under the default omniscient observer.
#[derive(Clone, Debug, PartialEq)]
pub struct AirRegionTint {
    pub region_id: u32,
    pub status: AirRegionStatus,
    /// The RGBA the region field is keyed with. `NoActivity` -> the FILE NEUTRAL
    /// colour; `Contested` -> the controller's country colour at the file overlay
    /// alpha (DERIVED observer choice, see module docs). `None` only when a
    /// contested region's controller has no colour entry (fail-closed).
    pub fill_rgba: Option<[f32; 4]>,
}

/// Compute the omniscient-observer tint for one air region.
///
/// `country_colors` is the resolved tag -> RGB table (`load_country_colors*`,
/// incl. cosmetic-tag overrides). A contested region whose controller has no
/// colour entry fails closed to `None` (no invented colour). NoActivity always
/// yields the file NEUTRAL colour.
pub fn air_region_tint(
    region_id: u32,
    input: &AirActivityInput,
    country_colors: &BTreeMap<String, [u8; 3]>,
    colors: &StrategicAirColors,
) -> AirRegionTint {
    let status = air_region_status(input);
    let fill_rgba = match &status {
        AirRegionStatus::NoActivity => Some(colors.neutral),
        AirRegionStatus::Contested { controller, .. } => {
            country_colors.get(controller).map(|rgb| {
                [
                    rgb[0] as f32 / 255.0,
                    rgb[1] as f32 / 255.0,
                    rgb[2] as f32 / 255.0,
                    air_overlay_alpha(colors),
                ]
            })
        }
    };
    AirRegionTint {
        region_id,
        status,
        fill_rgba,
    }
}

/// Compute tints for every air region: regions with an air-superiority input get
/// their computed tint; air regions absent from `inputs` degrade to the NEUTRAL
/// (no-activity) fill.
pub fn air_region_tints(
    regions: &AirRegionSet,
    inputs: &BTreeMap<u32, AirActivityInput>,
    country_colors: &BTreeMap<String, [u8; 3]>,
    colors: &StrategicAirColors,
) -> BTreeMap<u32, AirRegionTint> {
    let mut out = BTreeMap::new();
    for &region_id in regions.regions.keys() {
        let input = inputs.get(&region_id).cloned().unwrap_or_default();
        out.insert(
            region_id,
            air_region_tint(region_id, &input, country_colors, colors),
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
		STRATEGIC_AIR_COLOR_BAD = {0.8, 0, 0, 1}, -- rgb
		STRATEGIC_AIR_COLOR_GOOD = {0, 0.8, 0, 1},
		STRATEGIC_AIR_COLOR_AVERAGE = {0.8, 0.8, 0, 1},
		STRATEGIC_AIR_COLOR_NEUTRAL = {140.0/255, 131.0/255, 119.0/255, 1},
		STRATEGIC_AIR_COLOR_GOOD_WHILE_HIGHLIGHTING_HOLD = {0, 0.8, 0, 1},
		AIR_REGION_SUPERIORITY_PIXEL_SCALE = 0.04,
	}
}
"#;

    #[test]
    fn colors_parse_matches_vanilla_constant() {
        let parsed = strategic_air_colors_from_source(GRAPHICS_SNIPPET).expect("parse colours");
        assert_eq!(parsed, VANILLA_STRATEGIC_AIR_COLORS);
        // The overlay is opaque per the file (all four ship alpha 1.0).
        assert!((air_overlay_alpha(&parsed) - 1.0).abs() < 1e-6);
        assert!((parsed.bad[3] - 1.0).abs() < 1e-6);
        assert!((parsed.good[3] - 1.0).abs() < 1e-6);
        assert!((parsed.average[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn build_air_regions_keeps_all_provinces() {
        // region 10: land+sea mixed. region 20: pure land. Both are air regions
        // and keep EVERY province (no sea filtering).
        let mut r2p = BTreeMap::new();
        r2p.insert(10u32, vec![101u32, 100, 102]);
        r2p.insert(20u32, vec![200u32, 201]);
        let set = build_air_regions(&r2p, |id| format!("STRATEGICREGION_{id}"));
        assert_eq!(set.total_regions, 2);
        assert_eq!(set.regions.len(), 2);
        assert_eq!(set.regions[&10].provinces, vec![100, 101, 102]);
        assert_eq!(set.regions[&20].provinces, vec![200, 201]);
        assert_eq!(set.province_to_region.get(&100), Some(&10));
        assert_eq!(set.province_to_region.get(&201), Some(&20));
    }

    #[test]
    fn status_fails_closed() {
        // No controller -> no activity.
        assert_eq!(
            air_region_status(&AirActivityInput::default()),
            AirRegionStatus::NoActivity
        );
        // Controller present but zero planes -> no activity (no invented contest).
        assert_eq!(
            air_region_status(&AirActivityInput {
                controller: Some("ENG".to_string()),
                plane_count: Some(0),
                wing_count: Some(3),
                mission_mask: Some(5),
            }),
            AirRegionStatus::NoActivity
        );
        // Empty controller string -> no activity.
        assert_eq!(
            air_region_status(&AirActivityInput {
                controller: Some(String::new()),
                plane_count: Some(50),
                ..Default::default()
            }),
            AirRegionStatus::NoActivity
        );
    }

    #[test]
    fn tint_neutral_and_controller_and_missing_color() {
        let colors = VANILLA_STRATEGIC_AIR_COLORS;
        let country_colors: BTreeMap<String, [u8; 3]> =
            BTreeMap::from([("ENG".to_string(), [200u8, 40, 40])]);
        // No activity -> file NEUTRAL colour.
        let t = air_region_tint(1, &AirActivityInput::default(), &country_colors, &colors);
        assert_eq!(t.status, AirRegionStatus::NoActivity);
        assert_eq!(t.fill_rgba, Some(colors.neutral));
        // Contested with known controller -> controller colour at overlay alpha.
        let t = air_region_tint(
            1,
            &AirActivityInput {
                controller: Some("ENG".to_string()),
                plane_count: Some(120),
                wing_count: Some(2),
                mission_mask: Some(5),
            },
            &country_colors,
            &colors,
        );
        assert_eq!(
            t.fill_rgba,
            Some([200.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0, 1.0])
        );
        // Contested with UNKNOWN controller colour -> fail closed to None.
        let t = air_region_tint(
            1,
            &AirActivityInput {
                controller: Some("ZZZ".to_string()),
                plane_count: Some(120),
                ..Default::default()
            },
            &country_colors,
            &colors,
        );
        assert!(matches!(t.status, AirRegionStatus::Contested { .. }));
        assert_eq!(t.fill_rgba, None);
    }

    #[test]
    fn pov_status_colors_map_to_file() {
        let c = VANILLA_STRATEGIC_AIR_COLORS;
        assert_eq!(pov_air_status_color(PovAirRelation::Friendly, &c), c.good);
        assert_eq!(
            pov_air_status_color(PovAirRelation::Contested, &c),
            c.average
        );
        assert_eq!(pov_air_status_color(PovAirRelation::Enemy, &c), c.bad);
    }

    /// `fx_naval_air_activity` gate. Expectations derived from the fixture
    /// generator SOURCE (`replay_fixture_gen/src/main.rs::fx_naval_air_activity`),
    /// NOT by running the code under test:
    ///   air_superiority["301"] = {controller, mission_mask: 5,
    ///                             wing_count: 2 + tick, plane_count: 120 + tick*10}
    ///   controller = "ENG" if tick < 7 else "GER"   (SNAPSHOT_COUNT = 12)
    #[test]
    fn fx_naval_air_activity_region_statuses() {
        let colors = VANILLA_STRATEGIC_AIR_COLORS;
        let country_colors: BTreeMap<String, [u8; 3]> = BTreeMap::from([
            ("ENG".to_string(), [200u8, 40, 40]),
            ("GER".to_string(), [80u8, 80, 90]),
        ]);
        let expected = |tick: usize| -> (&'static str, i64, i64) {
            let controller = if tick < 7 { "ENG" } else { "GER" };
            let wing_count = 2 + tick as i64;
            let plane_count = 120 + tick as i64 * 10;
            (controller, wing_count, plane_count)
        };
        for tick in [0usize, 6, 7, 11] {
            let (controller, wing_count, plane_count) = expected(tick);
            let input = AirActivityInput {
                controller: Some(controller.to_string()),
                mission_mask: Some(5),
                wing_count: Some(wing_count),
                plane_count: Some(plane_count),
            };
            let t = air_region_tint(301, &input, &country_colors, &colors);
            assert_eq!(
                t.status,
                AirRegionStatus::Contested {
                    controller: controller.to_string(),
                    plane_count: Some(plane_count),
                    wing_count: Some(wing_count),
                    mission_mask: Some(5),
                },
                "tick {tick} status"
            );
            let rgb = country_colors[controller];
            assert_eq!(
                t.fill_rgba,
                Some([
                    rgb[0] as f32 / 255.0,
                    rgb[1] as f32 / 255.0,
                    rgb[2] as f32 / 255.0,
                    1.0
                ]),
                "tick {tick} controller fill"
            );
        }
        // Controller flips ENG (tick 6) -> GER (tick 7).
        assert_eq!(expected(6).0, "ENG");
        assert_eq!(expected(7).0, "GER");

        // An air region with no superiority input degrades to the NEUTRAL fill.
        let regions = AirRegionSet {
            regions: BTreeMap::from([(
                301,
                AirRegion {
                    region_id: 301,
                    name: "STRATEGICREGION_301".to_string(),
                    provinces: vec![1, 2, 3],
                },
            )]),
            province_to_region: BTreeMap::from([(1, 301), (2, 301), (3, 301)]),
            total_regions: 1,
        };
        let tints = air_region_tints(&regions, &BTreeMap::new(), &country_colors, &colors);
        assert_eq!(tints[&301].status, AirRegionStatus::NoActivity);
        assert_eq!(tints[&301].fill_rgba, Some(colors.neutral));
    }

    /// Vanilla parse gate (skips without `BATTLE_PLAN_GAME_DIR`). Air regions == every
    /// strategic region, so the count matches the naval module's
    /// `total_strategic_regions` (304) with nothing filtered out.
    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_air_region_parse() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let set = air_region_set_from_resolver(&resolver).expect("air region set");
        assert_eq!(set.total_regions, 304, "vanilla strategic (air) regions");
        assert_eq!(
            set.regions.len(),
            304,
            "every strategic region is an air region"
        );
        // Region 301 (South Island) is pure land -> excluded from naval regions
        // but PRESENT as an air region (air covers land).
        assert!(
            set.regions.contains_key(&301),
            "air regions include pure-land regions"
        );
        // The status colours also parse from the shipped file.
        let colors = strategic_air_colors_from_resolver(&resolver).expect("colours");
        assert_eq!(colors, VANILLA_STRATEGIC_AIR_COLORS);
    }
}
