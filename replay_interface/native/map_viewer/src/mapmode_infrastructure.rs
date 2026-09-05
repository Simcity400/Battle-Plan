
use crate::mod_resolver::GameFileResolver;
use crate::Result;
use std::collections::BTreeMap;

pub const INFRA_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";
pub const INFRA_BUILDINGS_SOURCE: &str = "common/buildings/00_buildings.txt";

pub const INFRASTRUCTURE_BUILDING_KEY: &str = "infrastructure";


#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InfraGradientColors {
    pub from: [f32; 4],
    pub to: [f32; 4],
}

pub const VANILLA_INFRA_GRADIENT_COLORS: InfraGradientColors = InfraGradientColors {
    from: [1.0, 0.125, 0.0, 0.1],
    to: [0.1, 0.9, 0.1, 0.6],
};

pub const VANILLA_INFRA_STATE_MAX: u32 = 5;

pub fn infra_gradient_colors_from_resolver(
    resolver: &GameFileResolver,
) -> Result<InfraGradientColors> {
    let (sources, _) = crate::xmap09_lighting_law::effective_define_sources(resolver, "effective defines")?;
    let evaluated = crate::xmap09_lighting_law::EffectiveDefines::from_sources(&sources)?;

    Ok(InfraGradientColors {
        from: evaluated.array("NMapMode", "MAP_MODE_INFRA_RANGE_COLOR_FROM")?,
        to: evaluated.array("NMapMode", "MAP_MODE_INFRA_RANGE_COLOR_TO")?,
    })

}


pub fn parse_infrastructure_state_max(source: &str) -> Option<u32> {
    let text = crate::strip_comments(source);
    let bytes = text.as_bytes();
    let key = INFRASTRUCTURE_BUILDING_KEY.as_bytes();
    let mut i = 0usize;
    while let Some(rel) = find_subslice(&bytes[i..], key) {
        let start = i + rel;
        let end = start + key.len();
        let left_ok = start == 0 || !is_ident_byte(bytes[start - 1]);
        if left_ok {
            if let Some(brace) = match_eq_open_brace(&bytes[end..]) {
                let block_start = end + brace + 1;
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

pub fn infra_state_max_from_resolver(resolver: &GameFileResolver) -> Result<u32> {
    let path =
        resolver.resolve_required(INFRA_BUILDINGS_SOURCE, "infrastructure level_cap.state_max")?;
    let source = resolver.read_required_text(INFRA_BUILDINGS_SOURCE, INFRA_BUILDINGS_SOURCE)?;
    parse_infrastructure_state_max(&source).ok_or_else(|| {
        format!("{INFRA_BUILDINGS_SOURCE}: infrastructure level_cap.state_max not found").into()
    })
}


#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InfraLevelInput {
    pub level: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InfraStateTint {
    pub state_id: u32,
    pub t: Option<f32>,
    pub fill_rgba: Option<[f32; 4]>,
}

pub fn infra_ratio(level: f64, state_max: u32) -> f32 {
    if state_max == 0 {
        return 0.0;
    }
    ((level as f32) / state_max as f32).clamp(0.0, 1.0)
}

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
        let parsed = crate::test_support::with_define_fixture(GRAPHICS_SNIPPET, infra_gradient_colors_from_resolver).expect("parse colours");
        assert_eq!(parsed, VANILLA_INFRA_GRADIENT_COLORS);
    }

    #[test]
    fn state_max_parse_picks_the_right_infrastructure_block() {
        assert_eq!(
            parse_infrastructure_state_max(BUILDINGS_SNIPPET),
            Some(VANILLA_INFRA_STATE_MAX)
        );
    }

    #[test]
    fn ratio_and_gradient_law() {
        let c = VANILLA_INFRA_GRADIENT_COLORS;
        assert!((infra_ratio(0.0, 5) - 0.0).abs() < 1e-6);
        assert_eq!(infra_gradient_rgba(0.0, &c), c.from);
        assert!((infra_ratio(5.0, 5) - 1.0).abs() < 1e-6);
        assert_eq!(infra_gradient_rgba(1.0, &c), c.to);
        assert!((infra_ratio(9.0, 5) - 1.0).abs() < 1e-6);
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
        assert_eq!(tints[&10].t, Some(1.0));
        assert_eq!(tints[&10].fill_rgba, Some(c.to));
        assert_eq!(tints[&20].fill_rgba, None);
        assert_eq!(tints[&30].fill_rgba, None);
    }

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
