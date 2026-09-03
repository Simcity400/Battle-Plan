//! The EIR-CW-XMAP14 producer: this hour's combat-indicator census, from the
//! domains the recording actually carries.
//!
//! # What it mints, and from what
//!
//! * **Land** — D21 `land_combats_live` gives identity, province and the two
//!   participant division-id lists, and THIS HOUR's D20 division roster
//!   completes each id into the `(type_id, stable_id)` pair XMAP13 is keyed by.
//!   The combat's progress is NOT taken from D55: see the fold's call site.
//! * **Naval** — D49/D50 `naval_combat_*_live` give identity, province and the
//!   two task-force reference lists.
//! * **Retained naval results** — D103 `naval_combat_results_live` gives the
//!   record census a729 holds on the gamestate's global retained list (lane
//!   XA). Each record is one row, dated by the sequence the fold first saw it
//!   under, and short of exactly the icon's world point, which XMAP14-PLACE-1
//!   makes the viewer's to derive. On an hour D103 did not state, the class is
//!   `Unobtainable` and says so. It is NOT reported as a measured-empty class:
//!   "no domain read it this hour" and "there were none" are different facts
//!   and only one of them is true.
//! * **The TAG map** — D18's roster, only when it landed this hour, because
//!   XMAP14-ID-1 (lane XA) established that `tag_db_id` IS the engine country
//!   id the retained-result predicates compare.
//!
//! # What it refuses to invent
//!
//! Every operand a729 reads that no domain carries — the icon world points, the
//! `CCombatSide` flag words, the frame selector, the comparison scalars, the
//! ship-category counts — is NAMED per row rather than defaulted. A row short of
//! any of them travels in `unplaced`, inside the census and inside its digest,
//! so the count of battles this hour stays right while the placement stays
//! honest. Which operands those are is the wire contract's business, not this
//! module's: [`portable_render_stream::xmap14_combat_indicators::MissingOperand`]
//! holds the reasons and the gates, and this module only decides which of them
//! apply to a row.

use portable_render_stream::xmap14_combat_indicators as wire;
use serde_json::Value;

/// One D21 active land combat, already joined to D20 and D55 by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LandCombatInput {
    pub combat_id: u32,
    pub generation: u64,
    /// `None` when D21's row carried no location flag.
    pub location_province_id: Option<u32>,
    /// `None` when D21's row carried no exact attacker list. A `None` ELEMENT is
    /// a division id the hour's D20 roster does not resolve to a type.
    pub attackers: Option<Vec<Option<wire::UnitIdentity>>>,
    pub defenders: Option<Vec<Option<wire::UnitIdentity>>>,
    /// The combat's progress in the engine's 1e5 fixed point, or `None` when
    /// the recording does not carry it.
    ///
    /// It is `None` on every hour today: see the fold's call site for why D55's
    /// per-side scalar is not promoted into it.
    pub progress_fixed5: Option<i64>,
}

/// One D49 active naval combat, already joined to D50 by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalCombatInput {
    pub combat_id: u32,
    pub generation: u64,
    pub location_province_id: Option<u32>,
    /// `None` when D50's member census was not known this hour, so the two
    /// participant lists are unstated rather than empty.
    pub attackers: Option<Vec<wire::UnitIdentity>>,
    pub defenders: Option<Vec<wire::UnitIdentity>>,
    /// The `tag_db_id`s D50 recorded for the members of each side.
    ///
    /// `None` for a side whose country set the recording cannot state: D50 was
    /// not known this hour, or it recorded no member at all on that side, or a
    /// member with no `owner_tag_db_id`. This is NOT derivable from
    /// `attackers`/`defenders`: a convoy member carries no task-force id, so a
    /// country can fight on a side and own none of its task forces.
    ///
    /// Ids, not tags. The recorder could resolve them through its own
    /// `tag_by_db_id`, but nothing on the replay side could check that pairing,
    /// and it would be the pairing that decides which side a viewer is on. The
    /// replay resolves its own observer through the frame's `_r2_tag_db_id`
    /// witness instead, and then both halves of the comparison are typed.
    pub attacker_owner_country_ids: Option<Vec<u32>>,
    pub defender_owner_country_ids: Option<Vec<u32>>,
    /// XMAP14-POV-2 from domain 103: every province this combat's icon occurs
    /// in (its own plus each transfer unit's), or `None` when D103 did not
    /// state this combat this hour.
    pub occurrence_provinces: Option<Vec<u32>>,
}

/// One D103 retained naval-result record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavalResultRecordInput {
    /// The record's own reference id (`record+0x08`).
    pub record_id: u32,
    pub province_id: u32,
    /// The compact record sequence the fold FIRST saw this record under. Zero
    /// is not a sequence, and a row carrying it costs the hour its census.
    pub create_sequence: u64,
}

/// Everything the producer is given for one hour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProducerInputs {
    pub hour_index: i64,
    /// The compact record sequence the combat census was last advanced by.
    /// Zero means no combat-domain record has been applied at all, which is not
    /// a census.
    pub capture_sequence: u64,
    /// `None` when D21 did not state an active census this hour.
    pub land: Option<Vec<LandCombatInput>>,
    /// `None` when D49 did not state an active census this hour.
    pub naval: Option<Vec<NavalCombatInput>>,
    /// `None` when D103 did not state the retained naval-result census this
    /// hour.
    pub naval_results: Option<Vec<NavalResultRecordInput>>,
    /// D18's roster as `(TAG, tag_db_id)`, ascending by tag; empty when D18 did
    /// not land this hour.
    pub country_id_by_tag: Vec<(String, i64)>,
}

