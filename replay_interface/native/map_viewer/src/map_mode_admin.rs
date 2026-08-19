//! Administrative map modes — **States** (engine layer `game_map_mode_states`,
//! mode 13), **Ideology** (`game_map_mode_ideology`, mode 16), and **Players**
//! (`game_map_mode_players`, mode 15).
//! XMAP00_VIEWER_VISUAL_INPUT: viewer.map_mode_selection
//! This module owns the *data law* for those three fills: the per-state RGBA the
//! GradientBorder field is keyed with, plus (for States) the state-boundary
//! outline geometry. It is the new-map-mode sibling of `naval_supremacy_tint`
//! and follows the same three contracts:
//!
//! 1. **Colours are computed replay-side from recorded inputs, never recorded**
//!    because the per-scope colour result is runtime/recording-gated, not
//!    file-carried. Every
//!    colour/threshold below is FILE-cited to a shipped game file or fails
//!    **closed** (no fill) — never an invented colour.
//! 2. **Decoupled from `replay_core`.** The fill functions take neutral input
//!    maps (`owner_by_state`, `ruling_ideology_by_tag`, `player_tags`) that
//!    mirror the READ-ONLY `replay_core` views (`WorldFrame::owner_by_state`
//!    item 2, `CountryIdentity::ruling_ideology` item 7b,
//!    `SessionMetadataView::player_countries` item 4). The live wiring
//!    (`WorldFrame` -> these inputs) belongs at the `replay_app` seam; the A5
//!    keying seam (`keying.rs`, `KeyingTables::state_owners`) already exposes
//!    owner-by-state, so the States/Players fills reuse it rather than re-parse.
//! 3. **Fail closed to no fill** wherever an input leaf is absent/unobtained or a
//!    colour cannot be resolved — matching the political fill's `NoOwner` law
//!    (`political_color_synthesis::political_texel_colors`).
//!
//! ## Per-mode law + FILE citations
//!
//! ### States (mode 13, `game_map_mode_states`)
//! - **Fill** = owner country colour — the SAME owner colour the political fill
//!   emits (`political_texel_colors` owner branch); States mode is the political
//!   fill re-drawn with STATE borders instead of country borders
//!   (`common/map_modes/documentation.md`: layer `type = state` "will render
//!   using state provinces"). Owner comes from the owner-by-state view.
//! - **Outlines** (the NEW part) = state-boundary edges: a 4-neighbour pixel pair
//!   whose provinces resolve to *different* states. Derived purely from the
//!   static province raster (`map/provinces.bmp` -> province id) + the
//!   province->state topology (`history/states/*.txt`), exactly the inputs the
//!   border/label laws already consume (`LabelLawInputs::province_ids`). Pixel
//!   rasterisation + parity vs a captured States-mode frame is capture-gated and
//!   deferred, as with `naval_supremacy_tint`.
//!
//! ### Ideology (mode 16, `game_map_mode_ideology`)
//! - **4 ideology colours** are read from the scriptable ideology defs
//!   (`common/ideologies/*.txt`), one `color = { r g b }` per top-level ideology
//!   group. FILE-confirmed by the ideology-map-mode define comment
//!   (`common/defines/00_graphics.lua` `MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY`:
//!   "the colors on the map are taken from the scriptable file 00_ideologies.txt
//!   for each group"). Vanilla values (`common/ideologies/00_ideologies.txt`):
//!   democratic `{0,0,255}`, communism `{255,0,0}`, fascism `{150,75,0}`,
//!   neutrality `{124,124,124}`.
//! - **Alpha** = `MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY` (FILE, 00_graphics.lua;
//!   vanilla `1`). The define exists because 100% opaque "looks bad on the map".
//! - **Keying**: each state is filled by its owner's *ruling ideology*
//!   (`CountryIdentity::ruling_ideology`, item 7b, which the view derives from
//!   `countries.<tag>.politics.ruling_party`). The four ruling-party names
//!   (`fascism`/`communism`/`democratic`/`neutrality`) are exactly the ideology
//!   group names, so ruling_party keys the colour table directly.
//!
//! ### Players (mode 15, `game_map_mode_players`)
//! - **Player<->country** from the session view
//!   (`SessionMetadataView::player_countries`, item 4;
//!   `world.scenario.players[*].tag`).
//! - **Colour rule (FILE)**: a player-held country keeps its normal country
//!   colour at full opacity; a country **not** held by any player is faded to
//!   `PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY`
//!   (`common/defines/00_graphics.lua`, comment: "How much is the country colors
//!   faded out, for countries that are not occupied by the any player"; vanilla
//!   `0.15`). This is the file-derivable law — the players map mode is the
//!   political fill with non-player countries alpha-faded, **not** a distinct
//!   per-player palette. The exact MP-lobby per-slot palette (if any beyond the
//!   country colour) is engine/lobby-runtime state and is NOT used here.

