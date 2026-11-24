
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_save::Save;
use replay_data::{rows::AirSortieModifierRow, rows::AirWingRow, rows::Recorded, rows::tables};
use replay_frame::frame::DecodedFrame;

use super::{CountryContext, Report, complete, invalid};

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    spotting_factors: &Result<Vec<i64>, String>,
    report: &mut Report,
) {
    report.attempt("Air wing fuel operands", |report| {
        report.compared("AirWings fuel columns", wing_fuel_inputs(frame)?);
        report.limitation(
            "Wing cached fuel statistics, execution, training capacity and registry liveness, and living ace raw fuel modifiers are unsaved. Demanded packet leaves and shared execution joins are checked; compare their signed values with an independent native read at the matched paused hour.",
            "AirWings fuel columns / AceRangeInputs.fuel_modifier",
        );
        Ok(())
    });
    report.attempt("country Air modifier operands", |report| {
        let rows = country_air_modifiers(frame)?;
        report.compared("CountryCounterInputs.air_modifiers", rows);
        report.limitation(
            "The save does not serialize the country accumulator entries used by Air detection, normalization, volunteer capacity, fuel and combat/patrol statistics. Compare each raw entry's presence and signed value with an independent native read at the matched paused hour; effective values also require the recorded modifier metadata and feature mask. Living ace own and type combat operands likewise require an independent matched native read.",
            "CountryCounterInputs.air_modifiers",
        );
        Ok(())
    });
    report.attempt_table(replay_data::rows::TableId::AirTransferInputs, |report| {
        transfers(frame, save, report)
    });
    report.attempt_table(replay_data::rows::TableId::AirGroundCrews, |report| {
        ground_crews(frame, save, countries, report)
    });
    report.attempt_table(replay_data::rows::TableId::AirRegionalMissions, |report| {
        report.compared("AirRegionalMissions", registrations(frame, countries)?);
        report.limitation(
            "The save stores each wing's mission, not its regional or all-active registration vectors. Membership, owner, region and contiguous regional ordinals are checked against AirWings. Certified all-active labels must be nonzero and unique within a country, with country-wide availability; both native orders need a live comparison.",
            "AirRegionalMissions order",
        );
        Ok(())
    });
    report.attempt_table(replay_data::rows::TableId::AirCountryRelations, |report| {
        let compared = relations(frame, save, countries, report)?;
        report.compared("AirCountryRelations", compared);
        report.limitation(
            "The save does not serialize the reciprocal-donor, wartime-ally or enemy vectors. Owners, references and contiguous positions are checked, and each enemy set against the saved wars or, where CountryWars read the same vector, exactly against that read; the native order of the other vectors needs a live comparison.",
            "AirCountryRelations order",
        );
        Ok(())
    });
    report.attempt_table(
        replay_data::rows::TableId::AirReferenceWitnesses,
        |report| witnesses(frame, save, report),
    );
    report.attempt_table(replay_data::rows::TableId::NavalRegionalMines, |report| {
        mines(frame, save, countries, report)
    });
    report.attempt_table(replay_data::rows::TableId::AirMissionInputs, |report| {
        let factors = spotting_factors.as_ref().map_err(|why| invalid(format!("Air spotting content factors: {why}")))?;
        let rows = mission_inputs(frame, save, factors, report)?;
        report.compared("AirMissionInputs", rows);
        for row in complete::<tables::AirMissionInputs>(frame)? {
            if matches!(&row.cas_ratio_raw, Recorded::NotRecorded) {
                report.limitation(
                    "The older recording did not capture the CAS diversion ratio.",
                    format!("AirMissionInputs {} cas_ratio_raw", row.wing),
                );
            }
        }
        report.limitation(
            "The save holds no other sortie operand: mission coverage, last target, reached provinces and pool-definition flags are checked for structure only. The mission's CAS diversion ratio, retained-count interception predicate, current execution/count and cached aircraft-detection statistics have no saved counterpart; compare its recorded value or read refusal with an independent native read at the same paused hour.",
            "AirMissionInputs",
        );
        Ok(())
    });
    report.attempt("air naval-patrol inputs", |report| {
        let (combats, histories) = naval_patrol(frame, countries, report)?;
        report.compared("AirNavalPatrolCombats", combats);
        report.compared("AirNavalPatrolHistories", histories);
        report.limitation(
            "The save orders no fleets or task forces and holds no air history window 0: owners, contiguous ordinals, availability witnesses and sparse ring indices are checked. Channels 43/3 and 10/17/51/58/44/24/65 require independent native window-zero reads and ordered ring sums at the same paused hour; saved annual air history does not certify them.",
            "AirNavalPatrolCombats / AirNavalPatrolHistories",
        );
        Ok(())
    });
    report.attempt("air sortie operands", |report| {
        sortie_operands(frame, countries, report)?;
        report.limitation(
            "The save holds no base containers, key-0x11 modifiers, host-base operands or modifier metadata: their keys are checked against the registered missions and their bases, and their values need a live comparison.",
            "AirSortieBaseCountries / AirSortieModifiers / AirSortieSpecialBases / AirSortieModifierMetadata",
        );
        Ok(())
    });
    report.attempt_table(replay_data::rows::TableId::AirWings, |report| {
        let wings = complete::<tables::AirWings>(frame)?;
        for wing in wings.iter().filter(|wing| wing.mission_mask & 8 != 0) {
            let subject = format!("AirWings {} strategic_bombing_raw", wing.id);
            match wing.strategic_bombing_raw {
                Recorded::NotRecorded => report.limitation(
                    "The older recording did not capture the selected strategic bombing statistic.",
                    subject,
                ),
                Recorded::Value(_) => report.limitation(
                    "The selected mission-8 stat 0x32 is not serialized. Its recorded value or read refusal requires an independent native read at the same paused hour.",
                    subject,
                ),
            }
        }
        Ok(())
    });
}

fn wing_fuel_inputs(frame: &DecodedFrame) -> io::Result<usize> {
    let wings = complete::<tables::AirWings>(frame)?;
    let missions = complete::<tables::AirMissionInputs>(frame)?;
    for wing in wings {
        let label = format!("AirWings {} fuel_inputs", wing.id);
        let input = wing.fuel_inputs().map_err(invalid)?;
        input
            .base_fuel
            .ok_or_else(|| invalid(format!("{label} base fuel unreadable")))?;
        let current = input
            .current_type
            .ok_or_else(|| invalid(format!("{label} current execution unreadable")))?;
        if (current != 0) != input.effective_aircraft.is_some() {
            return Err(invalid(format!("{label} current aircraft demand mismatch")));
        }
        let training = current == 0
            && wing.mission_mask & 0x800 != 0
            && wing.eligibility_veto == 0
            && wing.eligibility_counter <= wing.eligibility_limit
            && wing.planes > 0;
        if training && input.override_resolved.is_none() {
            return Err(invalid(format!(
                "{label} training override liveness unreadable"
            )));
        }
        let needs_training = training && input.override_resolved == Some(false);
        if needs_training && input.capacity_class.is_none() {
            return Err(invalid(format!(
                "{label} training capacity class unreadable"
            )));
        }
        if (current != 0 || needs_training) && input.context_mode.is_none() {
            return Err(invalid(format!("{label} fuel context mode unreadable")));
        }
        if let Ok(at) = missions.binary_search_by_key(&wing.id, |row| row.wing) {
            let mission = &missions[at];
            if (*mission
                .current_type
                .require("AirMissionInputs.current_type")
                .map_err(invalid)?)
                != Some(current as u32)
                || (*mission
                    .effective_aircraft
                    .require("AirMissionInputs.effective_aircraft")
                    .map_err(invalid)?)
                    != input.effective_aircraft
            {
                return Err(invalid(format!(
                    "{label} shared execution differs from AirMissionInputs"
                )));
            }
        }
    }
    for ace in complete::<tables::AceRangeInputs>(frame)? {
        let fuel_modifier = *ace
            .fuel_modifier
            .require("AceRangeInputs.fuel_modifier")
            .map_err(invalid)?;
        let combat_modifiers_known = *ace
            .combat_modifiers_known
            .require("AceRangeInputs.combat_modifiers_known")
            .map_err(invalid)?;
        let type_combat_flags = *ace
            .type_combat_flags
            .require("AceRangeInputs.type_combat_flags")
            .map_err(invalid)?;
        let speed_modifier_raw = *ace
            .speed_modifier_raw
            .require("AceRangeInputs.speed_modifier_raw")
            .map_err(invalid)?;
        let attack_modifier_raw = *ace
            .attack_modifier_raw
            .require("AceRangeInputs.attack_modifier_raw")
            .map_err(invalid)?;
        let defence_modifier_raw = *ace
            .defence_modifier_raw
            .require("AceRangeInputs.defence_modifier_raw")
            .map_err(invalid)?;
        let type_speed_modifier_raw = *ace
            .type_speed_modifier_raw
            .require("AceRangeInputs.type_speed_modifier_raw")
            .map_err(invalid)?;
        if (ace.alive != 0) != fuel_modifier.is_some() {
            return Err(invalid(format!(
                "AceRangeInputs {} living fuel demand mismatch",
                ace.ace
            )));
        }
        if (ace.alive != 0) != combat_modifiers_known
            || (ace.alive != 0) != (type_combat_flags & 1 != 0)
            || type_combat_flags & !7 != 0
        {
            return Err(invalid(format!(
                "AceRangeInputs {} living combat demand mismatch",
                ace.ace
            )));
        }
        if ace.alive == 0
            && [
                speed_modifier_raw,
                attack_modifier_raw,
                defence_modifier_raw,
                type_speed_modifier_raw,
            ]
            .iter()
            .any(Option::is_some)
        {
            return Err(invalid(format!(
                "AceRangeInputs {} dead combat operands were read",
                ace.ace
            )));
        }
    }
    Ok(wings.len())
}

