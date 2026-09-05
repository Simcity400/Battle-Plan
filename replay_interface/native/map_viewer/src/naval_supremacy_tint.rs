
use crate::mod_resolver::GameFileResolver;
use crate::Result;
use std::collections::BTreeMap;

pub const NAVAL_SUPREMACY_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
pub const NAVAL_SUPREMACY_STRATEGIC_REGIONS_DIR: &str = "map/strategicregions";
pub const NAVAL_SUPREMACY_DEFINITION_CSV_SOURCE: &str = "map/definition.csv";


#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NavalTintDefines {
    pub transparency_ratio_min: f32,
    pub transparency_ratio_max: f32,
    pub transparency_range: f32,
    pub enemy_stripes_color: [f32; 4],
    pub our_stripes_color: [f32; 4],
    pub ally_stripes_color: [f32; 4],
    pub country_color_stripes_transp: f32,
}

pub const VANILLA_NAVAL_TINT_DEFINES: NavalTintDefines = NavalTintDefines {
    transparency_ratio_min: 0.5,
    transparency_ratio_max: 0.8,
    transparency_range: 2000.0,
    enemy_stripes_color: [0.827, 0.172, 0.184, 0.0],
    our_stripes_color: [0.427, 0.619, 0.858, 0.0],
    ally_stripes_color: [0.427, 0.619, 0.858, 0.0],
    country_color_stripes_transp: 0.0,
};

pub fn naval_tint_defines_from_resolver(resolver: &GameFileResolver) -> Result<NavalTintDefines> {
    let (sources, _) = crate::xmap09_lighting_law::effective_define_sources(resolver, "effective defines")?;
    let evaluated = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&sources)?;

    Ok(NavalTintDefines {
        transparency_ratio_min: evaluated.scalar("NMapMode", "NAVAL_REGION_TRANSPARENCY_RATIO_MIN")?,
        transparency_ratio_max: evaluated.scalar("NMapMode", "NAVAL_REGION_TRANSPARENCY_RATIO_MAX")?,
        transparency_range: evaluated.scalar("NMapMode", "NAVAL_REGION_TRANSPARENCY_RANGE")?,
        enemy_stripes_color: evaluated.array("NMapMode", "STRATEGIC_MODE_ENEMY_STRIPES_COLOR")?,
        our_stripes_color: evaluated.array("NMapMode", "STRATEGIC_MODE_OUR_STRIPES_COLOR")?,
        ally_stripes_color: evaluated.array("NMapMode", "STRATEGIC_MODE_ALLY_STRIPES_COLOR")?,
        country_color_stripes_transp: evaluated.scalar("NMapMode", "STRATEGIC_MODE_COUNTRY_COLOR_STRIPES_TRANSP")?,
    })

}


