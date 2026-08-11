//! Infrastructure map mode (E3 lane; the mode set and its selection law are in
//! `replay_interface/documents/UI_LAYOUT_LAW.md` §6.2 "Map-mode manager").
//!
//! The infrastructure map mode (`ui_mapmode_infrastructure`, F9) fills each
//! **state** with a gradient keyed on its infrastructure building level. This
//! module owns the replay-side *data law* for that overlay:
//!
//! 1. **Gradient endpoints** — `MAP_MODE_INFRA_RANGE_COLOR_FROM` (low infra) and
//!    `MAP_MODE_INFRA_RANGE_COLOR_TO` (high infra), both RGBA, from
//!    `common/defines/00_graphics.lua`. FILE.
//! 2. **Normalisation range** — unlike the manpower mode
//!    (`MAP_MODE_MANPOWER_RANGE_MAX`, an explicit define), infrastructure has NO
//!    range-max define. The natural file-derivable ceiling is the infrastructure
//!    building's own `level_cap.state_max` (`common/buildings/00_buildings.txt`,
//!    vanilla = 5). We normalise `level` over `[0, state_max]`. The endpoint
//!    colours + the ceiling are FILE; the *assumption* that the engine
//!    interpolates linearly over exactly `[0, state_max]` (rather than a dynamic
//!    per-map max) is DERIVED / capture-gated.
//! 3. **Infra level per state** — read from `replay_core`'s buildings view:
//!    `world.buildings_by_state()[state].state_buildings["infrastructure"].level`
//!    (a `Leaf<f64>`). A present measured leaf maps to `Some`; an absent/
//!    unobtained leaf (or a state with no buildings block) maps to `None` and the
//!    state is **not** tinted (fail-closed — the engine paints nothing where it
//!    has no infra level).
//!
//! Colours are computed replay-side from recorded inputs, **never recorded**
//! (standing override). The module does **not** depend on `replay_core`; the live
//! `StateBuildings -> InfraLevelInput` wiring belongs at the `replay_app` seam,
//! keeping `map_viewer` decoupled from the crate the parallel lane edits.

use crate::mod_resolver::GameFileResolver;
use crate::Result;
use std::collections::BTreeMap;

/// `00_graphics.lua` — source of the infrastructure gradient endpoint colours.
pub const INFRA_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
/// `00_buildings.txt` — source of the infrastructure `level_cap.state_max`
/// (the gradient's normalisation ceiling).
pub const INFRA_BUILDINGS_SOURCE: &str = "common/buildings/00_buildings.txt";

/// The infrastructure building name / building-map key (the `state_buildings`
/// entry the level is read from, and the block scanned in `00_buildings.txt`).
pub const INFRASTRUCTURE_BUILDING_KEY: &str = "infrastructure";

// ---------------------------------------------------------------------------
// Gradient defines (FILE law).
// ---------------------------------------------------------------------------

/// The infrastructure map-mode gradient endpoints (`00_graphics.lua`), each RGBA.
///
/// FILE citations (`common/defines/00_graphics.lua`):
/// - `MAP_MODE_INFRA_RANGE_COLOR_FROM` (`:23`) — colour at infra level 0.
/// - `MAP_MODE_INFRA_RANGE_COLOR_TO` (`:24`) — colour at the max infra level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InfraGradientColors {
    pub from: [f32; 4],
    pub to: [f32; 4],
}

/// The vanilla `00_graphics.lua` values, verbatim (FILE, cited above). Offline
/// gate reference; the resolver path re-parses the shipped file so mods override.
pub const VANILLA_INFRA_GRADIENT_COLORS: InfraGradientColors = InfraGradientColors {
    from: [1.0, 0.125, 0.0, 0.1],
    to: [0.1, 0.9, 0.1, 0.6],
};

/// The vanilla infrastructure `level_cap.state_max` (`00_buildings.txt`). FILE.
pub const VANILLA_INFRA_STATE_MAX: u32 = 5;

fn parse_color4(source: &str, key: &str) -> Result<[f32; 4]> {
    let values = crate::map_furniture::parse_lua_number_list(source, key)
        .ok_or_else(|| format!("{INFRA_GRAPHICS_DEFINES_SOURCE}: missing colour define {key}"))?;
    if values.len() != 4 {
        return Err(format!(
            "{INFRA_GRAPHICS_DEFINES_SOURCE}: {key} expected 4 components, got {}",
            values.len()
        )
        .into());
    }
    Ok([values[0], values[1], values[2], values[3]])
}

