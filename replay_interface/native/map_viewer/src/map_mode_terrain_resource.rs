//! Terrain (map mode 6) and Resource (map mode 9) map-mode data laws.
//!
//! Two file-static map modes flagged **law-known** in
//! `MAPMODE_COVERAGE_MATRIX.md`. Both compute their fill/overlay replay-side from
//! shipped files, **never recorded** (standing override). This module owns the
//! *data law*: province/state -> the file-derived colour or overlay identity.
//! Follows the `naval_supremacy_tint` pattern (pure classify fns + mod-aware
//! resolver loaders + fail-closed on missing evidence).
//!
//! ## Terrain mode (6) — province terrain-type fill (fully static, FILE)
//!
//! Each province has a terrain **category** in `map/definition.csv` column 6
//! (0-indexed: `id;r;g;b;type;coastal;terrain;continent`) — already parsed into
//! [`crate::ProvinceDefinition::terrain`]. Each category's fill colour is the
//! `color = { r g b }` under `categories = { <name> = { color = ... } }` in
//! `common/terrain/00_terrain.txt`. So the terrain map-mode fill of a province is
//! `category_colours[definitions[province].terrain]`. Every colour is FILE-cited
//! to `00_terrain.txt`; a province whose terrain field names no defined category
//! fails **closed** (no fill, counted as unclassified) rather than inventing one.
//! The separate graphical `terrain = { terrain_N = { color = { <index> } } }`
//! block maps `terrain.bmp` palette indices to categories and is NOT used here —
//! the per-province category (definition.csv) is the exact, complete key.
//!
//! ## Resource mode (9) — state resource-deposit overlay (deposits FILE, fill UNKNOWN)
//!
//! Resource **definitions** come from `common/resources/00_resources.txt`
//! (`resources = { <name> = { icon_frame = N ... } }`); the vanilla set is 7
//! resources (oil/aluminium/rubber/tungsten/steel/chromium/coal) with
//! `icon_frame` 1..7. Deposit **locations + magnitudes** come from each state's
//! `resources = { <name> = <amount> }` block in `history/states/*.txt`. The
//! resource's on-map visual identity is its strip icon
//! (`GFX_resources_strip` -> `gfx/interface/resources_strip.dds`, frame =
//! `icon_frame`; `interface/general_stuff.gfx`).
//!
//! **Honesty (fail-closed):** NO shipped file defines a per-resource *map-mode
//! fill colour* or a display *threshold*. The resource map mode overlays the
//! strip icon on states holding a deposit; the fill/tint is engine-side and is an
//! open RE/capture item. Therefore [`resource_state_fill`] returns `None`
//! (UNKNOWN, fail-closed — no guessed colour), while the FILE-derived deliverable
//! is the deposit set + resource definition (icon frame) exposed by
//! [`resource_state_overlay`]. Controller tinting (colour a deposit by who holds
//! the state) uses the existing recorded control (requirements item 2) + country
//! colours and is left to the integration seam, exactly like the naval tint.

use crate::mod_resolver::GameFileResolver;
use crate::{ProvinceDefinition, Result};
use std::collections::BTreeMap;

// ---------------------------------------------------------------------------
// Source citations.
// ---------------------------------------------------------------------------

/// Terrain **category colour** table (`categories = { <name> = { color } }`).
pub const TERRAIN_CATEGORIES_SOURCE: &str = "common/terrain/00_terrain.txt";
/// Province -> terrain-category join (`definition.csv` column 6).
pub const TERRAIN_DEFINITION_CSV_SOURCE: &str = "map/definition.csv";
/// Resource **definitions** (`resources = { <name> = { icon_frame } }`).
pub const RESOURCE_DEFINITIONS_SOURCE: &str = "common/resources/00_resources.txt";
/// Resource **deposit** locations + magnitudes (per-state `resources` block).
pub const RESOURCE_DEPOSITS_STATES_DIR: &str = "history/states";
/// Resource on-map icon strip (display identity; `GFX_resources_strip`,
/// `interface/general_stuff.gfx`). FILE identity only — NOT a fill colour.
pub const RESOURCE_ICON_STRIP_SOURCE: &str = "gfx/interface/resources_strip.dds";

