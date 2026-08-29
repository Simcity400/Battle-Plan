
use crate::mod_resolver::GameFileResolver;
use crate::{ProvinceDefinition, Result};
use std::collections::BTreeMap;


pub const TERRAIN_CATEGORIES_SOURCE: &str = "common/terrain/00_terrain.txt";
pub const TERRAIN_DEFINITION_CSV_SOURCE: &str = "map/definition.csv";
pub const RESOURCE_DEFINITIONS_SOURCE: &str = "common/resources/00_resources.txt";
pub const RESOURCE_DEPOSITS_STATES_DIR: &str = "history/states";
pub const RESOURCE_ICON_STRIP_SOURCE: &str = "gfx/interface/resources_strip.dds";
pub const RESOURCE_DECLARATION_SOURCE: &str = "interface/general_stuff.gfx";


pub const VANILLA_TERRAIN_CATEGORY_COLORS: &[(&str, [u8; 3])] = &[
    ("unknown", [255, 0, 0]),
    ("ocean", [40, 83, 176]),
    ("lakes", [58, 91, 255]),
    ("forest", [89, 199, 85]),
    ("hills", [248, 255, 153]),
    ("mountain", [157, 192, 208]),
    ("plains", [255, 129, 66]),
    ("urban", [120, 120, 120]),
    ("jungle", [127, 191, 0]),
    ("marsh", [76, 96, 35]),
    ("desert", [255, 127, 0]),
    ("water_fjords", [75, 162, 198]),
    ("water_shallow_sea", [56, 118, 217]),
    ("water_deep_ocean", [2, 38, 150]),
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainMapMode {
    pub province_fill: BTreeMap<u32, [u8; 3]>,
    pub category_province_counts: BTreeMap<String, usize>,
    pub unclassified: Vec<u32>,
    pub total_provinces: usize,
}

impl TerrainMapMode {
    pub fn classified_count(&self) -> usize {
        self.province_fill.len()
    }
}

fn iter_top_level_blocks(block: &str) -> Vec<(String, String)> {
    let bytes = block.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        while i < bytes.len() && !(bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
            i += 1;
        }
        if i == name_start {
            break;
        }
        let name = block[name_start..i].to_string();
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'=' {
            i = name_start + name.len();
            continue;
        }
        j += 1;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'{' {
            i = j;
            continue;
        }
        let inner_start = j + 1;
        let mut depth = 0i32;
        let mut k = j;
        let mut end = None;
        while k < bytes.len() {
            match bytes[k] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(k);
                        break;
                    }
                }
                _ => {}
            }
            k += 1;
        }
        let body_end = end.unwrap_or(bytes.len());
        out.push((name, block[inner_start..body_end].to_string()));
        i = body_end + 1;
    }
    out
}

pub fn parse_terrain_category_colors(source: &str) -> Result<BTreeMap<String, [u8; 3]>> {
    let out = parse_terrain_category_colors_optional(source)?;
    if out.is_empty() {
        return Err(format!("{TERRAIN_CATEGORIES_SOURCE}: parsed 0 category colours").into());
    }
    Ok(out)
}

fn parse_terrain_category_colors_optional(source: &str) -> Result<BTreeMap<String, [u8; 3]>> {
    let text = crate::strip_comments(source);
    let Some((inner, _)) = crate::extract_block_after(&text, "categories", 0) else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for (name, body) in iter_top_level_blocks(&inner) {
        let Some((color_inner, _)) = crate::extract_block_after(&body, "color", 0) else {
            continue;
        };
        let nums = crate::parse_u32s(&color_inner);
        if nums.len() < 3 || nums[..3].iter().any(|v| *v > 255) {
            continue;
        }
        let rgb = [nums[0] as u8, nums[1] as u8, nums[2] as u8];
        out.insert(name, rgb);
    }
    Ok(out)
}

