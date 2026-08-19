
use crate::mod_resolver::GameFileResolver;
use crate::Result;
use replay_core::world::FactionRosterStatus;
use std::collections::BTreeMap;

pub const FACTIONS_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FactionsDefines {
    pub not_member_color: [f32; 4],
    pub member_transparency: f32,
}

pub const VANILLA_FACTIONS_DEFINES: FactionsDefines = FactionsDefines {
    not_member_color: [0.7, 0.7, 0.7, 0.7],
    member_transparency: 1.0,
};

fn parse_color4(source: &str, key: &str) -> Result<[f32; 4]> {
    let values = crate::map_furniture::parse_lua_number_list(source, key).ok_or_else(|| {
        format!("{FACTIONS_GRAPHICS_DEFINES_SOURCE}: missing colour define {key}")
    })?;
    if values.len() != 4 {
        return Err(format!(
            "{FACTIONS_GRAPHICS_DEFINES_SOURCE}: {key} expected 4 components, got {}",
            values.len()
        )
        .into());
    }
    Ok([values[0], values[1], values[2], values[3]])
}

pub fn factions_defines_from_source(source: &str) -> Result<FactionsDefines> {
    Ok(FactionsDefines {
        not_member_color: parse_color4(source, "FACTIONS_COLOR_NOT_MEMBER")?,
        member_transparency: crate::parse_lua_number_define(
            source,
            "FACTIONS_MEMBER_TRANSPARENCY",
        )?,
    })
}

pub fn factions_defines_from_resolver(resolver: &GameFileResolver) -> Result<FactionsDefines> {
    let path = resolver.resolve_required(
        FACTIONS_GRAPHICS_DEFINES_SOURCE,
        "factions graphics defines",
    )?;
    let source = crate::read_latin1(&path)?;
    factions_defines_from_source(&source)
}