fn country_air_modifiers(frame: &DecodedFrame) -> io::Result<usize> {
    let rows = complete::<tables::CountryCounterInputs>(frame)?;
    for row in rows {
        row.air_modifiers
            .require("CountryCounterInputs.air_modifiers")
            .map_err(invalid)?
            .as_ref()
            .ok_or_else(|| {
                invalid(format!(
                    "country {} Air modifier accumulator unreadable",
                    row.country
                ))
            })?;
        if row.air_combat_modifiers_known != replay_data::rows::Recorded::Value(true) {
            return Err(invalid(format!(
                "country {} Air combat modifier accumulator unreadable",
                row.country
            )));
        }
    }
    Ok(rows.len())
}

fn transfers(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::AirTransferInputs>(frame)?;
    let wings = complete::<tables::AirWings>(frame)?;
    let bases = complete::<tables::AirBases>(frame)?;
    for saved in save
        .air_wings
        .values()
        .filter(|wing| wing.transferring_to.is_some())
    {
        if rows
            .binary_search_by_key(&saved.id, |row| row.wing)
            .is_err()
        {
            return Err(invalid(format!(
                "Air transfer {} saved destination has no incoming membership",
                saved.id
            )));
        }
    }
    let mut ordinals = BTreeMap::<u64, BTreeSet<u32>>::new();
    for row in rows {
        let wing = wings
            .binary_search_by_key(&row.wing, |wing| wing.id)
            .ok()
            .map(|at| &wings[at])
            .ok_or_else(|| invalid("Air transfer has no current wing"))?;
        if row.base == 0 || row.base != wing.base_override {
            return Err(invalid(
                "Air transfer destination differs from its current wing",
            ));
        }
        let ordinal = row.ordinal.ok_or_else(|| {
            invalid(format!(
                "Air transfer {} incoming order is uncertified or unreadable",
                row.wing
            ))
        })?;
        if !ordinals.entry(row.base).or_default().insert(ordinal) {
            return Err(invalid("Air transfer incoming ordinal repeats"));
        }
        let saved = save
            .air_wings
            .get(&row.wing)
            .ok_or_else(|| invalid("Air transfer wing is absent from the save"))?;
        if let Some(target) = saved.transferring_to {
            let state = bases
                .binary_search_by_key(&row.base, |base| base.id)
                .ok()
                .and_then(|at| bases[at].state);
            if state != Some(target) {
                report.difference(
                    format!("Air transfer {} destination state", row.wing),
                    state.map_or_else(|| "absent".to_owned(), |id| id.to_string()),
                    target,
                );
            }
        }
        if wing.eligibility_counter > wing.eligibility_limit {
            continue;
        }
        let progress = row
            .progress_raw
            .ok_or_else(|| invalid(format!("Air transfer {} progress is unreadable", row.wing)))?;
        row.distance_raw.ok_or_else(|| {
            invalid(format!(
                "Air transfer {} total distance is unreadable",
                row.wing
            ))
        })?;
        if saved.transfer_progress != Some(progress) {
            report.difference(
                format!("Air transfer {} progress", row.wing),
                progress,
                saved
                    .transfer_progress
                    .map_or_else(|| "absent".to_owned(), |raw| raw.to_string()),
            );
        }
    }
    contiguous(ordinals.values(), "Air incoming-wing")?;
    report.compared("AirTransferInputs", rows.len());
    report.limitation("The save does not serialize direct incoming-wing order or the retained total relocation distance. Current admitted progress is compared exactly; incoming order and total distance need the integration live comparison.", "Air transfer order and total distance");
    Ok(())
}

fn ground_crews(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let wings: BTreeMap<_, _> = complete::<tables::AirWings>(frame)?
        .iter()
        .map(|row| (row.id, row))
        .collect();
    let mut demanded = BTreeSet::new();
    for row in complete::<tables::AirRegionalMissions>(frame)? {
        let wing = wings
            .get(&row.wing)
            .ok_or_else(|| invalid("Ground Crews registration has no wing"))?;
        let slot = countries
            .links
            .get(&wing.owner)
            .ok_or_else(|| invalid("Ground Crews owner has no country link"))?
            .canonical_slot;
        let region = wing
            .mission_region
            .ok_or_else(|| invalid("Ground Crews registration has no region"))?;
        demanded.insert((slot, region));
    }
    let mut expected = BTreeMap::new();
    for row in &save.air.ground_crews {
        let slot = super::saved_country_slot(&row.country, save, countries)?;
        if expected.insert((slot, row.region), row.enabled).is_some() {
            return Err(invalid("saved Ground Crews country/region is repeated"));
        }
    }
    let rows = complete::<tables::AirGroundCrews>(frame)?;
    for row in rows {
        if (!demanded.contains(&(row.country, row.region))
            && !matches!(row.supply, Recorded::Value(Some(_))))
            || row.enabled.is_some_and(|enabled| enabled > 1)
        {
            return Err(invalid(
                "Ground Crews names an undemanded region or invalid enabled flag",
            ));
        }
    }
    let mut compared = 0;
    for key in demanded {
        let actual = match rows.binary_search_by_key(&key, |row| (row.country, row.region)) {
            Ok(at) => rows[at].enabled,
            Err(_) => Some(0),
        };
        let Some(actual) = actual else {
            report.incomplete(
                "AirGroundCrews",
                format!(
                    "AirGroundCrews {}:{} enabled: writer certification or enabled-byte acquisition was unavailable",
                    key.0, key.1
                ),
            );
            continue;
        };
        let saved = expected.get(&key).copied().unwrap_or(false);
        if (actual != 0) != saved {
            report.difference(
                format!("AirGroundCrews {}:{} enabled", key.0, key.1),
                actual != 0,
                saved,
            );
        }
        compared += 1;
    }
    report.compared("AirGroundCrews", compared);
    Ok(())
}