/// Parse the infrastructure gradient endpoints out of `00_graphics.lua` source.
pub fn infra_gradient_colors_from_source(source: &str) -> Result<InfraGradientColors> {
    Ok(InfraGradientColors {
        from: parse_color4(source, "MAP_MODE_INFRA_RANGE_COLOR_FROM")?,
        to: parse_color4(source, "MAP_MODE_INFRA_RANGE_COLOR_TO")?,
    })
}

/// Load + parse the infrastructure gradient endpoints via the mod-aware resolver.
pub fn infra_gradient_colors_from_resolver(
    resolver: &GameFileResolver,
) -> Result<InfraGradientColors> {
    let path = resolver.resolve_required(
        INFRA_GRAPHICS_DEFINES_SOURCE,
        "infrastructure gradient colours",
    )?;
    let source = crate::read_latin1(&path)?;
    infra_gradient_colors_from_source(&source)
}

// ---------------------------------------------------------------------------
// Normalisation ceiling (FILE: infrastructure level_cap.state_max).
// ---------------------------------------------------------------------------

/// Parse the top-level `infrastructure` building's `level_cap.state_max` out of
/// `00_buildings.txt` source text. Matches only the standalone `infrastructure =
/// {` building block — not `energy_infrastructure`, `industrial_infrastructure`,
/// the `infrastructure = yes` flag, nor `infrastructure_construction_effect`.
/// Returns `None` if the block or the field is absent.
pub fn parse_infrastructure_state_max(source: &str) -> Option<u32> {
    let text = crate::strip_comments(source);
    let bytes = text.as_bytes();
    let key = INFRASTRUCTURE_BUILDING_KEY.as_bytes();
    let mut i = 0usize;
    while let Some(rel) = find_subslice(&bytes[i..], key) {
        let start = i + rel;
        let end = start + key.len();
        // Left boundary: the char before must not be part of a longer identifier
        // (so `energy_infrastructure` / `industrial_infrastructure` are rejected).
        let left_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        // Right boundary: `infrastructure` (ws)* `=` (ws)* `{` — a building block,
        // not `= yes` and not `infrastructure_construction_effect`.
        if left_ok {
            if let Some(brace) = match_eq_open_brace(&bytes[end..]) {
                let block_start = end + brace + 1; // just after `{`
                if let Some(block) = balanced_block(&bytes[block_start..]) {
                    let block_str = &text[block_start..block_start + block];
                    if let Some(v) = parse_int_after_key(block_str, "state_max") {
                        return Some(v);
                    }
                }
            }
        }
        i = end;
    }
    None
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// From just after a key, skip whitespace, require `=`, skip whitespace, require
/// `{`. Returns the offset of the `{` on success.
fn match_eq_open_brace(after: &[u8]) -> Option<usize> {
    let mut j = 0usize;
    while j < after.len() && after[j].is_ascii_whitespace() {
        j += 1;
    }
    if j >= after.len() || after[j] != b'=' {
        return None;
    }
    j += 1;
    while j < after.len() && after[j].is_ascii_whitespace() {
        j += 1;
    }
    if j >= after.len() || after[j] != b'{' {
        return None;
    }
    Some(j)
}

/// Given bytes starting just after an opening `{`, return the length of the block
/// content up to (not including) the matching `}`.
fn balanced_block(after_open: &[u8]) -> Option<usize> {
    let mut depth = 1i32;
    for (k, &b) in after_open.iter().enumerate() {
        match b {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(k);
                }
            }
            _ => {}
        }
    }
    None
}

/// Find `key` in `block`, then parse the integer after the following `=`.
fn parse_int_after_key(block: &str, key: &str) -> Option<u32> {
    let bytes = block.as_bytes();
    let needle = key.as_bytes();
    let mut i = 0usize;
    while let Some(rel) = find_subslice(&bytes[i..], needle) {
        let start = i + rel;
        let end = start + needle.len();
        let left_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        let right_ok = end >= bytes.len() || !is_ident_byte(bytes[end]);
        if left_ok && right_ok {
            let mut j = end;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < bytes.len() && bytes[j] == b'=' {
                j += 1;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let num_start = j;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j > num_start {
                    return block[num_start..j].parse::<u32>().ok();
                }
            }
        }
        i = end;
    }
    None
}