// ===========================================================================
// TERRAIN MODE (6).
// ===========================================================================

/// The vanilla `00_terrain.txt` `categories` colour table, verbatim (FILE, cited
/// per entry). Used as the offline gate reference; the resolver path re-parses the
/// shipped file so mods override correctly.
///
/// FILE citations (`common/terrain/00_terrain.txt`, `categories` block):
/// `unknown={255,0,0}`, `ocean={40,83,176}`, `lakes={58,91,255}`,
/// `forest={89,199,85}`, `hills={248,255,153}`, `mountain={157,192,208}`,
/// `plains={255,129,66}`, `urban={120,120,120}`, `jungle={127,191,0}`,
/// `marsh={76,96,35}`, `desert={255,127,0}`, `water_fjords={75,162,198}`,
/// `water_shallow_sea={56,118,217}`, `water_deep_ocean={2,38,150}`.
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

/// Terrain map-mode classification result over a full province set.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TerrainMapMode {
    /// province id -> terrain-category fill RGB (only classified provinces).
    pub province_fill: BTreeMap<u32, [u8; 3]>,
    /// category name -> number of provinces assigned to it.
    pub category_province_counts: BTreeMap<String, usize>,
    /// Provinces whose terrain field named no defined category (fail-closed, no
    /// fill). 0 for vanilla.
    pub unclassified: Vec<u32>,
    /// Total provinces considered.
    pub total_provinces: usize,
}

impl TerrainMapMode {
    /// Provinces that received a fill colour.
    pub fn classified_count(&self) -> usize {
        self.province_fill.len()
    }
}

