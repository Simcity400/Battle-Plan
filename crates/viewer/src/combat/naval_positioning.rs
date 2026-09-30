//! The active naval window's displayed positioning, `141609460`.
//!
//! The combatant's stored `positioning` is an hourly simulation input to
//! other combat calculations. The window recomputes the displayed value from
//! current shared combatants and matching NDefines instead of binding it.
use super::keyed::Keyed;
use super::naval_air_groups::{
    self, CarrierJoins, DenseBases, PositioningAir, PositioningAirSources,
};
use installed_content::naval_combat_defines::Defines;
use replay::rows::{
    AirBaseRow, NavalCombatDetailsRow, NavalCombatShipRow, NavalCombatSideDetails,
    NavalCombatSummaryInputs, ShipModelRow, ShipRow,
};

const ONE: i64 = 100_000;

/// `14160C940` returns the current weighted aircraft count and the opponent's
/// average carrier-base fill as signed fixed5 values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Air {
    pub weighted_planes: i64,
    pub average_base_fill: i64,
}

/// All values are derived from complete same-hour simulation tables. Counts
/// include unresolved forward member slots, while detection includes only
/// qualifying resolved ships; the two native traversals are deliberately
/// separate.
pub(super) struct Inputs<'a> {
    pub side: &'a NavalCombatSideDetails,
    pub opponent: &'a NavalCombatSideDetails,
    pub defender: bool,
    pub duration: i32,
    pub surface_detection: [i64; 2],
    pub active_ship_slots: [i32; 2],
    pub air: [Air; 2],
}

fn mul(a: i64, b: i64) -> i64 {
    a.wrapping_mul(b) / ONE
}

// `modifier_accumulator::value@140551820`: absence is zero; eligibility
// gates the raw cell before the optional [0, 1] clamp.
fn modifier(summary: &NavalCombatSummaryInputs, scope: usize, key: usize) -> Option<i64> {
    let cell = &summary.modifiers[scope][key];
    if cell.present == 0 {
        return Some(0);
    }
    let [flags, selector] = summary.modifier_metadata[key];
    if selector != 0 {
        match summary.eligibility_callback {
            0 => return Some(0),
            1 if selector & summary.feature_flags == 0 => return Some(0),
            1 => (),
            _ => return None,
        }
    }
    Some(if flags & 0x10 != 0 {
        cell.raw.clamp(0, ONE)
    } else {
        cell.raw
    })
}

/// Scopes 0-2 are the side's country, regional weather and sea terrain; 3 is
/// its own admiral and 4 the opposing admiral (`naval_combat_summary_inputs`).
/// `1415B4BB0` sums the three location scopes with one leader's modifiers.
fn summary_modifiers(
    summary: &NavalCombatSummaryInputs,
    leader_scope: usize,
    key: usize,
) -> Option<i64> {
    [0, 1, 2, leader_scope]
        .into_iter()
        .try_fold(0_i64, |sum, scope| {
            Some(sum.wrapping_add(modifier(summary, scope, key)?))
        })
}

// The native overflow predicate is shared by the detection and aircraft
// ratio branches. The ordinary branch uses signed truncating division.
fn ratio_numerator_valid(value: i64) -> bool {
    (value.wrapping_add(i64::MAX - 1) as u64) < u64::MAX - 2
}

fn detection_bonus(own: i64, opposite: i64, defines: &Defines) -> i64 {
    let difference = own.wrapping_sub(opposite);
    let ratio = if ratio_numerator_valid(difference) {
        difference.wrapping_mul(ONE) / 10_000_000
    } else {
        i64::from(u32::MAX)
    };
    mul(ratio, defines.summary_fixed[1])
        .min(defines.summary_fixed[2])
        .max(0)
}

fn ship_ratio_penalty(input: &Inputs<'_>, defines: &Defines) -> Option<i64> {
    let [own, opposite] = input.active_ship_slots;
    if opposite <= 0 || own < defines.summary_integer[0] {
        return Some(0);
    }
    let denominator = i64::from(opposite) * ONE;
    let ratio = if denominator == 0 || i64::from(own) * ONE < -(i64::MAX - 1) {
        i64::from(u32::MAX)
    } else {
        i64::from(own).wrapping_mul(10_000_000_000) / denominator
    };
    let mut penalty = mul(ratio.wrapping_sub(ONE), defines.summary_fixed[3]);
    if input.opponent.commander != 0 {
        let admiral = modifier(&input.side.summary, 4, 4)?;
        if admiral != 0 {
            penalty = mul(ONE.wrapping_add(admiral), penalty);
        }
    }
    Some(penalty.min(defines.summary_fixed[4]).max(0))
}

