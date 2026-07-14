//! Factions map mode (MAPMODE_COVERAGE_MATRIX row 11 / plan E3, toolbar button
//! `mapmode_button_11` / `ui_mapmode_factions` / F10, engine layer
//! `game_map_mode_factions`).
//!
//! The factions map mode colours every country by faction membership: members
//! take the faction **leader's** country colour (so each faction reads as one
//! bloc), and non-members are greyed out. Colours are computed replay-side from
//! the recorded faction rosters, **never recorded** (standing override).
//!
//! ## Colour law provenance
//!
//! * **Non-member fill** — FILE. `common/defines/00_graphics.lua`
//!   `FACTIONS_COLOR_NOT_MEMBER = { 0.7, 0.7, 0.7, 0.7 }`
//!   (`NDefines.NMapMode`). Parsed through the mod-aware resolver so mods
//!   override correctly; [`VANILLA_FACTIONS_DEFINES`] is the offline gate
//!   reference. `FACTIONS_MEMBER_TRANSPARENCY = 1.0` (members are opaque) is the
//!   companion FILE define.
//!
//! * **Member fill = faction-leader colour** — DERIVED. The engine layer
//!   `game_map_mode_factions` has no scriptable colour (it is a hard-coded C++
//!   layer; `common/map_modes/documentation.md` lists it among the hard-coded
//!   types, and the matrix records the rule as "colour = faction/leader"). A
//!   faction is keyed by its leader TAG, so members read the leader's country
//!   colour. TWO residuals ride this and are flagged for a captured factions
//!   frame (CAP) / binary RE:
//!     1. **leader-colour vs own-colour** — whether members show the leader's
//!        colour (bloc view, implemented here) or each their own colour.
//!     2. **raw vs COL3-modified colour** — whether the fill uses the raw
//!        country colour (as used here and by the naval tint) or the
//!        HSV-modified political colour (`COL3` law, `COUNTRY_COLOR_*_MODIFIER`).
//!   The membership CLASSIFICATION (which flips the gate) is fully determined by
//!   the recording and is independent of both residuals.
//!
//! ## Seam
//!
//! [`FactionInput`] mirrors `replay_core::world::DiplomacyView` faction data
//! (`faction_members` id→TAGs plus the per-country `faction_is_leader` /
//! `diplomacy_world.factions.leader`). The typed roster status is required at
//! this seam so an unwitnessed roster stays unknown.
//!
//! ## Production dispatch status
//!
//! The factions map render consumer is not built in-tree yet. Production
//! `map_viewer` currently re-exports this classifier/fill seam but has no live
//! factions-mode dispatch or draw call; all calls to
//! [`country_faction_memberships`] are tests. The tri-state is preserved through
//! typed replay and the game-UI binding, and this helper is ready. When the live
//! render consumer lands it MUST consume that typed status through this helper
//! (so `Unavailable` produces no classification/fill), with an integration test
//! that drives the real production dispatch. That renderer test is explicitly
//! owed; a test-only helper call is not evidence of production enforcement.

use crate::mod_resolver::GameFileResolver;
use crate::Result;
use replay_core::world::FactionRosterStatus;
use std::collections::BTreeMap;

/// `00_graphics.lua` — source of the factions map-mode FILE colours below.
pub const FACTIONS_GRAPHICS_DEFINES_SOURCE: &str = "common/defines/00_graphics.lua";

/// The `00_graphics.lua` constants that drive the factions map mode.
///
/// FILE citations (`common/defines/00_graphics.lua`, `NDefines.NMapMode`):
/// - `FACTIONS_COLOR_NOT_MEMBER` — the RGBA every non-faction country is painted.
/// - `FACTIONS_MEMBER_TRANSPARENCY` — the alpha applied to member (leader-colour)
///   fills; 1.0 = opaque in vanilla.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FactionsDefines {
    /// `FACTIONS_COLOR_NOT_MEMBER` (FILE).
    pub not_member_color: [f32; 4],
    /// `FACTIONS_MEMBER_TRANSPARENCY` (FILE).
    pub member_transparency: f32,
}