/// Walk a paradox-script block's top-level `ident = { ... }` entries, returning
/// `(name, inner_body)` pairs by brace matching. Pure; ignores non-block
/// top-level assignments. Input should be comment-stripped.
fn iter_top_level_blocks(block: &str) -> Vec<(String, String)> {
    let bytes = block.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        // Skip to the next identifier start.
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
        // Expect '=' then '{'.
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        if j >= bytes.len() || bytes[j] != b'=' {
            // Not a block assignment; resume scanning after the identifier.
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
        // Brace-match the body.
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

/// Parse the terrain-category colour table from `00_terrain.txt` source text.
///
/// Reads only the `categories = { ... }` block (the graphical `terrain = { ... }`
/// palette block is deliberately ignored). Each category's `color = { r g b }`
/// becomes its map-mode fill RGB. Categories without a 3-component colour are
/// skipped (fail-closed).
pub fn parse_terrain_category_colors(source: &str) -> Result<BTreeMap<String, [u8; 3]>> {
    let text = crate::strip_comments(source);
    let (inner, _) = crate::extract_block_after(&text, "categories", 0)
        .ok_or_else(|| format!("{TERRAIN_CATEGORIES_SOURCE}: missing `categories` block"))?;
    let mut out = BTreeMap::new();
    for (name, body) in iter_top_level_blocks(&inner) {
        let Some((color_inner, _)) = crate::extract_block_after(&body, "color", 0) else {
            continue;
        };
        let nums = crate::parse_u32s(&color_inner);
        if nums.len() < 3 {
            continue;
        }
        let rgb = [nums[0] as u8, nums[1] as u8, nums[2] as u8];
        out.insert(name, rgb);
    }
    if out.is_empty() {
        return Err(format!("{TERRAIN_CATEGORIES_SOURCE}: parsed 0 category colours").into());
    }
    Ok(out)
}

/// Load + parse the terrain-category colour table via the mod-aware resolver.
pub fn terrain_category_colors_from_resolver(
    resolver: &GameFileResolver,
) -> Result<BTreeMap<String, [u8; 3]>> {
    let path = resolver.resolve_required(TERRAIN_CATEGORIES_SOURCE, "terrain category colours")?;
    let source = crate::read_latin1(&path)?;
    parse_terrain_category_colors(&source)
}

/// The terrain-mode fill colour of a single province: its `definition.csv`
/// terrain category mapped through the category colour table. `None` (fail-closed)
/// when the province is unknown or its terrain field names no defined category.
pub fn province_terrain_fill(
    province_id: u32,
    definitions: &BTreeMap<u32, ProvinceDefinition>,
    category_colors: &BTreeMap<String, [u8; 3]>,
) -> Option<[u8; 3]> {
    let def = definitions.get(&province_id)?;
    category_colors.get(&def.terrain).copied()
}

/// Classify every province in `definitions` into a terrain-mode fill. Pure: no
/// I/O, deterministic. Provinces whose terrain category is undefined are collected
/// into `unclassified` (fail-closed, no fill).
pub fn classify_terrain_map_mode(
    definitions: &BTreeMap<u32, ProvinceDefinition>,
    category_colors: &BTreeMap<String, [u8; 3]>,
) -> TerrainMapMode {
    let mut province_fill = BTreeMap::new();
    let mut category_province_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut unclassified = Vec::new();
    for (&province_id, def) in definitions {
        match category_colors.get(&def.terrain) {
            Some(rgb) => {
                province_fill.insert(province_id, *rgb);
                *category_province_counts
                    .entry(def.terrain.clone())
                    .or_insert(0) += 1;
            }
            None => unclassified.push(province_id),
        }
    }
    TerrainMapMode {
        province_fill,
        category_province_counts,
        unclassified,
        total_provinces: definitions.len(),
    }
}

/// Load the terrain map mode from the shipped map files via the mod-aware
/// resolver (`map/definition.csv` + `common/terrain/00_terrain.txt`).
pub fn terrain_map_mode_from_resolver(resolver: &GameFileResolver) -> Result<TerrainMapMode> {
    let definition_csv =
        resolver.resolve_required(TERRAIN_DEFINITION_CSV_SOURCE, "terrain definition.csv")?;
    let (definitions, _color_to_id) = crate::load_definition_csv(&definition_csv)?;
    let category_colors = terrain_category_colors_from_resolver(resolver)?;
    Ok(classify_terrain_map_mode(&definitions, &category_colors))
}

// ===========================================================================
// RESOURCE MODE (9).
// ===========================================================================

/// A resource definition from `00_resources.txt`: its name + strip `icon_frame`
/// (the file-derived on-map visual identity; NOT a fill colour).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceDefinition {
    pub name: String,
    /// `icon_frame` into `gfx/interface/resources_strip.dds`.
    pub icon_frame: u32,
}

/// The vanilla `00_resources.txt` resource set, verbatim (FILE). 7 resources,
/// `icon_frame` 1..7 in file order. Offline gate reference; the resolver path
/// re-parses the shipped file so mods add/override resources.
pub const VANILLA_RESOURCE_DEFINITIONS: &[(&str, u32)] = &[
    ("oil", 1),
    ("aluminium", 2),
    ("rubber", 3),
    ("tungsten", 4),
    ("steel", 5),
    ("chromium", 6),
    ("coal", 7),
];

/// Parse resource definitions from `00_resources.txt` source text. Reads the
/// `resources = { <name> = { icon_frame = N ... } }` block; a resource without an
/// `icon_frame` is skipped (fail-closed). Preserves file order.
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

/// Load + parse resource definitions via the mod-aware resolver.
pub fn resource_definitions_from_resolver(
    resolver: &GameFileResolver,
) -> Result<Vec<ResourceDefinition>> {
    let path = resolver.resolve_required(RESOURCE_DEFINITIONS_SOURCE, "resource definitions")?;
    let source = crate::read_latin1(&path)?;
    parse_resource_definitions(&source)
}

/// Parse one state file's resource-deposit block into `resource name -> amount`.
///
/// `resource_names` is the known resource set (from [`ResourceDefinition`]), so
/// only defined resources are counted (mod-aware). Amounts are summed across
/// repeated keys. Returns `(state_id, deposits)`; `None` if the file has no `id`.
/// A missing/empty `resources` block yields empty deposits.
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

/// The full resource-deposit set derived from the shipped state files, with the
/// classification counts used by the vanilla parse gate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceDepositSet {
    /// The resource definitions (name + icon frame) this set was keyed with.
    pub definitions: Vec<ResourceDefinition>,
    /// state id -> (resource name -> summed deposit amount). Only states with at
    /// least one deposit entry appear.
    pub state_deposits: BTreeMap<u32, BTreeMap<String, i64>>,
    /// Number of states with >= 1 deposit entry.
    pub states_with_deposits: usize,
    /// Total (state, resource) deposit entries across all states.
    pub total_deposit_entries: usize,
    /// resource name -> number of states holding it.
    pub per_resource_state_count: BTreeMap<String, usize>,
    /// resource name -> summed deposit amount across all states.
    pub per_resource_total: BTreeMap<String, i64>,
}