/// Load the infrastructure normalisation ceiling (`state_max`) via the resolver.
/// Fails if the building block / field cannot be found (never silently defaults).
pub fn infra_state_max_from_resolver(resolver: &GameFileResolver) -> Result<u32> {
    let path =
        resolver.resolve_required(INFRA_BUILDINGS_SOURCE, "infrastructure level_cap.state_max")?;
    let source = crate::read_latin1(&path)?;
    parse_infrastructure_state_max(&source).ok_or_else(|| {
        format!("{INFRA_BUILDINGS_SOURCE}: infrastructure level_cap.state_max not found").into()
    })
}

// ---------------------------------------------------------------------------
// Infra level -> gradient tint.
// ---------------------------------------------------------------------------

/// Per-state infrastructure-level input, shaped like the `level` leaf of
/// `replay_core`'s `state_buildings["infrastructure"]` `BuildingStatus`. `None` =
/// the leaf is absent/`Unobtained` (or the state has no buildings block) →
/// fail-closed to no tint.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InfraLevelInput {
    pub level: Option<f64>,
}

/// The computed infrastructure tint for one state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InfraStateTint {
    pub state_id: u32,
    /// `clamp(level / state_max, 0, 1)` — the gradient parameter (`None` when no
    /// level is present).
    pub t: Option<f32>,
    /// The RGBA the state fill is keyed with (`None` = no infra level →
    /// fail-closed, no tint).
    pub fill_rgba: Option<[f32; 4]>,
}

/// The gradient parameter for an infra level: `clamp(level / state_max, 0, 1)`.
/// A non-positive `state_max` yields 0 (degenerate mod config, fail-safe).
pub fn infra_ratio(level: f64, state_max: u32) -> f32 {
    if state_max == 0 {
        return 0.0;
    }
    ((level as f32) / state_max as f32).clamp(0.0, 1.0)
}

/// Linearly interpolate the gradient endpoints per RGBA channel at `t`. The
/// endpoints are returned EXACTLY at `t<=0` / `t>=1` (a plain lerp accrues f32
/// rounding, e.g. `1.0 + (0.1-1.0)*1.0 = 0.100000024`; the map fill must land on
/// the file colours at the ends).
pub fn infra_gradient_rgba(t: f32, colors: &InfraGradientColors) -> [f32; 4] {
    if t <= 0.0 {
        return colors.from;
    }
    if t >= 1.0 {
        return colors.to;
    }
    [
        colors.from[0] + (colors.to[0] - colors.from[0]) * t,
        colors.from[1] + (colors.to[1] - colors.from[1]) * t,
        colors.from[2] + (colors.to[2] - colors.from[2]) * t,
        colors.from[3] + (colors.to[3] - colors.from[3]) * t,
    ]
}

/// Compute the infrastructure tint for one state (fail-closed to no tint when the
/// level leaf is absent).
pub fn infra_state_tint(
    state_id: u32,
    input: &InfraLevelInput,
    state_max: u32,
    colors: &InfraGradientColors,
) -> InfraStateTint {
    match input.level {
        Some(level) => {
            let t = infra_ratio(level, state_max);
            InfraStateTint {
                state_id,
                t: Some(t),
                fill_rgba: Some(infra_gradient_rgba(t, colors)),
            }
        }
        None => InfraStateTint {
            state_id,
            t: None,
            fill_rgba: None,
        },
    }
}