#[derive(Clone, Debug, PartialEq)]
pub struct FactionInput {
    pub faction_id: String,
    pub leader: Option<String>,
    pub members: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactionMembership {
    Unknown,
    Member {
        faction_id: String,
        leader: Option<String>,
    },
    NonMember,
}

pub fn country_faction_memberships(
    roster_status: FactionRosterStatus,
    factions: &[FactionInput],
) -> Option<BTreeMap<String, FactionMembership>> {
    let shape_valid = match roster_status {
        FactionRosterStatus::Unavailable => factions.is_empty(),
        FactionRosterStatus::MeasuredEmpty => factions.is_empty(),
        FactionRosterStatus::MeasuredNonEmpty => !factions.is_empty(),
    };
    if !shape_valid || roster_status == FactionRosterStatus::Unavailable {
        return None;
    }
    let mut out: BTreeMap<String, FactionMembership> = BTreeMap::new();
    for faction in factions {
        if faction.faction_id.is_empty() || faction.members.is_empty() {
            return None;
        }
        for member in &faction.members {
            if member.is_empty() || out.contains_key(member) {
                return None;
            }
            out.insert(
                member.clone(),
                FactionMembership::Member {
                    faction_id: faction.faction_id.clone(),
                    leader: faction.leader.clone(),
                },
            );
        }
    }
    Some(out)
}

pub fn membership_for(
    tag: &str,
    memberships: &Option<BTreeMap<String, FactionMembership>>,
) -> FactionMembership {
    let Some(memberships) = memberships else {
        return FactionMembership::Unknown;
    };
    memberships
        .get(tag)
        .cloned()
        .unwrap_or(FactionMembership::NonMember)
}

pub fn faction_fill(
    membership: &FactionMembership,
    col3_country_colors: &BTreeMap<String, [u8; 3]>,
    defines: &FactionsDefines,
) -> Option<[u8; 4]> {
    match membership {
        FactionMembership::Unknown => None,
        FactionMembership::NonMember => Some([
            carrier_channel(defines.not_member_color[0])?,
            carrier_channel(defines.not_member_color[1])?,
            carrier_channel(defines.not_member_color[2])?,
            0,
        ]),
        FactionMembership::Member { leader, .. } => {
            let leader = leader.as_deref().filter(|t| !t.is_empty())?;
            let rgb = col3_country_colors.get(leader)?;
            Some([rgb[0], rgb[1], rgb[2], 0])
        }
    }
}

fn carrier_channel(value: f32) -> Option<u8> {
    (value.is_finite() && (0.0..=1.0).contains(&value)).then(|| (value * 255.0) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_core::{open_recording, WorldTimeline};
    use serde_json::json;
    use std::fs;

    const GRAPHICS_SNIPPET: &str = r#"
NDefines = {
	NMapMode = {
		FACTIONS_COLOR_NOT_MEMBER = { 0.7, 0.7, 0.7, 0.7 },
		FACTIONS_MEMBER_TRANSPARENCY = 1.0,
		PLAYER_MAPMODE_NOT_SELECTED_COUNTRY_TRANSPARENCY = 0.15,
	}
}
"#;

    #[test]
    fn defines_parse_matches_vanilla_constant() {
        let parsed = factions_defines_from_source(GRAPHICS_SNIPPET).expect("parse defines");
        assert_eq!(parsed, VANILLA_FACTIONS_DEFINES);
    }

    #[test]
    fn non_member_takes_file_gray() {
        let d = VANILLA_FACTIONS_DEFINES;
        let colors: BTreeMap<String, [u8; 3]> = BTreeMap::new();
        assert_eq!(
            faction_fill(&FactionMembership::NonMember, &colors, &d),
            Some([178, 178, 178, 0])
        );
    }

    #[test]
    fn member_takes_col3_leader_color_with_zero_carrier_alpha() {
        let d = VANILLA_FACTIONS_DEFINES;
        let colors: BTreeMap<String, [u8; 3]> =
            BTreeMap::from([("GER".to_string(), [80u8, 80, 90])]);
        let m = FactionMembership::Member {
            faction_id: "axis".to_string(),
            leader: Some("GER".to_string()),
        };
        let fill = faction_fill(&m, &colors, &d).expect("member fill");
        assert_eq!(fill, [80, 80, 90, 0]);
    }

    #[test]
    fn member_fails_closed_without_leader_or_color() {
        let d = VANILLA_FACTIONS_DEFINES;
        let colors: BTreeMap<String, [u8; 3]> =
            BTreeMap::from([("GER".to_string(), [80u8, 80, 90])]);
        let m = FactionMembership::Member {
            faction_id: "axis".to_string(),
            leader: None,
        };
        assert_eq!(faction_fill(&m, &colors, &d), None);
        let m = FactionMembership::Member {
            faction_id: "axis".to_string(),
            leader: Some("ZZZ".to_string()),
        };
        assert_eq!(faction_fill(&m, &colors, &d), None);
    }


    fn fixture_factions(tick: usize) -> Vec<FactionInput> {
        if tick < 3 {
            Vec::new()
        } else {
            vec![
                FactionInput {
                    faction_id: "axis".to_string(),
                    leader: Some("GER".to_string()),
                    members: vec!["GER".to_string(), "ITA".to_string()],
                },
                FactionInput {
                    faction_id: "allies".to_string(),
                    leader: Some("ENG".to_string()),
                    members: vec!["ENG".to_string()],
                },
            ]
        }
    }

    fn fixture_roster_status(tick: usize) -> FactionRosterStatus {
        if tick < 3 {
            FactionRosterStatus::MeasuredEmpty
        } else {
            FactionRosterStatus::MeasuredNonEmpty
        }
    }

    #[test]
    fn fx_war_faction_matrix_membership_flips_at_tick_3() {
        for tick in 0..3 {
            let m =
                country_faction_memberships(fixture_roster_status(tick), &fixture_factions(tick));
            for tag in ["GER", "ITA", "ENG"] {
                assert_eq!(
                    membership_for(tag, &m),
                    FactionMembership::NonMember,
                    "{tag} tick {tick}"
                );
            }
        }
        for tick in 3..12 {
            let m =
                country_faction_memberships(fixture_roster_status(tick), &fixture_factions(tick));
            assert_eq!(
                membership_for("GER", &m),
                FactionMembership::Member {
                    faction_id: "axis".to_string(),
                    leader: Some("GER".to_string())
                },
                "GER tick {tick}"
            );
            assert_eq!(
                membership_for("ITA", &m),
                FactionMembership::Member {
                    faction_id: "axis".to_string(),
                    leader: Some("GER".to_string())
                },
                "ITA tick {tick}"
            );
            assert_eq!(
                membership_for("ENG", &m),
                FactionMembership::Member {
                    faction_id: "allies".to_string(),
                    leader: Some("ENG".to_string())
                },
                "ENG tick {tick}"
            );
        }
        let before = membership_for(
            "ITA",
            &country_faction_memberships(fixture_roster_status(2), &fixture_factions(2)),
        );
        let after = membership_for(
            "ITA",
            &country_faction_memberships(fixture_roster_status(3), &fixture_factions(3)),
        );
        assert_ne!(before, after);
    }

    #[test]
    fn fx_war_faction_matrix_fill_flips_at_tick_3() {
        let d = VANILLA_FACTIONS_DEFINES;
        let colors: BTreeMap<String, [u8; 3]> = BTreeMap::from([
            ("GER".to_string(), [80u8, 80, 90]),
            ("ITA".to_string(), [70u8, 120, 70]),
            ("ENG".to_string(), [200u8, 40, 40]),
        ]);
        let ger = [80, 80, 90, 0];
        let eng = [200, 40, 40, 0];
        let gray = [178, 178, 178, 0];

        let m2 = country_faction_memberships(fixture_roster_status(2), &fixture_factions(2));
        for tag in ["GER", "ITA", "ENG"] {
            assert_eq!(
                faction_fill(&membership_for(tag, &m2), &colors, &d),
                Some(gray),
                "{tag} tick 2"
            );
        }
        let m3 = country_faction_memberships(fixture_roster_status(3), &fixture_factions(3));
        assert_eq!(
            faction_fill(&membership_for("GER", &m3), &colors, &d),
            Some(ger)
        );
        assert_eq!(
            faction_fill(&membership_for("ITA", &m3), &colors, &d),
            Some(ger),
            "member takes LEADER colour"
        );
        assert_eq!(
            faction_fill(&membership_for("ENG", &m3), &colors, &d),
            Some(eng)
        );
        assert_eq!(
            faction_fill(&membership_for("SWE", &m3), &colors, &d),
            Some(gray)
        );
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "the game install: set BATTLE_PLAN_GAME_ROOT (or BATTLE_PLAN_GAME_DIR / BATTLE_PLAN_VANILLA) to it, or install it at the default Steam path"
    )]
    fn gate_vanilla_factions_defines() {
        let game_dir = replay_test_gates::require_vanilla_game_root();
        let resolver = GameFileResolver::vanilla(game_dir).expect("resolver");
        let defines = factions_defines_from_resolver(&resolver).expect("factions defines");
        assert_eq!(defines, VANILLA_FACTIONS_DEFINES);
    }

    #[test]
    fn open_typed_status_makes_unmeasured_unknown_and_measured_empty_nonmember() {
        let root = std::env::temp_dir().join(format!(
            "mapmode_factions_status_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let game = "mapmode_faction_status";
        let dir = root.join(game);
        fs::create_dir_all(&dir).unwrap();
        let mut index_rows = Vec::new();
        for (hour, status) in [(40, "unavailable"), (41, "measured_empty")] {
            let mut extra = json!({ "faction_roster_status": status });
            if status == "measured_empty" {
                extra["faction_rows"] = json!([]);
            }
            let snapshot = json!({
                "schema_version": "1",
                "game_id": game,
                "date": format!("1936.1.1.{}", hour - 40),
                "hour_index": hour,
                "countries": {},
                "diplomacy_world": { "domain53_extra": extra }
            });
            let file = format!("{hour:012}.snapshot.json");
            fs::write(dir.join(&file), serde_json::to_vec(&snapshot).unwrap()).unwrap();
            index_rows.push(json!({
                "hour_index": hour,
                "date": format!("1936.1.1.{}", hour - 40),
                "file": file
            }));
        }
        fs::write(
            dir.join("index.json"),
            serde_json::to_vec(&json!({
                "game_id": game,
                "schema_version": "1",
                "snapshots": index_rows
            }))
            .unwrap(),
        )
        .unwrap();
        let timeline = WorldTimeline::open(open_recording(&root).unwrap()).unwrap();

        let unknown = country_faction_memberships(
            timeline
                .frame(40)
                .unwrap()
                .diplomacy()
                .faction_roster_status(),
            &[],
        );
        assert_eq!(membership_for("GER", &unknown), FactionMembership::Unknown);

        let measured_empty = country_faction_memberships(
            timeline
                .frame(41)
                .unwrap()
                .diplomacy()
                .faction_roster_status(),
            &[],
        );
        assert_eq!(
            membership_for("GER", &measured_empty),
            FactionMembership::NonMember
        );
    }
}
