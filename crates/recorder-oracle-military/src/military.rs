use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_save::Save;
use recorder_oracle_support::support::equal;
use replay_data::{
    rows::CountryRow, rows::DivisionMovementRow, rows::DivisionRow, rows::DivisionStatusRow,
    rows::DivisionSupplyFooter, rows::Recorded, rows::TableId, rows::tables,
};
use replay_frame::frame::DecodedFrame;

use super::{
    CountryContext, Report, TokenNames, complete, country_tag, invalid, required_country_tag,
    tag_text,
};

mod air_transport;
mod division_history;
mod movement_operands;
mod ship_names;
#[cfg(test)]
mod tests;
mod unit_bonuses;
pub use air_transport::compare as compare_air_transport_inputs;
pub use movement_operands::compare as compare_movement_operands;
pub use unit_bonuses::compare as compare_country_unit_bonuses;

pub fn compare_land_combats(
    frame: &DecodedFrame,
    save: &Save,
    membership: &content_reader_save::PartialMapMembership,
    report: &mut Report,
) -> io::Result<()> {
    let mut combats = BTreeMap::new();
    for combat in frame.complete::<tables::LandCombats>()? {
        if combat.id == 0 || combats.insert(combat.id, combat).is_some() {
            return Err(invalid("null or duplicate full land combat handle"));
        }
    }
    for (&id, expected) in &save.land_combats {
        let label = format!("land combat type={} id={}", id as u32, id >> 32);
        if expected.province == 0 || !membership.contains_province(expected.province) {
            return Err(invalid(
                "saved land combat location is absent from verified map",
            ));
        }
        let actual = combats.remove(&id);
        let Some(actual) = actual else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.land_combats += 1;
        if actual.province != expected.province {
            report.difference(
                format!("{label}.province"),
                actual.province,
                expected.province,
            );
        }
        if actual.day != expected.day {
            report.difference(format!("{label}.day"), actual.day, expected.day);
        }
        if actual.duration != expected.duration {
            report.difference(
                format!("{label}.duration"),
                actual.duration,
                expected.duration,
            );
        }
        for (side, actual, expected) in [
            ("attackers", &actual.attackers, &expected.attackers),
            ("defenders", &actual.defenders, &expected.defenders),
        ] {
            if actual != expected {
                report.difference(format!("{label}.{side}"), actual, expected);
            }
        }
        for (side, classes, members) in [
            (
                "attacker_classes",
                &actual.attacker_classes,
                &expected.attackers,
            ),
            (
                "defender_classes",
                &actual.defender_classes,
                &expected.defenders,
            ),
        ] {
            let expected: Vec<_> = members.iter().map(|&unit| unit_class(save, unit)).collect();
            let matches = classes.len() == expected.len()
                && classes
                    .iter()
                    .zip(&expected)
                    .all(|(&class, expected)| expected.is_none_or(|expected| expected == class));
            if !matches {
                report.difference(format!("{label}.{side}"), classes, expected);
            }
        }
    }
    for &id in combats.keys() {
        report.difference(
            format!("land combat type={} id={}", id as u32, id >> 32),
            "present",
            "absent",
        );
    }
    report.limitation("land combat comparison covers recorded locations, native day/duration, ordered side participants and their unit classes; reduced saved combat details cannot certify intervening combat events", "LandCombats");
    Ok(())
}

fn unit_class(save: &Save, handle: u64) -> Option<u32> {
    if save.divisions.contains_key(&handle) {
        Some(0)
    } else if save.task_forces.contains_key(&handle) {
        Some(1)
    } else if save.railway_guns.contains_key(&handle) {
        Some(13)
    } else {
        None
    }
}

pub fn compare_battle_sounds(
    frame: &DecodedFrame,
    membership: &content_reader_save::PartialMapMembership,
    report: &mut Report,
) -> io::Result<()> {
    let rows = frame.complete::<tables::BattleSounds>()?;
    for (place, row) in rows.iter().enumerate() {
        let label = format!("battle sound {}", row.ordinal);
        if row.ordinal as usize != place {
            report.difference(&label, row.ordinal, format!("list place {place}"));
        }
        for &province in &row.provinces {
            if !membership.contains_province(province) {
                report.difference(
                    format!("{label}.provinces"),
                    province,
                    "a province of the verified map",
                );
            }
        }
    }
    report.compared("BattleSounds", rows.len());
    report.limitation(
        "Saves hold no battle-sound list: the battle ambience's battles, their circles, members and sounding flags are the recording client's runtime sound state (a merge keeps the battle sounding for that client); compare them against paused memory at the sampled hour",
        "BattleSounds",
    );
    Ok(())
}

pub fn compare_division_movement(
    frame: &DecodedFrame,
    save: &Save,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::DivisionMovement>(frame)?;
    let panels = frame.complete::<tables::DivisionPanelStats>().ok();
    let mut seen = BTreeSet::new();
    let mut base_speeds = 0;
    for row in rows {
        if row.division == 0 || !seen.insert(row.division) {
            return Err(invalid("null or duplicate movement division"));
        }
        let Some(saved) = save.divisions.get(&row.division) else {
            report.difference(
                format!("division movement {}", row.division),
                "present",
                "absent",
            );
            continue;
        };
        let saved = &saved.presentation;
        let label = format!("division movement {}", row.division);
        if (row.path_total as i32) > 0 {
            let panel = panels.and_then(|rows| {
                rows.binary_search_by_key(&row.division, |panel| panel.division)
                    .ok()
                    .map(|at| &rows[at])
            });
            match row.base_speed(panel) {
                Some(_) => base_speeds += 1,
                None => report.difference(
                    format!("{label}.base_speed"),
                    "unavailable",
                    "required stat-block speed for a nonempty route",
                ),
            }
        }
        match report.recorded(&row.fuel, "DivisionMovement.fuel", &label) {
            Some(Some(fuel)) => {
                report.scalar_checks += 1;
                let saved = saved.fuel.as_ref().map_or(0, |fuel| fuel.raw);
                if *fuel != saved {
                    report.difference(format!("{label}.fuel"), *fuel, saved);
                }
            }
            Some(None) if row.path_total != 0 => report.difference(
                format!("{label}.fuel"),
                "unavailable",
                "required fuel for a nonempty route",
            ),
            _ => {}
        }
        report.scalar_checks += 5;
        if row.path != saved.path || row.path_total as usize != saved.path.len() {
            report.difference(
                format!("{label}.path/total"),
                (&row.path, row.path_total),
                (&saved.path, saved.path.len()),
            );
        }
        if row.target != saved.support_attack.unwrap_or(0) {
            report.difference(
                format!("{label}.target"),
                row.target,
                saved.support_attack.unwrap_or(0),
            );
        }
        if row.alternate_path != saved.full_path {
            report.difference(
                format!("{label}.alternate_path"),
                &row.alternate_path,
                &saved.full_path,
            );
        }
        if row.source != saved.previous.unwrap_or(0) {
            report.difference(
                format!("{label}.source"),
                row.source,
                saved.previous.unwrap_or(0),
            );
        }
        if (row.strategic_redeployment != 0) != saved.strategic_redeployment {
            report.difference(
                format!("{label}.strategic_redeployment"),
                row.strategic_redeployment,
                saved.strategic_redeployment,
            );
        }
        if let Some(value) = &saved.movement_progress {
            report.scalar(&label, "numerator", Some(row.numerator), Some(value));
        } else {
            report.scalar_checks += 1;
            if row.numerator != 0 {
                report.difference(
                    format!("{label}.omitted movement numerator"),
                    row.numerator,
                    0,
                );
            }
        }
        if let Some(value) = report.recorded(
            &row.withdrawal,
            "DivisionMovement.withdrawal",
            format!("{label}.withdrawal"),
        ) {
            report.scalar_checks += 1;
            match value {
                Some(raw) if (*raw != 0) == saved.withdrawal => {}
                Some(raw) => {
                    report.difference(format!("{label}.withdrawal"), *raw != 0, saved.withdrawal)
                }
                None => report.difference(
                    format!("{label}.withdrawal: guarded read refusal"),
                    "unavailable",
                    saved.withdrawal,
                ),
            }
        }
        if let Some(value) = report.recorded(
            &row.movement_priority,
            "DivisionMovement.movement_priority",
            format!("{label}.movement_priority"),
        ) {
            report.scalar_checks += 1;
            let expected = saved.movement_priority.unwrap_or(1);
            match value {
                Some(raw) => {
                    let serialized = match *raw {
                        0..=3 => *raw,
                        _ => {
                            report.limitation(
                                "the game writes front_order for an unknown priority word; the save cannot certify that word's exact bits",
                                format!("{label}.movement_priority raw {raw}"),
                            );
                            0
                        }
                    };
                    if serialized != expected {
                        report.difference(
                            format!("{label}.movement_priority"),
                            serialized,
                            expected,
                        );
                    }
                }
                None => report.difference(
                    format!("{label}.movement_priority: guarded read refusal"),
                    "unavailable",
                    expected,
                ),
            }
        }
        if let Some(value) = report.recorded(
            &row.queued_actions,
            "DivisionMovement.queued_actions",
            format!("{label}.queued_actions"),
        ) {
            report.scalar_checks += 1;
            match value {
                Some(count) if usize::try_from(*count) == Ok(saved.queued_actions) => {}
                Some(count) => report.difference(
                    format!("{label}.queued_actions"),
                    *count,
                    saved.queued_actions,
                ),
                None => report.difference(
                    format!("{label}.queued_actions: guarded read refusal"),
                    "unavailable",
                    saved.queued_actions,
                ),
            }
        }
        if row.path.len() != row.path_total as usize
            || row.alternate_path.len() != row.alternate_total as usize
        {
            report.difference(
                format!("{label}.whole routes"),
                (row.path.len(), row.alternate_path.len()),
                (row.path_total, row.alternate_total),
            );
        }
    }
    for id in save.divisions.keys().filter(|id| !seen.contains(id)) {
        report.difference(format!("division movement {id}"), "absent", "present");
    }
    report.compared("DivisionMovement", rows.len());
    report.limitation("base_speed is the raw signed fixed5 maximum speed of the division's stat block (the stat block's maximum-speed field, DivisionPanelStats' selector 0x3e), from which movement-speed calculation starts. The division writer does not save the stat block, so the save has no counterpart; saved template, equipment and traits do not certify the composed cache. Rows without a route are outside its demand; a nonempty route without it is a difference. Matched native observations remain necessary", format!("DivisionMovement.base_speed: {base_speeds} rows recorded, not compared"));
    report.limitation("the unit save writer saves withdraw only as zero/nonzero truth, not the raw nonzero byte identity", "DivisionMovement.withdrawal raw byte");
    report.limitation("effective strategic-action modifiers and per-row query failures have no text-save counterpart; the save omits the queried runtime accumulators. Saved route, target and strategic truth remain compared.", "DivisionMovement.strategic_action");
    report.limitation("remaining hours are estimated at playback, not stored: the recorded route, progress numerator and base_speed with matching-content distances and the recorded speed terms (the stat block's and commander's movement words, country and commander block modifier entries, location weather entries, supply, days out of supply, enemy air superiority, retreat, organisation and fuel), with the combat movement base of a division in combat, the location state's speed modifiers and controller relation, the priority sub-unit's and its categories' terms, the noncore-supply term, a relation's river crossing factor and the transport speed of a naval transfer or a move to or from the sea. A strategic redeployment's road and rail speed, the armour term and the special-forces share derive from recorded rows and content; the save has no arrival time to compare", "DivisionMovement arrival estimates");
    report.limitation("the movement speed's runtime operands have no save counterpart: DivisionStatus.commander_modifiers the entries of the commander block the HQ selects, DivisionStatus.combat_movement the combat's movement word, OrdersGroupCounterInputs.commander_movement the commander's terrain movement, CountryCounterInputs.movement_modifiers country accumulator entries, WeatherProvinces.modifiers and WeatherRegions.combat the weather modifiers' entries, SimulationFeatures.movement_defines loaded define globals and the runtime operand tables (MovementStateTerms, CoreSupplyAccess, CountryUnitSpeedEntries, RiverRelations and DivisionTransportSpeeds). Matched native observations remain necessary", "movement speed operands");
    report.limitation("retreat/transfer predicates and the first-combat role and province are not independently compared here; the viewer derives progress and the stack kind from them and content", "DivisionMovement unverified fields");
    report.limitation("map_entity records whether the map-entity pointer is set; the army and unit writers do not serialize it, so it has no save counterpart and needs a paused live-memory check (the combat wedge points from the first attacker that has one)", "DivisionMovement.map_entity");
    Ok(())
}