/// The vanilla `00_graphics.lua` values, verbatim (FILE, cited above). Offline
/// gate reference; the resolver path re-parses the shipped file so mods override.
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

/// Parse the factions defines out of `00_graphics.lua` source text.
pub fn factions_defines_from_source(source: &str) -> Result<FactionsDefines> {
    Ok(FactionsDefines {
        not_member_color: parse_color4(source, "FACTIONS_COLOR_NOT_MEMBER")?,
        member_transparency: crate::parse_lua_number_define(
            source,
            "FACTIONS_MEMBER_TRANSPARENCY",
        )?,
    })
}

/// Load + parse the factions defines via the mod-aware resolver.
pub fn factions_defines_from_resolver(resolver: &GameFileResolver) -> Result<FactionsDefines> {
    let path = resolver.resolve_required(
        FACTIONS_GRAPHICS_DEFINES_SOURCE,
        "factions graphics defines",
    )?;
    let source = crate::read_latin1(&path)?;
    factions_defines_from_source(&source)
}

/// One faction roster, shaped like the recorded `diplomacy_world.factions` /
/// `faction_members` data: a faction id, its leader TAG (the colour key), and
/// its member TAGs. `leader` is `None` when the recording omits it (fail-closed).
#[derive(Clone, Debug, PartialEq)]
pub struct FactionInput {
    pub faction_id: String,
    pub leader: Option<String>,
    pub members: Vec<String>,
}

/// A country's faction membership in the factions map mode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FactionMembership {
    /// No completeness witness: membership is unknown and must not receive the
    /// positive non-member grey.
    Unknown,
    /// In a faction; painted with `leader`'s country colour (`None` leader ⇒
    /// fail-closed to no member colour at fill time).
    Member {
        faction_id: String,
        leader: Option<String>,
    },
    /// In no faction; painted `FACTIONS_COLOR_NOT_MEMBER`.
    NonMember,
}

/// Build the TAG → membership map from a witnessed faction roster. Any
/// unavailable or contradictory status/row shape fails closed to `None`, and a
/// duplicate member makes the entire roster unknown. Only a measured roster
/// may classify a country omitted from all rows as
/// [`FactionMembership::NonMember`].
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

/// Look up a country's membership. An unavailable roster yields
/// [`FactionMembership::Unknown`]; only a measured roster can establish
/// [`FactionMembership::NonMember`].
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