/// The value that goes under
/// [`wire::XMAP14_COMBAT_INDICATOR_WIRE_KEY`] this hour.
///
/// Never `Err`: a producer that cannot state a census states WHY, in the same
/// key, so an hour is never silently missing the family.
pub fn produce(inputs: &ProducerInputs) -> Value {
    match build(inputs) {
        Ok(envelope) => serde_json::to_value(envelope).unwrap_or_else(|error| {
            wire::gap_document(
                inputs.hour_index,
                &format!("the xmap14 combat-indicator census did not serialise: {error}"),
            )
        }),
        // The gap is DATED and sealed like the census is, so an hour cannot be
        // answered by another hour's reason.
        Err(reason) => wire::gap_document(inputs.hour_index, &reason),
    }
}

/// The build authority is NOT re-checked per hour, and that is deliberate.
///
/// `game_build::supported_game_builds` holds exactly one entry and the recorder
/// terminates on any other build, so a recording that exists at all is from
/// a729 and a runtime comparison here could not fail. What DOES protect the
/// claim is
/// `the_only_recorder_build_is_the_build_the_xmap14_laws_were_read_from`, which
/// fails the moment a second build is supported or the pin moves — at which
/// point this producer has to be looked at again rather than quietly emitting
/// a729's laws over other bytes.
fn build(inputs: &ProducerInputs) -> Result<wire::CombatEnvelope, String> {
    if inputs.capture_sequence == 0 {
        return Err(
            "no land- or naval-combat record has been applied in this recording, so this hour \
             has no combat census to state"
                .into(),
        );
    }

    let mut unplaced = Vec::new();
    for (combat_id, generation) in inputs
        .land
        .iter()
        .flatten()
        .map(|row| (row.combat_id, row.generation))
        .chain(
            inputs
                .naval
                .iter()
                .flatten()
                .map(|row| (row.combat_id, row.generation)),
        )
    {
        // The wire spells a generation signed and the fold holds it unsigned.
        // One past i64::MAX cannot be carried, and it cannot be DROPPED either:
        // the census claims to cover the whole active set, so a silently
        // shorter list would be a false claim rather than a partial one. The
        // hour states no census and says which combat cost it.
        if i64::try_from(generation).is_err() {
            return Err(format!(
                "d21/d49 list combat {combat_id} at generation {generation}, which the carrier's \
                 signed generation cannot spell; this hour states no combat census rather than \
                 one that has quietly lost a battle"
            ));
        }
    }

    let land_census = match &inputs.land {
        None => wire::ClassCensus::Unobtainable {
            missing: wire::MissingOperand::LandActiveCensus,
        },
        Some(rows) => {
            for row in rows {
                unplaced.push(land_row(row));
            }
            wire::ClassCensus::Complete
        }
    };

    let mut naval_occurrence_sets = Vec::new();
    let naval_census = match &inputs.naval {
        None => wire::ClassCensus::Unobtainable {
            missing: wire::MissingOperand::NavalActiveCensus,
        },
        Some(rows) => {
            for row in rows {
                unplaced.push(naval_row(row));
                if let Some(provinces) = &row.occurrence_provinces {
                    // The set as the fold measured it, in the contract's order;
                    // the combat's own province is a member by construction.
                    let mut province_ids: Vec<u32> = provinces
                        .iter()
                        .copied()
                        .chain(row.location_province_id)
                        .filter(|province| *province != 0)
                        .collect();
                    province_ids.sort_unstable();
                    province_ids.dedup();
                    naval_occurrence_sets.push(wire::NavalOccurrenceSet {
                        identity: wire::IndicatorIdentity::Naval {
                            combat_id: i64::from(row.combat_id),
                            generation: generation_i64(row.generation),
                        },
                        province_ids,
                    });
                }
            }
            naval_occurrence_sets.sort_by_key(|set| wire::NavalOccurrenceSet::naval_key(&set.identity));
            wire::ClassCensus::Complete
        }
    };

    let naval_result_census = match &inputs.naval_results {
        None => wire::ClassCensus::Unobtainable {
            missing: wire::MissingOperand::NavalResultRecordFamily,
        },
        Some(rows) => {
            for row in rows {
                unplaced.push(naval_result_row(row)?);
            }
            wire::ClassCensus::Complete
        }
    };

    wire::CombatEnvelope {
        schema: wire::XMAP14_COMBAT_INDICATOR_SCHEMA.into(),
        hour_index: inputs.hour_index,
        cadence: wire::XMAP14_COMBAT_INDICATOR_CADENCE.into(),
        identity_order: wire::XMAP14_COMBAT_INDICATOR_IDENTITY_ORDER.into(),
        capture_sequence: inputs.capture_sequence,
        producer_executable_sha256: wire::XMAP14_ENGINE_SHA256.into(),
        producer_evidence_manifest_sha256: wire::producer_evidence_manifest_sha256(),
        land_census,
        naval_census,
        naval_result_census,
        // FROM D18, under XMAP14-ID-1: `tag_db_id` is `CCountry+0x08`, the
        // CountryTag link index the retained-result predicates compare through
        // `gamestate+0x340`. The fold hands it over only on an hour D18 landed.
        country_id_by_tag: inputs.country_id_by_tag.clone(),
        ordered_identity_sha256: String::new(),
        census_sha256: String::new(),
        // Empty because nothing this hour carried a full operand set, not
        // because the classes are empty: every row the recording saw is in
        // `unplaced` with its own reasons.
        land: Vec::new(),
        naval: Vec::new(),
        naval_results: Vec::new(),
        naval_occurrence_sets,
        unplaced,
        deterministic_sha256: String::new(),
    }
    .seal()
}