pub fn compare_division_panel_stats(
    frame: &DecodedFrame,
    save: &Save,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::DivisionPanelStats>(frame)?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.division == 0 || !seen.insert(row.division) {
            return Err(invalid("null or duplicate panel stats division"));
        }
        if !save.divisions.contains_key(&row.division) {
            report.difference(
                format!("DivisionPanelStats {}", row.division),
                "present",
                "absent from saved divisions",
            );
        }
        for (index, value) in row.panel_stats.iter().enumerate() {
            if value.is_none() {
                report.difference(
                    format!("DivisionPanelStats {}.panel_stats[{index}]", row.division),
                    "unavailable",
                    "complete deployed cache operand",
                );
            }
        }
        let suppression = match row.suppression {
            Recorded::Value(Some(_)) => None,
            Recorded::Value(None) => Some("unavailable"),
            Recorded::NotRecorded => Some("not recorded"),
        };
        if let Some(actual) = suppression {
            report.difference(
                format!("DivisionPanelStats {}.suppression", row.division),
                actual,
                "complete deployed cache operand",
            );
        }
    }
    for id in save.divisions.keys().filter(|id| !seen.contains(id)) {
        report.difference(
            format!("DivisionPanelStats {id}"),
            "absent",
            "present in saved divisions",
        );
    }
    report.compared("DivisionPanelStats", rows.len());
    report.limitation("The 22 deployed active-stat cache values and the stat block's suppression word are not serialized by the division writer. Save membership and available operands do not certify their numbers; compare every column with the matched-hour native division panel (the suppression word with the division designer's suppression row, which the panel does not show), and use sampled carried-versus-fresh verification across rebuilds.", "DivisionPanelStats.panel_stats, suppression");
    Ok(())
}