pub fn terrain_category_colors_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, [u8; 3]>> {
    let files = resolver.resolve_dir_files_recursive("common/terrain", Some("txt"))?;
    let mut merged = BTreeMap::new();
    for (_, path) in &files {
        let source = crate::read_latin1(path)?;
        merged.extend(parse_terrain_category_colors_optional(&source)?);
    }
    if merged.is_empty() {
        return Err(format!(
            "common/terrain/*.txt parsed 0 category colours from {} effective files",
            files.len()
        )
        .into());
    }
    Ok(merged)
}

#[cfg(test)]
fn province_terrain_fill(
    _province_id: u32,
    _definitions: &BTreeMap<u32, ProvinceDefinition>,
    _category_colors: &BTreeMap<String, [u8; 3]>,
) -> Option<[u8; 3]> {
    None
}

pub(crate) fn classify_terrain_map_mode(
    definitions: &BTreeMap<u32, ProvinceDefinition>,
    _category_colors: &BTreeMap<String, [u8; 3]>,
) -> TerrainMapMode {
    let province_fill = BTreeMap::new();
    let category_province_counts = BTreeMap::new();
    let unclassified = definitions.keys().copied().collect();
    TerrainMapMode {
        province_fill,
        category_province_counts,
        unclassified,
        total_provinces: definitions.len(),
    }
}

pub fn terrain_map_mode_from_resolver(resolver: &GameFileResolver) -> Result<TerrainMapMode> {
    let definition_csv =
        resolver.resolve_required(TERRAIN_DEFINITION_CSV_SOURCE, "terrain definition.csv")?;
    let (definitions, _color_to_id) = crate::load_definition_csv(&definition_csv)?;
    let category_colors = terrain_category_colors_from_resolver(resolver)?;
    Ok(classify_terrain_map_mode(&definitions, &category_colors))
}

pub(crate) fn terrain_province_rgba_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<u32, [u8; 4]>> {
    let definition_csv =
        resolver.resolve_required(TERRAIN_DEFINITION_CSV_SOURCE, "terrain definition.csv")?;
    let (definitions, _) = crate::load_definition_csv(&definition_csv)?;
    let category_colors = terrain_category_colors_from_resolver(resolver)?;

    let mut naval_by_province = BTreeMap::<u32, String>::new();
    for (name, path) in resolver.resolve_dir_files_recursive("map/strategicregions", Some("txt"))? {
        let source = crate::strip_comments(&crate::read_latin1(&path)?);
        let Some(naval_terrain) = crate::parse_word_value(&source, "naval_terrain") else {
            continue;
        };
        let provinces = crate::extract_block_after(&source, "provinces", 0)
            .map(|(inner, _)| crate::parse_u32s(&inner))
            .unwrap_or_default();
        for province_id in provinces {
            if naval_by_province
                .insert(province_id, naval_terrain.clone())
                .is_some()
            {
                return Err(format!(
                    "map/strategicregions/{name}: province {province_id} has multiple naval terrain owners"
                )
                .into());
            }
        }
    }

    let mut out = BTreeMap::new();
    for (province_id, definition) in definitions {
        let is_land = definition.province_type.eq_ignore_ascii_case("land");
        let category = if is_land {
            definition.terrain.as_str()
        } else if definition.province_type.eq_ignore_ascii_case("lake") {
            "ocean"
        } else {
            naval_by_province
                .get(&province_id)
                .map(String::as_str)
                .unwrap_or(definition.terrain.as_str())
        };
        let rgb = category_colors.get(category).ok_or_else(|| {
            format!(
                "terrain province {province_id} selects undefined effective category {category}"
            )
        })?;
        out.insert(
            province_id,
            [rgb[0], rgb[1], rgb[2], if is_land { 127 } else { 204 }],
        );
    }
    Ok(out)
}


#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceDefinition {
    pub name: String,
    pub icon_frame: u32,
}

pub const VANILLA_RESOURCE_DEFINITIONS: &[(&str, u32)] = &[
    ("oil", 1),
    ("aluminium", 2),
    ("rubber", 3),
    ("tungsten", 4),
    ("steel", 5),
    ("chromium", 6),
    ("coal", 7),
];