/// The operands ONE land combat is short of.
///
/// Four are unconditional today and say so once, here, rather than in four
/// places: no domain in the recorder reads the land icon's world points, the
/// two `CCombatSide` flag words, the frame selector or the comparison scalars.
/// The rest are per-row facts about THIS combat's D21 row and this hour's D20
/// roster.
fn land_row(row: &LandCombatInput) -> wire::UnplacedIndicator {
    let mut missing = vec![
        wire::MissingOperand::LandIconAnchorPoints,
        wire::MissingOperand::LandSideFlags,
        wire::MissingOperand::LandFrameSelector,
        wire::MissingOperand::LandComparisonScalars,
    ];
    if row.progress_fixed5.is_none() {
        missing.push(wire::MissingOperand::LandProgress);
    }
    // THREE DIFFERENT FACTS, THREE DIFFERENT OPERANDS. An earlier draft
    // collapsed all of them into `LandParticipantTypeIds`, which told a reader
    // to go and fix the D20 join on an hour where D21 had simply not stated the
    // side.
    for side in [&row.attackers, &row.defenders] {
        match side {
            // D21 did not state this side at all.
            None => missing.push(wire::MissingOperand::LandParticipantMembership),
            // D21 stated it as EXACTLY EMPTY. a729 creates the land icon only
            // when both of the combat's side objects carry a non-zero count, so
            // this may be a combat it draws no icon for at all — but that D21's
            // participant list IS that count is unestablished, so the icon's
            // EXISTENCE is undecided rather than decided against, and the row
            // says that instead of being dropped from the census or invented
            // into it.
            Some(list) if list.is_empty() => {
                missing.push(wire::MissingOperand::LandIconExistencePredicate)
            }
            // A stated side whose ids the D20 roster could not type.
            Some(list) if list.iter().any(Option::is_none) => {
                missing.push(wire::MissingOperand::LandParticipantTypeIds)
            }
            Some(_) => {}
        }
    }
    if !row.location_province_id.is_some_and(|province| province != 0) {
        missing.push(wire::MissingOperand::LandLocationProvince);
    }
    missing.sort_unstable();
    missing.dedup();
    wire::UnplacedIndicator {
        identity: wire::IndicatorIdentity::Land {
            combat_id: i64::from(row.combat_id),
            generation: generation_i64(row.generation),
        },
        location_province_id: row.location_province_id.unwrap_or(0),
        missing_operands: missing,
    }
}

fn naval_row(row: &NavalCombatInput) -> wire::UnplacedIndicator {
    let mut missing = vec![
        wire::MissingOperand::NavalIconAnchorPoint,
        wire::MissingOperand::NavalCategoryCounts,
        // D49 carries the two POSITIONING scalars, which is a different
        // quantity from the progress the indicator's bar reads. Naming
        // positioning as progress would be a substitution, so progress is
        // unrecorded.
        wire::MissingOperand::NavalProgress,
    ];
    // ONE D49 COMBAT IS NOT ONE ICON. a729's naval producer also creates a
    // spill-over occurrence in a province other than the combat's own when
    // (XMAP14-POV-2). Domain 103 records the predicate's operands; when it
    // stated this combat this hour the envelope carries the set, and otherwise
    // the row says so rather than the class census implying one icon per combat.
    if row.occurrence_provinces.is_none() {
        missing.push(wire::MissingOperand::NavalIconOccurrenceSet);
    }
    if row.attackers.is_none() || row.defenders.is_none() {
        missing.push(wire::MissingOperand::NavalParticipantReferences);
    }
    // WHICH COUNTRIES ARE ON EACH SIDE is a separate input from which task
    // forces are, and it is the one the viewer is attributed with. A side whose
    // owner set the recording cannot state leaves the row unattributable, so it
    // stays unplaced and says which of the two facts is missing.
    if row.attacker_owner_country_ids.is_none() || row.defender_owner_country_ids.is_none() {
        missing.push(wire::MissingOperand::NavalSideOwnerCountries);
    }
    if !row.location_province_id.is_some_and(|province| province != 0) {
        missing.push(wire::MissingOperand::NavalLocationProvince);
    }
    missing.sort_unstable();
    missing.dedup();
    wire::UnplacedIndicator {
        identity: wire::IndicatorIdentity::Naval {
            combat_id: i64::from(row.combat_id),
            generation: generation_i64(row.generation),
        },
        location_province_id: row.location_province_id.unwrap_or(0),
        missing_operands: missing,
    }
}

/// One retained naval-result record as an unplaced row.
///
/// The record's own operands - the side lists, the flags, the tallies - are on
/// the `naval_combat_results` carrier keyed by this id; what the icon is short
/// of here is its world point, which XMAP14-PLACE-1 makes the viewer's to
/// derive and which it does not derive yet. The identity carries no
/// generation: a729 mints a record once and never re-numbers it.
fn naval_result_row(row: &NavalResultRecordInput) -> Result<wire::UnplacedIndicator, String> {
    if row.create_sequence == 0 {
        return Err(format!(
            "d103 lists retained naval result {} with no create sequence; this hour states no \
             combat census rather than one whose identity cannot be dated",
            row.record_id
        ));
    }
    Ok(wire::UnplacedIndicator {
        identity: wire::IndicatorIdentity::NavalResult {
            result_id: i64::from(row.record_id),
            generation: 0,
            create_sequence: row.create_sequence,
        },
        location_province_id: row.province_id,
        missing_operands: vec![wire::MissingOperand::NavalResultAnchorPoint],
    })
}

/// The half-open key range holding exactly the `"{div_id}:*"` entries of a map
/// keyed `"{div_id}:{div_type}"`.
///
/// `':'` is 0x3a and the digits are 0x30..0x39, so `"7:"` sorts ABOVE every
/// `"70".."79"` key and `"7;"` still sorts below `"8"`. That is what makes a
/// prefix range safe here and it is not obvious, which is why it is one named
/// function with its own test rather than two `format!`s at the call site.
pub fn division_key_range(div_id: u32) -> (String, String) {
    (format!("{div_id}:"), format!("{div_id};"))
}

/// The wire spells a generation signed and the fold holds it unsigned.
///
/// `build` has already refused the whole hour for any generation past
/// `i64::MAX`, so this cannot lose one here; the fallback exists so that a
/// future caller that skips that check gets an identity the consumer REFUSES
/// rather than one that collides with another generation.
fn generation_i64(generation: u64) -> i64 {
    i64::try_from(generation).unwrap_or(-1)
}

#[cfg(test)]
// Same reason as `portable_render_stream::xmap14_combat_indicators`: a
// `#[test]` that does not register is a guard that runs never, and the compiler
// knows both accidental shapes.
#[deny(duplicate_macro_attributes, unnameable_test_items)]
mod tests {
    use super::*;