pub fn compare_division_status(
    frame: &DecodedFrame,
    save: &Save,
    tokens: &TokenNames<'_>,
    camo_show_at: &Result<i64, String>,
    supply_grace: &Result<i32, String>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::DivisionStatus>(frame)?;
    division_history::compare(frame, save, tokens, report)?;
    let references = super::combat_details::MemberTemplates::new(rows);
    let templates =
        super::combat_details::NameTemplates::new(frame, save, report, references.any())?;
    let ordinals = save
        .counter_inputs
        .division_orders
        .values()
        .flat_map(|rows| {
            rows.iter()
                .enumerate()
                .map(|(ordinal, &division)| (division, ordinal as u32))
        })
        .collect::<BTreeMap<_, _>>();
    let movements = frame
        .complete::<tables::DivisionMovement>()
        .ok()
        .map(|rows| {
            rows.iter()
                .map(|row| (row.division, row))
                .collect::<BTreeMap<_, _>>()
        });
    let panels = frame.complete::<tables::DivisionPanelStats>().ok();
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.division == 0 || !seen.insert(row.division) {
            return Err(invalid("null or duplicate status division"));
        }
        report.scalar_checks += 1;
        let unknown = row.move_flags & !DEFINED_MOVE_FLAGS;
        if unknown != 0 {
            report.difference(
                format!("division status {}.move_flags", row.division),
                format!("undefined bits {unknown:#04x}"),
                "only the defined movement flags",
            );
        }
        compare_combat_movement(row, movements.as_ref(), report);
        let Some(saved) = save.divisions.get(&row.division) else {
            report.difference(
                format!("division status {}", row.division),
                "present",
                "absent",
            );
            continue;
        };
        for (name, present) in [
            ("officer_experience", row.officer_experience().is_some()),
            ("casualties_taken (saved killed)", row.casualties_taken().is_some()),
        ] {
            if !present {
                report.difference(
                    format!("division status {}.{name}", row.division),
                    "unavailable",
                    "available saved value (including omitted nonpositive values)",
                );
            }
        }
        if let Some(expected) = &saved.presentation.officer_experience {
            report.scalar(
                &format!("division status {}", row.division),
                "officer_experience",
                row.officer_experience(),
                Some(expected),
            );
        } else if let Some(actual) = row.officer_experience()
            && actual > 0
        {
            report.difference(
                format!("division status {}.officer_experience", row.division),
                actual,
                "omitted nonpositive saved XP",
            );
        }
        if let Some(expected) = saved.presentation.career_counter {
            if row.casualties_taken() != Some(expected) {
                report.difference(
                    format!("division status {}.casualties_taken (saved killed)", row.division),
                    format!("{:?}", row.casualties_taken()),
                    expected,
                );
            }
        } else if let Some(actual) = row.casualties_taken()
            && actual > 0
        {
            report.difference(
                format!("division status {}.casualties_taken (saved killed)", row.division),
                actual,
                "omitted nonpositive saved counter",
            );
        }
        let transition_label = format!("division status {}.pending_template", row.division);
        match report.recorded(
            &row.pending_template,
            "DivisionStatus.pending_template",
            &transition_label,
        ) {
            Some(Some(value)) => equal(
                report,
                &transition_label,
                (*value != 0).then_some(*value),
                saved.presentation.pending_template,
            ),
            Some(None) => report.difference(
                &transition_label,
                "unavailable captured transition",
                "complete saved transition",
            ),
            None => {}
        }
        let experience_label = format!("division status {}.experience", row.division);
        match report.recorded(
            &row.experience,
            "DivisionStatus.experience",
            &experience_label,
        ) {
            Some(Some(value)) => report.scalar(
                &format!("division status {}", row.division),
                "experience",
                Some(*value),
                saved.presentation.experience.as_ref(),
            ),
            Some(None) => report.difference(
                &experience_label,
                "unavailable captured experience",
                "complete saved experience",
            ),
            None => {}
        }
        let label = format!("division status {}", row.division);
        if let (Some(current), Some(maximum)) = (
            report.recorded(
                &row.organisation_exact,
                "DivisionStatus.organisation_exact",
                &label,
            ),
            report.recorded(
                &row.maximum_organisation_exact,
                "DivisionStatus.maximum_organisation_exact",
                &label,
            ),
        ) {
            let flags = row.move_flags
                & (DivisionStatusRow::MOVE_LOW_ORGANISATION
                    | DivisionStatusRow::MOVE_ORGANISATION_REFUSED);
            match (flags, current, maximum) {
                (DivisionStatusRow::MOVE_LOW_ORGANISATION, Some(current), Some(maximum)) => {
                    report.scalar(&label, "movement_organisation.current", Some(*current), Some(&saved.organisation));
                    report.scalar_checks += 1;
                    let displayed = DivisionStatusRow::MAXIMUM_STEP.round(*maximum);
                    let shown = panels
                        .and_then(|rows| rows.binary_search_by_key(&row.division, |panel| panel.division).ok().map(|at| &rows[at]))
                        .and_then(|panel| panel.maximum_organisation());
                    if Some(displayed) != shown {
                        report.difference(format!("{label}.movement_organisation.maximum displayed"), displayed, format!("{shown:?}"));
                    }
                    report.compared("DivisionStatus.organisation_exact saved current", 1);
                }
                (0, None, None) => { report.scalar_checks += 1; }
                _ => report.difference(format!("{label}.movement_organisation"),
                    format!("flags={flags}, current={current:?}, maximum={maximum:?}"),
                    "complete exact pair only when the native low-organisation movement factor applies"),
            }
        }
        if let Some(ordinal) = report.recorded(
            &row.country_ordinal,
            "DivisionStatus.country_ordinal",
            format!("division status {}.country_ordinal", row.division),
        ) {
            match ordinal {
                Some(ordinal) => {
                    let expected = ordinals.get(&row.division).copied();
                    if Some(*ordinal) != expected {
                        report.difference(
                            format!("division status {}.country_ordinal", row.division),
                            Some(*ordinal),
                            expected,
                        );
                    }
                }
                None => report.limitation(
                    "country division roster position unavailable from native capture",
                    format!("DivisionStatus.country_ordinal {}", row.division),
                ),
            }
        }
        for (name, value, saved) in [
            ("entrenchment", row.entrenchment, &saved.presentation.dig_in),
            (
                "entrenchment_cap",
                row.entrenchment_cap,
                &saved.presentation.dig_in_cap,
            ),
        ] {
            let label = format!("division status {}", row.division);
            if let Some(saved) = saved {
                report.scalar(&label, name, Some(value), Some(saved));
            } else {
                report.scalar_checks += 1;
                if value > 0 {
                    report.difference(
                        format!("{label}.{name} positive-only save omission"),
                        value,
                        "omitted => nonpositive",
                    );
                }
            }
        }
        compare_camouflage(
            &format!("division status {}", row.division),
            row,
            saved.presentation.acclimatization.as_ref(),
            tokens,
            camo_show_at,
            report,
        );
        if let Some(value) = report.recorded(
            &row.supply_grace_maximum,
            "DivisionStatus.supply_grace_maximum",
            format!("division status {}.supply_grace_maximum", row.division),
        ) {
            let label = format!("division status {}", row.division);
            report.scalar(&label, "supply_grace_maximum", Some(*value), saved.presentation.max_supply.as_ref());
        }
        if let Some(value) = report.recorded(
            &row.supply_grace_remaining,
            "DivisionStatus.supply_grace_remaining",
            format!("division status {}.supply_grace_remaining", row.division),
        ) {
            let label = format!("division status {}", row.division);
            let precision = match supply_grace {
                Ok(hours) => format!(
                    "whole percent of the share over NMilitary.SUPPLY_GRACE ({hours} hours), toward zero"
                ),
                Err(reason) => format!("0.01 (fixed5, toward zero): {reason}"),
            };
            let displayed = row.displayed_supply_grace(supply_grace.clone().ok());
            report.scalar_displayed_by(
                &label,
                "supply_grace_remaining",
                displayed,
                saved.presentation.army_current_supply_ratio.as_ref(),
                "DivisionStatus.supply_grace_remaining",
                &precision,
                |raw| DivisionStatusRow::displayed_grace(raw, supply_grace.clone().ok()),
            );
            if matches!(row.supply_grace_exact, Recorded::Value(Some(_)))
                && *value != 0
                && Some(*value) != displayed
            {
                report.difference(
                    format!("{label}.supply_grace_remaining"),
                    *value,
                    format!("0 or {displayed:?}"),
                );
            }
        }
        if let Some(days) = report.recorded(
            &row.out_of_supply_days,
            "DivisionStatus.out_of_supply_days",
            format!("division status {}.out_of_supply_days", row.division),
        ) {
            let label = format!("division status {}.out_of_supply_days", row.division);
            match days {
                Some(days) => {
                    report.scalar_checks += 1;
                    let saved = saved.presentation.out_of_supply_days.unwrap_or(0);
                    if *days != saved {
                        report.difference(label, *days, saved);
                    }
                }
                None => report.difference(label, "unavailable", "a recorded value"),
            }
        }
        match (&row.supply_ratio_exact, &row.supply_grace_exact) {
            (Recorded::NotRecorded, Recorded::NotRecorded) => {
                let _ = report.recorded(
                    &row.supply_ratio_exact,
                    "DivisionStatus.supply_ratio_exact",
                    format!("division status {}.supply_ratio_exact", row.division),
                );
                report.scalar_checks += 1;
                if row.supply_ratio.is_none() {
                    report.difference(
                        format!("division status {}.supply_ratio", row.division),
                        "None",
                        "recorded legacy supply_ratio when exact supply columns are not recorded",
                    );
                }
            }
            (Recorded::Value(Some(_)), Recorded::Value(Some(grace))) => {
                let label = format!("division status {}", row.division);
                report.scalar(
                    &label,
                    "supply_exact.grace_remaining",
                    Some(*grace),
                    saved.presentation.army_current_supply_ratio.as_ref(),
                );
                report.scalar_checks += 1;
                if row.supply_ratio.is_some() {
                    report.difference(
                        format!("{label}.supply_exact shown"),
                        format!("{:?}", row.supply_ratio),
                        "derived from the exact ratio",
                    );
                }
            }
            (Recorded::Value(None), Recorded::Value(None)) => {
                let label = format!("division status {}", row.division);
                let days = match &row.out_of_supply_days {
                    Recorded::Value(days) => *days,
                    Recorded::NotRecorded => None,
                };
                report.scalar_checks += 1;
                if !matches!(days, Some(0) | None) || row.supply_ratio.is_none() {
                    report.difference(
                        format!("{label}.supply_exact"),
                        format!(
                            "exact=(None, None), supply_ratio={:?}, days={days:?}",
                            row.supply_ratio
                        ),
                        "complete exact pair, or absent with recorded supply_ratio only at zero days out of supply",
                    );
                }
            }
            (ratio_exact, grace_exact) => {
                let label = format!("division status {}", row.division);
                report.scalar_checks += 1;
                report.difference(
                    format!("{label}.supply_exact"),
                    format!("({ratio_exact:?}, {grace_exact:?})"),
                    "both exact columns recorded together or both omitted before format support",
                );
            }
        }
        if let Some(supply) = report.recorded(
            &row.supply,
            "DivisionStatus.supply",
            format!("division status {} supply", row.division),
        ) {
            let label = format!("division status {}", row.division);
            match supply {
                Some(supply) => {
                    if let Some(saved) = saved.presentation.disrupted_supply.as_ref() {
                        report.scalar_displayed(
                            &label,
                            "supply.disrupted_supply",
                            Some(supply.disruption),
                            Some(saved),
                            "DivisionStatus.supply.disruption",
                            DivisionSupplyFooter::STEP,
                        );
                    } else {
                        report.scalar_checks += 1;
                        if supply.disruption > 0 {
                            report.difference(
                                format!(
                                    "{label}.supply.disrupted_supply positive-only save omission"
                                ),
                                supply.disruption,
                                "omitted => nonpositive",
                            );
                        }
                    }
                }
                None => report.difference(
                    format!("{label}.supply.disrupted_supply"),
                    "native supply scalar capture unavailable",
                    saved
                        .presentation
                        .disrupted_supply
                        .as_ref()
                        .map(|value| value.raw),
                ),
            }
        }
        if let Some(naming) = report.recorded(
            &row.naming,
            "DivisionStatus.naming",
            format!("division status {} naming", row.division),
        ) {
            match naming {
                Some(naming) => super::combat_details::naming(
                    report,
                    &format!("division status {}", row.division),
                    super::combat_details::Naming::from(naming)
                        .with_template(references.of(row.division)),
                    saved,
                    &templates,
                ),
                None => report.limitation(
                    "division naming operands unreadable at capture",
                    format!("DivisionStatus.naming {}", row.division),
                ),
            }
        }
        if row.invalid_orders != 0
            && report
                .recorded(
                    &row.invalid_orders_reason,
                    "DivisionStatus.invalid_orders_reason",
                    format!("division status {} invalid-order reason", row.division),
                )
                .is_some_and(Option::is_none)
        {
            report.limitation(
                "demanded invalid-order reason unreadable at capture",
                format!("DivisionStatus.invalid_orders_reason {}", row.division),
            );
        }
    }
    for id in save.divisions.keys().filter(|id| !seen.contains(id)) {
        report.difference(format!("division status {id}"), "absent", "present");
    }
    report.compared("DivisionStatus", rows.len());
    report.limitation("The exact movement organisation pair compares its current value with the saved organisation. The effective maximum is an unsaved stat-block result; its rounded value is checked against the separately recorded display maximum", "DivisionStatus.movement_organisation maximum: runtime operand");
    report.limitation("invalid_orders_reason is the raw signed reason word selected after the counter's first invalid-order gate. Neither the unit writer nor the army writer serializes it; a separate same-width saved field is a different operand. A fresh save cannot certify this tooltip reason; matched counter hovers remain necessary", "DivisionStatus.invalid_orders_reason");
    report.limitation("border_combat_stats copies defense/soft/hard attack from the army stat cache for the exact active-state army union. The army writer does not save that cache; membership/location and saved HP/organisation checks do not certify these values", "DivisionStatus.border_combat_stats");
    report.limitation("the supply consumer save writer saves only positive disruption; omission certifies only the nonpositive branch. The asked/received, state/air-drop receipts, demand pairs and origin rows of the supply footer are unsaved runtime caches, stored at the footer's display step. Their displayed amounts and aggregation still require matched native hovers", "DivisionStatus.supply runtime operands");
    report.limitation("effective maximum HP/organisation, the strength factor, counter supply input, attrition threshold/input/icon, actual exercising and order-membership predicates are derived or unsaved; entrenchment, its cap and camouflage (or an older recording's acclimatization) compare to save fields. Omitted entrenchment values prove nonpositive, not exactly zero. Source code and double reads do not replace live counter reference pairs", "DivisionStatus unverified fields");
    report.limitation("model_number is the stored army model number, which the army writer does not serialize; active-stat updates rewrite it once a day per division and on template change, deploy and unit creation, so it has no save counterpart and needs a matched-hour screenshot of the drawn model suffix", "DivisionStatus.model_number");
    report.limitation("camouflage is what the game takes from the division with the live NGraphics.ACCLIMATIZATION_CAMO_SHOW_AT; it compares with the same decision over the saved acclimatization block and the content's define. The army writer omits that block unless an entry weighs above zero, when no category can show, and the game skips zero weights, which never win. The saved block cannot show a null winning category pointer. A division whose camouflage was unreadable that hour is not compared. An older recording's acclimatization compares entry by entry instead: only the omission without a positive weight, nonzero entries in order and the maximum", "DivisionStatus.camouflage");
    report.limitation("supply_ratio is the raw fixed5 supply operand: the counter ignores it during naval transfer and applies presentation thresholds/visibility/stack selection. has_order is the order-membership predicate, false for ungrouped units", "DivisionStatus consumer contract");
    report.limitation("DivisionStatus.move_flags combines exile truth with can_exfiltrate_from_coast, the live stat block's AND over the template's subunits, the noncore-supply bypass byte's truth, whether the air-supply word is positive or unreadable (availability is local to reached noncore movement), and the conditional exact organisation branch and its refusal. The army save writer delegates to the unit save writer, which saves exile only as zero/nonzero truth; DivisionMovement.selected compares that current byte's truth. A bit outside the defined flags is a difference. The raw nonzero magnitude, the coast-exfiltration cache, the noncore-supply bypass and the air-supply sign remain unverified", format!("DivisionStatus.move_flags: {} rows decoded, combined flags not certified", rows.len()));
    report.limitation("combat_movement is the movement word of the first land or border-war combat in the division's combat list whose attacker side holds it, a runtime combat value the save omits, recorded for a division in combat and not withdrawing; its branch is checked against DivisionMovement's combat count and withdrawal byte. A division in combat whose first matching combat is naval, or with no matching combat side, records none, which the game takes as a zero starting term; whether a combat matches is not certified. Compare the word against paused memory at the sampled hour", "DivisionStatus.combat_movement runtime operand");
    report.limitation("attack_stats and movement_stats are the live division stat block's attack and movement operands (the terrain adjusters' attack and movement, the night, fort, river and snow attack terms, the fort, river and snow adjusters' movement, the fuel rate and anti-air words), which the active-stat update rewrites and the division writer does not save; weather_factor is the movement-availability calculation over the acclimatization block and the active climate category, which the save omits. Neither has a save counterpart: the attack_witness diagnostic with `recorder derived` compares the derived attack operands with native ones. The selected army's attack inputs the save omits likewise: OrdersGroupCounterInputs.commander_attack and commander_movement (the leader's rebuilt trait adjusters), CountryCounterInputs.attack_modifiers (accumulator entries), Provinces.fort_bunker (the state's country-specific modifier) and WeatherRegions.combat (the region weather modifier's entries)", "DivisionStatus.attack_stats, movement_stats and the attack-list inputs");
    report.limitation("the commanders' retreat bans are the cached leader modifier block the unit-leader modifier lookup selects (cannot_retreat_while_defending/attacking); the save serializes the leaders' traits, not that block, so the sliced daily DivisionRetreatBans rows are not compared", "DivisionRetreatBans");
    Ok(())
}

