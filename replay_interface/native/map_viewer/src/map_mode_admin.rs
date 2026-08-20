//! XMAP00_VIEWER_VISUAL_INPUT: viewer.map_mode_selection

use crate::mod_resolver::GameFileResolver;
use crate::Result;
use std::collections::{BTreeMap, BTreeSet};

pub const ADMIN_MAPMODE_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
pub const IDEOLOGIES_DIR: &str = "common/ideologies";


#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdminMapModeDefines {
    pub ideology_color_transparency: f32,
    pub player_not_selected_transparency: f32,
}

pub const VANILLA_ADMIN_MAPMODE_DEFINES: AdminMapModeDefines = AdminMapModeDefines {
    ideology_color_transparency: 1.0,
    player_not_selected_transparency: 0.15,
};

pub fn admin_mapmode_defines_from_source(source: &str) -> Result<AdminMapModeDefines> {
    Ok(AdminMapModeDefines {
        ideology_color_transparency: crate::parse_lua_number_define(
            source,
            "MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY",
        )?,
        player_not_selected_transparency: crate::parse_lua_number_define(
            source,
            "PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY",
        )?,
    })
}

pub fn admin_mapmode_defines_from_resolver(
    resolver: &GameFileResolver,
) -> Result<AdminMapModeDefines> {
    let path = resolver.resolve_required(
        ADMIN_MAPMODE_GRAPHICS_DEFINES_SOURCE,
        "admin map-mode graphics defines",
    )?;
    let source = crate::read_latin1(&path)?;
    admin_mapmode_defines_from_source(&source)
}

fn alpha_byte(ratio: f32) -> u8 {
    (ratio.clamp(0.0, 1.0) * 255.0).round() as u8
}


#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdeologyColorTable {
    pub colors: BTreeMap<String, [u8; 3]>,
}

impl IdeologyColorTable {
    pub fn get(&self, ideology: &str) -> Option<[u8; 3]> {
        self.colors.get(ideology).copied()
    }
}

pub fn vanilla_ideology_colors() -> IdeologyColorTable {
    IdeologyColorTable {
        colors: BTreeMap::from([
            ("democratic".to_string(), [0u8, 0, 255]),
            ("communism".to_string(), [255u8, 0, 0]),
            ("fascism".to_string(), [150u8, 75, 0]),
            ("neutrality".to_string(), [124u8, 124, 124]),
        ]),
    }
}

fn strip_hash_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split('#').next().unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn parse_ideology_colors_into(source: &str, out: &mut IdeologyColorTable) -> Result<()> {
    let text = strip_hash_comments(source);
    let bytes = text.as_bytes();
    let Some(block_start) = find_block_open(&text, "ideologies") else {
        return Ok(());
    };
    let mut i = block_start;
    let mut depth = 1i32;
    while i < bytes.len() && depth >= 1 {
        let b = bytes[i];
        if b == b'{' {
            depth += 1;
            i += 1;
            continue;
        }
        if b == b'}' {
            depth -= 1;
            i += 1;
            continue;
        }
        if depth == 1 && (b.is_ascii_alphanumeric() || b == b'_') {
            let (name, after_name) = read_token(&text, i);
            let rest = text[after_name..].trim_start();
            if rest.starts_with('=') {
                let after_eq = after_name + (text[after_name..].find('=').unwrap()) + 1;
                let tail = text[after_eq..].trim_start();
                if tail.starts_with('{') {
                    let group_open = after_eq + text[after_eq..].find('{').unwrap();
                    let group_close = match_brace(&text, group_open);
                    if let Some(close) = group_close {
                        let inner = &text[group_open + 1..close];
                        if let Some(rgb) = parse_group_color(inner) {
                            out.colors.insert(name, rgb);
                        }
                        i = close + 1;
                        continue;
                    }
                }
            }
            i = after_name;
            continue;
        }
        i += 1;
    }
    Ok(())
}

pub fn ideology_colors_from_source(source: &str) -> Result<IdeologyColorTable> {
    let mut out = IdeologyColorTable::default();
    parse_ideology_colors_into(source, &mut out)?;
    Ok(out)
}