    fn division(stable_id: u32) -> Option<wire::UnitIdentity> {
        Some(wire::UnitIdentity::Division {
            type_id: 51,
            stable_id,
        })
    }

    fn inputs() -> ProducerInputs {
        ProducerInputs {
            hour_index: 17_031_767,
            capture_sequence: 42,
            land: Some(vec![LandCombatInput {
                combat_id: 7,
                generation: 3,
                location_province_id: Some(910),
                attackers: Some(vec![division(11)]),
                defenders: Some(vec![division(12)]),
                progress_fixed5: None,
            }]),
            naval: Some(vec![NavalCombatInput {
                combat_id: 8,
                generation: 1,
                location_province_id: Some(920),
                attackers: Some(vec![wire::UnitIdentity::TaskForce {
                    type_id: 61,
                    stable_id: 490,
                }]),
                // A CONVOY-ONLY DEFENDER: no task force, and still a country
                // fighting on that side. It is the shape that makes the owner
                // set and the task-force set two different things.
                defenders: Some(Vec::new()),
                attacker_owner_country_ids: Some(vec![7]),
                defender_owner_country_ids: Some(vec![11]),
                occurrence_provinces: None,
            }]),
            naval_results: Some(vec![NavalResultRecordInput {
                record_id: 9001,
                province_id: 4001,
                create_sequence: 40,
            }]),
            country_id_by_tag: vec![("ENG".into(), 11), ("GER".into(), 7)],
        }
    }

    /// An hour where every CONDITIONAL operand fires: no province on either
    /// row, one land side unstated and the other stated empty, an untyped
    /// participant, no naval participants, no owner countries and no occurrence
    /// set. Two land rows because a side cannot be unstated, empty and untyped
    /// at once.
    fn starved_inputs() -> ProducerInputs {
        ProducerInputs {
            hour_index: 17_031_768,
            capture_sequence: 43,
            land: Some(vec![
                LandCombatInput {
                    combat_id: 70,
                    generation: 1,
                    location_province_id: None,
                    attackers: None,
                    defenders: Some(Vec::new()),
                    progress_fixed5: None,
                },
                LandCombatInput {
                    combat_id: 71,
                    generation: 1,
                    location_province_id: Some(911),
                    attackers: Some(vec![None]),
                    defenders: Some(vec![division(12)]),
                    progress_fixed5: Some(42_000),
                },
            ]),
            naval: Some(vec![NavalCombatInput {
                combat_id: 80,
                generation: 1,
                location_province_id: None,
                attackers: None,
                defenders: None,
                attacker_owner_country_ids: None,
                defender_owner_country_ids: None,
                occurrence_provinces: None,
            }]),
            naval_results: Some(vec![NavalResultRecordInput {
                record_id: 9002,
                province_id: 4002,
                create_sequence: 41,
            }]),
            country_id_by_tag: Vec::new(),
        }
    }

    fn envelope(inputs: &ProducerInputs) -> wire::CombatEnvelope {
        serde_json::from_value(produce(inputs)).expect("the producer stated a census")
    }

    #[test]
    fn the_census_holds_every_combat_the_recording_saw_and_places_none_of_them() {
        let sealed = envelope(&inputs());
        assert!(sealed.land.is_empty() && sealed.naval.is_empty());
        assert!(sealed.naval_results.is_empty());
        assert_eq!(sealed.unplaced.len(), 3);
        assert!(sealed.land_census.is_complete());
        assert!(sealed.naval_census.is_complete());
        assert!(sealed.naval_result_census.is_complete());
        assert_eq!(sealed.hour_index, 17_031_767);
        assert_eq!(sealed.capture_sequence, 42);
        sealed.validate_body().expect("the census is lawful");
    }