const CREW_NOT_RECORDED: &str = "crew not recorded at this hour: the crew writers were not observed, or one ran across the read";

const TRAINING_STOP_NOT_RECORDED: &str = "air wing stop_training_at_max_xp not recorded at this hour: the wing writers were not observed, one ran across the read, or the byte was unreadable";

const DEFINED_MOVE_FLAGS: u8 = DivisionStatusRow::MOVE_EXILED
    | DivisionStatusRow::MOVE_CAN_EXFILTRATE_FROM_COAST
    | DivisionStatusRow::MOVE_NONCORE_SUPPLY_BYPASS
    | DivisionStatusRow::MOVE_NONCORE_AIR
    | DivisionStatusRow::MOVE_LOW_ORGANISATION
    | DivisionStatusRow::MOVE_ORGANISATION_REFUSED
    | DivisionStatusRow::MOVE_NONCORE_AIR_REFUSED;

fn compare_combat_movement(
    row: &DivisionStatusRow,
    movements: Option<&BTreeMap<u64, &DivisionMovementRow>>,
    report: &mut Report,
) {
    let label = format!("division status {}.combat_movement", row.division);
    let Some(entry) = report.recorded(
        &row.combat_movement,
        "DivisionStatus.combat_movement",
        &label,
    ) else {
        return;
    };
    let Some(entry) = entry else {
        report.limitation(
            "the combat movement word was unreadable at capture",
            format!("DivisionStatus.combat_movement {}", row.division),
        );
        return;
    };
    if entry.is_none() {
        return;
    }
    let Some(movements) = movements else {
        report.limitation(
            "DivisionMovement is unavailable, so a recorded combat movement word's combat and withdrawal branch is not checked",
            label,
        );
        return;
    };
    let Some(movement) = movements.get(&row.division) else {
        report.scalar_checks += 1;
        report.difference(label, "a combat movement word", "no DivisionMovement row");
        return;
    };
    if movement.combat_count == 0 {
        report.scalar_checks += 1;
        report.difference(label, "a combat movement word", "a division outside combat");
        return;
    }
    match movement.withdrawal {
        Recorded::Value(Some(0)) => report.scalar_checks += 1,
        Recorded::Value(Some(withdrawal)) => {
            report.scalar_checks += 1;
            report.difference(
                label,
                "a combat movement word",
                format!("a withdrawing division (withdrawal {withdrawal})"),
            );
        }
        Recorded::Value(None) | Recorded::NotRecorded => report.limitation(
            "the division's withdrawal byte is refused or not recorded, so a recorded combat movement word's withdrawal branch is not checked",
            label,
        ),
    }
}

pub fn compare_camouflage(
    label: &str,
    row: &replay_data::rows::DivisionStatusRow,
    saved: Option<&content_reader_save::Acclimatization>,
    tokens: &TokenNames<'_>,
    show_at: &Result<i64, String>,
    report: &mut Report,
) {
    let camouflage = match &row.camouflage {
        Recorded::NotRecorded => {
            return compare_acclimatization(label, row, saved, tokens, report);
        }
        Recorded::Value(None) => {
            report.limitation(
                "the division's camouflage was unreadable at capture; that field alone is unavailable",
                format!("{label}.camouflage"),
            );
            return;
        }
        Recorded::Value(Some(camouflage)) => camouflage,
    };
    let label = format!("{label}.camouflage");
    let show_at = match show_at {
        Ok(show_at) => *show_at,
        Err(error) => {
            report.limitation(
                "the content's NGraphics.ACCLIMATIZATION_CAMO_SHOW_AT is unavailable; camouflage was not compared",
                format!("{label}: {error}"),
            );
            return;
        }
    };
    let captured: Option<&[u8]> = match camouflage.category {
        None => None,
        Some(token) => match tokens.get(&token) {
            Some(name) => Some(*name),
            None => {
                report.limitation(
                    "the camouflage category's token is absent from the captured lexical dictionary; it was not compared",
                    label,
                );
                return;
            }
        },
    };
    let expected = saved.and_then(|saved| {
        game_model::acclimatization::camouflage(
            saved
                .entries
                .iter()
                .map(|(name, weight)| (name, weight.raw)),
            saved.maximum.as_ref().map_or(
                content_reader_save::Acclimatization::OMITTED_MAXIMUM_RAW,
                |maximum| maximum.raw,
            ),
            show_at,
        )
        .map(Vec::as_slice)
    });
    report.scalar_checks += 1;
    if captured != expected {
        let shown = |name: Option<&[u8]>| name.map(|name| name.escape_ascii().to_string());
        report.difference(label, shown(captured), shown(expected));
    }
}

fn shown_entries(entries: &[(&[u8], i64)]) -> Vec<(String, i64)> {
    entries
        .iter()
        .map(|(name, weight)| (name.escape_ascii().to_string(), *weight))
        .collect()
}

pub fn compare_acclimatization(
    label: &str,
    row: &replay_data::rows::DivisionStatusRow,
    saved: Option<&content_reader_save::Acclimatization>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) {
    let label = format!("{label}.acclimatization");
    let Some(maximum) = row.acclimatization_maximum else {
        report.limitation(
            "the division's acclimatization was unreadable at capture; that field alone is unavailable",
            label,
        );
        return;
    };
    report.scalar_checks += 1;
    let written = row.acclimatization.iter().any(|entry| entry.weight > 0);
    let saved = match (written, saved) {
        (true, Some(saved)) => saved,
        (false, None) => return,
        (true, None) => {
            report.difference(
                label,
                "a positive weight, so the block is written",
                "missing saved acclimatization block",
            );
            return;
        }
        (false, Some(_)) => {
            report.difference(
                label,
                "no positive weight, so the block is omitted",
                "saved acclimatization block",
            );
            return;
        }
    };
    report.scalar_checks += 1;
    let expected = saved.maximum.as_ref().map_or(
        content_reader_save::Acclimatization::OMITTED_MAXIMUM_RAW,
        |maximum| maximum.raw,
    );
    if maximum != expected {
        report.difference(format!("{label}.maximum"), maximum, expected);
    }
    let mut captured = Vec::new();
    for entry in row.acclimatization.iter().filter(|entry| entry.weight != 0) {
        let Some(name) = tokens.get(&entry.category) else {
            report.limitation("acclimatization categories include tokens absent from the captured lexical dictionary; entries were not compared", label);
            return;
        };
        captured.push((*name, entry.weight));
    }
    let expected: Vec<_> = saved
        .entries
        .iter()
        .map(|(name, weight)| (name.as_slice(), weight.raw))
        .collect();
    report.scalar_checks += 1;
    if captured != expected {
        report.difference(
            format!("{label}.entries"),
            shown_entries(&captured),
            shown_entries(&expected),
        );
    }
}

fn compare_template_text(
    report: &mut Report,
    label: &str,
    stored: &[u8],
    saved: &[u8],
    saved_present: bool,
) {
    let prefix = content_reader_parser::parser::c_string(stored);
    if prefix.len() < stored.len() {
        report.limitation(
            "native text serialization ends at the first NUL; captured trailing bytes are not save-comparable",
            label,
        );
    }
    let actual = (!stored.is_empty()).then_some(prefix);
    let expected = saved_present.then_some(saved);
    report.scalar_checks += 1;
    if actual != expected {
        report.difference(label, actual, expected);
    }
}

pub fn compare_country_template_state(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::CountryTemplateState>(frame)?;
    let mut seen = BTreeSet::new();
    for row in rows {
        let tag = required_country_tag(row.country, &countries.links)?;
        if !seen.insert(tag) {
            return Err(invalid("duplicate country template configuration"));
        }
        let Some(expected) = save.production.template_states.get(tag) else {
            report.difference(
                format!("country {}.template configuration", tag_text(Some(tag))),
                "recorded",
                "absent in save",
            );
            continue;
        };
        report.scalar_checks += 2;
        if (row.lock_all != 0) != expected.lock_all {
            report.difference(
                format!("country {}.templates_locked", tag_text(Some(tag))),
                row.lock_all,
                expected.lock_all,
            );
        }
        if matches!(row.garrison_priority, Recorded::NotRecorded) {
            report.difference(
                format!("country {}.garrison priority", tag_text(Some(tag))),
                "unrecorded acquisition",
                expected.garrison_priority,
            );
        }
        if let Some(priority) = report.recorded(
            &row.garrison_priority,
            "CountryTemplateState.garrison_priority",
            format!("country {}.garrison priority", tag_text(Some(tag))),
        ) && (priority.is_none() || *priority != expected.garrison_priority)
        {
            report.difference(
                format!("country {}.occupation_status.priority", tag_text(Some(tag))),
                priority,
                expected.garrison_priority,
            );
        }
        let selected = save
            .division_templates
            .contains_key(&row.selected_staff)
            .then_some(row.selected_staff);
        if Some(selected) != expected.selected_staff {
            report.difference(
                format!("country {}.default_hq_template", tag_text(Some(tag))),
                Some(selected),
                expected.selected_staff,
            );
        }
        report.scalar_checks += 1;
        let next_hq_order = (row.next_hq_order > 0).then_some(row.next_hq_order);
        if next_hq_order != expected.next_hq_order {
            report.difference(
                format!("country {}.hq_next_deploy_order", tag_text(Some(tag))),
                next_hq_order,
                expected.next_hq_order,
            );
        }
        let garrisons = match &row.garrisons {
            Recorded::Value(Some(rows)) => {
                let mut actual = BTreeMap::new();
                for selection in rows.iter() {
                    let scope = selection
                        .occupied
                        .map(|id| required_country_tag(id, &countries.links).map(<[u8]>::to_vec))
                        .transpose()?;
                    if actual
                        .insert((scope, selection.state), selection.template)
                        .is_some()
                    {
                        return Err(invalid("duplicate recorded garrison template scope"));
                    }
                }
                Some(actual)
            }
            _ => return Err(invalid("garrison template selections unavailable")),
        };
        report.scalar_checks += 1;
        if garrisons != expected.garrisons {
            report.difference(
                format!(
                    "country {}.garrison template selections",
                    tag_text(Some(tag))
                ),
                &garrisons,
                &expected.garrisons,
            );
        }
    }
    for (tag, _) in report.canonical_countries(&save.production.template_states) {
        if !seen.contains(tag.as_slice()) {
            report.difference(
                format!("country {}.template configuration", tag_text(Some(tag))),
                "absent in recording",
                "saved",
            );
        }
    }
    Ok(())
}