/// Classify a set of parsed state deposits into a [`ResourceDepositSet`]. Pure.
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

/// Load the resource-deposit set from the shipped files via the mod-aware
/// resolver (`common/resources` + `history/states`).
pub fn resource_deposit_set_from_resolver(
    resolver: &GameFileResolver,
) -> Result<ResourceDepositSet> {
    let definitions = resource_definitions_from_resolver(resolver)?;
    let names: Vec<String> = definitions.iter().map(|d| d.name.clone()).collect();
    let state_files = resolver.resolve_dir_files(RESOURCE_DEPOSITS_STATES_DIR, Some("txt"))?;
    let mut parsed = Vec::new();
    for (_, path) in &state_files {
        let text = crate::read_latin1(path)?;
        if let Some(entry) = parse_state_resource_deposits(&text, &names) {
            parsed.push(entry);
        }
    }
    Ok(classify_resource_deposits(definitions, parsed))
}

/// The FILE-derived resource-mode overlay for one state: the deposits it holds
/// and, for each, the resource's strip icon frame. This is what the resource map
/// mode actually places on the map (per `00_resources.txt` + `history/states`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceStateOverlay {
    pub state_id: u32,
    /// (resource name, icon_frame, amount), sorted by resource name.
    pub deposits: Vec<(String, u32, i64)>,
}

/// The FILE-derived resource overlay for a state, or `None` if the state holds no
/// deposit. Resources with no matching definition (icon frame) are skipped.
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