    /// kills: this producer's operand vocabulary drifting off the contract
    /// that says who owes each leaf, in either direction, on every class and
    /// every branch that names an operand.
    ///
    /// The rule: an operand must name a leaf the PRODUCER could owe - a
    /// contract gate, or a recorded leaf this hour did not carry - never one
    /// XMAP14-PLACE-1 gives the viewer. The two that do today are declared with
    /// reasons, as is the one class whose gated leaves no operand names.
    #[test]
    fn every_operand_this_producer_names_answers_a_leaf_the_contract_owes_it() {
        use portable_render_stream::{
            validate_xmap14_producer_contract, xmap14_classify_envelope_leaf,
            xmap14_classify_land_leaf, xmap14_classify_naval_leaf,
            xmap14_classify_naval_result_leaf, CarrierGateOwner, CarrierLeafSource,
            XMAP14_LAND_ANCHOR_LEAVES, XMAP14_LAND_ROW_CONTRACT, XMAP14_NAVAL_RESULT_ROW_CONTRACT,
            XMAP14_NAVAL_ROW_CONTRACT,
        };
        use std::collections::BTreeSet;

        validate_xmap14_producer_contract().expect("the shared producer contract is well formed");

        // Every operand, with the leaf it names and the table that classifies
        // that leaf. `envelope` is a real answer: an operand can name a leaf of
        // the envelope rather than of the row.
        let mapping: &[(wire::MissingOperand, &str, &str)] = &[
            (wire::MissingOperand::LandSideFlags, "land", "attacker_side_flags"),
            (wire::MissingOperand::LandSideFlags, "land", "defender_side_flags"),
            (wire::MissingOperand::LandSideFlags, "land", "side_flags_sha256"),
            (wire::MissingOperand::LandFrameSelector, "land", "frame_selector"),
            (
                wire::MissingOperand::LandComparisonScalars,
                "land",
                "attacker_comparison_fixed",
            ),
            (
                wire::MissingOperand::LandComparisonScalars,
                "land",
                "defender_comparison_fixed",
            ),
            (wire::MissingOperand::LandProgress, "land", "progress_fixed5"),
            (
                wire::MissingOperand::LandIconAnchorPoints,
                "land",
                "first_live_attacker_stack_point_f32_bits",
            ),
            (wire::MissingOperand::LandParticipantMembership, "land", "attackers"),
            (wire::MissingOperand::LandParticipantMembership, "land", "defenders"),
            (wire::MissingOperand::LandIconExistencePredicate, "land", "attackers"),
            (wire::MissingOperand::LandParticipantTypeIds, "land", "attackers"),
            (wire::MissingOperand::LandLocationProvince, "land", "location_province_id"),
            (wire::MissingOperand::NavalProgress, "naval", "progress_fixed5"),
            (
                wire::MissingOperand::NavalCategoryCounts,
                "naval",
                "attacker_category_counts",
            ),
            (
                wire::MissingOperand::NavalCategoryCounts,
                "naval",
                "defender_category_counts",
            ),
            (
                wire::MissingOperand::NavalCategoryCounts,
                "naval",
                "category_counts_sha256",
            ),
            (
                wire::MissingOperand::NavalParticipantReferences,
                "naval",
                "attackers",
            ),
            (
                wire::MissingOperand::NavalParticipantReferences,
                "naval",
                "defenders",
            ),
            (
                wire::MissingOperand::NavalSideOwnerCountries,
                "naval",
                "attacker_owner_country_ids",
            ),
            (
                wire::MissingOperand::NavalSideOwnerCountries,
                "naval",
                "defender_owner_country_ids",
            ),
            (
                wire::MissingOperand::NavalLocationProvince,
                "naval",
                "location_province_id",
            ),
            (
                wire::MissingOperand::NavalIconOccurrenceSet,
                "envelope",
                "naval_occurrence_sets",
            ),
        ];

        // Operands that name a leaf the VIEWER owns, so no producer owes them.
        let unowed: &[(wire::MissingOperand, &str, &str)] = &[
            (
                wire::MissingOperand::NavalIconAnchorPoint,
                "naval",
                "anchor_point_f32_bits",
            ),
            (
                wire::MissingOperand::NavalResultAnchorPoint,
                "naval_result",
                "anchor_point_f32_bits",
            ),
        ];

        let classify = |table: &str, leaf: &str| -> Option<&'static CarrierLeafSource> {
            match table {
                "land" => xmap14_classify_land_leaf(leaf),
                "naval" => xmap14_classify_naval_leaf(leaf),
                "naval_result" => xmap14_classify_naval_result_leaf(leaf),
                "envelope" => xmap14_classify_envelope_leaf(leaf),
                other => panic!("no table named {other}"),
            }
        };

        // Every operand the producer can name, over every branch: the fully
        // joined hour and the hour where every conditional fires.
        // PER ROW, against the input that produced it: an operand is right
        // only if that row's own input state warrants it. A union would let a
        // side-specific operand be answered by the other side's state.
        for inputs in [inputs(), starved_inputs()] {
            let sealed = envelope(&inputs);
            let named_on = |identity: wire::IndicatorIdentity| -> BTreeSet<wire::MissingOperand> {
                sealed
                    .unplaced
                    .iter()
                    .find(|row| row.identity == identity)
                    .unwrap_or_else(|| panic!("{identity:?} is in the census"))
                    .missing_operands
                    .iter()
                    .copied()
                    .collect()
            };
            for input in inputs.land.iter().flatten() {
                let mut expected: BTreeSet<wire::MissingOperand> = [
                    wire::MissingOperand::LandIconAnchorPoints,
                    wire::MissingOperand::LandSideFlags,
                    wire::MissingOperand::LandFrameSelector,
                    wire::MissingOperand::LandComparisonScalars,
                ]
                .into_iter()
                .collect();
                if input.progress_fixed5.is_none() {
                    expected.insert(wire::MissingOperand::LandProgress);
                }
                for side in [&input.attackers, &input.defenders] {
                    match side {
                        None => {
                            expected.insert(wire::MissingOperand::LandParticipantMembership);
                        }
                        Some(list) if list.is_empty() => {
                            expected.insert(wire::MissingOperand::LandIconExistencePredicate);
                        }
                        Some(list) if list.iter().any(Option::is_none) => {
                            expected.insert(wire::MissingOperand::LandParticipantTypeIds);
                        }
                        Some(_) => {}
                    }
                }
                if !input.location_province_id.is_some_and(|province| province != 0) {
                    expected.insert(wire::MissingOperand::LandLocationProvince);
                }
                assert_eq!(
                    named_on(wire::IndicatorIdentity::Land {
                        combat_id: i64::from(input.combat_id),
                        generation: input.generation as i64,
                    }),
                    expected,
                    "land combat {} names exactly the operands its own input warrants",
                    input.combat_id
                );
            }
            for input in inputs.naval.iter().flatten() {
                let mut expected: BTreeSet<wire::MissingOperand> = [
                    wire::MissingOperand::NavalIconAnchorPoint,
                    wire::MissingOperand::NavalCategoryCounts,
                    wire::MissingOperand::NavalProgress,
                ]
                .into_iter()
                .collect();
                if input.occurrence_provinces.is_none() {
                    expected.insert(wire::MissingOperand::NavalIconOccurrenceSet);
                }
                if input.attackers.is_none() || input.defenders.is_none() {
                    expected.insert(wire::MissingOperand::NavalParticipantReferences);
                }
                if input.attacker_owner_country_ids.is_none()
                    || input.defender_owner_country_ids.is_none()
                {
                    expected.insert(wire::MissingOperand::NavalSideOwnerCountries);
                }
                if !input.location_province_id.is_some_and(|province| province != 0) {
                    expected.insert(wire::MissingOperand::NavalLocationProvince);
                }
                assert_eq!(
                    named_on(wire::IndicatorIdentity::Naval {
                        combat_id: i64::from(input.combat_id),
                        generation: input.generation as i64,
                    }),
                    expected,
                    "naval combat {} names exactly the operands its own input warrants",
                    input.combat_id
                );
            }
            for input in inputs.naval_results.iter().flatten() {
                assert_eq!(
                    named_on(wire::IndicatorIdentity::NavalResult {
                        result_id: i64::from(input.record_id),
                        generation: 0,
                        create_sequence: input.create_sequence,
                    }),
                    [wire::MissingOperand::NavalResultAnchorPoint]
                        .into_iter()
                        .collect::<BTreeSet<_>>(),
                    "retained result {} names exactly the anchor operand",
                    input.record_id
                );
            }
        }