pub fn compare_division_templates(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::DivisionTemplates>(frame)?;
    if !report.refresh_is_current(TableId::DivisionTemplates, frame.hour) {
        report.limitation(
            replay_data::cadence::STAGGER_LIMITATION,
            "DivisionTemplates",
        );
        return Ok(());
    }
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.template == 0 || !seen.insert(row.template) {
            return Err(invalid("null or duplicate division template handle"));
        }
        let Some(saved) = save.division_templates.get(&row.template) else {
            report.difference(format!("template {}", row.template), "present", "absent");
            continue;
        };
        let owner = required_country_tag(row.owner, &countries.links)?;
        report.scalar_checks += 1;
        if save
            .division_template_owners
            .get(&row.template)
            .map(Vec::as_slice)
            != Some(owner)
        {
            report.difference(
                format!("template {}.country roster", row.template),
                owner,
                save.division_template_owners.get(&row.template),
            );
        }
        report.scalar_checks += 1;
        if owner != saved.country {
            report.difference(
                format!("template {}.owner", row.template),
                owner,
                &saved.country,
            );
        }
        compare_template_text(
            report,
            &format!("template {}.name", row.template),
            &row.name,
            &saved.name,
            saved.name_present,
        );
        if let Some(key) = report.recorded(
            &row.name_key,
            "DivisionTemplates.name_key",
            format!("template {}.name_key", row.template),
        ) {
            compare_template_text(
                report,
                &format!("template {}.name_key", row.template),
                key,
                &saved.name_key,
                saved.name_key_present,
            );
        }
    }
    for template in save
        .division_template_owners
        .keys()
        .copied()
        .chain(
            save.divisions
                .values()
                .filter_map(|division| division.template),
        )
        .collect::<BTreeSet<_>>()
    {
        if !seen.contains(&template) {
            report.difference(
                format!("owned or used division template {template}"),
                "absent",
                "present",
            );
        }
    }
    report.compared("DivisionTemplates", rows.len());
    report.limitation("dictionary census compares the saved country rosters, including unused templates; save-only unowned global registry residues are outside this dictionary. The unit_type category is a derived engine cache absent from text saves; effective maxima belong to divisions; no twelve-frame strip mapping or fake-intel presentation is claimed", "DivisionTemplates");
    Ok(())
}

pub fn compare_division_template_icons(
    frame: &DecodedFrame,
    save: &Save,
    countries: &CountryContext<'_>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::DivisionTemplateIcons>(frame)?;
    let mut seen = BTreeSet::new();
    for row in rows {
        if row.template == 0 || !seen.insert(row.template) {
            return Err(invalid("null or duplicate division template icon handle"));
        }
        let Some(saved) = save.division_templates.get(&row.template) else {
            report.difference(
                format!("template icons {}", row.template),
                "present",
                "absent",
            );
            continue;
        };
        report.scalar_checks += 3;
        let expected_exile = saved.origin_type.as_deref() == Some(b"exile");
        if row
            .manpower_exile
            .require("template manpower exile")
            .ok()
            .copied()
            != Some(expected_exile)
        {
            report.difference(
                format!("template {}.manpower_exile", row.template),
                row.manpower_exile,
                expected_exile,
            );
        }
        if row.custom_icon != saved.custom_icon {
            report.difference(
                format!("template {}.custom_icon", row.template),
                row.custom_icon,
                saved.custom_icon,
            );
        }
        if (row.custom_icon_nato != 0) != saved.custom_icon_nato {
            report.difference(
                format!("template {}.custom_icon_nato", row.template),
                row.custom_icon_nato,
                saved.custom_icon_nato,
            );
        }
        for (field, reference, saved_tag) in [
            ("country", row.country, Some(&saved.country)),
            (
                "foreign_template_tag",
                row.foreign_country,
                saved.foreign_template_tag.as_ref(),
            ),
        ] {
            let label = format!("template {}.{field}", row.template);
            let captured = match u32::try_from(reference) {
                Ok(reference) => country_tag(reference, &countries.links)?,
                Err(_) => None,
            };
            match (captured, saved_tag) {
                (Some(captured), Some(saved_tag)) => {
                    report.scalar_checks += 1;
                    if captured != saved_tag.as_slice() {
                        report.difference(
                            label,
                            tag_text(Some(captured)),
                            tag_text(Some(saved_tag.as_slice())),
                        );
                    }
                }
                (captured, None) => report.difference(label, tag_text(captured), "missing serialized tag"),
                (None, Some(_)) => report.limitation("a null template country reference is written through the unrecorded country-tag entry0; its literal spelling is unmapped here, not assumed", label),
            }
        }
        if let Some(captured) = report.recorded(
            &row.display_name,
            "DivisionTemplateIcons.display_name",
            format!("template {}.display_name", row.template),
        ) {
            if captured.is_none() {
                report.failed_comparisons.insert(
                    format!("DivisionTemplateIcons {}.display_name", row.template),
                    "fresh saved template name comparison refused: guarded read or name arena refusal".into(),
                );
            } else {
                let daily = frame
                    .complete::<tables::DivisionTemplates>()
                    .ok()
                    .and_then(|rows| rows.iter().find(|daily| daily.template == row.template));
                match row.display_name_inputs(frame.hour, daily) {
                    Ok(parts) => {
                        compare_template_text(
                            report,
                            &format!("template {}.hourly name", row.template),
                            parts.literal,
                            &saved.name,
                            saved.name_present,
                        );
                        if let Some(key) = parts.fallback_key {
                            compare_template_text(
                                report,
                                &format!("template {}.hourly name_key", row.template),
                                key,
                                &saved.name_key,
                                saved.name_key_present,
                            );
                        }
                    }
                    Err(reason) => {
                        report.failed_comparisons.insert(
                            format!("DivisionTemplateIcons {}.display_name", row.template),
                            format!("fresh saved template name comparison refused: {reason}"),
                        );
                    }
                }
            }
        }
        macro_rules! setting {
            ($name:literal, $actual:expr, $expected:expr) => {{
                let actual = $actual;
                let expected = $expected;
                report.scalar_checks += 1;
                if actual != expected {
                    report.difference(
                        format!("template {}.{}", row.template, $name),
                        actual,
                        expected,
                    );
                }
            }};
        }
        if matches!(row.settings, Recorded::NotRecorded | Recorded::Value(None)) {
            report.difference(
                format!("template {}.settings", row.template),
                "settings acquisition refused",
                "saved template settings",
            );
        }
        if let Some(Some(settings)) = report.recorded(
            &row.settings,
            "DivisionTemplateIcons.settings",
            format!("template {}.settings", row.template),
        ) {
            let expected = &saved.settings;
            let names_group = settings
                .names_group
                .as_deref()
                .map(content_reader_parser::parser::c_string)
                .filter(|key| !key.is_empty());
            setting!(
                "division_names_group",
                names_group,
                saved.names_group.as_deref()
            );
            setting!("priority", Some(settings.priority), expected.priority);
            let role = match settings.role {
                None => None,
                Some(token) => match tokens.get(&token) {
                    Some(name) => Some(*name),
                    None => {
                        report.difference(
                            format!("template {}.role token", row.template),
                            token,
                            "unresolved",
                        );
                        None
                    }
                },
            };
            setting!("role", role, expected.role.as_deref());
            setting!("is_army_hq", settings.is_army_hq != 0, expected.is_army_hq);
            setting!("is_locked", settings.locked != 0, expected.locked);
            setting!("obsolete", settings.obsolete != 0, expected.obsolete);
            let date = (settings.obsolete != 0
                && settings.obsolete_change_date != engine_native_memory::cgame_date::NULL_RAW_HOUR)
                .then_some(settings.obsolete_change_date);
            setting!("obsolete_change_date", date, expected.obsolete_change_date);
            setting!(
                "force_allow_recruiting",
                settings.force_allow_recruiting != 0,
                expected.force_allow_recruiting
            );
            setting!(
                "allow_new_equipment",
                Some(settings.allow_new_equipment != 0),
                expected.allow_new_equipment
            );
            setting!(
                "allow_foreign_equipment",
                Some(settings.allow_foreign_equipment != 0),
                expected.allow_foreign_equipment
            );
            setting!(
                "parent",
                (settings.parent != 0).then_some(settings.parent),
                expected.parent
            );
            setting!(
                "is_fake_intel_division",
                settings.fake_intel != 0,
                expected.fake_intel
            );
            setting!("division_cap", settings.division_cap, expected.division_cap);
            setting!(
                "equipment_niche",
                (settings.equipment_niche != 0)
                    .then_some(i32::from(settings.equipment_niche as i8)),
                expected.equipment_niche
            );
            let origin = match settings.origin_type {
                0 => Some(b"master_host".as_slice()),
                1 => Some(b"subject".as_slice()),
                2 => Some(b"exile".as_slice()),
                _ => None,
            };
            setting!("origin_type", origin, expected.origin_type.as_deref());
            let creator = u32::try_from(settings.creator_country)
                .ok()
                .map(|reference| country_tag(reference, &countries.links))
                .transpose()?
                .flatten();
            setting!("original_tag", creator, expected.creator_country.as_deref());
            setting!(
                "forbidden_equipment_types",
                settings.forbidden_equipment.as_ref(),
                Some(&expected.forbidden_equipment)
            );
        }
        if let Some(keys) = report.recorded(
            &row.speed_keys,
            "DivisionTemplateIcons.speed_keys",
            format!("template {}.speed_keys", row.template),
        ) {
            match keys {
                Some(keys) => {
                    report.scalar_checks += 1;
                    if keys.contains(&0) {
                        report.difference(
                            format!("template {}.speed_keys", row.template),
                            keys,
                            "only nonzero modifier keys",
                        );
                    }
                }
                None => report.limitation(
                    "the template's movement speed keys were unreadable at capture",
                    format!("DivisionTemplateIcons.speed_keys {}", row.template),
                ),
            }
        }
        if row.model_unreadable != 0 {
            report.difference(
                format!("DivisionTemplateIcons {}.model inputs", row.template),
                "unreadable",
                "saved composition and model choices",
            );
            continue;
        }
        setting!(
            "not_allowed_equipment",
            &row.excluded_variants,
            &saved.excluded_variants
        );
        setting!(
            "override_model_equipment_category_filter",
            row.model_category,
            saved.model_category
        );
        compare_template_text(
            report,
            &format!("template {}.override_model", row.template),
            &row.model_entity,
            &saved.model_entity,
            saved.model_entity_present,
        );
        if matches!(row.layout, Recorded::NotRecorded) {
            report.difference(
                format!("template {}.layout", row.template),
                "unrecorded acquisition",
                "saved spatial composition",
            );
        }
        if let Some(layout) = report.recorded(
            &row.layout,
            "DivisionTemplateIcons.layout",
            format!("template {}.layout", row.template),
        ) {
            if let Some(layout) = layout {
                let mut actual = BTreeMap::new();
                for (kind, grid) in [
                    &layout.regiments,
                    &layout.support,
                    &layout.regimental_support,
                ]
                .into_iter()
                .enumerate()
                {
                    for (index, cell) in grid
                        .cells
                        .iter()
                        .enumerate()
                        .filter(|(_, cell)| cell.occupied && cell.token != 0)
                    {
                        let Some(name) = tokens.get(&cell.token) else {
                            report.difference(
                                format!("template {}.grid token", row.template),
                                cell.token,
                                "unresolved",
                            );
                            continue;
                        };
                        if grid.rows == 0 {
                            report.difference(
                                format!("template {}.grid rows", row.template),
                                0,
                                "nonempty grid needs rows",
                            );
                            continue;
                        }
                        let index = u32::try_from(index)
                            .map_err(|_| invalid("template cell index overflow"))?;
                        actual.insert(
                            (kind as u8, index / grid.rows, index % grid.rows),
                            name.to_vec(),
                        );
                    }
                }
                setting!("spatial composition", &actual, &saved.composition);
            } else {
                report.difference(
                    format!("template {}.layout", row.template),
                    "unreadable",
                    "saved composition",
                );
            }
        }
        let grids = report.recorded(
            &row.grids,
            "DivisionTemplateIcons.grids",
            format!("template {}.grids", row.template),
        );
        let mut compositions = vec![("composition", &row.subunits[..], None)];
        if let Some(grids) = grids {
            compositions.extend([
                ("regiments", &grids.regiments[..], Some(0_u8)),
                ("support", &grids.support[..], Some(1)),
                ("regimental_support", &grids.regimental_support[..], Some(2)),
            ]);
        }
        for (field, entries, grid) in compositions {
            let label = format!("template {}.{field}", row.template);
            let mut composition: BTreeMap<Vec<u8>, i64> = BTreeMap::new();
            let mut unresolved = false;
            for unit in entries.iter().filter(|unit| unit.token != 0) {
                let Some(name) = tokens.get(&unit.token) else {
                    unresolved = true;
                    continue;
                };
                *composition.entry(name.to_vec()).or_default() += i64::from(unit.count);
            }
            let mut expected: BTreeMap<Vec<u8>, i64> = BTreeMap::new();
            for (_, name) in saved
                .composition
                .iter()
                .filter(|((kind, _, _), _)| grid.is_none_or(|grid| *kind == grid))
            {
                *expected.entry(name.clone()).or_default() += 1;
            }
            if unresolved {
                report.difference(label, "unresolved subunit tokens", &expected);
            } else {
                report.scalar_checks += 1;
                if composition != expected {
                    report.difference(label, composition, &expected);
                }
            }
        }
    }
    for template in save
        .division_template_owners
        .keys()
        .copied()
        .chain(
            save.divisions
                .values()
                .filter_map(|division| division.template),
        )
        .collect::<BTreeSet<_>>()
    {
        if !seen.contains(&template) {
            report.difference(
                format!("owned or used division template icons {template}"),
                "absent",
                "present",
            );
        }
    }
    report.compared("DivisionTemplateIcons", rows.len());
    report.limitation("priority_unit and priority_unit_name are the sub-unit the game selects, a derived engine cache absent from text saves", "DivisionTemplateIcons");
    report.limitation("grouped composition compares per-name counts alongside the independent spatial comparison; a local clone and its global definition have the same saved name. Empty cells and grid dimensions have no independent saved representation", "DivisionTemplateIcons composition representation");
    report.limitation("the representative and local-clone flags are derived engine caches absent from text saves; model choices and excluded variants compare their saved counterparts", "DivisionTemplateIcons model inputs");

    report.limitation("speed_keys are the modifier keys of the movement priority sub-unit the game selects (which can differ from priority_unit) and of its unit categories, in native order with repeats kept; that selection is a derived engine cache absent from text saves, so only their nonzero contract is checked. Compare them against paused memory at the sampled hour", "DivisionTemplateIcons.speed_keys");
    report.limitation("the division template save writer omits the composition's signed division-count word consumed by formation counting. Saved grids and formation membership cannot certify this exact runtime count or the ability late-join arithmetic; a matched-hour native comparison remains necessary",
        "DivisionTemplateIcons.formation_division_count");
    Ok(())
}