fn carrier_ratio_penalty(input: &Inputs<'_>, defines: &Defines) -> i64 {
    let [own, opposite] = input.air;
    if opposite.weighted_planes <= 0 {
        return 0;
    }
    let ratio = if ratio_numerator_valid(own.weighted_planes) {
        own.weighted_planes.wrapping_mul(ONE) / opposite.weighted_planes
    } else {
        i64::from(u32::MAX)
    };
    mul(
        mul(ONE.wrapping_sub(ratio), defines.summary_fixed[5]),
        opposite.average_base_fill,
    )
    .min(defines.summary_fixed[6])
    .max(0)
}

/// Native `141609460` without its optional localized tooltip construction.
/// `None` denotes a modifier callback or operand that cannot be resolved.
pub(super) fn displayed(input: &Inputs<'_>, defines: &Defines) -> Option<i64> {
    let mut value = defines.summary_fixed[0];
    let summary = &input.side.summary;
    if !summary.countries.is_empty() {
        // 141609643-14160964C: key 0x1B2 with the side's own commander
        // (combatant+0x110, loaded at 14160960B).
        value = value.wrapping_add(summary_modifiers(summary, 3, 2)?);
        if input.defender && input.duration < defines.summary_integer[1] {
            // 1416094B1-1416094B8 keep the opposing combatant's +0x110
            // commander; 14160969A-1416096B5 pass it for key 0x5A.
            value = value.wrapping_add(summary_modifiers(summary, 4, 0)?);
        }
        value = value.wrapping_add(summary.escort_bonus);
    }
    value = value.wrapping_add(detection_bonus(
        input.surface_detection[0],
        input.surface_detection[1],
        defines,
    ));
    value = value.wrapping_sub(ship_ratio_penalty(input, defines)?);
    value = value.wrapping_sub(carrier_ratio_penalty(input, defines));
    if summary.recent_join_penalty > 0 {
        value = value.wrapping_sub(summary.recent_join_penalty);
    }
    Some(value.clamp(0, ONE))
}

/// `14160C660` counts each qualifying forward member slot, even when its ship
/// or convoy handle no longer resolves. Caller supplies one combat's rows.
pub(super) fn active_ship_slots(rows: &[&NavalCombatShipRow], side: u8) -> i32 {
    rows.iter()
        .filter(|row| {
            row.side == side
                && row.member_source == 0
                && row.group_index != u32::MAX
                && row.state & 0x34 == 0
        })
        .fold(0_i32, |count, _| count.wrapping_add(1))
}