        let rows: Vec<wire::UnplacedIndicator> = [inputs(), starved_inputs()]
            .iter()
            .flat_map(|inputs| envelope(inputs).unplaced)
            .collect();
        let named = |class: wire::IndicatorClass| -> BTreeSet<wire::MissingOperand> {
            rows.iter()
                .filter(|row| wire::IndicatorClass::of(&row.identity) == class)
                .flat_map(|row| row.missing_operands.iter().copied())
                .collect()
        };
        let every_class = [
            wire::IndicatorClass::Land,
            wire::IndicatorClass::Naval,
            wire::IndicatorClass::NavalResult,
        ];
        for class in every_class {
            assert!(
                !named(class).is_empty(),
                "{class:?} contributes rows across the two hours"
            );
        }

        for class in every_class {
            let operands = named(class);
            let operands = &operands;
            for operand in operands {
                if let Some((_, table, leaf)) =
                    unowed.iter().find(|(candidate, _, _)| candidate == operand)
                {
                    assert!(
                        matches!(
                            classify(table, leaf),
                            Some(CarrierLeafSource::ReplayDerived { .. })
                        ),
                        "{operand:?} is declared unowed because {table}[].{leaf} is the viewer's, \
                         and it is not classified that way"
                    );
                    continue;
                }
                let (_, table, leaf) = mapping
                    .iter()
                    .find(|(candidate, _, _)| candidate == operand)
                    .unwrap_or_else(|| {
                        panic!("{class:?} names {operand:?}, which maps to no leaf at all")
                    });
                match classify(table, leaf) {
                    Some(CarrierLeafSource::Gate { owed_by, .. }) => {
                        assert_eq!(*owed_by, CarrierGateOwner::Producer, "{operand:?}");
                    }
                    // A recorded leaf this hour did not carry: the operand is an
                    // hour gate, not a contract gate.
                    Some(CarrierLeafSource::RecordedPlayTimeState { .. }) => {}
                    other => panic!("{operand:?} names {table}[].{leaf}, classified {other:?}"),
                }
            }
        }

        // The other direction, per class: a leaf the contract owes the producer
        // must be named by an operand somewhere. The retained-result class is
        // the declared gap - d103 hands this producer only the id, the province
        // and the create sequence, and the rows it emits name only the anchor -
        // so its gated leaves are listed rather than found.
        let unnamed_naval_result: &[&str] = &[
            "subtype",
            "members",
            "retire_sequence",
            "lifecycle_sha256",
        ];
        for (class, contract_table, table_name) in [
            (
                wire::IndicatorClass::Land,
                XMAP14_LAND_ROW_CONTRACT.as_slice(),
                "land",
            ),
            (
                wire::IndicatorClass::Naval,
                XMAP14_NAVAL_ROW_CONTRACT.as_slice(),
                "naval",
            ),
            (
                wire::IndicatorClass::NavalResult,
                XMAP14_NAVAL_RESULT_ROW_CONTRACT.as_slice(),
                "naval_result",
            ),
        ] {
            let operands = named(class);
            for (leaf, source) in contract_table {
                if source.gate_owner() != Some(CarrierGateOwner::Producer) {
                    continue;
                }
                if class == wire::IndicatorClass::NavalResult
                    && unnamed_naval_result.contains(leaf)
                {
                    continue;
                }
                let answered = mapping.iter().any(|(operand, mapped_table, mapped_leaf)| {
                    *mapped_table == table_name
                        && mapped_leaf == leaf
                        && operands.contains(operand)
                });
                assert!(
                    answered,
                    "{class:?} owes the producer {leaf} and no operand it names answers it"
                );
            }
        }

        // The declared gap is exactly those four leaves and no more.
        let naval_result_gated: BTreeSet<&str> = XMAP14_NAVAL_RESULT_ROW_CONTRACT
            .iter()
            .filter(|(_, source)| source.gate_owner() == Some(CarrierGateOwner::Producer))
            .map(|(leaf, _)| *leaf)
            .collect();
        assert_eq!(
            naval_result_gated,
            unnamed_naval_result.iter().copied().collect::<BTreeSet<&str>>()
        );