#[derive(Clone, Debug, PartialEq)]
pub struct SeaZone {
    pub region_id: u32,
    pub name: String,
    pub sea_provinces: Vec<u32>,
    pub land_province_count: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NavalRegionSet {
    pub zones: BTreeMap<u32, SeaZone>,
    pub province_to_zone: BTreeMap<u32, u32>,
    pub total_strategic_regions: usize,
    pub pure_sea_region_count: usize,
    pub sea_province_count: usize,
}

pub fn province_is_sea(province_type: &str) -> bool {
    province_type == "sea"
}

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
            continue;
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

pub fn naval_region_set_from_resolver(resolver: &GameFileResolver) -> Result<NavalRegionSet> {
    let region_files =
        resolver.resolve_dir_files_recursive(NAVAL_SUPREMACY_STRATEGIC_REGIONS_DIR, Some("txt"))?;
    let (region_to_provinces, region_meta, _province_to_region) =
        crate::load_strategic_regions(resolver, &region_files)?;
    let definition_csv = resolver.resolve_required(
        NAVAL_SUPREMACY_DEFINITION_CSV_SOURCE,
        "naval supremacy definition.csv",
    )?;
    let (definitions, _color_to_id) = crate::load_definition_csv(resolver, &definition_csv)?;
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


#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoneSupremacyInput {
    pub controller: Option<String>,
    pub supremacy: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ZoneSupremacyStatus {
    OpenOcean,
    Controlled { controller: String, supremacy: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ZoneTint {
    pub region_id: u32,
    pub status: ZoneSupremacyStatus,
    pub fill_rgba: Option<[f32; 4]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PovNavalRelation {
    Own,
    Ally,
    Enemy,
}

pub fn pov_stripe_color(relation: PovNavalRelation, defines: &NavalTintDefines) -> [f32; 4] {
    match relation {
        PovNavalRelation::Own => defines.our_stripes_color,
        PovNavalRelation::Ally => defines.ally_stripes_color,
        PovNavalRelation::Enemy => defines.enemy_stripes_color,
    }
}

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

pub fn supremacy_ratio(supremacy: f64, defines: &NavalTintDefines) -> f32 {
    if defines.transparency_range <= 0.0 {
        return 0.0;
    }
    ((supremacy as f32) / defines.transparency_range).clamp(0.0, 1.0)
}

pub fn supremacy_alpha(supremacy: f64, defines: &NavalTintDefines) -> f32 {
    let ratio = supremacy_ratio(supremacy, defines);
    defines.transparency_ratio_min
        + (defines.transparency_ratio_max - defines.transparency_ratio_min) * ratio
}

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
	NMapMode = {
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
        let parsed = crate::test_support::with_define_fixture(GRAPHICS_SNIPPET, naval_tint_defines_from_resolver).expect("parse defines");
        assert_eq!(parsed, VANILLA_NAVAL_TINT_DEFINES);
    }

    #[test]
    fn classify_splits_sea_land_and_islands() {
        let mut r2p = BTreeMap::new();
        r2p.insert(10u32, vec![100u32, 101]);
        r2p.insert(20u32, vec![200u32, 201, 202]);
        r2p.insert(30u32, vec![300u32, 301]);
        let types: BTreeMap<u32, &str> = BTreeMap::from([
            (100, "sea"),
            (101, "sea"),
            (200, "sea"),
            (201, "sea"),
            (202, "land"),
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
        assert_eq!(set.zones[&20].sea_provinces, vec![200, 201]);
        assert_eq!(set.zones[&20].land_province_count, 1);
        assert_eq!(set.province_to_zone.get(&202), None);
        assert_eq!(set.province_to_zone.get(&200), Some(&20));
    }

    #[test]
    fn supremacy_alpha_law() {
        let d = VANILLA_NAVAL_TINT_DEFINES;
        assert!((supremacy_alpha(0.0, &d) - 0.5).abs() < 1e-6);
        assert!((supremacy_alpha(2000.0, &d) - 0.8).abs() < 1e-6);
        assert!((supremacy_alpha(5000.0, &d) - 0.8).abs() < 1e-6);
        assert!((supremacy_alpha(1000.0, &d) - 0.65).abs() < 1e-6);
    }

    #[test]
    fn fail_closed_open_ocean() {
        let d = VANILLA_NAVAL_TINT_DEFINES;
        let colors: BTreeMap<String, [u8; 3]> = BTreeMap::from([("ENG".to_string(), [1u8, 2, 3])]);
        let t = zone_tint(1, &ZoneSupremacyInput::default(), &colors, &d);
        assert_eq!(t.status, ZoneSupremacyStatus::OpenOcean);
        assert_eq!(t.fill_rgba, None);
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

    #[test]
    fn fx_naval_air_activity_zone_statuses() {
        let d = VANILLA_NAVAL_TINT_DEFINES;
        let colors: BTreeMap<String, [u8; 3]> = BTreeMap::from([
            ("ENG".to_string(), [200u8, 40, 40]),
            ("GER".to_string(), [80u8, 80, 90]),
        ]);
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
        assert_eq!(expected(4).0, "ENG");
        assert_eq!(expected(5).0, "GER");

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
        assert!(!set.zones.contains_key(&301), "301 is a land region");
        let defines = naval_tint_defines_from_resolver(&resolver).expect("defines");
        assert_eq!(defines, VANILLA_NAVAL_TINT_DEFINES);
    }
}