use crate::mod_resolver::GameFileResolver;
use crate::Result;
use std::collections::{BTreeMap, BTreeSet};

/// `common/defines/00_graphics.lua` — source of the two admin-mode alpha defines.
pub const ADMIN_MAPMODE_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
/// `common/ideologies` — the scriptable ideology group defs (colour table).
pub const IDEOLOGIES_DIR: &str = "common/ideologies";

// ---------------------------------------------------------------------------
// Defines (FILE law).
// ---------------------------------------------------------------------------

/// The `00_graphics.lua` constants that drive the ideology + players map modes.
///
/// FILE citations (`common/defines/00_graphics.lua`):
/// - `MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY` — ideology-fill alpha (0..=1).
/// - `PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY` — alpha of a country not
///   held by any player in the players map mode.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AdminMapModeDefines {
    /// `MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY` (FILE).
    pub ideology_color_transparency: f32,
    /// `PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY` (FILE).
    pub player_not_selected_transparency: f32,
}

/// The vanilla `00_graphics.lua` values, verbatim (FILE, cited above). Used as
/// the offline gate reference; the resolver path re-parses the shipped file so
/// mods override correctly.
pub const VANILLA_ADMIN_MAPMODE_DEFINES: AdminMapModeDefines = AdminMapModeDefines {
    ideology_color_transparency: 1.0,
    player_not_selected_transparency: 0.15,
};

/// Parse the admin-mode defines out of `00_graphics.lua` source text.
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

/// Load + parse the admin-mode defines via the mod-aware resolver.
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

/// Convert a FILE transparency ratio (0..=1) to an 8-bit alpha byte, rounded.
fn alpha_byte(ratio: f32) -> u8 {
    (ratio.clamp(0.0, 1.0) * 255.0).round() as u8
}

// ---------------------------------------------------------------------------
// Ideology colour table (FILE law: common/ideologies/*.txt).
// ---------------------------------------------------------------------------

/// The ideology -> RGB colour table (one entry per top-level ideology group).
/// Vanilla ships exactly four; mods may add more, so this is an open map keyed
/// by ideology group name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IdeologyColorTable {
    /// Ideology group name (`fascism`/`communism`/`democratic`/`neutrality`,
    /// plus any mod groups) -> `color = { r g b }`.
    pub colors: BTreeMap<String, [u8; 3]>,
}

impl IdeologyColorTable {
    pub fn get(&self, ideology: &str) -> Option<[u8; 3]> {
        self.colors.get(ideology).copied()
    }
}

/// The vanilla four ideology colours, verbatim from
/// `common/ideologies/00_ideologies.txt` (FILE): democratic `{0,0,255}`,
/// communism `{255,0,0}`, fascism `{150,75,0}`, neutrality `{124,124,124}`.
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