/// The fill colour for a country in the factions map mode.
///
/// - [`FactionMembership::NonMember`] → `FACTIONS_COLOR_NOT_MEMBER` (FILE).
/// - [`FactionMembership::Member`] → the leader's country colour at
///   `member_transparency` alpha (DERIVED; see module header residuals). Fails
///   **closed** to `None` when the leader tag is missing or has no colour entry
///   — no invented colour.
///
/// `country_colors` is the resolved tag → RGB table
/// (`game_data::*country_colors*`), the same table the political fill and naval
/// tint use (raw colours; the raw-vs-COL3 question is a flagged residual).
pub fn faction_fill(
    membership: &FactionMembership,
    country_colors: &BTreeMap<String, [u8; 3]>,
    defines: &FactionsDefines,
) -> Option<[f32; 4]> {
    match membership {
        FactionMembership::Unknown => None,
        FactionMembership::NonMember => Some(defines.not_member_color),
        FactionMembership::Member { leader, .. } => {
            let leader = leader.as_deref().filter(|t| !t.is_empty())?;
            let rgb = country_colors.get(leader)?;
            Some([
                rgb[0] as f32 / 255.0,
                rgb[1] as f32 / 255.0,
                rgb[2] as f32 / 255.0,
                defines.member_transparency,
            ])
        }
    }
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
            Some([0.7, 0.7, 0.7, 0.7])
        );
    }

    #[test]
    fn member_takes_leader_color_at_member_alpha() {
        let d = VANILLA_FACTIONS_DEFINES;
        let colors: BTreeMap<String, [u8; 3]> =
            BTreeMap::from([("GER".to_string(), [80u8, 80, 90])]);
        let m = FactionMembership::Member {
            faction_id: "axis".to_string(),
            leader: Some("GER".to_string()),
        };
        let fill = faction_fill(&m, &colors, &d).expect("member fill");
        assert!((fill[0] - 80.0 / 255.0).abs() < 1e-6);
        assert!((fill[1] - 80.0 / 255.0).abs() < 1e-6);
        assert!((fill[2] - 90.0 / 255.0).abs() < 1e-6);
        assert!((fill[3] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn member_fails_closed_without_leader_or_color() {
        let d = VANILLA_FACTIONS_DEFINES;
        let colors: BTreeMap<String, [u8; 3]> =
            BTreeMap::from([("GER".to_string(), [80u8, 80, 90])]);
        // Missing leader.
        let m = FactionMembership::Member {
            faction_id: "axis".to_string(),
            leader: None,
        };
        assert_eq!(faction_fill(&m, &colors, &d), None);
        // Leader with no colour entry.
        let m = FactionMembership::Member {
            faction_id: "axis".to_string(),
            leader: Some("ZZZ".to_string()),
        };
        assert_eq!(faction_fill(&m, &colors, &d), None);
    }

    // ---------------------------------------------------------------------
    // fx_war_faction_matrix gate (matrix row 11).
    //
    // Expectations derived from the FIXTURE GENERATOR SOURCE
    // (`replay_fixture_gen/src/main.rs::fx_war_faction_matrix`), NOT by running
    // the code under test. SNAPSHOT_COUNT = 12 (ticks 0..=11):
    //   tick <  3 : no factions -> everyone NonMember.
    //   tick >= 3 : diplomacy_world.factions = { axis{leader GER, [GER,ITA]},
    //               allies{leader ENG, [ENG]} } (and faction_members keyed by id).
    // Membership flips at tick 3.
    // ---------------------------------------------------------------------

    /// The generator's faction rosters at a given tick.
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
        // Pre-faction: all three are non-members.
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
        // Post-faction: GER+ITA -> Axis (leader GER); ENG -> Allies (leader ENG).
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
        // The flip itself: ITA non-member at tick 2, member at tick 3.
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

    /// The full fill flips at tick 3: everyone grey before, leader-coloured blocs
    /// after (Axis = GER colour, Allies = ENG colour).
    #[test]
    fn fx_war_faction_matrix_fill_flips_at_tick_3() {
        let d = VANILLA_FACTIONS_DEFINES;
        let colors: BTreeMap<String, [u8; 3]> = BTreeMap::from([
            ("GER".to_string(), [80u8, 80, 90]),
            ("ITA".to_string(), [70u8, 120, 70]),
            ("ENG".to_string(), [200u8, 40, 40]),
        ]);
        let ger = [80.0 / 255.0, 80.0 / 255.0, 90.0 / 255.0, 1.0];
        let eng = [200.0 / 255.0, 40.0 / 255.0, 40.0 / 255.0, 1.0];
        let gray = [0.7, 0.7, 0.7, 0.7];

        // Tick 2: all grey.
        let m2 = country_faction_memberships(fixture_roster_status(2), &fixture_factions(2));
        for tag in ["GER", "ITA", "ENG"] {
            assert_eq!(
                faction_fill(&membership_for(tag, &m2), &colors, &d),
                Some(gray),
                "{tag} tick 2"
            );
        }
        // Tick 3: Axis members (GER, ITA) both take GER (leader) colour; ENG takes ENG.
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
        // A country in no faction stays grey after tick 3.
        assert_eq!(
            faction_fill(&membership_for("SWE", &m3), &colors, &d),
            Some(gray)
        );
    }

    /// Vanilla defines parse gate (skips without `BATTLE_PLAN_GAME_DIR`).
    #[test]
    fn gate_vanilla_factions_defines() {
        let Ok(game_dir) = std::env::var("BATTLE_PLAN_GAME_DIR").or_else(|_| std::env::var("HOI4_GAME_DIR")) else {
            eprintln!("gate_vanilla_factions_defines: BATTLE_PLAN_GAME_DIR unset; skipped");
            return;
        };
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