pub fn compare_naval_units(
    frame: &DecodedFrame,
    save: &Save,
    membership: &content_reader_save::PartialMapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    fn province(
        value: Option<u32>,
        membership: &content_reader_save::PartialMapMembership,
    ) -> io::Result<u32> {
        match value {
            Some(value) if value == 0 || !membership.contains_province(value) => Err(invalid(
                "saved naval unit province is absent from verified map or explicitly zero",
            )),
            value => Ok(value.unwrap_or(0)),
        }
    }
    let mut fleets = BTreeMap::new();
    for row in frame.complete::<tables::Fleets>()? {
        if row.id == 0 || fleets.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate fleet handle"));
        }
    }
    for (&id, expected) in &save.fleets {
        let label = format!("fleet type={} id={}", id as u32, id >> 32);
        let home = province(expected.home_base, membership)?;
        let actual = fleets.remove(&id);
        let Some(actual) = actual else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.fleets += 1;
        let expected_name = expected
            .name
            .as_ref()
            .ok_or_else(|| invalid("fleet stored name missing from ordinary text save"))?;
        match &actual.name {
            Recorded::NotRecorded => report.limitation(
                "fleet stored name not in this recording",
                format!("{label}.name"),
            ),
            Recorded::Value(None) => return Err(invalid("fleet stored name capture refused")),
            Recorded::Value(Some(bytes)) => {
                let comparable = content_reader_parser::parser::c_string(bytes);
                if comparable.len() != bytes.len() {
                    report.limitation("native name serialization ends at the first NUL; trailing owned bytes need a live comparison", format!("{label}.name"));
                }
                if comparable != expected_name.as_slice() {
                    report.difference(format!("{label}.name"), comparable, expected_name);
                }
            }
        }
        match actual.leader {
            Recorded::NotRecorded => report.limitation(
                "fleet leader reference not in this recording",
                format!("{label}.leader"),
            ),
            Recorded::Value(None) => return Err(invalid("fleet leader reference capture refused")),
            Recorded::Value(Some(reference)) => {
                let expected = expected.leader.unwrap_or(0);
                if reference != expected {
                    report.difference(format!("{label}.leader"), reference, expected);
                }
            }
        }
        let owner = required_country_tag(actual.owner, links)?;
        if owner != expected.owner {
            report.difference(
                format!("{label}.owner"),
                tag_text(Some(owner)),
                tag_text(Some(&expected.owner)),
            );
        }
        if actual.home_base != home {
            report.difference(
                format!("{label}.home_base"),
                actual.home_base,
                expected.home_base,
            );
        }
    }
    for &id in fleets.keys() {
        report.difference(format!("fleet {id}"), "present", "absent");
    }
    let mut forces = BTreeMap::new();
    for row in frame.complete::<tables::TaskForces>()? {
        if row.id == 0 || forces.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate task force handle"));
        }
    }
    for (&id, expected) in &save.task_forces {
        let label = format!("task force type={} id={}", id as u32, id >> 32);
        let location = province(expected.province, membership)?;
        if !save.fleets.contains_key(&expected.fleet) {
            return Err(invalid("saved task force references missing fleet"));
        }
        let actual = forces.remove(&id);
        let Some(actual) = actual else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.task_forces += 1;
        if actual.fleet != expected.fleet {
            report.difference(format!("{label}.fleet"), actual.fleet, expected.fleet);
        }
        if actual.province != location {
            report.difference(
                format!("{label}.province"),
                actual.province,
                expected.province,
            );
        }
        if actual.mission as i32 != expected.mission {
            report.difference(
                format!("{label}.mission"),
                actual.mission as i32,
                expected.mission,
            );
        }
        if usize::try_from(actual.route_count).ok() != Some(expected.path.len()) {
            report.difference(
                format!("{label}.route_count"),
                actual.route_count,
                expected.path.len(),
            );
        }
        for (field, raw, expected) in [
            (
                "roster_owner",
                actual.roster_owner,
                Some(expected.roster_owner.as_slice()),
            ),
            (
                "logical_owner",
                actual.logical_owner,
                Some(expected.logical_owner.as_slice()),
            ),
            (
                "expeditionary_owner",
                actual.expeditionary_owner,
                expected.expeditionary_owner.as_deref(),
            ),
        ] {
            let actual = if field == "expeditionary_owner" && (raw as i32) <= 0 {
                None
            } else {
                country_tag(raw, links)?
            };
            if actual != expected {
                report.difference(
                    format!("{label}.{field}"),
                    tag_text(actual),
                    tag_text(expected),
                );
            }
        }
    }
    for &id in forces.keys() {
        report.difference(format!("task force {id}"), "present", "absent");
    }
    let mut ships = BTreeMap::new();
    for row in frame.complete::<tables::Ships>()? {
        if row.id == 0 || ships.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate ship handle"));
        }
    }
    for (&id, expected) in &save.ships {
        let label = format!("ship type={} id={}", id as u32, id >> 32);
        if !save.task_forces.contains_key(&expected.task_force) {
            return Err(invalid("saved ship references missing task force"));
        }
        let Some(actual) = ships.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.ships += 1;
        ship_names::compare(&label, &actual.naming, expected.ship_name.as_ref(), report);
        for (field, actual, expected) in [
            ("manpower", &actual.acquired_manpower, expected.manpower),
            (
                "max_manpower",
                &actual.required_manpower,
                expected.max_manpower,
            ),
        ] {
            match *actual {
                Recorded::NotRecorded | Recorded::Value(None) => {
                    report.limitation(CREW_NOT_RECORDED, format!("{label}.{field}"));
                }
                Recorded::Value(Some(value)) => {
                    report.compared(&format!("Ships.{field}"), 1);
                    if Some(value) != expected {
                        report.difference(format!("{label}.{field}"), value, expected);
                    }
                }
            }
        }
        if actual.task_force != expected.task_force {
            report.difference(
                format!("{label}.task_force"),
                actual.task_force,
                expected.task_force,
            );
        }
        if actual.sunk_convoys != expected.sunk_convoys {
            report.difference(
                format!("{label}.sunk_convoys"),
                actual.sunk_convoys,
                expected.sunk_convoys,
            );
        }
        report.scalar(
            &label,
            "strength",
            Some(actual.strength),
            Some(&expected.strength),
        );
        report.scalar(
            &label,
            "organisation",
            Some(actual.organisation),
            Some(&expected.organisation),
        );
        report.scalar_displayed(
            &label,
            "experience",
            Some(actual.experience),
            Some(&expected.experience),
            "Ships.experience",
            replay_data::rows::ShipRow::EXPERIENCE_STEP,
        );
    }
    for id in ships.keys() {
        report.difference(format!("ship {id}"), "present", "absent");
    }
    report.limitation("ship naming flags serialize Boolean meaning only; raw non-Boolean bits need matched native evidence", "Ships.naming.ordered and programmatic");
    report.limitation("retained ship display cache is unsaved; matched native evidence at the same full reference and supplying hour is required", "Ships.name");
    report.limitation("ship category and numeric class id are not independently serialized; saved definition names are not substituted for native ids", "Ships");
    report.limitation("task-force serializer omits signed-nonpositive expeditionary references; absent fields verify the omission predicate, not those raw bits", "TaskForces");
    report.limitation("the task-force save record holds no combat-reference count; it needs a same-hour live comparison", "TaskForces.combat_references");
    Ok(())
}