/// Compose each side once for the window's positioning and downstream AA
/// binders. A complete combat-ship census and matching definitions are always
/// required. Ship models and dense air-base counts are branch-dependent:
/// noncarrier and zero-air paths can still resolve when those tables are
/// incomplete, while a relevant missing join leaves only the affected side
/// unavailable. `models` is the binding's shared ShipModels lookup; only the
/// dense air-base rows this combat's planes and members reference are summed.
#[expect(
    clippy::too_many_arguments,
    reason = "positioning joins independent simulation sources"
)]
pub(super) fn positioning_values(
    detail: &NavalCombatDetailsRow,
    ships: Option<&[NavalCombatShipRow]>,
    models: Option<Keyed<'_, ShipModelRow>>,
    bases: Option<&[AirBaseRow]>,
    live_ships: Option<&[ShipRow]>,
    dense: Option<DenseBases<'_>>,
    content: &naval_air_groups::Content,
    defines: Option<&Defines>,
) -> [Option<i64>; 2] {
    let (Some(ships), Some(defines)) = (ships, defines) else {
        return [None; 2];
    };
    let members: Vec<_> = ships
        .iter()
        .filter(|ship| ship.combat == detail.combat)
        .collect();
    let slots = [0, 1].map(|side| active_ship_slots(&members, side));
    let dense_counts = dense.and_then(|dense| {
        dense.counts(
            detail
                .sides
                .iter()
                .flat_map(|side| side.planes.iter().map(|plane| plane.air_base))
                .chain(members.iter().map(|member| member.weapon_context.air_base)),
        )
    });
    let dense_counts = dense_counts.as_ref();
    let detection = [0, 1].map(|side| {
        super::naval_surface_detection::sum_active_surface_detection(
            &members,
            models,
            dense_counts,
            side,
        )
    });
    // Both sides' carrier-air passes share one set of reference indexes.
    let joins = CarrierJoins::new(Some(&members), bases, live_ships, models);
    let air = [0, 1].map(|side| {
        naval_air_groups::positioning_air(
            &detail.sides[side as usize],
            &PositioningAirSources {
                side_index: side,
                duration: detail.duration,
                joins: &joins,
                base_aircraft: dense_counts,
                content,
                defines: Some(defines),
            },
        )
    });
    std::array::from_fn(|side| {
        let opposite = 1 - side;
        let opposing_air = air[opposite]?;
        // 141609460 calls the own-side air pass only after it sees a positive
        // opposing weighted total. An unknown own pass is irrelevant when
        // there are no opposing aircraft to impose a carrier-ratio penalty.
        let own_air = if opposing_air.weighted_planes > 0 {
            air[side]?
        } else {
            PositioningAir::default()
        };
        displayed(
            &Inputs {
                side: &detail.sides[side],
                opponent: &detail.sides[opposite],
                defender: side == 1,
                duration: detail.duration,
                surface_detection: [detection[side]?, detection[opposite]?],
                active_ship_slots: [slots[side], slots[opposite]],
                air: [
                    Air {
                        weighted_planes: own_air.weighted_planes,
                        average_base_fill: own_air.average_base_fill,
                    },
                    Air {
                        weighted_planes: opposing_air.weighted_planes,
                        average_base_fill: opposing_air.average_base_fill,
                    },
                ],
            },
            defines,
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::rows::{NavalCombatPlane, NavalCombatSummaryModifier};

    fn defines() -> Defines {
        Defines {
            summary_fixed: [0; 14],
            summary_integer: [0; 3],
            sortie_stance_ratios: vec![ONE],
            base_carrier_sortie_efficiency: ONE,
            selected_sortie_initial_time: 24,
        }
    }

    #[test]
    fn initial_defender_modifiers_end_at_duration_boundary_and_stored_value_is_not_displayed() {
        let mut definitions = defines();
        definitions.summary_fixed[0] = 40_000;
        definitions.summary_integer[1] = 12;
        let mut side = NavalCombatSideDetails {
            positioning: 91_000,
            ..Default::default()
        };
        side.summary.countries.push(42);
        side.summary.modifiers[0][0] = NavalCombatSummaryModifier {
            raw: 5_000,
            present: 1,
        };
        side.summary.modifiers[3][2] = NavalCombatSummaryModifier {
            raw: 7_000,
            present: 1,
        };
        side.summary.escort_bonus = 2_000;
        let opponent = NavalCombatSideDetails::default();
        let mut input = Inputs {
            side: &side,
            opponent: &opponent,
            defender: true,
            duration: 11,
            surface_detection: [0; 2],
            active_ship_slots: [0; 2],
            air: [Air::default(); 2],
        };
        assert_eq!(displayed(&input, &definitions), Some(54_000));
        input.duration = 12;
        assert_eq!(displayed(&input, &definitions), Some(49_000));
        input.defender = false;
        input.duration = 11;
        assert_eq!(displayed(&input, &definitions), Some(49_000));
    }

    #[test]
    fn initial_positioning_reads_the_opposing_admiral_and_ordinary_the_own() {
        let mut definitions = defines();
        definitions.summary_fixed[0] = 40_000;
        definitions.summary_integer[1] = 12;
        let mut side = NavalCombatSideDetails::default();
        side.summary.countries.push(42);
        let cell = |raw| NavalCombatSummaryModifier { raw, present: 1 };
        // Own admiral (scope 3) and opposing admiral (scope 4) carry
        // different bonuses for both keys.
        side.summary.modifiers[3][0] = cell(20_000);
        side.summary.modifiers[4][0] = cell(3_000);
        side.summary.modifiers[3][2] = cell(7_000);
        side.summary.modifiers[4][2] = cell(11_000);
        let opponent = NavalCombatSideDetails::default();
        let mut input = Inputs {
            side: &side,
            opponent: &opponent,
            defender: true,
            duration: 11,
            surface_detection: [0; 2],
            active_ship_slots: [0; 2],
            air: [Air::default(); 2],
        };
        // Base, own-admiral positioning, opposing-admiral initial positioning.
        assert_eq!(displayed(&input, &definitions), Some(50_000));
        input.duration = 12;
        assert_eq!(displayed(&input, &definitions), Some(47_000));
        input.duration = 11;
        input.defender = false;
        assert_eq!(displayed(&input, &definitions), Some(47_000));
    }

    #[test]
    fn empty_primary_country_list_skips_modifiers_and_escort() {
        let mut definitions = defines();
        definitions.summary_fixed[0] = 30_000;
        definitions.summary_integer[1] = 12;
        let mut side = NavalCombatSideDetails::default();
        side.summary.escort_bonus = 20_000;
        side.summary.modifiers[0][2] = NavalCombatSummaryModifier {
            raw: 10_000,
            present: 1,
        };
        let opponent = NavalCombatSideDetails::default();
        let input = Inputs {
            side: &side,
            opponent: &opponent,
            defender: true,
            duration: 0,
            surface_detection: [0; 2],
            active_ship_slots: [0; 2],
            air: [Air::default(); 2],
        };
        assert_eq!(displayed(&input, &definitions), Some(30_000));
    }

    #[test]
    fn modifiers_gate_feature_and_clamp_before_sum() {
        let mut side = NavalCombatSideDetails::default();
        let summary = &mut side.summary;
        summary.modifier_metadata[2] = [0x10, 4];
        summary.modifiers[0][2] = NavalCombatSummaryModifier {
            raw: 120_000,
            present: 1,
        };
        assert_eq!(modifier(summary, 0, 2), Some(0));
        summary.eligibility_callback = 1;
        summary.feature_flags = 4;
        assert_eq!(modifier(summary, 0, 2), Some(ONE));
        summary.eligibility_callback = 2;
        assert_eq!(modifier(summary, 0, 2), None);
    }

    #[test]
    fn ratio_penalties_respect_count_gate_opposing_admiral_fill_and_positive_only() {
        let mut definitions = defines();
        definitions.summary_fixed[0] = 90_000;
        definitions.summary_fixed[3] = 10_000;
        definitions.summary_fixed[4] = 30_000;
        definitions.summary_fixed[5] = 20_000;
        definitions.summary_fixed[6] = 30_000;
        definitions.summary_integer[0] = 4;
        let mut side = NavalCombatSideDetails::default();
        side.summary.modifiers[4][4] = NavalCombatSummaryModifier {
            raw: 50_000,
            present: 1,
        };
        let opponent = NavalCombatSideDetails {
            commander: 1,
            ..Default::default()
        };
        let mut input = Inputs {
            side: &side,
            opponent: &opponent,
            defender: false,
            duration: 0,
            surface_detection: [0; 2],
            active_ship_slots: [4, 2],
            air: [
                Air {
                    weighted_planes: 50_000,
                    average_base_fill: 0,
                },
                Air {
                    weighted_planes: 100_000,
                    average_base_fill: 50_000,
                },
            ],
        };
        // Ratio 2 gives 10% ship penalty, enemy admiral raises it to 15%;
        // carrier ratio 0.5 at half base fill contributes 5%.
        assert_eq!(displayed(&input, &definitions), Some(70_000));
        input.active_ship_slots = [3, 2];
        assert_eq!(displayed(&input, &definitions), Some(85_000));
        input.active_ship_slots = [4, 8];
        assert_eq!(displayed(&input, &definitions), Some(85_000));
    }

    #[test]
    fn surface_detection_adds_only_positive_capped_advantage() {
        let mut definitions = defines();
        definitions.summary_fixed[0] = 50_000;
        definitions.summary_fixed[1] = ONE;
        definitions.summary_fixed[2] = 2_000;
        let side = NavalCombatSideDetails::default();
        let opponent = NavalCombatSideDetails::default();
        let mut input = Inputs {
            side: &side,
            opponent: &opponent,
            defender: false,
            duration: 0,
            surface_detection: [120_000, 20_000],
            active_ship_slots: [0; 2],
            air: [Air::default(); 2],
        };
        assert_eq!(displayed(&input, &definitions), Some(51_000));
        input.surface_detection = [1_000_000, 0];
        assert_eq!(displayed(&input, &definitions), Some(52_000));
        input.surface_detection = [0, 1_000_000];
        assert_eq!(displayed(&input, &definitions), Some(50_000));
    }

    #[test]
    fn native_signed_ratio_guard_excludes_only_extreme_numerators() {
        assert!(!ratio_numerator_valid(i64::MIN));
        assert!(!ratio_numerator_valid(i64::MIN + 1));
        assert!(ratio_numerator_valid(i64::MIN + 2));
        assert!(ratio_numerator_valid(-1));
        assert!(ratio_numerator_valid(0));
        assert!(ratio_numerator_valid(i64::MAX - 1));
        assert!(!ratio_numerator_valid(i64::MAX));

        // No opposing forward slots means the ship-ratio denominator is
        // absent. The native branch skips the penalty even at its minimum
        // ship threshold.
        let mut definitions = defines();
        definitions.summary_integer[0] = 0;
        definitions.summary_fixed[3] = ONE;
        definitions.summary_fixed[4] = ONE;
        let side = NavalCombatSideDetails::default();
        let opponent = NavalCombatSideDetails::default();
        let input = Inputs {
            side: &side,
            opponent: &opponent,
            defender: false,
            duration: 0,
            surface_detection: [0; 2],
            active_ship_slots: [i32::MAX, 0],
            air: [Air::default(); 2],
        };
        assert_eq!(ship_ratio_penalty(&input, &definitions), Some(0));
    }

    #[test]
    fn ship_slots_include_unresolved_members_but_skip_retired_or_inactive() {
        let row = |slot, source, state| NavalCombatShipRow {
            side: 0,
            battle_slot: slot,
            member_source: source,
            group_index: 0,
            state,
            live_ship: 0,
            live_convoy: 0,
            ..Default::default()
        };
        let a = row(0, 0, 0);
        let b = row(1, 0, 0x34);
        let c = row(2, 1, 0);
        let d = row(3, 0, 0);
        let rows = [&a, &b, &c, &d];
        assert_eq!(active_ship_slots(&rows, 0), 2);
        assert_eq!(active_ship_slots(&rows, 1), 0);
    }

    #[test]
    fn missing_modifier_witness_and_recent_join_use_final_clamp() {
        let mut definitions = defines();
        definitions.summary_fixed[0] = 80_000;
        let mut side = NavalCombatSideDetails::default();
        side.summary.countries.push(42);
        side.summary.recent_join_penalty = 90_000;
        side.summary.modifiers[0][2] = NavalCombatSummaryModifier {
            raw: -10_000,
            present: 1,
        };
        let opponent = NavalCombatSideDetails::default();
        let input = Inputs {
            side: &side,
            opponent: &opponent,
            defender: false,
            duration: 0,
            surface_detection: [0; 2],
            active_ship_slots: [0; 2],
            air: [Air::default(); 2],
        };
        assert_eq!(displayed(&input, &definitions), Some(0));
        side.summary.recent_join_penalty = -90_000;
        let input = Inputs {
            side: &side,
            opponent: &opponent,
            defender: false,
            duration: 0,
            surface_detection: [0; 2],
            active_ship_slots: [0; 2],
            air: [Air::default(); 2],
        };
        assert_eq!(displayed(&input, &definitions), Some(70_000));
    }

    #[test]
    fn adapter_preserves_provable_zero_paths_without_dense_or_model_tables() {
        let mut definitions = defines();
        definitions.summary_fixed[0] = 40_000;
        let detail = NavalCombatDetailsRow {
            combat: 7,
            ..Default::default()
        };
        let content = naval_air_groups::Content::default();
        assert_eq!(
            positioning_values(
                &detail,
                Some(&[]),
                None,
                None,
                None,
                None,
                &content,
                Some(&definitions),
            ),
            [Some(40_000); 2]
        );
        assert_eq!(
            positioning_values(
                &detail,
                None,
                None,
                None,
                None,
                None,
                &content,
                Some(&definitions),
            ),
            [None; 2]
        );
        assert_eq!(
            positioning_values(&detail, Some(&[]), None, None, None, None, &content, None,),
            [None; 2]
        );
    }

    #[test]
    fn adapter_requires_dense_witness_only_for_a_side_that_uses_its_air() {
        let mut definitions = defines();
        definitions.summary_fixed[0] = 40_000;
        let mut detail = NavalCombatDetailsRow {
            combat: 7,
            ..Default::default()
        };
        detail.sides[0].planes.push(NavalCombatPlane {
            source: 0,
            group_index: 0,
            air_base: 9,
            current_planes: 10,
            efficiency: replay::rows::NavalCombatAirEfficiency {
                source: 2,
                ..Default::default()
            },
            ..Default::default()
        });
        let content = naval_air_groups::Content::default();
        assert_eq!(
            positioning_values(
                &detail,
                Some(&[]),
                None,
                None,
                None,
                None,
                &content,
                Some(&definitions),
            ),
            [Some(40_000), None]
        );
        let dense_rows = [replay::rows::NavalCombatAirBaseWingHandlesRow {
            base: 9,
            wings: Vec::new(),
        }];
        assert_eq!(
            positioning_values(
                &detail,
                Some(&[]),
                None,
                None,
                None,
                DenseBases::new(&dense_rows, &[]),
                &content,
                Some(&definitions),
            ),
            [Some(40_000); 2]
        );
    }

    /// The owner's design scale doubled: 40,000 ships (with their live rows,
    /// models and carrier air bases) and 100,000 wings of 10 planes (1,000,000
    /// planes) in the bases' dense rows.
    const SHIPS: u32 = 40_000;
    const WINGS: u32 = 100_000;

    struct World {
        members: Vec<NavalCombatShipRow>,
        bases: Vec<AirBaseRow>,
        live_ships: Vec<ShipRow>,
        models: Vec<ShipModelRow>,
        dense: Vec<replay::rows::NavalCombatAirBaseWingHandlesRow>,
        wings: Vec<replay::rows::AirWingRow>,
        detail: NavalCombatDetailsRow,
        definitions: Defines,
    }

    /// Combat 7 holds `carriers` carriers, alternately attacker and defender,
    /// each with one admitted carrier-air slot linked to its combat member and
    /// flying from its own base, which holds one 10-plane wing. Combat 8 holds
    /// the same ships again and combat 9 every other ship; the other bases
    /// share the remaining wings.
    fn world(carriers: u32) -> World {
        use replay::rows::{
            AirWingRow, NavalCombatAirBaseWingHandlesRow, NavalCombatAirEfficiency,
            NavalShipModifierSource, ShipCounterInputs,
        };
        let ship_id = |ship: u32| 1_000 + u64::from(ship);
        let base_id = |ship: u32| 100_000 + u64::from(ship);
        let wing_id = |wing: u32| 1_000_000 + u64::from(wing);
        let side_of = |ship: u32| (ship % 2) as u8;
        let mut members = Vec::new();
        for (combat, ships) in [(7, 0..carriers), (8, 0..carriers), (9, carriers..SHIPS)] {
            for side in [0, 1] {
                for ship in ships.clone().filter(|&ship| side_of(ship) == side) {
                    let mut row = NavalCombatShipRow {
                        combat,
                        side,
                        battle_slot: ship / 2,
                        ship: ship_id(ship),
                        live_ship: 1,
                        state: 2,
                        ..Default::default()
                    };
                    row.weapon_context.sources.push(NavalShipModifierSource {
                        scope: 26,
                        ..Default::default()
                    });
                    members.push(row);
                }
            }
        }
        let bases = (0..SHIPS)
            .map(|ship| AirBaseRow {
                id: base_id(ship),
                host: ship_id(ship),
                ..Default::default()
            })
            .collect();
        let live_ships = (0..SHIPS)
            .map(|ship| ShipRow {
                id: ship_id(ship),
                organisation: 500_000,
                ..Default::default()
            })
            .collect();
        let models = (0..SHIPS)
            .map(|ship| ShipModelRow {
                ship: ship_id(ship),
                counter: Some(ShipCounterInputs {
                    maximum_organisation: 1_000_000,
                    ..Default::default()
                }),
                ..Default::default()
            })
            .collect();
        let mut dense: Vec<_> = (0..SHIPS)
            .map(|ship| NavalCombatAirBaseWingHandlesRow {
                base: base_id(ship),
                wings: Vec::new(),
            })
            .collect();
        for wing in 0..WINGS {
            let base = if wing < carriers {
                wing
            } else {
                carriers + (wing - carriers) % (SHIPS - carriers)
            };
            dense[base as usize].wings.push(wing_id(wing));
        }
        let wings = (0..WINGS)
            .map(|wing| AirWingRow {
                id: wing_id(wing),
                planes: 10,
                ..Default::default()
            })
            .collect();
        let mut detail = NavalCombatDetailsRow {
            combat: 7,
            duration: 24,
            ..Default::default()
        };
        for ship in 0..carriers {
            let side = side_of(ship);
            detail.sides[usize::from(side)]
                .planes
                .push(NavalCombatPlane {
                    group_index: 0,
                    air_base: base_id(ship),
                    // The defenders fly twice the attackers' planes.
                    current_planes: 10 * (1 + i32::from(side)),
                    carrier_slot: (ship / 2) as i32,
                    carrier_member_present: 1,
                    efficiency: NavalCombatAirEfficiency {
                        source: 3,
                        container_present: 1,
                        base_crowding: ONE,
                        ..Default::default()
                    },
                    ..Default::default()
                });
        }
        let mut definitions = defines();
        definitions.summary_fixed[0] = 40_000;
        definitions.summary_fixed[5] = ONE;
        definitions.summary_fixed[6] = ONE;
        definitions.summary_fixed[12] = 20 * ONE;
        World {
            members,
            bases,
            live_ships,
            models,
            dense,
            wings,
            detail,
            definitions,
        }
    }

    /// Every carrier-air slot joins its air base, host ship, ship model and
    /// member by lookup (the old per-slot scans made about 62.5 million row
    /// visits at 5,000 carriers); ShipModels is checked once and shared by
    /// surface detection, carrier air, screening and anti-air; and only the
    /// selected combat's dense base rows are summed. The timed span is the
    /// window's per-refresh naval work in `details::naval`.
    #[test]
    fn carrier_air_joins_are_indexed_at_country_scale() {
        use super::super::{naval_anti_air, naval_screening};
        for carriers in [1_500_u32, 5_000] {
            let w = world(carriers);
            let content = naval_air_groups::Content::default();
            let refresh = |models: &[ShipModelRow], wings: &[replay::rows::AirWingRow]| {
                let models = Keyed::new(models, |model| model.ship);
                let values = positioning_values(
                    &w.detail,
                    Some(&w.members),
                    models,
                    Some(&w.bases),
                    Some(&w.live_ships),
                    DenseBases::new(&w.dense, wings),
                    &content,
                    Some(&w.definitions),
                );
                let models = models.expect("ordered ship models");
                let others = std::array::from_fn::<_, 2, _>(|side| {
                    let screening = naval_screening::calculate(
                        7,
                        side as u8,
                        &w.detail.sides[side],
                        &w.members,
                        &models,
                        &w.definitions,
                    );
                    let anti_air = values[side].map(|positioning| {
                        naval_anti_air::calculate(
                            7,
                            side as u8,
                            positioning,
                            &w.members,
                            w.definitions.summary_fixed[13],
                            |ship| {
                                models
                                    .get(ship)
                                    .and_then(|row| row.counter.as_ref())
                                    .map(|counter| counter.anti_air_base)
                            },
                        )
                    });
                    (screening.is_ok(), anti_air.is_some_and(|aa| aa.is_ok()))
                });
                (values, others)
            };
            let started = std::time::Instant::now();
            let (values, others) = refresh(&w.models, &w.wings);
            let elapsed = started.elapsed();
            // Every slot sorties at half organisation (0.5) from a half-full
            // base (0.5), so each side's average base fill is 0.25. The
            // attackers' weighted planes are half the defenders': penalty
            // (1 - 0.5) x 1 x 0.25.
            assert_eq!(values, [Some(27_500), Some(40_000)]);
            assert_eq!(others, [(true, true); 2]);
            eprintln!(
                "naval window refresh, {carriers} carriers in combat, {SHIPS} ships, {WINGS} wings: {elapsed:?} (test build)"
            );

            // A wing missing from the roster in another combat's base row
            // leaves this combat's counts alone.
            let mut roster = w.wings.clone();
            let outside = roster.pop().expect("wings");
            assert!(
                w.dense[carriers as usize..]
                    .iter()
                    .any(|row| row.wings.contains(&outside.id))
            );
            assert_eq!(refresh(&w.models, &roster).0, values);
            // One defender model without counter operands is refused, not
            // guessed: the defenders' air pass and surface detection need
            // it, and the attackers' positioning reads the defenders' air.
            let mut models = w.models.clone();
            models[1].counter = None;
            assert_eq!(refresh(&models, &w.wings).0, [None; 2]);
        }
    }
}