pub fn parse_resource_definitions(source: &str) -> Result<Vec<ResourceDefinition>> {
    let text = crate::strip_comments(source);
    let (inner, _) = crate::extract_block_after(&text, "resources", 0)
        .ok_or_else(|| format!("{RESOURCE_DEFINITIONS_SOURCE}: missing `resources` block"))?;
    let mut out = Vec::new();
    for (name, body) in iter_top_level_blocks(&inner) {
        let Some(icon_frame) = crate::parse_u32_value(&body, "icon_frame") else {
            continue;
        };
        out.push(ResourceDefinition { name, icon_frame });
    }
    if out.is_empty() {
        return Err(format!("{RESOURCE_DEFINITIONS_SOURCE}: parsed 0 resource definitions").into());
    }
    Ok(out)
}

pub fn resource_definitions_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<ResourceDefinition>> {
    let path = resolver.resolve_required(RESOURCE_DEFINITIONS_SOURCE, "resource definitions")?;
    let source = crate::read_latin1(&path)?;
    parse_resource_definitions(&source)
}

pub fn parse_state_resource_deposits(
    state_text: &str,
    resource_names: &[String],
) -> Option<(u32, BTreeMap<String, i64>)> {
    let text = crate::strip_comments(state_text);
    let state_id = crate::parse_u32_value(&text, "id")?;
    let mut deposits = BTreeMap::new();
    if let Some((block, _)) = crate::extract_block_after(&text, "resources", 0) {
        for name in resource_names {
            let mut pos = 0usize;
            while let Some(idx) = crate::find_assignment(&block, name, pos) {
                if let Some(amount) = crate::parse_u32_at(&block[idx..]) {
                    *deposits.entry(name.clone()).or_insert(0) += amount as i64;
                }
                pos = idx;
            }
        }
    }
    Some((state_id, deposits))
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceDepositSet {
    pub definitions: Vec<ResourceDefinition>,
    pub state_deposits: BTreeMap<u32, BTreeMap<String, i64>>,
    pub states_with_deposits: usize,
    pub total_deposit_entries: usize,
    pub per_resource_state_count: BTreeMap<String, usize>,
    pub per_resource_total: BTreeMap<String, i64>,
}

pub fn classify_resource_deposits(
    definitions: Vec<ResourceDefinition>,
    states: impl IntoIterator<Item = (u32, BTreeMap<String, i64>)>,
) -> ResourceDepositSet {
    let mut state_deposits = BTreeMap::new();
    let mut states_with_deposits = 0usize;
    let mut total_deposit_entries = 0usize;
    let mut per_resource_state_count: BTreeMap<String, usize> = BTreeMap::new();
    let mut per_resource_total: BTreeMap<String, i64> = BTreeMap::new();
    for (state_id, deposits) in states {
        if deposits.is_empty() {
            continue;
        }
        states_with_deposits += 1;
        for (name, &amount) in &deposits {
            total_deposit_entries += 1;
            *per_resource_state_count.entry(name.clone()).or_insert(0) += 1;
            *per_resource_total.entry(name.clone()).or_insert(0) += amount;
        }
        state_deposits.insert(state_id, deposits);
    }
    ResourceDepositSet {
        definitions,
        state_deposits,
        states_with_deposits,
        total_deposit_entries,
        per_resource_state_count,
        per_resource_total,
    }
}

pub fn resource_deposit_set_from_resolver(
    resolver: &GameFileResolver,
) -> Result<ResourceDepositSet> {
    let definitions = resource_definitions_from_resolver(resolver)?;
    let names: Vec<String> = definitions.iter().map(|d| d.name.clone()).collect();
    let state_files =
        resolver.resolve_dir_files_recursive(RESOURCE_DEPOSITS_STATES_DIR, Some("txt"))?;
    let mut parsed = Vec::new();
    for (_, path) in &state_files {
        let text = crate::read_latin1(path)?;
        if let Some(entry) = parse_state_resource_deposits(&text, &names) {
            parsed.push(entry);
        }
    }
    Ok(classify_resource_deposits(definitions, parsed))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceStateOverlay {
    pub state_id: u32,
    pub deposits: Vec<(String, u32, i64)>,
}

pub fn resource_state_overlay(
    state_id: u32,
    deposits: &BTreeMap<String, i64>,
    definitions: &[ResourceDefinition],
) -> Option<ResourceStateOverlay> {
    let frame = |name: &str| {
        definitions
            .iter()
            .find(|d| d.name == name)
            .map(|d| d.icon_frame)
    };
    let mut rows = Vec::new();
    for (name, &amount) in deposits {
        if let Some(f) = frame(name) {
            rows.push((name.clone(), f, amount));
        }
    }
    if rows.is_empty() {
        return None;
    }
    Some(ResourceStateOverlay {
        state_id,
        deposits: rows,
    })
}

pub fn resource_state_fill(
    _state_id: u32,
    _deposits: &BTreeMap<String, i64>,
    _definitions: &[ResourceDefinition],
) -> Option<[u8; 3]> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(terrain: &str) -> ProvinceDefinition {
        ProvinceDefinition {
            rgb: [1, 2, 3],
            province_type: "land".to_string(),
            coastal: false,
            terrain: terrain.to_string(),
            continent: 1,
        }
    }

    const TERRAIN_SNIPPET: &str = r#"
categories =  {
	unknown = { color = { 255 0 0 } }
	ocean = { naval_terrain = yes color = { 40 83 176 } is_water = yes }
	forest = {
		color = { 89 199 85 }
		movement_cost = 1.5
		buildings_max_level = { bunker = 4 coastal_bunker = 6 }
	}
	plains = { color = { 255 129 66 } }
}

terrain = {
	terrain_0 = { type = plains color = { 0 } texture = 1 }
	ocean_15  = { type = ocean  color = { 15 } texture = 9 }
}
"#;

    const RESOURCE_SNIPPET: &str = r#"
resources = {
	oil = { icon_frame = 1 cic = 0.125 convoys = 0.1 }
	steel = { icon_frame = 5 cic = 0.125 convoys = 0.1 }
	coal = { icon_frame = 7 cic = 0.125 convoys = 0.1 }
}
"#;

    #[test]
    fn terrain_category_colors_parse() {
        let colors = parse_terrain_category_colors(TERRAIN_SNIPPET).expect("parse");
        assert_eq!(colors.get("unknown"), Some(&[255u8, 0, 0]));
        assert_eq!(colors.get("ocean"), Some(&[40u8, 83, 176]));
        assert_eq!(colors.get("forest"), Some(&[89u8, 199, 85]));
        assert_eq!(colors.get("plains"), Some(&[255u8, 129, 66]));
        assert!(!colors.contains_key("terrain_0"));
        assert!(!colors.contains_key("ocean_15"));
        assert_eq!(colors.len(), 4);
    }

    #[test]
    fn terrain_category_color_out_of_range_component_is_skipped() {
        let snippet = r#"
categories = {
	bad = { color = { 300 10 10 } }
	good = { color = { 10 20 30 } }
}
"#;
        let colors = parse_terrain_category_colors(snippet).expect("parse");
        assert!(!colors.contains_key("bad"));
        assert_eq!(colors.get("good"), Some(&[10u8, 20, 30]));
        assert_eq!(colors.len(), 1);
    }

    #[test]
    fn vanilla_terrain_constant_is_self_consistent() {
        let mut names: Vec<&str> = VANILLA_TERRAIN_CATEGORY_COLORS
            .iter()
            .map(|(n, _)| *n)
            .collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 14, "14 distinct vanilla terrain categories");
    }

    #[test]
    fn terrain_category_proxy_is_retained_only_as_evidence() {
        let colors: BTreeMap<String, [u8; 3]> = VANILLA_TERRAIN_CATEGORY_COLORS
            .iter()
            .map(|(n, c)| (n.to_string(), *c))
            .collect();
        let mut defs = BTreeMap::new();
        defs.insert(10u32, def("plains"));
        defs.insert(11u32, def("ocean"));
        defs.insert(12u32, def("forest"));
        defs.insert(13u32, def("no_such_category"));
        let mode = classify_terrain_map_mode(&defs, &colors);
        assert_eq!(mode.total_provinces, 4);
        assert_eq!(mode.classified_count(), 0);
        assert_eq!(mode.unclassified, vec![10, 11, 12, 13]);
        assert!(mode.province_fill.is_empty());
        assert!(mode.category_province_counts.is_empty());
        assert_eq!(province_terrain_fill(12, &defs, &colors), None);
        assert_eq!(province_terrain_fill(13, &defs, &colors), None);
        assert_eq!(province_terrain_fill(999, &defs, &colors), None);
    }

    #[test]
    fn resource_definitions_parse_in_file_order() {
        let defs = parse_resource_definitions(RESOURCE_SNIPPET).expect("parse");
        assert_eq!(
            defs,
            vec![
                ResourceDefinition {
                    name: "oil".to_string(),
                    icon_frame: 1
                },
                ResourceDefinition {
                    name: "steel".to_string(),
                    icon_frame: 5
                },
                ResourceDefinition {
                    name: "coal".to_string(),
                    icon_frame: 7
                },
            ]
        );
    }

    #[test]
    fn vanilla_resource_constant_frames_1_to_7() {
        let frames: Vec<u32> = VANILLA_RESOURCE_DEFINITIONS
            .iter()
            .map(|(_, f)| *f)
            .collect();
        assert_eq!(frames, vec![1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(VANILLA_RESOURCE_DEFINITIONS.len(), 7);
    }

    #[test]
    fn parse_state_deposits_block_scoped_and_comment_stripped() {
        let state = r#"
			id = 42
			resources={
				aluminium=7 # was: 10
				coal = 2 # initial distribution
			}
			state_category = town
			history = {
				owner = GER
				# steel = 999 (this is a comment, ignored)
			}
"#;
        let names: Vec<String> = [
            "oil",
            "aluminium",
            "rubber",
            "tungsten",
            "steel",
            "chromium",
            "coal",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (id, deposits) = parse_state_resource_deposits(state, &names).expect("id");
        assert_eq!(id, 42);
        assert_eq!(deposits.get("aluminium"), Some(&7));
        assert_eq!(deposits.get("coal"), Some(&2));
        assert_eq!(deposits.len(), 2, "steel in a later block must not leak in");
    }

    #[test]
    fn empty_resources_block_yields_no_deposits() {
        let state = "id = 9\nresources={\n}\nstate_category = rural\n";
        let names = vec!["steel".to_string()];
        let (id, deposits) = parse_state_resource_deposits(state, &names).expect("id");
        assert_eq!(id, 9);
        assert!(deposits.is_empty());
    }

    #[test]
    fn classify_resource_deposits_counts() {
        let defs: Vec<ResourceDefinition> = VANILLA_RESOURCE_DEFINITIONS
            .iter()
            .map(|(n, f)| ResourceDefinition {
                name: n.to_string(),
                icon_frame: *f,
            })
            .collect();
        let states = vec![
            (1u32, BTreeMap::from([("steel".to_string(), 6i64)])),
            (
                2u32,
                BTreeMap::from([("oil".to_string(), 3i64), ("coal".to_string(), 2)]),
            ),
            (3u32, BTreeMap::new()),
            (4u32, BTreeMap::from([("steel".to_string(), 4i64)])),
        ];
        let set = classify_resource_deposits(defs, states);
        assert_eq!(set.states_with_deposits, 3);
        assert_eq!(set.total_deposit_entries, 4);
        assert_eq!(set.per_resource_state_count.get("steel"), Some(&2));
        assert_eq!(set.per_resource_total.get("steel"), Some(&10));
        assert_eq!(set.per_resource_total.get("oil"), Some(&3));
        assert!(!set.state_deposits.contains_key(&3), "empty state excluded");
    }

    #[test]
    fn resource_overlay_is_file_derived_and_fill_has_no_override() {
        let defs: Vec<ResourceDefinition> = VANILLA_RESOURCE_DEFINITIONS
            .iter()
            .map(|(n, f)| ResourceDefinition {
                name: n.to_string(),
                icon_frame: *f,
            })
            .collect();
        let deposits = BTreeMap::from([("steel".to_string(), 6i64), ("oil".to_string(), 3)]);
        let overlay = resource_state_overlay(7, &deposits, &defs).expect("overlay");
        assert_eq!(overlay.state_id, 7);
        assert_eq!(
            overlay.deposits,
            vec![("oil".to_string(), 1, 3), ("steel".to_string(), 5, 6),]
        );
        assert!(resource_state_overlay(8, &BTreeMap::new(), &defs).is_none());
        assert_eq!(resource_state_fill(7, &deposits, &defs), None);
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_terrain_map_mode() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let colors = terrain_category_colors_from_resolver(&resolver).expect("colors");
        assert_eq!(colors.len(), 14, "14 vanilla terrain categories");
        for (name, rgb) in VANILLA_TERRAIN_CATEGORY_COLORS {
            assert_eq!(colors.get(*name), Some(rgb), "category {name} colour");
        }
        let mode = terrain_map_mode_from_resolver(&resolver).expect("mode");
        // 13413, not 13414: the definition map no longer carries row position
        // engine's null instance and never a province.
        assert_eq!(mode.total_provinces, 13413, "vanilla province count");
        assert_eq!(mode.unclassified.len(), 13413);
        assert_eq!(mode.classified_count(), 0);
        assert!(mode.province_fill.is_empty());
        assert!(mode.category_province_counts.is_empty());
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_resource_deposit_set() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let defs = resource_definitions_from_resolver(&resolver).expect("defs");
        let def_pairs: Vec<(String, u32)> = defs
            .iter()
            .map(|d| (d.name.clone(), d.icon_frame))
            .collect();
        let expected: Vec<(String, u32)> = VANILLA_RESOURCE_DEFINITIONS
            .iter()
            .map(|(n, f)| (n.to_string(), *f))
            .collect();
        assert_eq!(
            def_pairs, expected,
            "7 resource defs, frames 1..7, file order"
        );

        let set = resource_deposit_set_from_resolver(&resolver).expect("deposit set");
        assert_eq!(set.states_with_deposits, 467, "states with >=1 deposit");
        assert_eq!(set.total_deposit_entries, 717, "total deposit entries");
        assert_eq!(set.per_resource_state_count.get("oil"), Some(&83));
        assert_eq!(set.per_resource_state_count.get("aluminium"), Some(&95));
        assert_eq!(set.per_resource_state_count.get("rubber"), Some(&44));
        assert_eq!(set.per_resource_state_count.get("tungsten"), Some(&83));
        assert_eq!(set.per_resource_state_count.get("steel"), Some(&166));
        assert_eq!(set.per_resource_state_count.get("chromium"), Some(&93));
        assert_eq!(set.per_resource_state_count.get("coal"), Some(&153));
        assert_eq!(set.per_resource_total.get("oil"), Some(&1209));
        assert_eq!(set.per_resource_total.get("aluminium"), Some(&1201));
        assert_eq!(set.per_resource_total.get("rubber"), Some(&1032));
        assert_eq!(set.per_resource_total.get("tungsten"), Some(&1346));
        assert_eq!(set.per_resource_total.get("steel"), Some(&2591));
        assert_eq!(set.per_resource_total.get("chromium"), Some(&1345));
        assert_eq!(set.per_resource_total.get("coal"), Some(&1463));
        let sum: usize = set.per_resource_state_count.values().sum();
        assert_eq!(sum, set.total_deposit_entries);
    }
}