fn naval_patrol(
    frame: &DecodedFrame,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<(usize, usize)> {
    let slots: BTreeSet<_> = countries
        .by_tag
        .values()
        .map(|country| country.canonical_slot)
        .collect();
    let combats = complete::<tables::AirNavalPatrolCombats>(frame)?;
    let histories = complete::<tables::AirNavalPatrolHistories>(frame)?;
    let mut ordinals = BTreeMap::<u32, BTreeSet<u32>>::new();
    for row in combats {
        if !slots.contains(&row.country) || row.region == 0 {
            return Err(invalid(format!(
                "naval-patrol combat {}:{} names no live country or region",
                row.country, row.ordinal
            )));
        }
        ordinals.entry(row.country).or_default().insert(row.ordinal);
    }
    contiguous(ordinals.values(), "naval-patrol combat")?;
    histories_structure(histories, &slots)?;
    for row in histories {
        if !row.is_menu() && matches!(row.scenario_channels, Recorded::Value(None)) {
            report.limitation(
                "AirNavalPatrolHistories.scenario_channels unavailable after a current read refusal; original naval-patrol history remains readable",
                format!("air history {}:{}:{}", row.country, row.region, row.index),
            );
        }
    }
    Ok((combats.len(), histories.len()))
}

fn histories_structure(
    rows: &[replay_data::rows::AirNavalPatrolHistoryRow],
    slots: &BTreeSet<u32>,
) -> io::Result<()> {
    let mut witnesses = BTreeSet::new();
    let mut values = BTreeSet::new();
    for row in rows {
        if row.is_availability_witness() {
            if row
                != &replay_data::rows::AirNavalPatrolHistoryRow::availability_witness(row.country)
                || !witnesses.insert(row.country)
            {
                return Err(invalid(
                    "air history availability witness is malformed or repeated",
                ));
            }
            continue;
        }
        if row.is_window() {
            if !slots.contains(&row.country)
                || row.enemy_ships_sunk_raw != 0
                || row.our_fighters_lost_raw != 0
                || row.scenario_channels != Recorded::Value(None)
            {
                return Err(invalid("air history ring marker is malformed"));
            }
            row.window
                .ok_or_else(|| invalid("air history ring metadata is missing"))?
                .validate()?;
            values.insert(row.country);
            continue;
        }
        row.summation_index(rows).map_err(invalid)?;
        if !slots.contains(&row.country)
            || !row.has_values()
            || row.index == replay_data::rows::AIR_HISTORY_AVAILABILITY_INDEX
        {
            return Err(invalid(format!(
                "air history {}:{}:{} names no live country, valid ring index or value",
                row.country, row.region, row.index
            )));
        }
        if matches!(&row.scenario_channels, Recorded::Value(_)) {
            values.insert(row.country);
        }
    }
    if !values.is_subset(&witnesses) {
        return Err(invalid(
            "air scenario history values lack their availability witness",
        ));
    }
    Ok(())
}

fn mission_inputs(
    frame: &DecodedFrame,
    save: &Save,
    spotting_factors: &[i64],
    report: &mut Report,
) -> io::Result<usize> {
    let wings: BTreeMap<_, _> = complete::<tables::AirWings>(frame)?
        .iter()
        .map(|wing| (wing.id, wing))
        .collect();
    let registered: BTreeSet<_> = complete::<tables::AirRegionalMissions>(frame)?
        .iter()
        .map(|row| row.wing)
        .collect();
    let rows = complete::<tables::AirMissionInputs>(frame)?;
    if rows
        .iter()
        .map(|row| row.wing)
        .ne(registered.iter().copied())
    {
        return Err(invalid(
            "air mission inputs differ from the registered missions",
        ));
    }
    for row in rows {
        match *row
            .current_type
            .require("AirMissionInputs.current_type")
            .map_err(invalid)?
        {
            None => {
                return Err(invalid(format!(
                    "air mission {} current execution is unreadable",
                    row.wing
                )));
            }
            Some(0)
                if (*row
                    .effective_aircraft
                    .require("AirMissionInputs.effective_aircraft")
                    .map_err(invalid)?)
                .is_some() =>
            {
                return Err(invalid(format!(
                    "air mission {} count was read outside execution demand",
                    row.wing
                )));
            }
            Some(mask)
                if mask != 0
                    && (*row
                        .effective_aircraft
                        .require("AirMissionInputs.effective_aircraft")
                        .map_err(invalid)?)
                    .is_none() =>
            {
                return Err(invalid(format!(
                    "air mission {} executing aircraft count is unreadable",
                    row.wing
                )));
            }
            _ => (),
        }
        mission_normalization(row)?;
        mission_spotting(frame, row, spotting_factors)?;
        let mask = wings
            .get(&row.wing)
            .ok_or_else(|| invalid("air mission input lacks its AirWings row"))?
            .mission_mask;
        let membership = mask & replay_data::rows::AIR_MISSION_REACHED_IDS != 0;
        if (membership && row.reached_provinces.len() != row.reached_count as usize)
            || (!membership && !row.reached_provinces.is_empty())
        {
            return Err(invalid(format!(
                "air mission {} reached provinces disagree with its mask or count",
                row.wing
            )));
        }
        let saved = save
            .air_wings
            .get(&row.wing)
            .and_then(|wing| wing.region_change_penalty);
        if saved != Some(row.reassignment_penalty_raw) {
            report.difference(
                format!("air mission {} region_change_penalty", row.wing),
                row.reassignment_penalty_raw,
                saved.map_or_else(|| "absent".to_owned(), |raw| raw.to_string()),
            );
        }
    }
    Ok(rows.len())
}

struct SortieKeys {
    bases: BTreeMap<u64, (i32, Option<u32>)>,
    countries: BTreeSet<u32>,
    positive: BTreeSet<u32>,
    regions: BTreeSet<u32>,
    country_regions: BTreeSet<(u32, u32)>,
}

fn mission_spotting(
    frame: &DecodedFrame,
    row: &replay_data::rows::AirMissionInputRow,
    factors: &[i64],
) -> io::Result<()> {
    let admitted = (*row
        .current_type
        .require("AirMissionInputs.current_type")
        .map_err(invalid)?)
    .is_some_and(|mask| {
        mask as i32 > 0
            && mask.is_power_of_two()
            && factors
                .get(mask.trailing_zeros() as usize)
                .is_some_and(|factor| *factor != 0)
    });
    if admitted {
        mission_detection(row)?;
        mission_patrol_disruption(row)?;
        mission_patrol_night(frame, row)
    } else if [
        (*row
            .detection_stat_13
            .require("AirMissionInputs.detection_stat_13")
            .map_err(invalid)?),
        (*row
            .detection_stat_14
            .require("AirMissionInputs.detection_stat_14")
            .map_err(invalid)?),
        (*row
            .patrol_disruption_raw
            .require("AirMissionInputs.patrol_disruption_raw")
            .map_err(invalid)?),
        (*row
            .patrol_attack_raw
            .require("AirMissionInputs.patrol_attack_raw")
            .map_err(invalid)?),
        (*row
            .patrol_defence_raw
            .require("AirMissionInputs.patrol_defence_raw")
            .map_err(invalid)?),
        (*row
            .patrol_speed_raw
            .require("AirMissionInputs.patrol_speed_raw")
            .map_err(invalid)?),
        (*row
            .patrol_wing_attack_raw
            .require("AirMissionInputs.patrol_wing_attack_raw")
            .map_err(invalid)?),
        (*row
            .patrol_wing_defence_raw
            .require("AirMissionInputs.patrol_wing_defence_raw")
            .map_err(invalid)?),
        (*row
            .patrol_night_applied_raw
            .require("AirMissionInputs.patrol_night_applied_raw")
            .map_err(invalid)?),
    ]
    .into_iter()
    .any(|value| value.is_some())
    {
        Err(invalid(
            "Air patrol operands acquired outside the recorded-content spotting factor demand",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[test]
fn recorded_content_spotting_factor_distinguishes_skipped_and_refused_leaves() {
    use replay_data::rows::AirMissionInputRow;
    let (frame, _, _, _) = super::fixtures::fixture();
    let row = AirMissionInputRow {
        current_type: (Some(8)).into(),
        effective_aircraft: (Some(17)).into(),
        normalization: (Some(replay_data::rows::AirMissionNormalization {
            night_stat: Some(0),
            modifiers: [Some(Default::default()); 2],
        }))
        .into(),
        ..Default::default()
    };
    assert!(mission_spotting(&frame, &row, &[0; 8]).is_ok());
    assert!(mission_spotting(&frame, &row, &[]).is_ok());
    assert!(mission_spotting(&frame, &row, &[100_000; 8]).is_err());
    assert!(
        mission_spotting(
            &frame,
            &AirMissionInputRow {
                detection_stat_13: (Some(0)).into(),
                ..row
            },
            &[0; 8]
        )
        .is_err()
    );
}

fn mission_detection(row: &replay_data::rows::AirMissionInputRow) -> io::Result<()> {
    let demanded = (*row
        .current_type
        .require("AirMissionInputs.current_type")
        .map_err(invalid)?)
    .is_some_and(|mask| mask as i32 > 0 && mask.is_power_of_two())
        && (*row
            .effective_aircraft
            .require("AirMissionInputs.effective_aircraft")
            .map_err(invalid)?)
        .is_some_and(|count| count != 0);
    let values = [
        (*row
            .detection_stat_13
            .require("AirMissionInputs.detection_stat_13")
            .map_err(invalid)?),
        (*row
            .detection_stat_14
            .require("AirMissionInputs.detection_stat_14")
            .map_err(invalid)?),
    ];
    if demanded && values.iter().any(Option::is_none) {
        return Err(invalid(
            "executing Air patrol detection statistic unreadable",
        ));
    }
    if !demanded && values.iter().any(Option::is_some) {
        return Err(invalid(
            "Air patrol detection statistics outside execution demand",
        ));
    }
    Ok(())
}

fn mission_patrol_disruption(row: &replay_data::rows::AirMissionInputRow) -> io::Result<()> {
    let mask = (*row
        .current_type
        .require("AirMissionInputs.current_type")
        .map_err(invalid)?)
    .unwrap_or(0);
    let count = (*row
        .effective_aircraft
        .require("AirMissionInputs.effective_aircraft")
        .map_err(invalid)?)
    .unwrap_or(0);
    let demanded = mask as i32 > 0 && mask.is_power_of_two() && mask & 0x1f7fa != 0 && count != 0;
    if demanded
        != (*row
            .patrol_disruption_raw
            .require("AirMissionInputs.patrol_disruption_raw")
            .map_err(invalid)?)
        .is_some()
    {
        return Err(invalid(
            "Air patrol disruption demand or acquisition is incomplete",
        ));
    }
    let combat = demanded
        && (*row
            .patrol_disruption_raw
            .require("AirMissionInputs.patrol_disruption_raw")
            .map_err(invalid)?)
            != Some(0)
        && mask & 0x4400 == 0
        && count > 0;
    let leaves = [
        (*row
            .patrol_attack_raw
            .require("AirMissionInputs.patrol_attack_raw")
            .map_err(invalid)?),
        (*row
            .patrol_defence_raw
            .require("AirMissionInputs.patrol_defence_raw")
            .map_err(invalid)?),
        (*row
            .patrol_speed_raw
            .require("AirMissionInputs.patrol_speed_raw")
            .map_err(invalid)?),
        (*row
            .patrol_wing_attack_raw
            .require("AirMissionInputs.patrol_wing_attack_raw")
            .map_err(invalid)?),
        (*row
            .patrol_wing_defence_raw
            .require("AirMissionInputs.patrol_wing_defence_raw")
            .map_err(invalid)?),
    ];
    if leaves.into_iter().any(|leaf| leaf.is_some() != combat) {
        return Err(invalid(
            "Air patrol disruption combat leaves do not match their demand",
        ));
    }
    if !combat
        && (*row
            .patrol_night_applied_raw
            .require("AirMissionInputs.patrol_night_applied_raw")
            .map_err(invalid)?)
        .is_some()
    {
        return Err(invalid(
            "Air patrol held night marker outside disruption demand",
        ));
    }
    Ok(())
}

fn mission_patrol_night(
    frame: &DecodedFrame,
    row: &replay_data::rows::AirMissionInputRow,
) -> io::Result<()> {
    let mask = (*row
        .current_type
        .require("AirMissionInputs.current_type")
        .map_err(invalid)?)
    .unwrap_or(0);
    if mask & 0x4400 != 0
        || mask & 0x1f7fa == 0
        || (*row
            .effective_aircraft
            .require("AirMissionInputs.effective_aircraft")
            .map_err(invalid)?)
        .is_none_or(|count| count <= 0)
        || (*row
            .patrol_disruption_raw
            .require("AirMissionInputs.patrol_disruption_raw")
            .map_err(invalid)?)
        .is_none_or(|raw| raw == 0)
    {
        return Ok(());
    }
    let clocks = complete::<tables::WeatherProvinces>(frame)?;
    let mut observed = false;
    let mut held = false;
    for row in clocks.iter().filter(|row| row.id != 0) {
        observed = true;
        match row.clock.state(row.id).map_err(invalid)? {
            replay_data::rows::ClockState::Updated => (),
            replay_data::rows::ClockState::Held(_) => held = true,
            replay_data::rows::ClockState::NotRecorded => {
                return Err(invalid("Air patrol sampled clock was not recorded"));
            }
        }
    }
    if !observed
        || (held
            && (*row
                .patrol_night_applied_raw
                .require("AirMissionInputs.patrol_night_applied_raw")
                .map_err(invalid)?)
            .is_none())
    {
        return Err(invalid(
            "Air patrol held night application marker is unavailable",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[test]
fn patrol_held_night_oracle_never_interprets_a_missing_marker_as_updated() {
    use replay_data::rows::*;
    use replay_frame::frame::TableView;
    let (mut frame, _, _, _) = super::fixtures::fixture();
    let mut row = AirMissionInputRow {
        wing: 7,
        current_type: (Some(8)).into(),
        effective_aircraft: (Some(9)).into(),
        patrol_disruption_raw: (Some(1)).into(),
        ..Default::default()
    };
    *frame.tables[TableId::WeatherProvinces.index()] =
        TableView::Complete(TableRows::WeatherProvinces(vec![WeatherProvinceRow {
            id: 1,
            clock: DayNightClock::UPDATED,
            ..Default::default()
        }]));
    assert!(mission_patrol_night(&frame, &row).is_ok());
    *frame.tables[TableId::WeatherProvinces.index()] =
        TableView::Complete(TableRows::WeatherProvinces(vec![WeatherProvinceRow {
            id: 1,
            clock: DayNightClock::held(replay_data::day_night::Clock {
                local_hour: 0,
                night_factor: 50_001,
            }),
            ..Default::default()
        }]));
    assert!(mission_patrol_night(&frame, &row).is_err());
    row.patrol_night_applied_raw = Some(50_001).into();
    assert!(mission_patrol_night(&frame, &row).is_ok());
    row.patrol_night_applied_raw = Some(-100_000).into();
    assert!(mission_patrol_night(&frame, &row).is_ok());
}

#[cfg(test)]
#[test]
fn patrol_disruption_oracle_preserves_bypass_and_independent_demand() {
    use replay_data::rows::AirMissionInputRow;
    let quiet = AirMissionInputRow {
        current_type: (Some(8)).into(),
        effective_aircraft: (Some(7)).into(),
        patrol_disruption_raw: (Some(0)).into(),
        ..Default::default()
    };
    assert!(mission_patrol_disruption(&quiet).is_ok());
    assert!(
        mission_patrol_disruption(&AirMissionInputRow {
            patrol_disruption_raw: (None).into(),
            ..quiet.clone()
        })
        .is_err()
    );
    let combat = AirMissionInputRow {
        patrol_disruption_raw: (Some(1)).into(),
        patrol_attack_raw: (Some(2)).into(),
        patrol_defence_raw: (Some(3)).into(),
        patrol_speed_raw: (Some(4)).into(),
        patrol_wing_attack_raw: (Some(0)).into(),
        patrol_wing_defence_raw: (Some(-5)).into(),
        ..quiet.clone()
    };
    assert!(mission_patrol_disruption(&combat).is_ok());
    for row in [
        AirMissionInputRow {
            patrol_speed_raw: (None).into(),
            ..combat.clone()
        },
        AirMissionInputRow {
            patrol_wing_attack_raw: (None).into(),
            ..combat.clone()
        },
        AirMissionInputRow {
            current_type: (Some(0x400)).into(),
            ..combat.clone()
        },
        AirMissionInputRow {
            effective_aircraft: (Some(-1)).into(),
            ..combat
        },
    ] {
        assert!(mission_patrol_disruption(&row).is_err());
    }
    assert!(
        mission_patrol_disruption(&AirMissionInputRow {
            current_type: (Some(0x400)).into(),
            patrol_disruption_raw: (Some(1)).into(),
            ..quiet.clone()
        })
        .is_ok()
    );
    assert!(
        mission_patrol_disruption(&AirMissionInputRow {
            effective_aircraft: (Some(-1)).into(),
            patrol_disruption_raw: (Some(1)).into(),
            ..quiet
        })
        .is_ok()
    );
}

fn mission_normalization(row: &replay_data::rows::AirMissionInputRow) -> io::Result<()> {
    let demanded = (*row
        .current_type
        .require("AirMissionInputs.current_type")
        .map_err(invalid)?)
    .is_some_and(|mask| mask != 0)
        && (*row
            .effective_aircraft
            .require("AirMissionInputs.effective_aircraft")
            .map_err(invalid)?)
        .is_some_and(|count| count != 0);
    if !demanded {
        if row
            .normalization
            .require("AirMissionInputs.normalization")
            .map_err(invalid)?
            .is_some()
        {
            return Err(invalid(
                "Air normalization was read outside execution demand",
            ));
        }
        return Ok(());
    }
    let packet = row
        .normalization
        .require("AirMissionInputs.normalization")
        .map_err(invalid)?
        .as_ref()
        .ok_or_else(|| invalid("executing Air normalization packet is missing"))?;
    if packet.night_stat.is_none() || packet.modifiers.iter().any(Option::is_none) {
        return Err(invalid(
            "executing Air normalization contains an unreadable operand",
        ));
    }
    if packet
        .modifiers
        .iter()
        .flatten()
        .any(|entry| !entry.present && entry.raw.is_some())
    {
        return Err(invalid("absent Air normalization modifier has a raw value"));
    }
    Ok(())
}

fn sortie_keys(frame: &DecodedFrame, countries: &CountryContext<'_>) -> io::Result<SortieKeys> {
    let wings: BTreeMap<_, _> = complete::<tables::AirWings>(frame)?
        .iter()
        .map(|wing| (wing.id, wing))
        .collect();
    let manager: BTreeMap<_, _> = complete::<tables::AirBases>(frame)?
        .iter()
        .map(|base| (base.id, (base.kind, base.state)))
        .collect();
    let mut witnessed = BTreeMap::new();
    for row in complete::<tables::AirReferenceWitnesses>(frame)? {
        let base = match row.base_kind {
            Some(kind) => (kind, row.state),
            None => *manager
                .get(&row.handle)
                .ok_or_else(|| invalid("handle-only air reference witness has no AirBases row"))?,
        };
        witnessed.insert(row.handle, base);
    }
    let mut keys = SortieKeys {
        bases: BTreeMap::new(),
        countries: countries
            .by_tag
            .values()
            .map(|country| country.canonical_slot)
            .collect(),
        positive: BTreeSet::new(),
        regions: BTreeSet::new(),
        country_regions: BTreeSet::new(),
    };
    for registration in complete::<tables::AirRegionalMissions>(frame)? {
        let wing: &AirWingRow = wings
            .get(&registration.wing)
            .ok_or_else(|| invalid("registered air mission lacks its AirWings row"))?;
        if let Some(&base) = witnessed.get(&wing.air_base) {
            keys.bases.insert(wing.air_base, base);
        }
        let effective = if wing.country_override > 0 {
            wing.country_override as u32
        } else {
            wing.owner
        };
        let slot = countries
            .links
            .get(&effective)
            .ok_or_else(|| invalid("registered air wing country is not a linked country"))?
            .canonical_slot;
        let region = wing
            .mission_region
            .ok_or_else(|| invalid("registered air mission has no region"))?;
        keys.regions.insert(region);
        if (effective as i32) > 0 {
            keys.positive.insert(slot);
            keys.country_regions.insert((slot, region));
        }
    }
    Ok(keys)
}

fn sortie_operands(
    frame: &DecodedFrame,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let keys = sortie_keys(frame, countries)?;
    let containers = complete::<tables::AirSortieBaseCountries>(frame)?;
    for row in containers {
        if !keys.bases.contains_key(&row.base)
            || !keys.countries.contains(&row.country)
            || row.fuel_demand_raw.is_some() != row.fuel_allocated_raw.is_some()
            || [
                row.capacity_raw,
                row.supply_raw,
                row.fuel_demand_raw.unwrap_or(0),
                row.fuel_allocated_raw.unwrap_or(0),
            ] == [0; 4]
        {
            return Err(invalid(format!(
                "air sortie container {}:{} names no registered base or live country, is zero or has half its fuel",
                row.base, row.country
            )));
        }
    }
    report.compared("AirSortieBaseCountries", containers.len());

    let modifiers = complete::<tables::AirSortieModifiers>(frame)?;
    let menu: BTreeMap<_, _> = modifiers
        .iter()
        .filter(|row| row.scope == AirSortieModifierRow::MENU_MODIFIERS_STATUS)
        .map(|row| ((row.subject, row.region), row.raw))
        .collect();
    for row in modifiers {
        let keyed = match row.scope {
            AirSortieModifierRow::WEATHER => {
                row.subject == 0
                    && (keys.regions.contains(&row.region)
                        || menu.get(&(0, row.region)) == Some(&1))
            }
            AirSortieModifierRow::COUNTRY => {
                row.region == 0
                    && (keys.positive.contains(&row.subject)
                        || menu.get(&(row.subject, 0)) == Some(&1))
            }
            AirSortieModifierRow::COUNTRY_REGION => {
                keys.country_regions.contains(&(row.subject, row.region))
                    || (row.region != 0 && menu.get(&(row.subject, 0)) == Some(&1))
            }
            AirSortieModifierRow::EFFICIENCY_OVERRIDE
            | AirSortieModifierRow::DETECTION_OVERRIDE => row.region == 0,
            AirSortieModifierRow::DETECTION_OVERRIDE_STATUS => {
                row.region == 0 && row.subject == 0 && row.raw == 1
            }
            AirSortieModifierRow::MENU_MODIFIERS_STATUS => {
                matches!(row.raw, 0 | 1)
                    && ((row.subject == 0 && row.region != 0)
                        || (row.region == 0
                            && row.subject != 0
                            && keys.countries.contains(&row.subject)))
            }
            _ => false,
        };
        if !keyed
            || (row.raw == 0
                && !matches!(
                    row.scope,
                    AirSortieModifierRow::EFFICIENCY_OVERRIDE
                        | AirSortieModifierRow::DETECTION_OVERRIDE
                        | AirSortieModifierRow::MENU_MODIFIERS_STATUS
                ))
        {
            return Err(invalid(format!(
                "air sortie modifier {}:{}:{} has an invalid scope, key, value or availability witness",
                row.scope, row.subject, row.region
            )));
        }
    }
    let mut witnesses = modifiers
        .iter()
        .filter(|row| row.scope == AirSortieModifierRow::DETECTION_OVERRIDE_STATUS);
    if witnesses.next().is_none() || witnesses.next().is_some() {
        return Err(invalid(
            "Air detection override availability witness is missing or duplicated",
        ));
    }
    report.compared("AirSortieModifiers", modifiers.len());

    let special = complete::<tables::AirSortieSpecialBases>(frame)?;
    for row in special {
        let Some(&(kind, state)) = keys.bases.get(&row.base) else {
            return Err(invalid(format!(
                "air sortie special base {} is not a registered pool base",
                row.base
            )));
        };
        let host = state.is_none() && kind != 2;
        let consistent = (row.rocket_slots.is_none() || (kind == 1 && state.is_some()))
            && (row.carrier_stance.is_none() || (host && kind != 1))
            && (row.leader_modifier_raw == 0 || host)
            && (row.task_force_combats.is_none() || host)
            && (row.task_force_region.is_none() || row.task_force_combats.is_some_and(|n| n > 0))
            && (row.rocket_slots.is_some()
                || row.carrier_stance.is_some()
                || row.leader_modifier_raw != 0
                || row.task_force_combats.is_some());
        if !consistent {
            return Err(invalid(format!(
                "air sortie special base {} operands disagree with its kind {kind} and state",
                row.base
            )));
        }
    }
    report.compared("AirSortieSpecialBases", special.len());

    let [metadata] = complete::<tables::AirSortieModifierMetadata>(frame)? else {
        return Err(invalid("air sortie modifier metadata is not a singleton"));
    };
    if metadata.callback_kind > 1 || (metadata.feature_selector == 0 && metadata.callback_kind != 0)
    {
        return Err(invalid(
            "air sortie modifier metadata names an unknown feature callback",
        ));
    }
    report.compared("AirSortieModifierMetadata", 1);
    Ok(())
}

fn registrations(frame: &DecodedFrame, countries: &CountryContext<'_>) -> io::Result<usize> {
    let wings: BTreeMap<_, _> = complete::<tables::AirWings>(frame)?
        .iter()
        .map(|wing| (wing.id, wing))
        .collect();
    let rows = complete::<tables::AirRegionalMissions>(frame)?;
    let mut ordinals = BTreeMap::<(u32, u32), BTreeSet<u32>>::new();
    let mut labels = BTreeSet::new();
    let mut known = BTreeMap::new();
    for row in rows {
        let wing = wings
            .get(&row.wing)
            .ok_or_else(|| invalid("registered air mission lacks its AirWings row"))?;
        let owner = countries
            .links
            .get(&wing.owner)
            .ok_or_else(|| invalid("registered air mission owner is not a linked country"))?;
        let region = wing
            .mission_region
            .ok_or_else(|| invalid("registered air mission has no region"))?;
        if let Some(previous) = known.insert(owner.canonical_slot, row.active_order.is_some())
            && previous != row.active_order.is_some()
        {
            return Err(invalid(
                "Air all-active order availability differs within one country",
            ));
        }
        if let Some(label) = row.active_order
            && (label == 0 || !labels.insert((owner.canonical_slot, label)))
        {
            return Err(invalid("Air all-active order label is zero or repeats"));
        }
        if !ordinals
            .entry((owner.canonical_slot, region))
            .or_default()
            .insert(row.ordinal)
        {
            return Err(invalid("regional air mission ordinal repeats"));
        }
    }
    contiguous(ordinals.values(), "regional air mission")?;
    Ok(rows.len())
}

fn relations(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<usize> {
    let slots: BTreeSet<_> = countries
        .by_tag
        .values()
        .map(|country| country.canonical_slot)
        .collect();
    let mut read = BTreeMap::<u32, &[u32]>::new();
    for row in complete::<tables::CountryWars>(frame)? {
        if row.enemies_read == 0 {
            continue;
        }
        let country = countries
            .links
            .get(&row.country)
            .ok_or_else(|| invalid("CountryWars row names no linked country"))?;
        if read
            .insert(country.canonical_slot, row.enemies.as_slice())
            .is_some()
        {
            return Err(invalid("CountryWars reads one country's enemies twice"));
        }
    }
    let rows = complete::<tables::AirCountryRelations>(frame)?;
    let mut ordinals = BTreeMap::<(u32, u8), BTreeSet<u32>>::new();
    let mut enemies = BTreeMap::<u32, BTreeSet<u32>>::new();
    let mut vectors = BTreeMap::<u32, BTreeMap<u32, u32>>::new();
    for row in rows {
        if !slots.contains(&row.country)
            || row.kind > replay_data::rows::AirCountryRelationRow::ENEMY
            || !countries.links.contains_key(&row.other)
        {
            return Err(invalid(format!(
                "air country relation {}:{}:{} names no live country or kind",
                row.country, row.kind, row.ordinal
            )));
        }
        ordinals
            .entry((row.country, row.kind))
            .or_default()
            .insert(row.ordinal);
        if row.kind == replay_data::rows::AirCountryRelationRow::ENEMY {
            enemies
                .entry(row.country)
                .or_default()
                .insert(countries.links[&row.other].canonical_slot);
            vectors
                .entry(row.country)
                .or_default()
                .insert(row.ordinal, row.other);
        }
    }
    contiguous(ordinals.values(), "air country relation")?;
    let mut expected = BTreeMap::<u32, BTreeSet<u32>>::new();
    let mut cancelled = BTreeSet::new();
    for war in &save.war_relations {
        let first = super::saved_country_slot(&war.first, save, countries)?;
        let second = super::saved_country_slot(&war.second, save, countries)?;
        if war.cancel {
            cancelled.extend([first, second]);
            continue;
        }
        expected.entry(first).or_default().insert(second);
        expected.entry(second).or_default().insert(first);
    }
    let empty = BTreeSet::new();
    for (tag, country) in &countries.by_tag {
        let slot = country.canonical_slot;
        let tag = String::from_utf8_lossy(tag);
        if let Some(&read) = read.get(&slot) {
            let recorded: Vec<u32> = vectors
                .get(&slot)
                .map(|vector| vector.values().copied().collect())
                .unwrap_or_default();
            if recorded != read {
                report.difference(
                    format!("{tag} air enemies against CountryWars"),
                    recorded,
                    read,
                );
            }
            report.limitation(
                "CountryWars read this country's enemy vector because it can lag the saved wars (a civil-war revolter whose transferred wars were never rebuilt into it, or the owner of a retired war relation), so it is compared exactly with that read, not with the saved wars",
                tag,
            );
            continue;
        }
        if cancelled.contains(&slot) {
            report.limitation(
                "A cancelled saved war may still hold its status, so the enemy set of a country in one is not compared",
                tag,
            );
            continue;
        }
        let actual = enemies.get(&slot).unwrap_or(&empty);
        let wars = expected.get(&slot).unwrap_or(&empty);
        if actual != wars {
            report.difference(format!("{tag} air enemies"), actual, wars);
        }
    }
    Ok(rows.len())
}

fn contiguous<'a>(groups: impl Iterator<Item = &'a BTreeSet<u32>>, label: &str) -> io::Result<()> {
    for positions in groups {
        if positions.iter().copied().ne(0..positions.len() as u32) {
            return Err(invalid(format!("{label} positions are not contiguous")));
        }
    }
    Ok(())
}

fn witnesses(frame: &DecodedFrame, save: &Save, report: &mut Report) -> io::Result<()> {
    let saved: BTreeMap<_, _> = save.air.bases.iter().map(|base| (base.id, base)).collect();
    let manager: BTreeMap<_, _> = complete::<tables::AirBases>(frame)?
        .iter()
        .map(|base| (base.id, base))
        .collect();
    let mut probed: BTreeSet<u64> = complete::<tables::AirWings>(frame)?
        .iter()
        .flat_map(|wing| [wing.air_base, wing.base_override])
        .filter(|&handle| handle != 0)
        .collect();
    let mut compared = 0;
    for row in complete::<tables::AirReferenceWitnesses>(frame)? {
        let label = format!("air reference witness {}", row.handle);
        if !probed.remove(&row.handle) {
            report.difference(&label, "present", "unprobed");
            continue;
        }
        let Some(expected) = saved.get(&row.handle) else {
            report.difference(&label, "resolved", "unsaved");
            continue;
        };
        let (kind, state, host) = match row.base_kind {
            Some(kind) => (kind, row.state, row.host.unwrap_or(0)),
            None => {
                let base = manager.get(&row.handle).ok_or_else(|| {
                    invalid("handle-only air reference witness has no AirBases row")
                })?;
                (base.kind, base.state, base.host)
            }
        };
        if row.host_without_task_force && (row.host_owner.is_some() || state.is_some() || host == 0)
        {
            return Err(invalid(format!(
                "{label} marks a null host task force on a base without a resolved host"
            )));
        }
        compared += 1;
        if (kind, state) != (expected.kind, expected.state) {
            report.difference(
                format!("{label}.kind/state"),
                format!("{kind}/{state:?}"),
                format!("{}/{:?}", expected.kind, expected.state),
            );
        }
        if let Some(expected) = expected.host
            && host != expected
        {
            report.difference(format!("{label}.host"), host, expected);
        }
    }
    for handle in probed {
        if saved.contains_key(&handle) {
            report.difference(
                format!("air reference witness {handle}"),
                "unresolved",
                "saved",
            );
        }
    }
    report.compared("AirReferenceWitnesses", compared);
    Ok(())
}

fn mines(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::NavalRegionalMines>(frame)?;
    let mut navies: BTreeMap<u32, Vec<replay_data::rows::NavalRegionalMineRow>> = BTreeMap::new();
    for row in rows {
        let owner = countries
            .links
            .get(&row.country)
            .ok_or_else(|| invalid("regional mine country has no live link"))?;
        navies.entry(owner.canonical_slot).or_default().push(*row);
    }
    let mut compared = 0;
    for (tag, country) in &countries.by_tag {
        if country.canonical_slot == 0 {
            continue;
        }
        let label = format!("naval mines {}", String::from_utf8_lossy(tag));
        let Some(rows) = navies.remove(&country.canonical_slot) else {
            report.difference(label, "owner witness absent", "owner witness present");
            continue;
        };
        let (count, positive) = project_mines(&rows)?;
        let saved = save
            .countries
            .get(*tag)
            .ok_or_else(|| invalid("regional mine country absent from save"))?;
        match (count, saved.naval_mines.as_ref()) {
            (0, None) => {}
            (0, Some(_)) => {
                report.difference(format!("{label}.per_region_mines"), "absent", "present")
            }
            (_, None) => {
                report.difference(format!("{label}.per_region_mines"), "present", "absent")
            }
            (_, Some(saved)) => {
                let saved: BTreeMap<_, _> = saved
                    .iter()
                    .map(|(&region, value)| (region, value.raw))
                    .collect();
                if positive != saved {
                    report.difference(label, positive, saved);
                }
            }
        }
        compared += 1;
    }
    if !navies.is_empty() {
        return Err(invalid(
            "regional mine witness names no saved positive country",
        ));
    }
    report.compared("NavalRegionalMines", compared);
    report.limitation("The mine vector length and nonpositive native values are not serialized; match the complete raw vector and its count with the native observation, including negative values allowed by content.","NavalRegionalMines.region/amount");
    Ok(())
}

fn project_mines(
    rows: &[replay_data::rows::NavalRegionalMineRow],
) -> io::Result<(u32, BTreeMap<u32, i64>)> {
    let mut count = None;
    let mut values = BTreeMap::new();
    for row in rows {
        match row.kind {
            0 if count.is_none() && row.amount == 0 => count = Some(row.region),
            1 if row.amount != 0 && !values.contains_key(&row.region) => {
                values.insert(row.region, row.amount);
            }
            _ => {
                return Err(invalid(
                    "regional mine vector has duplicate or malformed rows",
                ));
            }
        }
    }
    let count = count.ok_or_else(|| invalid("regional mine completeness witness missing"))?;
    if values.keys().any(|&region| region >= count) {
        return Err(invalid(
            "regional mine value lies outside its witnessed vector",
        ));
    }
    values.retain(|_, amount| *amount > 0);
    Ok((count, values))
}

#[cfg(test)]
#[path = "air_missions/witness_tests.rs"]
mod witness_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patrol_air_stat_oracle_distinguishes_execution_demand_and_independent_faults() {
        use replay_data::rows::AirMissionInputRow;
        let row = AirMissionInputRow {
            current_type: (Some(8)).into(),
            effective_aircraft: (Some(-3)).into(),
            detection_stat_13: (Some(i64::MIN)).into(),
            detection_stat_14: (Some(0)).into(),
            ..Default::default()
        };
        assert!(mission_detection(&row).is_ok());
        assert!(
            mission_detection(&AirMissionInputRow {
                detection_stat_14: (None).into(),
                ..row.clone()
            })
            .is_err()
        );
        assert!(
            mission_detection(&AirMissionInputRow {
                effective_aircraft: (Some(0)).into(),
                ..row.clone()
            })
            .is_err()
        );
        assert!(
            mission_detection(&AirMissionInputRow {
                effective_aircraft: (Some(0)).into(),
                detection_stat_13: (None).into(),
                detection_stat_14: (None).into(),
                ..row.clone()
            })
            .is_ok()
        );
        assert!(
            mission_detection(&AirMissionInputRow {
                current_type: (Some(3)).into(),
                detection_stat_13: (None).into(),
                detection_stat_14: (None).into(),
                ..row
            })
            .is_ok()
        );
    }

    #[test]
    fn wing_fuel_oracle_checks_idle_and_shared_execution_without_selected_missions() {
        use replay_data::rows::AirMissionInputRow;
        use replay_data::rows::AirWingFuelInputs;
        use replay_data::rows::TableId;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
        *frame.tables[TableId::AceRangeInputs.index()] =
            TableView::Complete(TableRows::AceRangeInputs(Vec::new()));
        *frame.tables[TableId::AirMissionInputs.index()] =
            TableView::Complete(TableRows::AirMissionInputs(Vec::new()));
        let row = |input: AirWingFuelInputs| AirWingRow {
            id: 1,
            fuel_base_raw: input.base_fuel.into(),
            fuel_current_type: input.current_type.into(),
            fuel_effective_aircraft: input.effective_aircraft.into(),
            fuel_context_mode: input.context_mode.map(u32::from).into(),
            fuel_capacity_class: input.capacity_class.into(),
            fuel_override_resolved: input.override_resolved.map(u32::from).into(),
            ..Default::default()
        };
        let idle = replay_data::rows::AirWingFuelInputs {
            base_fuel: Some(123_456),
            current_type: Some(0),
            ..Default::default()
        };
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![row(idle)]));
        assert_eq!(wing_fuel_inputs(&frame).unwrap(), 1);
        let executing = AirWingFuelInputs {
            current_type: Some(4),
            effective_aircraft: Some(0),
            context_mode: Some(2),
            ..idle
        };
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![row(executing)]));
        assert_eq!(wing_fuel_inputs(&frame).unwrap(), 1);
        *frame.tables[TableId::AirMissionInputs.index()] =
            TableView::Complete(TableRows::AirMissionInputs(vec![AirMissionInputRow {
                wing: 1,
                current_type: (Some(4)).into(),
                effective_aircraft: (Some(1)).into(),
                ..Default::default()
            }]));
        assert!(
            wing_fuel_inputs(&frame)
                .unwrap_err()
                .to_string()
                .contains("shared execution")
        );
        *frame.tables[TableId::AirMissionInputs.index()] =
            TableView::Complete(TableRows::AirMissionInputs(Vec::new()));
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![row(AirWingFuelInputs {
                effective_aircraft: Some(0),
                ..idle
            })]));
        assert!(
            wing_fuel_inputs(&frame)
                .unwrap_err()
                .to_string()
                .contains("demand mismatch")
        );
    }

    #[test]
    fn combat_modifier_oracle_checks_ace_living_demand_and_signed_absence() {
        use replay_data::rows::AceRangeInputRow;
        use replay_data::rows::TableId;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
        *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(vec![]));
        *frame.tables[TableId::AirMissionInputs.index()] =
            TableView::Complete(TableRows::AirMissionInputs(vec![]));
        let row = AceRangeInputRow {
            ace: 7,
            alive: 1,
            fuel_modifier: Some([None]).into(),
            combat_modifiers_known: true.into(),
            type_combat_flags: 5.into(),
            attack_modifier_raw: Some(i64::MIN).into(),
            ..Default::default()
        };
        for (row, valid) in [
            (row.clone(), true),
            (
                AceRangeInputRow {
                    combat_modifiers_known: false.into(),
                    ..row.clone()
                },
                false,
            ),
            (
                AceRangeInputRow {
                    type_combat_flags: 0.into(),
                    ..row.clone()
                },
                false,
            ),
            (
                AceRangeInputRow {
                    type_combat_flags: 9.into(),
                    ..row.clone()
                },
                false,
            ),
            (
                AceRangeInputRow {
                    alive: 0,
                    fuel_modifier: None.into(),
                    combat_modifiers_known: false.into(),
                    type_combat_flags: 0.into(),
                    ..row
                },
                false,
            ),
            (
                AceRangeInputRow {
                    ace: 7,
                    ..Default::default()
                },
                true,
            ),
        ] {
            *frame.tables[TableId::AceRangeInputs.index()] =
                TableView::Complete(TableRows::AceRangeInputs(vec![row]));
            assert_eq!(wing_fuel_inputs(&frame).is_ok(), valid);
        }
    }

    #[test]
    fn current_execution_oracle_requires_exact_demand_independently_of_selected_mask() {
        use replay_data::rows::AirMissionInputRow;
        use replay_data::rows::AirRegionalMissionRow;
        use replay_data::rows::TableId;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
        saved.air_wings.insert(
            1,
            content_reader_save::AirWing {
                government_in_exile_tag: None,
                manpower: None,
                reinforcement_setting: None,
                id: 1,
                owner: b"AAA".to_vec(),
                name: Some(Vec::new()),
                country_override: b"AAA".to_vec(),
                air_base: 3,
                planes: 1,
                mission_mask: 0,
                mission_region: None,
                mission_active: Some(false),
                stop_training_at_max_xp: false,
                region_change_penalty: Some(0),
                transfer_progress: None,
                transferring_to: None,
                pool_ordinal: 0,
            },
        );
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                id: 1,
                ..Default::default()
            }]));
        *frame.tables[TableId::AirRegionalMissions.index()] =
            TableView::Complete(TableRows::AirRegionalMissions(vec![
                AirRegionalMissionRow {
                    wing: 1,
                    ..Default::default()
                },
            ]));
        for (mask, count, expected) in [
            (None, None, false),
            (Some(0), None, true),
            (Some(0), Some(0), false),
            (Some(8), None, false),
            (Some(8), Some(-4), true),
        ] {
            *frame.tables[TableId::AirMissionInputs.index()] =
                TableView::Complete(TableRows::AirMissionInputs(vec![AirMissionInputRow {
                    wing: 1,
                    current_type: (mask).into(),
                    effective_aircraft: (count).into(),
                    patrol_disruption_raw: ((mask.is_some_and(|mask| {
                        mask as i32 > 0 && mask.is_power_of_two() && mask & 0x1f7fa != 0
                    }) && count.is_some_and(|count| count != 0))
                    .then_some(0))
                    .into(),
                    detection_stat_13: ((mask
                        .is_some_and(|mask| mask as i32 > 0 && mask.is_power_of_two())
                        && count.is_some_and(|count| count != 0))
                    .then_some(0))
                    .into(),
                    detection_stat_14: ((mask
                        .is_some_and(|mask| mask as i32 > 0 && mask.is_power_of_two())
                        && count.is_some_and(|count| count != 0))
                    .then_some(0))
                    .into(),
                    normalization: ((mask.is_some_and(|mask| mask != 0)
                        && count.is_some_and(|count| count != 0))
                    .then_some(replay_data::rows::AirMissionNormalization {
                        night_stat: Some(0),
                        modifiers: [Some(replay_data::rows::AirNormalizationEntry::default()); 2],
                    }))
                    .into(),
                    ..Default::default()
                }]));
            let mut report = Report::default();
            assert_eq!(
                mission_inputs(&frame, &saved, &[100_000; 32], &mut report).is_ok(),
                expected
            );
            assert!(report.differences.is_empty());
        }
    }

    #[test]
    fn incoming_transfer_oracle_compares_progress_and_rejects_unknown_order() {
        use replay_data::rows::AirBaseRow;
        use replay_data::rows::AirTransferInputRow;
        use replay_data::rows::TableId;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
        saved.air_wings.clear();
        saved.air_wings.insert(
            1,
            content_reader_save::AirWing {
                government_in_exile_tag: None,
                manpower: None,
                reinforcement_setting: None,
                id: 1,
                owner: b"AAA".to_vec(),
                name: Some(Vec::new()),
                country_override: b"AAA".to_vec(),
                air_base: 3,
                planes: 1,
                mission_mask: 0,
                mission_region: None,
                mission_active: Some(false),
                stop_training_at_max_xp: false,
                region_change_penalty: None,
                transfer_progress: Some(123_456),
                transferring_to: Some(9),
                pool_ordinal: 0,
            },
        );
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                id: 1,
                air_base: 3,
                base_override: 7,
                ..Default::default()
            }]));
        *frame.tables[TableId::AirBases.index()] =
            TableView::Complete(TableRows::AirBases(vec![AirBaseRow {
                id: 7,
                state: Some(9),
                ..Default::default()
            }]));
        for (ordinal, progress, valid, differences) in [
            (Some(0), Some(123_456), true, 0),
            (Some(0), Some(123_457), true, 1),
            (None, Some(123_456), false, 0),
            (Some(0), None, false, 0),
        ] {
            *frame.tables[TableId::AirTransferInputs.index()] =
                TableView::Complete(TableRows::AirTransferInputs(vec![AirTransferInputRow {
                    wing: 1,
                    base: 7,
                    ordinal,
                    progress_raw: progress,
                    distance_raw: Some(987_654),
                }]));
            let mut report = Report::default();
            assert_eq!(transfers(&frame, &saved, &mut report).is_ok(), valid);
            assert_eq!(report.differences.len(), differences);
        }
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![AirWingRow {
                id: 1,
                air_base: 3,
                base_override: 7,
                eligibility_counter: 2,
                eligibility_limit: 1,
                ..Default::default()
            }]));
        saved.air_wings.get_mut(&1).unwrap().transferring_to = Some(10);
        let mut report = Report::default();
        transfers(&frame, &saved, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        *frame.tables[TableId::AirTransferInputs.index()] =
            TableView::Complete(TableRows::AirTransferInputs(Vec::new()));
        assert!(transfers(&frame, &saved, &mut Report::default()).is_err());
    }
    #[test]
    fn global_order_labels_are_independent_but_country_wide_and_unique() {
        use replay_data::rows::AirRegionalMissionRow;
        use replay_data::rows::TableId;
        use replay_data::rows::TableRows;
        use replay_frame::frame::TableView;
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
        *frame.tables[TableId::AirWings.index()] = TableView::Complete(TableRows::AirWings(
            [1, 2]
                .map(|id| AirWingRow {
                    id,
                    owner: 102,
                    mission_region: Some(9),
                    ..Default::default()
                })
                .into(),
        ));
        for (labels, expected) in [
            ([None, None], true),
            ([Some(20), Some(10)], true),
            ([Some(10), Some(10)], false),
            ([Some(0), Some(10)], false),
            ([None, Some(10)], false),
        ] {
            *frame.tables[TableId::AirRegionalMissions.index()] =
                TableView::Complete(TableRows::AirRegionalMissions(
                    labels
                        .into_iter()
                        .enumerate()
                        .map(|(at, active_order)| AirRegionalMissionRow {
                            wing: at as u64 + 1,
                            ordinal: at as u32,
                            active_order,
                        })
                        .collect(),
                ));
            let context = super::super::country_context(&frame).unwrap();
            assert_eq!(
                registrations(&frame, &context).is_ok(),
                expected,
                "{labels:?}"
            );
        }
    }
    use replay_data::{
        rows::AirCountryRelationRow, rows::AirNavalPatrolHistoryRow, rows::CountryLinkRow,
        rows::CountryWarRow, rows::TableId, rows::TableRows,
    };
    use replay_frame::frame::TableView;

    #[test]
    fn ground_crews_compare_saved_flags_including_sparse_false_and_optional_reads() {
        let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
        *frame.tables[TableId::AirWings.index()] =
            TableView::Complete(TableRows::AirWings(vec![replay_data::rows::AirWingRow {
                id: 123,
                owner: 101,
                country_override: 999,
                mission_region: Some(9),
                ..Default::default()
            }]));
        *frame.tables[TableId::AirRegionalMissions.index()] =
            TableView::Complete(TableRows::AirRegionalMissions(vec![
                replay_data::rows::AirRegionalMissionRow {
                    wing: 123,
                    ordinal: 0,
                    ..Default::default()
                },
            ]));
        saved
            .air
            .ground_crews
            .push(content_reader_save::air::GroundCrews {
                country: b"GER".to_vec(),
                region: 9,
                enabled: true,
            });
        for (enabled, difference, incomplete) in
            [(Some(1), 0, false), (Some(0), 1, false), (None, 0, true)]
        {
            *frame.tables[TableId::AirGroundCrews.index()] =
                TableView::Complete(TableRows::AirGroundCrews(vec![
                    replay_data::rows::AirGroundCrewRow {
                        supply: Recorded::Value(None),
                        country: 7,
                        region: 9,
                        enabled,
                    },
                ]));
            let context = super::super::country_context(&frame).unwrap();
            let mut report = Report::default();
            ground_crews(&frame, &saved, &context, &mut report).unwrap();
            assert_eq!(report.differences.len(), difference);
            assert_eq!(
                report.failed_comparisons.contains_key("AirGroundCrews"),
                incomplete
            );
            assert_eq!(report.outcome().is_err(), difference != 0 || incomplete);
        }
        *frame.tables[TableId::AirGroundCrews.index()] =
            TableView::Complete(TableRows::AirGroundCrews(Vec::new()));
        let context = super::super::country_context(&frame).unwrap();
        let mut report = Report::default();
        ground_crews(&frame, &saved, &context, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        saved.air.ground_crews.clear();
        let mut report = Report::default();
        ground_crews(&frame, &saved, &context, &mut report).unwrap();
        assert!(report.differences.is_empty());
    }

    #[test]
    fn naval_patrol_histories_are_sparse_nonzero_entries() {
        let slots = BTreeSet::from([4]);
        let row = |index, sunk, lost| AirNavalPatrolHistoryRow {
            country: 4,
            region: 9,
            index,
            enemy_ships_sunk_raw: sunk,
            our_fighters_lost_raw: lost,
            scenario_channels: Recorded::NotRecorded,
            menu: Default::default(),
            ..Default::default()
        };
        let marker = AirNavalPatrolHistoryRow::window_marker(
            4,
            9,
            0,
            replay_data::rows::AirHistoryWindow {
                complete: false,
                capacity: 30,
                current: 0,
                wrapped: true,
            },
            false,
        );
        let witness = AirNavalPatrolHistoryRow::availability_witness(4);
        assert!(
            histories_structure(&[row(0, 5, 0), row(7, 0, -2), marker, witness], &slots).is_ok()
        );
        assert!(histories_structure(&[row(3, 0, 0)], &slots).is_err());
        let foreign = AirNavalPatrolHistoryRow {
            country: 5,
            ..row(0, 1, 1)
        };
        assert!(histories_structure(&[foreign], &slots).is_err());
    }

    #[test]
    fn scenario_histories_validate_empty_witnesses_and_new_channel_only_records() {
        let slots = BTreeSet::from([4]);
        let witness = replay_data::rows::AirNavalPatrolHistoryRow::availability_witness;
        let row = AirNavalPatrolHistoryRow {
            country: 4,
            region: 9,
            index: 7,
            scenario_channels: Recorded::Value(Some([0, 0, 0, 0, 0, -2, 0])),
            ..Default::default()
        };
        assert!(histories_structure(&[witness(0), witness(4)], &slots).is_ok());
        let marker = AirNavalPatrolHistoryRow::window_marker(
            4,
            9,
            0,
            replay_data::rows::AirHistoryWindow {
                complete: false,
                capacity: 30,
                current: 0,
                wrapped: true,
            },
            false,
        );
        assert!(histories_structure(&[row.clone(), marker, witness(4)], &slots).is_ok());
        assert!(histories_structure(&[row], &slots).is_err());
        assert!(histories_structure(&[witness(4), witness(4)], &slots).is_err());
        let mut malformed = witness(4);
        malformed.scenario_channels = Recorded::NotRecorded;
        assert!(histories_structure(&[malformed], &slots).is_err());
        let mut malformed = witness(4);
        malformed.enemy_ships_sunk_raw = 1;
        assert!(histories_structure(&[malformed], &slots).is_err());
        let mut malformed = witness(4);
        malformed.region = 9;
        assert!(histories_structure(&[malformed], &slots).is_err());
    }

    #[test]
    fn current_scenario_refusals_remain_independent_named_history_rows() {
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
        let mut rows = vec![
            AirNavalPatrolHistoryRow {
                country: 7,
                region: 9,
                index: 0,
                scenario_channels: Recorded::Value(None),
                ..Default::default()
            },
            AirNavalPatrolHistoryRow {
                country: 7,
                region: 9,
                index: 1,
                enemy_ships_sunk_raw: 300_000,
                our_fighters_lost_raw: 200_000,
                scenario_channels: Recorded::Value(None),
                menu: Default::default(),
                ..Default::default()
            },
            AirNavalPatrolHistoryRow {
                country: 7,
                region: 8,
                index: 0,
                scenario_channels: Recorded::Value(Some([1; 7])),
                ..Default::default()
            },
            AirNavalPatrolHistoryRow::availability_witness(7),
        ];
        rows.push(AirNavalPatrolHistoryRow::window_marker(
            7,
            8,
            0,
            replay_data::rows::AirHistoryWindow {
                complete: false,
                capacity: 30,
                current: 0,
                wrapped: true,
            },
            false,
        ));
        rows.push(AirNavalPatrolHistoryRow::window_marker(
            7,
            9,
            0,
            replay_data::rows::AirHistoryWindow {
                complete: false,
                capacity: 30,
                current: 0,
                wrapped: true,
            },
            false,
        ));
        rows.sort_unstable_by_key(|row| (row.country, row.region, row.level, row.index));
        assert!(histories_structure(&rows, &BTreeSet::from([7])).is_ok());
        *frame.tables[TableId::AirNavalPatrolHistories.index()] =
            TableView::Complete(TableRows::AirNavalPatrolHistories(rows));
        let context = super::super::country_context(&frame).unwrap();
        let mut report = Report::default();
        assert_eq!(naval_patrol(&frame, &context, &mut report).unwrap(), (0, 6));
        assert_eq!(
            report.limitations["AirNavalPatrolHistories.scenario_channels unavailable after a current read refusal; original naval-patrol history remains readable"],
            ["air history 7:9:0", "air history 7:9:1"]
        );
    }

    fn add_country(frame: &mut DecodedFrame, saved: &mut Save, id: u32, slot: u32, tag: &[u8]) {
        let mut country = saved.countries[b"GER".as_slice()].clone();
        country.tag = tag.to_vec();
        saved.countries.insert(tag.to_vec(), country);
        let TableView::Complete(TableRows::Countries(rows)) =
            &mut *frame.tables[TableId::Countries.index()]
        else {
            unreachable!()
        };
        let mut row = rows[0].clone();
        row.id = id;
        row.canonical_slot = slot;
        row.tag = tag.to_vec();
        rows.push(row);
        let TableView::Complete(TableRows::CountryLinks(rows)) =
            &mut *frame.tables[TableId::CountryLinks.index()]
        else {
            unreachable!()
        };
        rows.push(CountryLinkRow {
            id,
            canonical_slot: slot,
        });
    }

    fn enemies(frame: &mut DecodedFrame, rows: &[(u32, &[u32])]) {
        *frame.tables[TableId::AirCountryRelations.index()] =
            TableView::Complete(TableRows::AirCountryRelations(
                rows.iter()
                    .flat_map(|&(country, others)| {
                        others.iter().enumerate().map(move |(ordinal, &other)| {
                            AirCountryRelationRow {
                                country,
                                kind: AirCountryRelationRow::ENEMY,
                                ordinal: ordinal as u32,
                                other,
                            }
                        })
                    })
                    .collect(),
            ));
    }

    fn wars(frame: &mut DecodedFrame, rows: Vec<CountryWarRow>) {
        *frame.tables[TableId::CountryWars.index()] =
            TableView::Complete(TableRows::CountryWars(rows));
    }

    fn run(frame: &DecodedFrame, saved: &Save) -> Report {
        let context = super::super::country_context(frame).unwrap();
        let mut report = Report::default();
        compare(
            frame,
            saved,
            &context,
            &Ok(vec![100_000; u32::BITS as usize]),
            &mut report,
        );
        assert!(
            report.failed_comparisons.is_empty(),
            "{:?}",
            report.failed_comparisons
        );
        report
    }

    #[test]
    fn an_unrebuilt_revolter_vector_is_held_to_the_country_wars_read() {
        let (mut frame, mut saved, _, _) = super::super::fixtures::fixture();
        *frame.tables[TableId::AirSortieModifiers.index()] =
            TableView::Complete(TableRows::AirSortieModifiers(vec![AirSortieModifierRow {
                scope: AirSortieModifierRow::DETECTION_OVERRIDE_STATUS,
                raw: 1,
                ..Default::default()
            }]));
        *frame.tables[TableId::AirSortieModifierMetadata.index()] =
            TableView::Complete(TableRows::AirSortieModifierMetadata(vec![
                Default::default(),
            ]));
        add_country(&mut frame, &mut saved, 201, 8, b"REB");
        add_country(&mut frame, &mut saved, 301, 9, b"SOV");
        *frame.tables[TableId::NavalRegionalMines.index()] =
            TableView::Complete(TableRows::NavalRegionalMines(
                [101, 201, 301]
                    .into_iter()
                    .map(|country| replay_data::rows::NavalRegionalMineRow {
                        country,
                        ..Default::default()
                    })
                    .collect(),
            ));
        saved.war_relations.push(content_reader_save::WarRelation {
            participation: [None, None],
            first_goals: Vec::new(),
            second_goals: Vec::new(),
            roster_owner: b"REB".to_vec(),
            target: b"SOV".to_vec(),
            first: b"REB".to_vec(),
            second: b"SOV".to_vec(),
            start_date: Some(saved.date),
            end_date: None,
            cancel: false,
            first_casualties: 0,
            second_casualties: 0,
            first_unknown_casualties: 0,
            second_unknown_casualties: 0,
            first_was_instigator: false,
        });
        enemies(&mut frame, &[(9, &[201])]);
        let revolter = |enemies: Vec<u32>| CountryWarRow {
            country: 201,
            can_occupy_non_war: 0,
            enemies_read: 1,
            enemies,
            ordered: Default::default(),
        };
        let limited = |report: &Report| {
            report
                .limitations
                .iter()
                .filter(|(reason, _)| reason.starts_with("CountryWars read"))
                .flat_map(|(_, subjects)| subjects.clone())
                .collect::<Vec<_>>()
        };

        wars(&mut frame, Vec::new());
        let report = run(&frame, &saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("REB air enemies"));

        wars(&mut frame, vec![revolter(Vec::new())]);
        let report = run(&frame, &saved);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(limited(&report), ["REB"]);

        wars(&mut frame, vec![revolter(vec![301])]);
        let report = run(&frame, &saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("REB air enemies against CountryWars"));
        enemies(&mut frame, &[(9, &[201]), (8, &[301, 101])]);
        wars(&mut frame, vec![revolter(vec![101, 301])]);
        let report = run(&frame, &saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        wars(&mut frame, vec![revolter(vec![301, 101])]);
        let report = run(&frame, &saved);
        assert!(report.differences.is_empty(), "{:?}", report.differences);

        enemies(&mut frame, &[(8, &[301, 101])]);
        wars(
            &mut frame,
            vec![
                revolter(vec![301, 101]),
                CountryWarRow {
                    country: 301,
                    can_occupy_non_war: 1,
                    enemies_read: 0,
                    enemies: Vec::new(),
                    ordered: Default::default(),
                },
            ],
        );
        let report = run(&frame, &saved);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("SOV air enemies"));
    }

    #[test]
    fn full_mine_projection_requires_witness_and_matches_positive_only_serialization() {
        use replay_data::rows::NavalRegionalMineRow as Row;
        let witness = Row {
            country: 1,
            kind: 0,
            region: 4,
            amount: 0,
        };
        let positive = Row {
            country: 1,
            kind: 1,
            region: 2,
            amount: 250_000,
        };
        let negative = Row {
            country: 1,
            kind: 1,
            region: 3,
            amount: -9,
        };
        assert_eq!(
            project_mines(&[witness, positive, negative]).unwrap(),
            (4, BTreeMap::from([(2, 250_000)]))
        );
        assert!(project_mines(&[positive]).is_err());
        assert!(project_mines(&[witness, witness]).is_err());
        assert!(
            project_mines(&[
                witness,
                Row {
                    region: 4,
                    ..positive
                }
            ])
            .is_err()
        );
        assert_eq!(
            project_mines(&[Row {
                region: 0,
                ..witness
            }])
            .unwrap(),
            (0, BTreeMap::new())
        );
    }
}