pub fn compare_naval_combat_sources(
    frame: &DecodedFrame,
    save: &Save,
    report: &mut Report,
) -> io::Result<()> {
    use replay_data::rows::NavalProvinceCombatRow as Entry;
    let naval: BTreeSet<u64> = frame
        .complete::<tables::NavalCombats>()?
        .iter()
        .map(|row| row.id)
        .collect();
    let land: BTreeSet<u64> = frame
        .complete::<tables::LandCombats>()?
        .iter()
        .map(|row| row.id)
        .collect();
    let provinces = frame.complete::<tables::NavalProvinceCombats>()?;
    for row in provinces {
        for (combat, &kind) in row.combats.iter().zip(&row.kinds) {
            let census = if kind == Entry::NAVAL { &naval } else { &land };
            if !census.contains(combat) {
                report.difference(
                    format!("NavalProvinceCombats province {} entry", row.province),
                    (combat, kind),
                    "absent from its combat census",
                );
            }
        }
    }
    let transfers: BTreeMap<u64, &[u64]> = frame
        .complete::<tables::NavalCombatTransfers>()?
        .iter()
        .map(|row| (row.combat, row.transfers.as_slice()))
        .collect();
    for (&combat, expected) in &save.naval_combats {
        let actual = transfers.get(&combat).copied().unwrap_or_default();
        if actual != expected.transfers.as_slice() {
            report.difference(
                format!(
                    "NavalCombatTransfers combat type={} id={}",
                    combat as u32,
                    combat >> 32
                ),
                actual,
                &expected.transfers,
            );
        }
    }
    for combat in transfers
        .keys()
        .filter(|combat| !save.naval_combats.contains_key(combat))
    {
        report.difference("NavalCombatTransfers combat", combat, "absent");
    }
    let saved = &save.counter_inputs.naval_transfers;
    let named: BTreeSet<u64> = save
        .naval_combats
        .values()
        .flat_map(|combat| combat.transfers.iter().copied())
        .filter(|&transfer| transfer != 0 && saved.contains_key(&transfer))
        .collect();
    let units: BTreeMap<u64, &[u64]> = frame
        .complete::<tables::NavalTransferUnits>()?
        .iter()
        .map(|row| (row.transfer, row.units.as_slice()))
        .collect();
    for &transfer in &named {
        let label = format!(
            "NavalTransferUnits transfer type={} id={}",
            transfer as u32,
            transfer >> 32
        );
        match units.get(&transfer) {
            Some(&actual) if actual == saved[&transfer].as_slice() => {}
            Some(actual) => report.difference(label, actual, &saved[&transfer]),
            None => report.difference(label, "absent", &saved[&transfer]),
        }
    }
    for transfer in units.keys().filter(|transfer| !named.contains(transfer)) {
        report.difference(
            "NavalTransferUnits transfer",
            transfer,
            "not a saved transfer of a saved combat",
        );
    }
    report.compared("NavalProvinceCombats", provinces.len());
    report.compared("NavalCombatTransfers", save.naval_combats.len());
    report.compared("NavalTransferUnits", named.len());
    report.limitation(
        "the province combat vectors are not serialized; each entry is only joined to the combat census of its class",
        "NavalProvinceCombats",
    );
    Ok(())
}

pub fn compare_naval_combats(
    frame: &DecodedFrame,
    save: &Save,
    membership: &content_reader_save::PartialMapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut combats = BTreeMap::new();
    let mut ordinals = BTreeSet::new();
    for row in frame.complete::<tables::NavalCombats>()? {
        if row.id == 0 || combats.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate full naval combat handle"));
        }
        if !ordinals.insert(row.registry_ordinal) {
            return Err(invalid("two naval combats share a registry ordinal"));
        }
    }
    report.limitation(
        "the mixed combat registry order is not serialized; only its uniqueness is checked",
        "NavalCombats.registry_ordinal",
    );
    for (&id, expected) in &save.naval_combats {
        let label = format!("naval combat type={} id={}", id as u32, id >> 32);
        if expected.province == 0 || !membership.contains_province(expected.province) {
            return Err(invalid(
                "saved naval combat location is absent from verified map",
            ));
        }
        let actual = combats.remove(&id);
        let Some(actual) = actual else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.naval_combats += 1;
        if actual.province != expected.province {
            report.difference(
                format!("{label}.province"),
                actual.province,
                expected.province,
            );
        }
        for (field, captured, saved) in [
            (
                "attacker_positioning",
                actual.attacker_positioning,
                &expected.attacker_positioning,
            ),
            (
                "defender_positioning",
                actual.defender_positioning,
                &expected.defender_positioning,
            ),
            (
                "attacker_anti_air",
                actual.attacker_anti_air,
                &expected.attacker_anti_air,
            ),
            (
                "defender_anti_air",
                actual.defender_anti_air,
                &expected.defender_anti_air,
            ),
        ] {
            report.scalar(&label, field, Some(captured), Some(saved));
        }
    }
    for &id in combats.keys() {
        report.difference(format!("naval combat {id}"), "present", "absent");
    }
    let mut participants = BTreeMap::new();
    for row in frame.complete::<tables::NavalCombatParticipants>()? {
        if row.combat == 0
            || row.side > 1
            || participants
                .insert((row.combat, row.side, row.battle_slot), row)
                .is_some()
        {
            return Err(invalid(
                "invalid or duplicate naval combat participant identity",
            ));
        }
    }
    for (&key, expected) in &save.naval_combat_participants {
        let label = format!(
            "naval participant combat={} side={} slot={}",
            key.0, key.1, key.2
        );
        if !save.naval_combats.contains_key(&expected.combat) {
            return Err(invalid("saved participant has missing combat"));
        }
        let Some(actual) = participants.remove(&key) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.naval_combat_participants += 1;
        if actual.ship != expected.ship.unwrap_or(0) {
            report.difference(format!("{label}.ship"), actual.ship, expected.ship);
        }
        if actual.state != expected.state {
            report.difference(format!("{label}.state"), actual.state, expected.state);
        }
        let owner = country_tag(actual.owner, links)?.unwrap_or(b"---");
        if owner != expected.owner {
            report.difference(
                format!("{label}.owner"),
                tag_text(Some(owner)),
                tag_text(Some(&expected.owner)),
            );
        }
    }
    for key in participants.keys() {
        report.difference(
            format!(
                "naval participant combat={} side={} slot={}",
                key.0, key.1, key.2
            ),
            "present",
            "absent",
        );
    }
    report.limitation("naval combat comparison covers captured location, side scalars and participant identity/state; convoy/client handles and other reduced save details are not recorded, and hourly state cannot certify intervening events", "NavalCombats / NavalCombatParticipants");
    Ok(())
}