/// Compute infrastructure tints for a set of states. States absent from `inputs`
/// are never tinted (they are not inserted into the output).
pub fn infra_state_tints(
    inputs: &BTreeMap<u32, InfraLevelInput>,
    state_max: u32,
    colors: &InfraGradientColors,
) -> BTreeMap<u32, InfraStateTint> {
    inputs
        .iter()
        .map(|(&state_id, input)| {
            (
                state_id,
                infra_state_tint(state_id, input, state_max, colors),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRAPHICS_SNIPPET: &str = r#"
NDefines = {
	NGraphics = {
		MAP_MODE_MANPOWER_RANGE_COLOR_FROM = { 0.2, 0.2, 0.7, 0.18 },
		MAP_MODE_MANPOWER_RANGE_COLOR_TO = { 1, 0.125, 0.0, 0.6 },
		MAP_MODE_INFRA_RANGE_COLOR_FROM = { 1, 0.125, 0.0, 0.1 },
		MAP_MODE_INFRA_RANGE_COLOR_TO = { 0.1, 0.9, 0.1, 0.6 },
	}
}
"#;

    const BUILDINGS_SNIPPET: &str = r#"
buildings = {
	infrastructure = {
		base_cost = 6000
		infrastructure = yes # a flag, not the state_max
		value = 1
		infrastructure_construction_effect = yes
		level_cap = {
			state_max = 5
		}
	}
	energy_infrastructure = {
		level_cap = { state_max = 99 }
	}
	industrial_infrastructure = {
		level_cap = { state_max = 42 }
	}
}
"#;

    #[test]
    fn gradient_colors_parse_matches_vanilla_constant() {
        let parsed = infra_gradient_colors_from_source(GRAPHICS_SNIPPET).expect("parse colours");
        assert_eq!(parsed, VANILLA_INFRA_GRADIENT_COLORS);
    }

    #[test]
    fn state_max_parse_picks_the_right_infrastructure_block() {
        // Must be 5 (the standalone `infrastructure` building), NOT 99/42 from the
        // energy_/industrial_ blocks, and not tripped by the `infrastructure = yes`
        // flag or `infrastructure_construction_effect`.
        assert_eq!(
            parse_infrastructure_state_max(BUILDINGS_SNIPPET),
            Some(VANILLA_INFRA_STATE_MAX)
        );
    }

    #[test]
    fn ratio_and_gradient_law() {
        let c = VANILLA_INFRA_GRADIENT_COLORS;
        // level 0 -> t 0 -> FROM exactly.
        assert!((infra_ratio(0.0, 5) - 0.0).abs() < 1e-6);
        assert_eq!(infra_gradient_rgba(0.0, &c), c.from);
        // level == max -> t 1 -> TO exactly.
        assert!((infra_ratio(5.0, 5) - 1.0).abs() < 1e-6);
        assert_eq!(infra_gradient_rgba(1.0, &c), c.to);
        // over the max clamps to 1.
        assert!((infra_ratio(9.0, 5) - 1.0).abs() < 1e-6);
        // midpoint interpolates each channel.
        let mid = infra_gradient_rgba(infra_ratio(2.5, 5), &c);
        for ch in 0..4 {
            let want = c.from[ch] + (c.to[ch] - c.from[ch]) * 0.5;
            assert!((mid[ch] - want).abs() < 1e-6, "channel {ch}");
        }
    }

    #[test]
    fn tint_fails_closed_without_level() {
        let c = VANILLA_INFRA_GRADIENT_COLORS;
        let t = infra_state_tint(7, &InfraLevelInput::default(), 5, &c);
        assert_eq!(t.t, None);
        assert_eq!(t.fill_rgba, None);
    }

    /// `fx_building_damage` gate. Expectations derived from the fixture generator
    /// SOURCE (`replay_fixture_gen/src/main.rs::fx_building_damage`), NOT by
    /// running the code under test:
    ///   states.10.buildings.infrastructure = 5   (constant across all 12 ticks)
    ///   states 20 / 30 have NO `buildings` block  -> no infra level.
    /// So `state_buildings["infrastructure"].level` = 5.0 for state 10 (→ t=1.0,
    /// gradient TO endpoint) and absent for states 20/30 (→ fail-closed, no tint).
    #[test]
    fn fx_building_damage_infra_tints() {
        let c = VANILLA_INFRA_GRADIENT_COLORS;
        let state_max = VANILLA_INFRA_STATE_MAX;
        let inputs: BTreeMap<u32, InfraLevelInput> = BTreeMap::from([
            (10, InfraLevelInput { level: Some(5.0) }),
            (20, InfraLevelInput { level: None }),
            (30, InfraLevelInput { level: None }),
        ]);
        let tints = infra_state_tints(&inputs, state_max, &c);
        // State 10: infra 5 of 5 -> the TO endpoint exactly.
        assert_eq!(tints[&10].t, Some(1.0));
        assert_eq!(tints[&10].fill_rgba, Some(c.to));
        // States 20/30: no infra level -> fail-closed, no tint.
        assert_eq!(tints[&20].fill_rgba, None);
        assert_eq!(tints[&30].fill_rgba, None);
    }

    /// Vanilla parse gate (skips without `BATTLE_PLAN_GAME_DIR`). Confirms the shipped
    /// gradient colours + the infrastructure `state_max` ceiling.
    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_infra_defines() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let colors = infra_gradient_colors_from_resolver(&resolver).expect("gradient colours");
        assert_eq!(colors, VANILLA_INFRA_GRADIENT_COLORS);
        let state_max = infra_state_max_from_resolver(&resolver).expect("state_max");
        assert_eq!(state_max, VANILLA_INFRA_STATE_MAX);
    }
}