#[cfg(test)]
mod country_air_modifier_tests {
    use super::*;
    use replay_data::rows::CountryCounterInputRow;
    use replay_data::rows::TableId;
    use replay_data::rows::TableRows;
    use replay_frame::frame::TableView;

    #[test]
    fn combat_country_census_distinguishes_unknown_from_known_absent_keys() {
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
        for (known, raw, valid) in [
            (false, None, false),
            (true, None, true),
            (true, Some(i64::MIN), true),
        ] {
            *frame.tables[TableId::CountryCounterInputs.index()] =
                TableView::Complete(TableRows::CountryCounterInputs(vec![
                    CountryCounterInputRow {
                        country: 1,
                        air_modifiers: Recorded::Value(Some([None; 10])),
                        air_combat_modifiers_known: (known).into(),
                        air_attack_factor: (raw).into(),
                        ..Default::default()
                    },
                ]));
            assert_eq!(country_air_modifiers(&frame).is_ok(), valid);
        }
    }

    #[test]
    fn absent_keys_are_complete_but_an_unreadable_country_packet_fails() {
        let (mut frame, _, _, _) = super::super::fixtures::fixture();
        *frame.tables[TableId::CountryCounterInputs.index()] =
            TableView::Complete(TableRows::CountryCounterInputs(vec![
                CountryCounterInputRow {
                    country: 1,
                    air_modifiers: Recorded::Value(Some([None; 10])),
                    air_combat_modifiers_known: true.into(),
                    ..Default::default()
                },
            ]));
        assert_eq!(country_air_modifiers(&frame).unwrap(), 1);
        let TableView::Complete(TableRows::CountryCounterInputs(rows)) =
            &mut *frame.tables[TableId::CountryCounterInputs.index()]
        else {
            unreachable!()
        };
        rows[0].air_modifiers = Recorded::Value(None);
        assert!(
            country_air_modifiers(&frame)
                .unwrap_err()
                .to_string()
                .contains("country 1 Air modifier accumulator unreadable")
        );
    }
}