pub fn compare_air_wings(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut wings = BTreeMap::new();
    for row in frame.complete::<tables::AirWings>()? {
        if row.id == 0 || wings.insert(row.id, row).is_some() {
            return Err(invalid("null or duplicate full air wing handle"));
        }
    }
    let mut saved_aces = BTreeMap::new();
    for wing in &save.air.wings {
        if saved_aces.insert(wing.id, wing.ace).is_some() {
            return Err(invalid("duplicate saved air wing in the air pack"));
        }
    }
    let ace_ids: BTreeSet<u64> = save.air.aces.iter().map(|ace| ace.id).collect();
    for (&id, expected) in &save.air_wings {
        let label = format!("air wing type={} id={}", id as u32, id >> 32);
        let Some(actual) = wings.remove(&id) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.air_wings += 1;
        match actual.exile_country {
            Recorded::NotRecorded | Recorded::Value(None) => {
                report.limitation(CREW_NOT_RECORDED, format!("{label}.government_in_exile_tag"));
            }
            Recorded::Value(Some(raw)) => {
                report.compared("AirWings.exile_country", 1);
                let tag = if raw > 0 {
                    Some(required_country_tag(raw as u32, links)?)
                } else {
                    None
                };
                if tag != expected.government_in_exile_tag.as_deref() {
                    report.difference(
                        format!("{label}.government_in_exile_tag"),
                        tag_text(tag),
                        tag_text(expected.government_in_exile_tag.as_deref()),
                    );
                }
            }
        }
        match actual.acquired_manpower {
            Recorded::NotRecorded | Recorded::Value(None) => {
                report.limitation(CREW_NOT_RECORDED, format!("{label}.manpower"));
            }
            Recorded::Value(Some(value)) => {
                report.compared("AirWings.acquired_manpower", 1);
                if Some(value) != expected.manpower {
                    report.difference(format!("{label}.manpower"), value, expected.manpower);
                }
            }
        }
        report.compared("AirWings.replacement_setting", 1);
        let setting = match &actual.replacement_setting {
            replay_data::rows::Recorded::Value(Some(value)) => Some(*value),
            _ => None,
        };
        if setting.is_none()
            || expected.reinforcement_setting.is_none()
            || setting != expected.reinforcement_setting
        {
            report.difference(
                format!("{label}.reinforcement_setting"),
                setting,
                expected.reinforcement_setting,
            );
        }
        if expected.name.is_none() {
            report.difference(
                format!("{label}.name presence"),
                "absent from ordinary save",
                "required name key",
            );
        }
        match &actual.name {
            replay_data::rows::Recorded::NotRecorded => report.limitation(
                "stored air wing name not recorded by this format",
                format!("{label}.name"),
            ),
            replay_data::rows::Recorded::Value(None) => report.difference(
                format!("{label}.name: guarded read or name arena refusal"),
                "unavailable",
                &expected.name,
            ),
            replay_data::rows::Recorded::Value(Some(name)) => {
                report.compared("AirWings.name", 1);
                let text = super::serialized_text(report, &format!("{label}.name"), name);
                if expected
                    .name
                    .as_ref()
                    .is_some_and(|expected| expected.as_slice() != text)
                {
                    report.difference(format!("{label}.name"), text, &expected.name);
                }
            }
        }
        let owner = required_country_tag(actual.owner, links)?;
        if owner != expected.owner {
            report.difference(
                format!("{label}.pool_owner"),
                tag_text(Some(owner)),
                tag_text(Some(&expected.owner)),
            );
        }
        let country_override =
            country_tag(actual.country_override as u32, links)?.unwrap_or(b"---");
        if country_override != expected.country_override {
            report.difference(
                format!("{label}.country_override"),
                tag_text(Some(country_override)),
                tag_text(Some(&expected.country_override)),
            );
        }
        for (field, actual, expected) in [
            ("air_base", actual.air_base, expected.air_base),
            (
                "mission_mask",
                u64::from(actual.mission_mask),
                u64::from(expected.mission_mask),
            ),
        ] {
            if actual != expected {
                report.difference(format!("{label}.{field}"), actual, expected);
            }
        }
        if actual.planes != expected.planes {
            report.difference(format!("{label}.planes"), actual.planes, expected.planes);
        }
        if actual.pool_ordinal != expected.pool_ordinal {
            report.difference(
                format!("{label}.pool_ordinal"),
                actual.pool_ordinal,
                expected.pool_ordinal,
            );
        }
        if let Some(active) = expected.mission_active
            && (actual.mission_active != 0) != active
        {
            report.difference(
                format!("{label}.mission_active"),
                actual.mission_active != 0,
                active,
            );
        }
        if actual.mission_mask & 0x800 != 0 {
            match actual.training_stop() {
                None => report.limitation(
                    TRAINING_STOP_NOT_RECORDED,
                    format!("{label}.stop_training_at_max_xp"),
                ),
                Some(stop) => {
                    report.compared("AirWings.training_stop", 1);
                    if stop != expected.stop_training_at_max_xp {
                        report.difference(
                            format!("{label}.stop_training_at_max_xp"),
                            stop,
                            expected.stop_training_at_max_xp,
                        );
                    }
                }
            }
        }
        if actual.mission_region != expected.mission_region {
            report.difference(
                format!("{label}.mission_region"),
                actual.mission_region,
                expected.mission_region,
            );
        }
        let saved_ace = *saved_aces
            .get(&id)
            .ok_or_else(|| invalid(format!("{label} is absent from the saved air pack")))?;
        report.compared("AirWings.ace", 1);
        match saved_ace {
            Some(ace) if actual.ace != ace => {
                report.difference(format!("{label}.ace"), actual.ace, ace);
            }
            Some(_) => {}
            None if actual.ace == 0 => {}
            None if ace_ids.contains(&actual.ace) => {
                report.difference(
                    format!("{label}.ace"),
                    actual.ace,
                    "omitted: the reference must not resolve",
                );
            }
            None => report.limitation(
                "the wing writer omits a nonzero ace reference that does not resolve to a registered ace, and no saved ace has this reference; its exact bits are not saved",
                format!("{label}.ace"),
            ),
        }
    }
    for id in wings.keys() {
        report.difference(format!("air wing {id}"), "present", "absent");
    }
    report.limitation("air wing transfer override is serialized through a resolved state or carrier, not its raw airbase handle; an omitted transfer field can also be an unresolved override", "AirWings.base_override");
    report.limitation("the wing save record holds no eligibility veto, counter or limit and no attached orders-group identity; they need a same-hour live comparison", "AirWings eligibility operands / attached_orders_group");
    report.limitation("the wing save writer omits the retained attachment-coverage byte. Compare the Air grid markers and hovers at the matched hour; training-stop and resolved raid links remain independently save-compared", "AirWings.map_flags attachment coverage");
    report.limitation(
        "the air wing save writer does not write the wing base range that the map-range calculation scales; the save has no field for it",
        "AirWings.base_range_raw",
    );
    Ok(())
}

pub fn compare_army_orders(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut orders = BTreeMap::new();
    for row in frame.complete::<tables::ArmyOrders>()? {
        if row.owner_group == 0
            || orders
                .insert((row.owner_group, row.instance), row)
                .is_some()
        {
            return Err(invalid("null or duplicate scoped army order identity"));
        }
    }
    for (&key, expected) in &save.army_orders {
        let label = format!("army order group={} instance={}", key.0, key.1);
        let Some(actual) = orders.remove(&key) else {
            report.difference(label, "absent", "present");
            continue;
        };
        report.army_orders += 1;
        let country = required_country_tag(actual.country, links)?;
        if country != expected.country {
            report.difference(
                format!("{label}.country"),
                tag_text(Some(country)),
                tag_text(Some(&expected.country)),
            );
        }
        if actual.order_type != expected.order_type {
            report.difference(
                format!("{label}.type"),
                actual.order_type,
                expected.order_type,
            );
        }
        if actual.active != expected.active {
            report.difference(
                format!("{label}.can_execute"),
                actual.active,
                expected.active,
            );
        }
        if actual.paradrop_target != expected.paradrop_target {
            report.difference(
                format!("{label}.invasion_source"),
                actual.paradrop_target,
                expected.paradrop_target,
            );
        }
        for (field, actual, expected) in [
            ("path", &actual.path, &expected.path),
            ("states", &actual.target_states, &expected.target_states),
        ] {
            if actual != expected {
                report.difference(format!("{label}.{field}"), actual, expected);
            }
        }
    }
    for key in orders.keys() {
        report.difference(
            format!("army order group={} instance={}", key.0, key.1),
            "present",
            "absent",
        );
    }
    Ok(())
}

pub fn compare_divisions(
    frame: &DecodedFrame,
    save: &Save,
    membership: &content_reader_save::PartialMapMembership,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut divisions = BTreeMap::new();
    for division in frame.complete::<tables::Divisions>()? {
        if division.id == 0 || divisions.insert(division.id, division).is_some() {
            return Err(invalid("null or duplicate full division handle in replay"));
        }
    }
    for (&id, expected) in &save.divisions {
        let label = format!("division type={} id={}", id as u32, id >> 32);
        for tag in [
            Some(&expected.roster_owner),
            Some(&expected.logical_owner),
            expected.expeditionary_owner.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if !save.countries.contains_key(tag) {
                return Err(invalid(format!(
                    "{label} references unknown saved country {}",
                    String::from_utf8_lossy(tag)
                )));
            }
        }
        if expected
            .province
            .is_some_and(|province| province == 0 || !membership.contains_province(province))
        {
            return Err(invalid(format!(
                "{label} location is absent from the verified map or is its sentinel"
            )));
        }
        let actual = divisions.remove(&id);
        let Some(actual) = actual else {
            report.difference(&label, "absent", "present");
            continue;
        };
        report.divisions += 1;
        for (field, captured, saved) in [
            (
                "roster_owner",
                actual.roster_owner,
                Some(expected.roster_owner.as_slice()),
            ),
            (
                "logical_owner",
                actual.logical_owner,
                Some(expected.logical_owner.as_slice()),
            ),
            (
                "expeditionary_owner",
                actual.expeditionary_owner,
                expected.expeditionary_owner.as_deref(),
            ),
        ] {
            if field == "expeditionary_owner" && (captured as i32) < 0 {
                return Err(invalid(format!(
                    "{label}.expeditionary_owner has invalid signed country reference {}; the save omits nonpositive references and cannot verify these bits",
                    captured as i32
                )));
            }
            let captured = country_tag(captured, links)?;
            if captured != saved {
                report.difference(
                    format!("{label}.{field}"),
                    tag_text(captured),
                    tag_text(saved),
                );
            }
        }
        if actual.province != expected.province.unwrap_or(0) {
            report.difference(
                format!("{label}.province"),
                actual.province,
                expected.province,
            );
        }
        if actual.template != expected.template.unwrap_or(0) {
            report.difference(
                format!("{label}.template"),
                actual.template,
                expected.template,
            );
        } else if expected.template.is_none() {
            report.limitation("division has a null standard template and the save omits its template reference; no fake-intel template was substituted", label.clone());
        }
        report.scalar_displayed(
            &label,
            "hit_points (save strength)",
            Some(actual.hit_points),
            Some(&expected.hit_points),
            "Divisions.hit_points",
            DivisionRow::HIT_POINTS_STEP,
        );
        report.scalar_displayed(
            &label,
            "organisation",
            Some(actual.organisation),
            Some(&expected.organisation),
            "Divisions.organisation",
            DivisionRow::ORGANISATION_STEP,
        );
    }
    for &id in divisions.keys() {
        report.difference(
            format!("division type={} id={}", id as u32, id >> 32),
            "present",
            "absent",
        );
    }
    Ok(())
}