pub fn ideology_colors_from_resolver(resolver: &GameFileResolver) -> Result<IdeologyColorTable> {
    let files = resolver.resolve_dir_files_recursive(IDEOLOGIES_DIR, Some("txt"))?;
    let mut out = IdeologyColorTable::default();
    for (_, path) in &files {
        let source = crate::read_latin1(path)?;
        parse_ideology_colors_into(&source, &mut out)?;
    }
    if out.colors.is_empty() {
        return Err(format!(
            "no ideology colours parsed from {IDEOLOGIES_DIR}/*.txt ({} files)",
            files.len()
        )
        .into());
    }
    Ok(out)
}

fn parse_group_color(group_inner: &str) -> Option<[u8; 3]> {
    let open = find_block_open(group_inner, "color")?;
    let bytes = group_inner.as_bytes();
    let brace = group_inner[..open].rfind('{')?;
    let close = match_brace(group_inner, brace)?;
    let list = &group_inner[open..close.min(bytes.len())];
    let nums: Vec<u8> = list
        .split_whitespace()
        .filter_map(|t| t.parse::<i64>().ok())
        .map(|v| v.clamp(0, 255) as u8)
        .collect();
    if nums.len() >= 3 {
        Some([nums[0], nums[1], nums[2]])
    } else {
        None
    }
}