/// The resource map-mode **fill colour** of a state.
///
/// **UNKNOWN — always fail-closed to `None`.** No shipped file defines a
/// per-resource map-mode fill colour or display threshold; the resource mode
/// overlays the strip ICON (see [`resource_state_overlay`]) and its fill/tint is
/// engine-side (open RE/capture item, `FIDELITY_LEDGER` §2.20). Returning `None`
/// keeps the honesty rule: no guessed colour. The signature mirrors
/// [`province_terrain_fill`] so the two modes share a fill contract.
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
        // The graphical `terrain = {}` block must NOT be parsed as a category.
        assert!(!colors.contains_key("terrain_0"));
        assert!(!colors.contains_key("ocean_15"));
        assert_eq!(colors.len(), 4);
    }

    #[test]
    fn vanilla_terrain_constant_is_self_consistent() {
        // The 14 vanilla categories, unique names.
        let mut names: Vec<&str> = VANILLA_TERRAIN_CATEGORY_COLORS
            .iter()
            .map(|(n, _)| *n)
            .collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 14, "14 distinct vanilla terrain categories");
    }

    #[test]
    fn classify_terrain_fills_and_fails_closed() {
        let colors: BTreeMap<String, [u8; 3]> = VANILLA_TERRAIN_CATEGORY_COLORS
            .iter()
            .map(|(n, c)| (n.to_string(), *c))
            .collect();
        let mut defs = BTreeMap::new();
        defs.insert(10u32, def("plains"));
        defs.insert(11u32, def("ocean"));
        defs.insert(12u32, def("forest"));
        defs.insert(13u32, def("no_such_category")); // fail-closed
        let mode = classify_terrain_map_mode(&defs, &colors);
        assert_eq!(mode.total_provinces, 4);
        assert_eq!(mode.classified_count(), 3);
        assert_eq!(mode.unclassified, vec![13]);
        assert_eq!(mode.province_fill.get(&10), Some(&[255u8, 129, 66]));
        assert_eq!(mode.province_fill.get(&11), Some(&[40u8, 83, 176]));
        assert_eq!(mode.category_province_counts.get("plains"), Some(&1));
        // Single-province lookup matches.
        assert_eq!(
            province_terrain_fill(12, &defs, &colors),
            Some([89u8, 199, 85])
        );
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
        // Handles no-space `chromium=23`, inline `# was:` comments, and a resource
        // key OUTSIDE the resources block (must NOT count).
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
            (3u32, BTreeMap::new()), // empty -> skipped
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
    fn resource_overlay_is_file_derived_fill_is_unknown() {
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
        // sorted by resource name: oil(frame 1), steel(frame 5).
        assert_eq!(
            overlay.deposits,
            vec![("oil".to_string(), 1, 3), ("steel".to_string(), 5, 6),]
        );
        // No deposits -> no overlay.
        assert!(resource_state_overlay(8, &BTreeMap::new(), &defs).is_none());
        // Fill colour is UNKNOWN / fail-closed for every state (no file colour).
        assert_eq!(resource_state_fill(7, &deposits, &defs), None);
    }

    /// Vanilla terrain parse gate (skips without `BATTLE_PLAN_GAME_DIR`). Asserts the
    /// full category table + 0 unclassified provinces against the shipped files.
    #[test]
    fn gate_vanilla_terrain_map_mode() {
        let Ok(game_dir) = std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")) else {
            eprintln!("gate_vanilla_terrain_map_mode: BATTLE_PLAN_GAME_DIR unset; skipped");
            return;
        };
        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        // Category colour table matches the vanilla constant byte-for-byte.
        let colors = terrain_category_colors_from_resolver(&resolver).expect("colors");
        assert_eq!(colors.len(), 14, "14 vanilla terrain categories");
        for (name, rgb) in VANILLA_TERRAIN_CATEGORY_COLORS {
            assert_eq!(colors.get(*name), Some(rgb), "category {name} colour");
        }
        // Every province is classified: 0 unclassified.
        let mode = terrain_map_mode_from_resolver(&resolver).expect("mode");
        assert_eq!(mode.total_provinces, 13414, "vanilla province count");
        assert_eq!(mode.unclassified.len(), 0, "0 unclassified provinces");
        assert_eq!(mode.classified_count(), 13414);
        // The 11 categories actually used by vanilla provinces + their counts.
        assert_eq!(mode.category_province_counts.get("ocean"), Some(&3133));
        assert_eq!(mode.category_province_counts.get("plains"), Some(&3031));
        assert_eq!(mode.category_province_counts.get("forest"), Some(&2364));
        assert_eq!(mode.category_province_counts.get("mountain"), Some(&1708));
        assert_eq!(mode.category_province_counts.get("hills"), Some(&1352));
        assert_eq!(mode.category_province_counts.get("desert"), Some(&815));
        assert_eq!(mode.category_province_counts.get("jungle"), Some(&477));
        assert_eq!(mode.category_province_counts.get("urban"), Some(&265));
        assert_eq!(mode.category_province_counts.get("marsh"), Some(&142));
        assert_eq!(mode.category_province_counts.get("lakes"), Some(&126));
        assert_eq!(mode.category_province_counts.get("unknown"), Some(&1));
        assert_eq!(
            mode.category_province_counts.len(),
            11,
            "11 used categories"
        );
    }

    /// Vanilla resource parse gate (skips without `BATTLE_PLAN_GAME_DIR`). Asserts the
    /// resource definitions + full deposit tally against the shipped files.
    #[test]
    fn gate_vanilla_resource_deposit_set() {
        let Ok(game_dir) = std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")) else {
            eprintln!("gate_vanilla_resource_deposit_set: BATTLE_PLAN_GAME_DIR unset; skipped");
            return;
        };
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
        // Per-resource state counts + summed amounts (block-scoped, comment-stripped).
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
        // Sum of per-resource state counts == total entries.
        let sum: usize = set.per_resource_state_count.values().sum();
        assert_eq!(sum, set.total_deposit_entries);
    }
}