        // And the land base point stays the viewer's, which is the other half
        // of the LandIconAnchorPoints reconciliation.
        assert!(matches!(
            xmap14_classify_land_leaf(XMAP14_LAND_ANCHOR_LEAVES[0]),
            Some(CarrierLeafSource::ReplayDerived { .. })
        ));
    }

    #[test]
    fn a_fully_joined_land_row_is_still_short_of_exactly_the_five_unrecorded_operands() {
        let sealed = envelope(&inputs());
        let land = sealed
            .unplaced
            .iter()
            .find(|row| matches!(row.identity, wire::IndicatorIdentity::Land { .. }))
            .expect("the land row is in the census");
        assert_eq!(
            land.missing_operands,
            vec![
                wire::MissingOperand::LandIconAnchorPoints,
                wire::MissingOperand::LandSideFlags,
                wire::MissingOperand::LandFrameSelector,
                wire::MissingOperand::LandComparisonScalars,
                wire::MissingOperand::LandProgress,
            ]
        );
        assert_eq!(land.location_province_id, 910);
    }

    #[test]
    fn a_land_row_names_its_own_missing_province_and_unresolved_participant() {
        let mut with_holes = inputs();
        let land = with_holes.land.as_mut().unwrap();
        land[0].location_province_id = None;
        land[0].attackers = Some(vec![None]);
        let sealed = envelope(&with_holes);
        let row = &sealed.unplaced[0];
        assert!(row
            .missing_operands
            .contains(&wire::MissingOperand::LandProgress));
        assert!(row
            .missing_operands
            .contains(&wire::MissingOperand::LandLocationProvince));
        assert!(row
            .missing_operands
            .contains(&wire::MissingOperand::LandParticipantTypeIds));
        assert_eq!(row.location_province_id, 0);
        // Every operand a land row names belongs to the LAND class.
        assert!(row
            .missing_operands
            .iter()
            .all(|operand| operand.applies_to(wire::IndicatorClass::Land)));
    }

    #[test]
    fn a_zero_province_is_unrecorded_and_not_province_zero() {
        let mut zero = inputs();
        zero.land.as_mut().unwrap()[0].location_province_id = Some(0);
        let sealed = envelope(&zero);
        assert!(sealed.unplaced[0]
            .missing_operands
            .contains(&wire::MissingOperand::LandLocationProvince));
    }

    /// Three different facts about a participant side, three different operands.
    ///
    /// kills the first draft, which collapsed all three into
    /// `LandParticipantTypeIds` and so told a reader to go and fix a D20 join on
    /// an hour where D21 had simply not stated the side.
    #[test]
    fn each_way_a_participant_side_can_be_missing_gets_its_own_operand() {
        // D21 did not state the side at all.
        let mut unstated = inputs();
        unstated.land.as_mut().unwrap()[0].defenders = None;
        assert!(envelope(&unstated).unplaced[0]
            .missing_operands
            .contains(&wire::MissingOperand::LandParticipantMembership));

        // D21 stated it as EXACTLY EMPTY. a729 creates the icon only when both
        // CCombatSide+0x48 counts are non-zero, so this may be a combat it draws
        // nothing for — but D21's list being that count is unestablished, so the
        // icon's existence is UNDECIDED. It is neither dropped from the census
        // nor asserted into it.
        let mut empty = inputs();
        empty.land.as_mut().unwrap()[0].defenders = Some(Vec::new());
        let sealed = envelope(&empty);
        assert!(sealed.unplaced[0]
            .missing_operands
            .contains(&wire::MissingOperand::LandIconExistencePredicate));
        assert!(!sealed.unplaced[0]
            .missing_operands
            .contains(&wire::MissingOperand::LandParticipantTypeIds));
        // Still in the census: the hour's battle count does not change because
        // one of its sides is empty (two land rows and the retained naval result).
        assert_eq!(sealed.unplaced.len(), 3);

        // A stated side whose ids the D20 roster could not type.
        let mut untyped = inputs();
        untyped.land.as_mut().unwrap()[0].defenders = Some(vec![None]);
        let sealed = envelope(&untyped);
        assert!(sealed.unplaced[0]
            .missing_operands
            .contains(&wire::MissingOperand::LandParticipantTypeIds));
        assert!(!sealed.unplaced[0]
            .missing_operands
            .contains(&wire::MissingOperand::LandParticipantMembership));
    }

    /// One D49 combat is not one icon, and the row says so.
    ///
    /// a729's naval producer also creates a spill-over occurrence in a province
    /// other than the combat's own (XMAP14-POV-2). Without that operand named,
    /// a `naval_census: Complete` would be claiming an icon count the recording
    /// cannot know.
    /// XMAP14-POV-2 answered: when D103 stated the combat, the envelope carries
    /// its occurrence set and the row no longer names the operand.
    ///
    /// kills: a producer that names the operand regardless, or one that states
    /// a set without the combat's own province in it.
    #[test]
    fn a_naval_row_d103_stated_carries_its_occurrence_set_instead_of_the_operand() {
        let mut stated = inputs();
        stated.naval.as_mut().unwrap()[0].occurrence_provinces = Some(vec![931, 920]);
        let sealed = envelope(&stated);
        let naval = sealed
            .unplaced
            .iter()
            .find(|row| matches!(row.identity, wire::IndicatorIdentity::Naval { .. }))
            .expect("the naval row is in the census");
        assert!(!naval
            .missing_operands
            .contains(&wire::MissingOperand::NavalIconOccurrenceSet));
        assert_eq!(
            sealed.naval_occurrence_sets,
            vec![wire::NavalOccurrenceSet {
                identity: wire::IndicatorIdentity::Naval {
                    combat_id: 8,
                    generation: 1,
                },
                province_ids: vec![920, 931],
            }]
        );
        // A D103 set that somehow lacks the combat's own province still carries
        // it: the row's province is the set's member by construction.
        let mut spill_only = inputs();
        spill_only.naval.as_mut().unwrap()[0].occurrence_provinces = Some(vec![931]);
        assert_eq!(
            envelope(&spill_only).naval_occurrence_sets[0].province_ids,
            vec![920, 931]
        );
    }

    #[test]
    fn every_naval_row_names_its_unknown_occurrence_set() {
        let sealed = envelope(&inputs());
        let naval = sealed
            .unplaced
            .iter()
            .find(|row| matches!(row.identity, wire::IndicatorIdentity::Naval { .. }))
            .expect("the naval row is in the census");
        assert!(naval
            .missing_operands
            .contains(&wire::MissingOperand::NavalIconOccurrenceSet));
    }

    /// The progress the bar is drawn from is NOT carried from D55.
    ///
    /// kills a regression to promoting D55's per-side `progress_raw` into
    /// `progress_fixed5`: that the two are the same quantity in the same fixed
    /// point is unestablished, and the consumer reads `progress_fixed5` as a
    /// measured value.
    #[test]
    fn land_progress_is_named_unrecorded_and_never_substituted() {
        let sealed = envelope(&inputs());
        assert!(sealed.unplaced[0]
            .missing_operands
            .contains(&wire::MissingOperand::LandProgress));
    }

    #[test]
    fn a_class_d21_or_d49_did_not_state_is_unobtainable_and_contributes_no_rows() {
        let mut unknown = inputs();
        unknown.land = None;
        let sealed = envelope(&unknown);
        assert!(!sealed.land_census.is_complete());
        assert!(sealed
            .unplaced
            .iter()
            .all(|row| !matches!(row.identity, wire::IndicatorIdentity::Land { .. })));
        sealed.validate_body().expect("the census is lawful");
    }

    /// kills: a producer that reports an hour D103 did not state as a measured
    /// empty class, or one that invents a TAG map on an hour D18 did not land.
    #[test]
    fn the_retained_naval_result_class_is_unstated_and_never_measured_empty() {
        let mut unknown = inputs();
        unknown.naval_results = None;
        unknown.country_id_by_tag = Vec::new();
        let sealed = envelope(&unknown);
        assert_eq!(
            sealed.naval_result_census,
            wire::ClassCensus::Unobtainable {
                missing: wire::MissingOperand::NavalResultRecordFamily
            }
        );
        assert!(sealed.naval_results.is_empty());
        assert!(sealed
            .unplaced_of(wire::IndicatorClass::NavalResult)
            .next()
            .is_none());
        assert!(sealed.country_id_by_tag.is_empty());
        sealed.validate_body().expect("the census is lawful");
    }

    /// kills: a producer that places a retained result without its point, one
    /// that names any operand but the point, one that drops the TAG map, and
    /// one that dates a record with a sequence that is not one.
    #[test]
    fn xa_a_retained_naval_result_is_one_dated_row_short_of_exactly_its_point() {
        let sealed = envelope(&inputs());
        let rows: Vec<&wire::UnplacedIndicator> = sealed
            .unplaced_of(wire::IndicatorClass::NavalResult)
            .collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].identity,
            wire::IndicatorIdentity::NavalResult {
                result_id: 9001,
                generation: 0,
                create_sequence: 40,
            }
        );
        assert_eq!(rows[0].location_province_id, 4001);
        assert_eq!(
            rows[0].missing_operands,
            vec![wire::MissingOperand::NavalResultAnchorPoint]
        );
        assert_eq!(
            sealed.country_id_by_tag,
            vec![("ENG".to_string(), 11), ("GER".to_string(), 7)]
        );
        sealed.validate_body().expect("the census is lawful");

        let mut undated = inputs();
        undated.naval_results = Some(vec![NavalResultRecordInput {
            record_id: 9002,
            province_id: 4001,
            create_sequence: 0,
        }]);
        let gap: wire::GapDocument =
            serde_json::from_value(produce(&undated)).expect("the hour states a gap");
        assert!(gap.reason.contains("9002"));
    }

    #[test]
    fn the_only_recorder_build_is_the_build_the_xmap14_laws_were_read_from() {
        // The producer states a729's engine laws unconditionally. That is only
        // honest while a729 is the ONLY build a recording can come from, so
        // this walks the supported-build list rather than the pin alone: adding
        // a second entry fails here, which is the point at which someone has to
        // decide what XMAP14 does about it.
        let builds = crate::game_build::supported_game_builds();
        assert_eq!(builds.len(), 1, "xmap14 assumes exactly one supported build");
        assert_eq!(
            snapshot::sha256::to_hex(&builds[0].exe_sha256),
            wire::XMAP14_ENGINE_SHA256
        );
    }

    #[test]
    fn a_recording_with_no_applied_combat_record_states_no_census() {
        let mut none = inputs();
        none.capture_sequence = 0;
        let document = produce(&none);
        assert_eq!(
            document["schema"],
            wire::XMAP14_COMBAT_INDICATOR_GAP_SCHEMA
        );
    }

    #[test]
    fn a_generation_the_wire_cannot_carry_costs_the_hour_its_whole_census() {
        let mut huge = inputs();
        huge.land.as_mut().unwrap()[0].generation = u64::MAX;
        let document = produce(&huge);
        assert_eq!(document["schema"], wire::XMAP14_COMBAT_INDICATOR_GAP_SCHEMA);
        // Dated and sealed: the consumer reads it back for THIS hour or not at
        // all.
        assert!(wire::read_carrier_document(&document, huge.hour_index).is_ok());
        assert!(wire::read_carrier_document(&document, huge.hour_index + 1).is_err());
        let reason = document["reason"].as_str().unwrap();
        assert!(reason.contains("combat 7"), "{reason}");
        assert!(reason.contains("cannot spell"), "{reason}");
        // Not a shorter census: a list that has quietly lost a battle would
        // pass every downstream length check while being wrong.
        assert!(serde_json::from_value::<wire::CombatEnvelope>(document).is_err());

        // The same number on the NAVAL side costs the hour just as much.
        let mut naval = inputs();
        naval.naval.as_mut().unwrap()[0].generation = u64::MAX;
        assert_eq!(
            produce(&naval)["schema"],
            wire::XMAP14_COMBAT_INDICATOR_GAP_SCHEMA
        );
    }

    #[test]
    fn the_same_inputs_mint_the_same_bytes() {
        assert_eq!(produce(&inputs()), produce(&inputs()));
    }

    #[test]
    fn a_division_key_range_holds_that_id_alone_and_no_id_it_is_a_prefix_of() {
        // The keys a real roster holds around division 7: nine ids that START
        // with a 7 and one that IS 7. A naive `starts_with("7")` prefix scan
        // takes all ten and calls the id ambiguous; the range takes one.
        let roster: std::collections::BTreeSet<String> = [
            "6:3", "7:41", "70:5", "71:5", "79:9", "700:1", "8:2", "80:2",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let (low, high) = division_key_range(7);
        assert_eq!(
            roster.range(low..high).cloned().collect::<Vec<_>>(),
            vec!["7:41".to_string()]
        );

        let (low, high) = division_key_range(70);
        assert_eq!(
            roster.range(low..high).cloned().collect::<Vec<_>>(),
            vec!["70:5".to_string()]
        );

        // An id with no row is an empty range, not a neighbour's row.
        let (low, high) = division_key_range(72);
        assert_eq!(roster.range(low..high).count(), 0);
    }
}