fn find_block_open(source: &str, key: &str) -> Option<usize> {
    let mut search = 0usize;
    while let Some(rel) = source[search..].find(key) {
        let pos = search + rel;
        let before_ok = pos == 0
            || !matches!(source.as_bytes()[pos - 1], b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9');
        let after = pos + key.len();
        let after_ok = after >= source.len()
            || !matches!(source.as_bytes()[after], b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9');
        if before_ok && after_ok {
            let tail = source[after..].trim_start();
            if let Some(eq) = tail.strip_prefix('=') {
                if eq.trim_start().starts_with('{') {
                    let brace = after + source[after..].find('{').unwrap();
                    return Some(brace + 1);
                }
            }
        }
        search = pos + key.len();
    }
    None
}

fn match_brace(source: &str, open_brace: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    if bytes.get(open_brace) != Some(&b'{') {
        return None;
    }
    let mut depth = 0i32;
    let mut i = open_brace;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn read_token(source: &str, start: usize) -> (String, usize) {
    let bytes = source.as_bytes();
    let mut i = start;
    while i < bytes.len() && matches!(bytes[i], b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9') {
        i += 1;
    }
    (source[start..i].to_string(), i)
}


#[derive(Clone, Debug, PartialEq)]
pub struct StateAdminFill {
    pub state_id: u32,
    pub owner: Option<String>,
    pub fill_rgba: Option<[u8; 4]>,
}

pub fn states_admin_fill(
    owner_by_state: &BTreeMap<u32, String>,
    country_colors: &BTreeMap<String, [u8; 3]>,
) -> BTreeMap<u32, StateAdminFill> {
    let mut out = BTreeMap::new();
    for (&state_id, owner) in owner_by_state {
        let fill_rgba = country_colors
            .get(owner)
            .map(|rgb| [rgb[0], rgb[1], rgb[2], 255u8]);
        out.insert(
            state_id,
            StateAdminFill {
                state_id,
                owner: Some(owner.clone()),
                fill_rgba,
            },
        );
    }
    out
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateBoundaryOutline {
    pub boundary_pixels: BTreeSet<usize>,
    pub adjacent_state_pairs: BTreeSet<(u32, u32)>,
}

pub fn state_boundary_outline(
    province_ids: &[u32],
    width: usize,
    height: usize,
    province_to_state: &BTreeMap<u32, u32>,
) -> StateBoundaryOutline {
    let mut outline = StateBoundaryOutline::default();
    let state_of =
        |idx: usize| -> Option<u32> { province_to_state.get(&province_ids[idx]).copied() };
    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            let Some(s) = state_of(idx) else { continue };
            let mut check = |nidx: usize, s: u32, outline: &mut StateBoundaryOutline| {
                if let Some(ns) = state_of(nidx) {
                    if ns != s {
                        outline.boundary_pixels.insert(idx);
                        outline.boundary_pixels.insert(nidx);
                        let pair = if s < ns { (s, ns) } else { (ns, s) };
                        outline.adjacent_state_pairs.insert(pair);
                    }
                }
            };
            if x + 1 < width {
                check(idx + 1, s, &mut outline);
            }
            if y + 1 < height {
                check(idx + width, s, &mut outline);
            }
        }
    }
    outline
}


#[derive(Clone, Debug, PartialEq)]
pub struct StateIdeologyFill {
    pub state_id: u32,
    pub ideology: Option<String>,
    pub fill_rgba: Option<[u8; 4]>,
}

pub fn ideology_fill(
    owner_by_state: &BTreeMap<u32, String>,
    ruling_ideology_by_tag: &BTreeMap<String, String>,
    ideology_colors: &IdeologyColorTable,
    defines: &AdminMapModeDefines,
) -> BTreeMap<u32, StateIdeologyFill> {
    let alpha = alpha_byte(defines.ideology_color_transparency);
    let mut out = BTreeMap::new();
    for (&state_id, owner) in owner_by_state {
        let ideology = ruling_ideology_by_tag.get(owner);
        let fill_rgba = ideology
            .and_then(|id| ideology_colors.get(id))
            .map(|rgb| [rgb[0], rgb[1], rgb[2], alpha]);
        out.insert(
            state_id,
            StateIdeologyFill {
                state_id,
                ideology: fill_rgba.as_ref().and(ideology.cloned()),
                fill_rgba,
            },
        );
    }
    out
}


#[derive(Clone, Debug, PartialEq)]
pub struct StatePlayersFill {
    pub state_id: u32,
    pub owner: Option<String>,
    pub player_held: bool,
    pub fill_rgba: Option<[u8; 4]>,
}

pub fn players_fill(
    owner_by_state: &BTreeMap<u32, String>,
    player_tags: &BTreeSet<String>,
    country_colors: &BTreeMap<String, [u8; 3]>,
    defines: &AdminMapModeDefines,
) -> BTreeMap<u32, StatePlayersFill> {
    let faded = alpha_byte(defines.player_not_selected_transparency);
    let mut out = BTreeMap::new();
    for (&state_id, owner) in owner_by_state {
        let player_held = player_tags.contains(owner);
        let alpha = if player_held { 255 } else { faded };
        let fill_rgba = country_colors
            .get(owner)
            .map(|rgb| [rgb[0], rgb[1], rgb[2], alpha]);
        out.insert(
            state_id,
            StatePlayersFill {
                state_id,
                owner: Some(owner.clone()),
                player_held,
                fill_rgba,
            },
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
		MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY = 1,   -- comment
		PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY = 0.15,  -- comment
		OTHER = { 1, 2, 3 },
	}
}
"#;

    #[test]
    fn defines_parse_matches_vanilla_constant() {
        let parsed = admin_mapmode_defines_from_source(GRAPHICS_SNIPPET).expect("parse defines");
        assert_eq!(parsed, VANILLA_ADMIN_MAPMODE_DEFINES);
        assert_eq!(alpha_byte(parsed.ideology_color_transparency), 255);
        assert_eq!(alpha_byte(parsed.player_not_selected_transparency), 38);
    }

    const IDEOLOGIES_SNIPPET: &str = r#"
ideologies = {

	democratic = {
		types = { conservatism = { } liberalism = { } }
		dynamic_faction_names = { "FACTION_NAME_DEMOCRATIC_1" }
		color = { 0 0 255 }
		rules = { can_force_government = yes }
	}

	communism = {
		types = { marxism = { } }
		color = { 255 0 0 }
		can_be_boosted = yes
	}

	fascism = {
		color = { 150 75 0 }
	}

	neutrality = {
		color = { 124 124 124 }
	}
}
"#;

    #[test]
    fn ideology_colors_parse_matches_vanilla() {
        let table = ideology_colors_from_source(IDEOLOGIES_SNIPPET).expect("parse ideologies");
        assert_eq!(table, vanilla_ideology_colors());
        assert_eq!(table.get("democratic"), Some([0, 0, 255]));
        assert_eq!(table.get("communism"), Some([255, 0, 0]));
        assert_eq!(table.get("fascism"), Some([150, 75, 0]));
        assert_eq!(table.get("neutrality"), Some([124, 124, 124]));
        assert_eq!(table.colors.len(), 4, "exactly four vanilla ideologies");
    }

    #[test]
    fn states_fill_owner_color_fail_closed() {
        let owners = BTreeMap::from([
            (10u32, "GER".to_string()),
            (20u32, "SOV".to_string()),
            (30u32, "ZZZ".to_string()),
        ]);
        let colors = BTreeMap::from([
            ("GER".to_string(), [10u8, 20, 30]),
            ("SOV".to_string(), [40u8, 50, 60]),
        ]);
        let fills = states_admin_fill(&owners, &colors);
        assert_eq!(fills[&10].fill_rgba, Some([10, 20, 30, 255]));
        assert_eq!(fills[&20].fill_rgba, Some([40, 50, 60, 255]));
        assert_eq!(fills[&30].owner.as_deref(), Some("ZZZ"));
        assert_eq!(fills[&30].fill_rgba, None);
    }

    #[test]
    fn states_fill_equals_political_owner_color_bytewise() {
        use crate::political_color_synthesis::{political_texel_colors, PoliticalColorTexelClass};
        use crate::types::{ProvinceDefinition, StateMeta};
        use crate::InitialStateControl;

        let land = ProvinceDefinition {
            rgb: [0, 0, 0],
            province_type: "land".to_string(),
            coastal: false,
            terrain: String::new(),
            continent: 0,
        };
        let defs = BTreeMap::from([(1000u32, land.clone()), (2000u32, land)]);
        let p2s = BTreeMap::from([(1000u32, 10u32), (2000u32, 20u32)]);
        let states = BTreeMap::from([(10u32, StateMeta::default()), (20u32, StateMeta::default())]);
        let colors = BTreeMap::from([("GER".to_string(), [10u8, 20, 30])]);

        let owner_by_state = BTreeMap::from([(10u32, "GER".to_string())]);
        let controls = BTreeMap::from([(
            10u32,
            InitialStateControl {
                owner: Some("GER".into()),
                controller: Some("GER".into()),
            },
        )]);

        let admin = states_admin_fill(&owner_by_state, &colors);

        let (class, primary, secondary) = political_texel_colors(
            1000,
            &defs,
            &p2s,
            &states,
            &owner_by_state,
            &controls,
            &colors,
        )
        .expect("political texel");
        assert_eq!(class, PoliticalColorTexelClass::OwnerControllerEqual);
        assert_eq!(
            secondary, None,
            "no occupation stripe when owner==controller"
        );
        assert_eq!(
            admin[&10].fill_rgba, primary,
            "states fill == political owner colour"
        );
        assert_eq!(admin[&10].fill_rgba, Some([10, 20, 30, 255]));

        let (class20, primary20, _) = political_texel_colors(
            2000,
            &defs,
            &p2s,
            &states,
            &owner_by_state,
            &controls,
            &colors,
        )
        .expect("political texel");
        assert_eq!(class20, PoliticalColorTexelClass::NoOwner);
        assert_eq!(primary20, None, "political emits no owner colour");
        assert!(
            !admin.contains_key(&20),
            "states fill omits the unowned state"
        );
    }

    #[test]
    fn state_boundary_outline_marks_only_state_edges() {
        let w = 4usize;
        let h = 2usize;
        let province_ids = vec![
            1, 1, 2, 9,
            1, 1, 2, 2,
        ];
        let p2s = BTreeMap::from([(1u32, 10u32), (2u32, 20u32)]);
        let outline = state_boundary_outline(&province_ids, w, h, &p2s);
        assert!(outline.boundary_pixels.contains(&1));
        assert!(outline.boundary_pixels.contains(&2));
        assert!(outline.boundary_pixels.contains(&5));
        assert!(outline.boundary_pixels.contains(&6));
        assert!(!outline.boundary_pixels.contains(&3));
        assert!(!outline.boundary_pixels.contains(&0));
        assert_eq!(
            outline.adjacent_state_pairs,
            BTreeSet::from([(10u32, 20u32)])
        );
    }

    #[test]
    fn state_boundary_outline_single_state_has_no_edges() {
        let province_ids = vec![1u32, 1, 1, 1];
        let p2s = BTreeMap::from([(1u32, 10u32)]);
        let outline = state_boundary_outline(&province_ids, 2, 2, &p2s);
        assert!(outline.boundary_pixels.is_empty());
        assert!(outline.adjacent_state_pairs.is_empty());
    }

    #[test]
    fn ideology_fill_law_and_fail_closed() {
        let colors = vanilla_ideology_colors();
        let d = VANILLA_ADMIN_MAPMODE_DEFINES;
        let owners = BTreeMap::from([
            (10u32, "GER".to_string()),
            (20u32, "SOV".to_string()),
            (30u32, "ENG".to_string()),
            (40u32, "FRA".to_string()),
        ]);
        let ruling = BTreeMap::from([
            ("GER".to_string(), "fascism".to_string()),
            ("SOV".to_string(), "communism".to_string()),
            ("ENG".to_string(), "democratic".to_string()),
        ]);
        let fills = ideology_fill(&owners, &ruling, &colors, &d);
        assert_eq!(fills[&10].fill_rgba, Some([150, 75, 0, 255]));
        assert_eq!(fills[&10].ideology.as_deref(), Some("fascism"));
        assert_eq!(fills[&20].fill_rgba, Some([255, 0, 0, 255]));
        assert_eq!(fills[&30].fill_rgba, Some([0, 0, 255, 255]));
        assert_eq!(fills[&40].fill_rgba, None);
        assert_eq!(fills[&40].ideology, None);
        let ruling2 = BTreeMap::from([("GER".to_string(), "theocracy".to_string())]);
        let owners2 = BTreeMap::from([(10u32, "GER".to_string())]);
        let f2 = ideology_fill(&owners2, &ruling2, &colors, &d);
        assert_eq!(f2[&10].fill_rgba, None);
    }

    #[test]
    fn players_fill_fade_law_and_fail_closed() {
        let d = VANILLA_ADMIN_MAPMODE_DEFINES;
        let colors = BTreeMap::from([
            ("GER".to_string(), [10u8, 20, 30]),
            ("FRA".to_string(), [40u8, 50, 60]),
            ("ITA".to_string(), [70u8, 80, 90]),
        ]);
        let players = BTreeSet::from(["GER".to_string(), "SOV".to_string(), "ENG".to_string()]);
        let owners = BTreeMap::from([
            (10u32, "GER".to_string()),
            (20u32, "FRA".to_string()),
            (30u32, "ZZZ".to_string()),
        ]);
        let fills = players_fill(&owners, &players, &colors, &d);
        assert_eq!(fills[&10].player_held, true);
        assert_eq!(fills[&10].fill_rgba, Some([10, 20, 30, 255]));
        assert_eq!(fills[&20].player_held, false);
        assert_eq!(fills[&20].fill_rgba, Some([40, 50, 60, 38]));
        assert_eq!(fills[&30].fill_rgba, None);
    }

    use replay_core::{open_recording, Leaf, WorldTimeline};
    use std::path::PathBuf;

    fn fixture_path(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/positive")
            .join(format!("{name}.battleplan"))
    }
    fn open_fixture(name: &str) -> WorldTimeline {
        let path = fixture_path(name);
        replay_test_gates::require_existing_file(&path);
        WorldTimeline::open(open_recording(&path).expect("open recording")).expect("open timeline")
    }

    fn owners_at(t: &WorldTimeline, hour: i64) -> BTreeMap<u32, String> {
        let frame = t.state_at(hour).expect("state_at");
        let mut owners = BTreeMap::new();
        for (sid, leaf) in frame.owner_by_state() {
            if let Leaf::Measured(tag) = leaf {
                owners.insert(*sid, tag.clone());
            }
        }
        owners
    }
    fn ruling_at(t: &WorldTimeline, hour: i64) -> BTreeMap<String, String> {
        let frame = t.state_at(hour).expect("state_at");
        let mut out = BTreeMap::new();
        for (tag, identity) in frame.country_identities() {
            if let Leaf::Measured(id) = &identity.ruling_ideology {
                out.insert(tag.clone(), id.clone());
            }
        }
        out
    }

    #[test]
    #[cfg_attr(
        not(gate_replay_fixtures),
        ignore = "the replay fixtures directory at replay_interface/fixtures/positive"
    )]
    fn gate_fixture_ideology_flip_reaches_fill() {
        let t = open_fixture("fx_cosmetic_ideology");
        let hours = t.hours().to_vec();
        let h = |tick: usize| hours[tick];
        let colors = vanilla_ideology_colors();
        let d = VANILLA_ADMIN_MAPMODE_DEFINES;

        let fill_state10 = |tick: usize| -> Option<[u8; 4]> {
            let owners = owners_at(&t, h(tick));
            let ruling = ruling_at(&t, h(tick));
            let fills = ideology_fill(&owners, &ruling, &colors, &d);
            fills.get(&10).and_then(|f| f.fill_rgba)
        };

        assert_eq!(fill_state10(3), Some([150, 75, 0, 255]), "tick3 fascism");
        assert_eq!(fill_state10(4), Some([0, 0, 255, 255]), "tick4 democratic");
        assert_eq!(fill_state10(7), Some([0, 0, 255, 255]), "tick7 democratic");
        assert_eq!(
            fill_state10(8),
            Some([124, 124, 124, 255]),
            "tick8 neutrality"
        );
        for tick in [0usize, 5, 11] {
            assert!(fill_state10(tick).is_some(), "tick {tick} has a fill");
        }
    }

    #[test]
    #[cfg_attr(
        not(gate_replay_fixtures),
        ignore = "the replay fixtures directory at replay_interface/fixtures/positive"
    )]
    fn gate_fixture_players_session_and_fade() {
        let t = open_fixture("fx_owner_controller_flip");
        let frame = t.state_at(t.hours()[0]).expect("state_at");
        let players: BTreeSet<String> = match &frame.session_metadata().player_countries {
            Leaf::Measured(list) => list
                .iter()
                .filter_map(|p| match &p.tag {
                    Leaf::Measured(tag) => Some(tag.clone()),
                    _ => None,
                })
                .collect(),
            other => panic!("expected measured player list, got {other:?}"),
        };
        assert_eq!(
            players,
            BTreeSet::from(["GER".to_string(), "SOV".to_string(), "ENG".to_string()]),
            "session player tags"
        );

        let d = VANILLA_ADMIN_MAPMODE_DEFINES;
        let colors = BTreeMap::from([
            ("GER".to_string(), [10u8, 20, 30]),
            ("FRA".to_string(), [40u8, 50, 60]),
        ]);
        let owners = BTreeMap::from([(10u32, "GER".to_string()), (99u32, "FRA".to_string())]);
        let fills = players_fill(&owners, &players, &colors, &d);
        assert_eq!(fills[&10].fill_rgba, Some([10, 20, 30, 255]), "player full");
        assert_eq!(
            fills[&99].fill_rgba,
            Some([40, 50, 60, 38]),
            "non-player fade"
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_admin_defines_and_ideology_colors() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = crate::mod_resolver::GameFileResolver::vanilla(game_dir).expect("resolver");
        let defines = admin_mapmode_defines_from_resolver(&resolver).expect("defines");
        assert_eq!(defines, VANILLA_ADMIN_MAPMODE_DEFINES);
        let colors = ideology_colors_from_resolver(&resolver).expect("ideology colors");
        assert_eq!(colors.get("democratic"), Some([0, 0, 255]));
        assert_eq!(colors.get("communism"), Some([255, 0, 0]));
        assert_eq!(colors.get("fascism"), Some([150, 75, 0]));
        assert_eq!(colors.get("neutrality"), Some([124, 124, 124]));
        assert_eq!(colors.colors.len(), 4, "vanilla ships four ideologies");
    }
}
