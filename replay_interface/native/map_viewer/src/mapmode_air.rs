
use crate::mod_resolver::GameFileResolver;
use crate::Result;
use std::collections::BTreeMap;

pub const AIR_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
pub const AIR_STRATEGIC_REGIONS_DIR: &str = "map/strategicregions";


#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StrategicAirColors {
    pub bad: [f32; 4],
    pub good: [f32; 4],
    pub average: [f32; 4],
    pub neutral: [f32; 4],
}

pub const VANILLA_STRATEGIC_AIR_COLORS: StrategicAirColors = StrategicAirColors {
    bad: [0.8, 0.0, 0.0, 1.0],
    good: [0.0, 0.8, 0.0, 1.0],
    average: [0.8, 0.8, 0.0, 1.0],
    neutral: [140.0 / 255.0, 131.0 / 255.0, 119.0 / 255.0, 1.0],
};

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

pub fn strategic_air_colors_from_source(source: &str) -> Result<StrategicAirColors> {
    Ok(StrategicAirColors {
        bad: parse_color4(source, "STRATEGIC_AIR_COLOR_BAD")?,
        good: parse_color4(source, "STRATEGIC_AIR_COLOR_GOOD")?,
        average: parse_color4(source, "STRATEGIC_AIR_COLOR_AVERAGE")?,
        neutral: parse_color4(source, "STRATEGIC_AIR_COLOR_NEUTRAL")?,
    })
}

pub fn strategic_air_colors_from_resolver(
    resolver: &GameFileResolver,
) -> Result<StrategicAirColors> {
    let path =
        resolver.resolve_required(AIR_GRAPHICS_DEFINES_SOURCE, "strategic air status colours")?;
    let source = crate::read_latin1(&path)?;
    strategic_air_colors_from_source(&source)
}

pub fn air_overlay_alpha(colors: &StrategicAirColors) -> f32 {
    colors.neutral[3]
}


#[derive(Clone, Debug, PartialEq)]
pub struct AirRegion {
    pub region_id: u32,
    pub name: String,
    pub provinces: Vec<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AirRegionSet {
    pub regions: BTreeMap<u32, AirRegion>,
    pub province_to_region: BTreeMap<u32, u32>,
    pub total_regions: usize,
}

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


#[derive(Clone, Debug, Default, PartialEq)]
pub struct AirActivityInput {
    pub controller: Option<String>,
    pub mission_mask: Option<i64>,
    pub wing_count: Option<i64>,
    pub plane_count: Option<i64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum AirRegionStatus {
    NoActivity,
    Contested {
        controller: String,
        plane_count: Option<i64>,
        wing_count: Option<i64>,
        mission_mask: Option<i64>,
    },
}

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PovAirRelation {
    Friendly,
    Contested,
    Enemy,
}

pub fn pov_air_status_color(relation: PovAirRelation, colors: &StrategicAirColors) -> [f32; 4] {
    match relation {
        PovAirRelation::Friendly => colors.good,
        PovAirRelation::Contested => colors.average,
        PovAirRelation::Enemy => colors.bad,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AirRegionTint {
    pub region_id: u32,
    pub status: AirRegionStatus,
    pub fill_rgba: Option<[f32; 4]>,
}

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
        assert!((air_overlay_alpha(&parsed) - 1.0).abs() < 1e-6);
        assert!((parsed.bad[3] - 1.0).abs() < 1e-6);
        assert!((parsed.good[3] - 1.0).abs() < 1e-6);
        assert!((parsed.average[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn build_air_regions_keeps_all_provinces() {
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
        assert_eq!(
            air_region_status(&AirActivityInput::default()),
            AirRegionStatus::NoActivity
        );
        assert_eq!(
            air_region_status(&AirActivityInput {
                controller: Some("ENG".to_string()),
                plane_count: Some(0),
                wing_count: Some(3),
                mission_mask: Some(5),
            }),
            AirRegionStatus::NoActivity
        );
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
        let t = air_region_tint(1, &AirActivityInput::default(), &country_colors, &colors);
        assert_eq!(t.status, AirRegionStatus::NoActivity);
        assert_eq!(t.fill_rgba, Some(colors.neutral));
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
        assert_eq!(expected(6).0, "ENG");
        assert_eq!(expected(7).0, "GER");

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
        assert!(
            set.regions.contains_key(&301),
            "air regions include pure-land regions"
        );
        let colors = strategic_air_colors_from_resolver(&resolver).expect("colours");
        assert_eq!(colors, VANILLA_STRATEGIC_AIR_COLORS);
    }
}