/// Strip Paradox-script `#` line comments (values never legally contain `#`).
fn strip_hash_comments(source: &str) -> String {
    source
        .lines()
        .map(|line| line.split('#').next().unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parse the ideology colour table from one `common/ideologies/*.txt` source.
///
/// Structure: `ideologies = { <name> = { ... color = { r g b } ... } }`. For
/// each top-level ideology group (depth 1 under `ideologies`), the group's own
/// `color = { r g b }` (the first `color` block at the group's top level) is the
/// map colour; nested blocks (`types`, `rules`, ...) carry no `color`. Merges
/// into `out` so a caller can fold multiple files (mod support).
pub fn parse_ideology_colors_into(source: &str, out: &mut IdeologyColorTable) -> Result<()> {
    let text = strip_hash_comments(source);
    let bytes = text.as_bytes();
    // Locate the `ideologies` block.
    let Some(block_start) = find_block_open(&text, "ideologies") else {
        return Ok(()); // file without an ideologies block contributes nothing
    };
    // Walk the direct children of the ideologies block (depth 1 groups).
    let mut i = block_start; // index just after the `{` of `ideologies`
    let mut depth = 1i32; // we are inside `ideologies { ... }`
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
        // At depth 1, a token followed by `= {` opens an ideology group.
        if depth == 1 && (b.is_ascii_alphanumeric() || b == b'_') {
            let (name, after_name) = read_token(&text, i);
            let rest = text[after_name..].trim_start();
            if rest.starts_with('=') {
                let after_eq = after_name + (text[after_name..].find('=').unwrap()) + 1;
                let tail = text[after_eq..].trim_start();
                if tail.starts_with('{') {
                    // Group block: find its matching close, extract, parse its color.
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

/// Parse the ideology colour table from a single source string (convenience).
pub fn ideology_colors_from_source(source: &str) -> Result<IdeologyColorTable> {
    let mut out = IdeologyColorTable::default();
    parse_ideology_colors_into(source, &mut out)?;
    Ok(out)
}

/// Load + parse the ideology colour table via the mod-aware resolver, folding
/// every `common/ideologies/*.txt` (mods may split/extend the set).
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

/// The group-level `color = { r g b }` inside one ideology block. Takes the
/// FIRST such block at any depth in the group body; the group's own colour is
/// the only `color` block present (nested `types`/`rules` carry none).
fn parse_group_color(group_inner: &str) -> Option<[u8; 3]> {
    let open = find_block_open(group_inner, "color")?;
    let bytes = group_inner.as_bytes();
    // find the matching `}` from the `{` just before `open`.
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

/// Index just after the `{` that opens the block named `key` (`key = {`), or
/// `None`. Matches `key` as a whole token immediately followed by `=` then `{`.
fn find_block_open(source: &str, key: &str) -> Option<usize> {
    let mut search = 0usize;
    while let Some(rel) = source[search..].find(key) {
        let pos = search + rel;
        // whole-token boundary before
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

/// Given the index of a `{`, return the index of its matching `}`.
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

/// Read the identifier token starting at `start`; return (token, index-after).
fn read_token(source: &str, start: usize) -> (String, usize) {
    let bytes = source.as_bytes();
    let mut i = start;
    while i < bytes.len() && matches!(bytes[i], b'_' | b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9') {
        i += 1;
    }
    (source[start..i].to_string(), i)
}

// ---------------------------------------------------------------------------
// STATES mode (13): owner fill + state-boundary outlines.
// ---------------------------------------------------------------------------

/// One state's administrative fill in the States map mode. Fails closed to
/// `None` (no fill) when the owner is absent or the owner colour is missing.
#[derive(Clone, Debug, PartialEq)]
pub struct StateAdminFill {
    pub state_id: u32,
    /// Owner tag whose colour fills the state (`None` = unowned -> no fill).
    pub owner: Option<String>,
    /// RGBA the GB field is keyed with (opaque owner colour, `None` if unowned
    /// or the owner has no colour entry — fail closed).
    pub fill_rgba: Option<[u8; 4]>,
}

/// Compute the States-mode fill for every state in `owner_by_state`. Fill =
/// opaque owner country colour (the political owner colour). Unowned states or
/// owners with no colour entry fail closed to no fill.
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

/// The state-boundary outline geometry derived from a province raster.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateBoundaryOutline {
    /// Linear indices (`y*width + x`) of pixels that sit on a state boundary
    /// (at least one 4-neighbour resolves to a different state).
    pub boundary_pixels: BTreeSet<usize>,
    /// Count of distinct unordered state-id pairs that share a boundary.
    pub adjacent_state_pairs: BTreeSet<(u32, u32)>,
}

/// Derive state-boundary outline pixels from a province-id raster and the
/// province->state topology. A boundary is drawn between two 4-adjacent pixels
/// whose provinces resolve to **different** known states. Pixels whose province
/// has no state (sea/lake/unknown) never start a state boundary — matching the
/// political law's `NoState` transparency. Pure: no I/O, deterministic.
///
/// `province_ids[y*width + x]` = province id at that pixel;
/// `province_to_state` = `history/states/*.txt` join.
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
            // right + down neighbours cover every edge exactly once.
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

// ---------------------------------------------------------------------------
// IDEOLOGY mode (16): ruling-ideology fill.
// ---------------------------------------------------------------------------

/// One state's ideology-mode fill. Fails closed to `None` when the owner, the
/// owner's ruling ideology, or the ideology's colour is missing.
#[derive(Clone, Debug, PartialEq)]
pub struct StateIdeologyFill {
    pub state_id: u32,
    /// The ruling ideology group name that coloured the state (`None` = no fill).
    pub ideology: Option<String>,
    /// RGBA the GB field is keyed with: ideology colour + the FILE ideology
    /// transparency alpha (`None` = fail closed).
    pub fill_rgba: Option<[u8; 4]>,
}

/// Compute the Ideology-mode fill for every state in `owner_by_state`. Each
/// state is coloured by its owner's ruling ideology
/// (`ruling_ideology_by_tag[owner]`) looked up in the ideology colour table,
/// with alpha = `MAP_MODE_IDEOLOGY_COLOR_TRANSPARENCY`. Any missing link
/// (owner absent, ruling ideology absent, ideology name not in the table) fails
/// closed to no fill.
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
                // Only report the ideology name when it actually produced a fill
                // (a ruling ideology with no colour entry stays fail-closed).
                ideology: fill_rgba.as_ref().and(ideology.cloned()),
                fill_rgba,
            },
        );
    }
    out
}

// ---------------------------------------------------------------------------
// PLAYERS mode (15): player-country colour, non-players faded.
// ---------------------------------------------------------------------------

/// One state's players-mode fill. A player-held owner keeps its opaque country
/// colour; a non-player owner is faded to the FILE not-selected transparency.
#[derive(Clone, Debug, PartialEq)]
pub struct StatePlayersFill {
    pub state_id: u32,
    /// Owner tag (`None` = unowned -> no fill).
    pub owner: Option<String>,
    /// True when the owner is a human player (full opacity); false = faded.
    pub player_held: bool,
    /// RGBA: owner country colour, alpha 255 if player-held else the FILE fade
    /// alpha (`PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY`). `None` = fail
    /// closed (unowned or owner colour missing).
    pub fill_rgba: Option<[u8; 4]>,
}

/// Compute the Players-mode fill for every state in `owner_by_state`. `player_tags`
/// is the set of human-player country tags (from
/// `SessionMetadataView::player_countries`). A player-held state keeps its opaque
/// country colour; a non-player state is faded to
/// `PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY`. Unowned states or owners
/// with no colour entry fail closed.
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

    // -------------------------------------------------------------------
    // Defines (FILE).
    // -------------------------------------------------------------------
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

    // -------------------------------------------------------------------
    // Ideology colour parse (FILE).
    // -------------------------------------------------------------------
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

    // -------------------------------------------------------------------
    // STATES: fill + boundary outline (pure).
    // -------------------------------------------------------------------
    #[test]
    fn states_fill_owner_color_fail_closed() {
        let owners = BTreeMap::from([
            (10u32, "GER".to_string()),
            (20u32, "SOV".to_string()),
            (30u32, "ZZZ".to_string()), // no colour entry
        ]);
        let colors = BTreeMap::from([
            ("GER".to_string(), [10u8, 20, 30]),
            ("SOV".to_string(), [40u8, 50, 60]),
        ]);
        let fills = states_admin_fill(&owners, &colors);
        assert_eq!(fills[&10].fill_rgba, Some([10, 20, 30, 255]));
        assert_eq!(fills[&20].fill_rgba, Some([40, 50, 60, 255]));
        // Owner with no colour entry -> fail closed (owner still reported).
        assert_eq!(fills[&30].owner.as_deref(), Some("ZZZ"));
        assert_eq!(fills[&30].fill_rgba, None);
    }

    // MISSION gate (States): the States-mode fill is the political fill
    // BYTE-WISE. Drive `political_texel_colors` (the political law itself) on a
    // synthetic world and assert the States-mode fill equals its owner-colour
    // output byte-for-byte, for both the owner==controller (opaque owner colour)
    // and the unowned (NoOwner -> fail closed) cases.
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
        // province 1000 -> state 10 (GER), province 2000 -> state 20 (unowned).
        let defs = BTreeMap::from([(1000u32, land.clone()), (2000u32, land)]);
        let p2s = BTreeMap::from([(1000u32, 10u32), (2000u32, 20u32)]);
        let states = BTreeMap::from([(10u32, StateMeta::default()), (20u32, StateMeta::default())]);
        let colors = BTreeMap::from([("GER".to_string(), [10u8, 20, 30])]);

        // Owner==controller for state 10; state 20 owner omitted (unowned).
        let owner_by_state = BTreeMap::from([(10u32, "GER".to_string())]);
        let controls = BTreeMap::from([(
            10u32,
            InitialStateControl {
                owner: Some("GER".into()),
                controller: Some("GER".into()),
            },
        )]);

        // States-mode fill (module under test).
        let admin = states_admin_fill(&owner_by_state, &colors);

        // Political law for province 1000 (state 10, GER owner==controller).
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
        // BYTE-WISE equality: States fill == political primary owner colour.
        assert_eq!(
            admin[&10].fill_rgba, primary,
            "states fill == political owner colour"
        );
        assert_eq!(admin[&10].fill_rgba, Some([10, 20, 30, 255]));

        // Unowned state 20 -> political NoOwner (no colour) and States fill omits
        // it (fail closed) — same absent form.
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

    // A 4x2 raster: left half province 1 (state 10), right half province 2
    // (state 20), plus a sea province 9 (no state) in the top-right corner.
    //   row0: 1 1 2 9
    //   row1: 1 1 2 2
    #[test]
    fn state_boundary_outline_marks_only_state_edges() {
        let w = 4usize;
        let h = 2usize;
        let province_ids = vec![
            1, 1, 2, 9, // row 0
            1, 1, 2, 2, // row 1
        ];
        let p2s = BTreeMap::from([(1u32, 10u32), (2u32, 20u32)]); // 9 has no state
        let outline = state_boundary_outline(&province_ids, w, h, &p2s);
        // The 10|20 boundary runs down the column between x=1 and x=2.
        // Boundary pixels: (1,0),(2,0),(1,1),(2,1) -> idx 1,2,5,6.
        assert!(outline.boundary_pixels.contains(&1));
        assert!(outline.boundary_pixels.contains(&2));
        assert!(outline.boundary_pixels.contains(&5));
        assert!(outline.boundary_pixels.contains(&6));
        // The sea pixel (9) never starts/receives a state boundary.
        assert!(!outline.boundary_pixels.contains(&3));
        // Interior same-state pixel (0,0)->idx0 only borders state 10 -> not marked.
        assert!(!outline.boundary_pixels.contains(&0));
        // Exactly one adjacent state pair.
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

    // -------------------------------------------------------------------
    // IDEOLOGY: ruling-ideology fill law (pure).
    // -------------------------------------------------------------------
    #[test]
    fn ideology_fill_law_and_fail_closed() {
        let colors = vanilla_ideology_colors();
        let d = VANILLA_ADMIN_MAPMODE_DEFINES;
        let owners = BTreeMap::from([
            (10u32, "GER".to_string()),
            (20u32, "SOV".to_string()),
            (30u32, "ENG".to_string()),
            (40u32, "FRA".to_string()), // owner present, ruling ideology absent
        ]);
        let ruling = BTreeMap::from([
            ("GER".to_string(), "fascism".to_string()),
            ("SOV".to_string(), "communism".to_string()),
            ("ENG".to_string(), "democratic".to_string()),
            // FRA has no ruling ideology entry.
        ]);
        let fills = ideology_fill(&owners, &ruling, &colors, &d);
        assert_eq!(fills[&10].fill_rgba, Some([150, 75, 0, 255]));
        assert_eq!(fills[&10].ideology.as_deref(), Some("fascism"));
        assert_eq!(fills[&20].fill_rgba, Some([255, 0, 0, 255]));
        assert_eq!(fills[&30].fill_rgba, Some([0, 0, 255, 255]));
        // Owner with no ruling ideology -> fail closed.
        assert_eq!(fills[&40].fill_rgba, None);
        assert_eq!(fills[&40].ideology, None);
        // A ruling ideology not in the colour table also fails closed.
        let ruling2 = BTreeMap::from([("GER".to_string(), "theocracy".to_string())]);
        let owners2 = BTreeMap::from([(10u32, "GER".to_string())]);
        let f2 = ideology_fill(&owners2, &ruling2, &colors, &d);
        assert_eq!(f2[&10].fill_rgba, None);
    }

    // -------------------------------------------------------------------
    // PLAYERS: player fill law (pure).
    // -------------------------------------------------------------------
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
            (10u32, "GER".to_string()), // player -> full alpha
            (20u32, "FRA".to_string()), // non-player -> faded
            (30u32, "ZZZ".to_string()), // no colour -> fail closed
        ]);
        let fills = players_fill(&owners, &players, &colors, &d);
        assert_eq!(fills[&10].player_held, true);
        assert_eq!(fills[&10].fill_rgba, Some([10, 20, 30, 255]));
        assert_eq!(fills[&20].player_held, false);
        assert_eq!(fills[&20].fill_rgba, Some([40, 50, 60, 38])); // 0.15*255 = 38
        assert_eq!(fills[&30].fill_rgba, None);
    }

    // ===================================================================
    // FIXTURE gates (open the REAL recording through replay_core; the fill
    // laws stay decoupled). Expectations derived from the fixture generator
    // SOURCE (replay_fixture_gen/src/main.rs), NOT the code under test.
    // ===================================================================
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

    /// Read the owner-by-state map from a frame (mirrors keying.rs).
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
    /// Read ruling_ideology-by-tag from a frame's country_identities view.
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

    /// IDEOLOGY gate. fx_cosmetic_ideology mutates GER's ruling_party
    /// (== ruling ideology) by tick (generator SOURCE):
    ///   tick <4  -> "fascism"     -> vanilla fascism colour     {150,75,0}
    ///   4..=7    -> "democratic"  -> vanilla democratic colour  {0,0,255}
    ///   tick >=8 -> "neutrality"  -> vanilla neutrality colour  {124,124,124}
    /// GER owns state 10 in the base snapshot. The fill must flip EXACTLY at
    /// the generator's tick boundaries, using the FILE ideology colours.
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

        // Generator boundaries: tick3 fascism, tick4 democratic, tick8 neutrality.
        assert_eq!(fill_state10(3), Some([150, 75, 0, 255]), "tick3 fascism");
        assert_eq!(fill_state10(4), Some([0, 0, 255, 255]), "tick4 democratic");
        assert_eq!(fill_state10(7), Some([0, 0, 255, 255]), "tick7 democratic");
        assert_eq!(
            fill_state10(8),
            Some([124, 124, 124, 255]),
            "tick8 neutrality"
        );
        // The generator gives GER ruling_party at every tick, so state 10 is
        // never fail-closed in this fixture.
        for tick in [0usize, 5, 11] {
            assert!(fill_state10(tick).is_some(), "tick {tick} has a fill");
        }
    }

    /// PLAYERS gate. base_snapshot sets `world.scenario.players` =
    /// [GER, SOV, ENG] (generator SOURCE), so the session view yields exactly
    /// those three player tags. Confirm the view read, then exercise the fade
    /// law with a synthetic owner map that includes a non-player owner (FRA,
    /// generator `is_human=false`) so the 0.15 fade path is covered.
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
        // Derived from generator base_snapshot players list.
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
        // GER is a player (full alpha); FRA is not (faded to 38).
        let owners = BTreeMap::from([(10u32, "GER".to_string()), (99u32, "FRA".to_string())]);
        let fills = players_fill(&owners, &players, &colors, &d);
        assert_eq!(fills[&10].fill_rgba, Some([10, 20, 30, 255]), "player full");
        assert_eq!(
            fills[&99].fill_rgba,
            Some([40, 50, 60, 38]),
            "non-player fade"
        );
    }

    // ===================================================================
    // VANILLA file gates (skip without BATTLE_PLAN_GAME_DIR).
    // ===================================================================
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
