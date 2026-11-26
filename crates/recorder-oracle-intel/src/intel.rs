
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_save::{GameDate, characters as saved_characters, intel as saved};
use replay_data::{
    rows::{CountryRow, TableId, tables},
};
use replay_frame::frame::{DecodedFrame, TableView};

use super::{Report, TokenNames, complete, country_tag, invalid, required_country_tag};

mod network_auxiliary;
mod sources;
pub use sources::compare as compare_sources;

#[derive(Clone, Copy)]
pub struct ScopeKinds {
    pub global: u8,
    pub country: u8,
    pub state: u8,
    pub character: u8,
}

pub fn compare(
    frame: &DecodedFrame,
    save: &saved::Pack,
    rosters: &content_reader_save::country_rosters::Pack,
    characters: Option<&saved_characters::Manager>,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    intelligence: Result<&content_reader_military::intelligence::AgencyUpgrades, &str>,
    scopes: ScopeKinds,
    report: &mut Report,
) -> io::Result<()> {
    completion_event_limitation(report);
    let mut completed = report.attempt("IntelligenceAgencies", |report| {
        compare_agencies(frame, save, countries, report)
    });
    completed &= report.attempt("AgencyUpgradeLevels", |report| {
        compare_agency_upgrade_levels(frame, save, countries, tokens, report)
    });
    completed &= report.attempt("AgencyUpgradeLevels identities", |report| {
        compare_agency_upgrade_identities(frame, countries, tokens, intelligence, report)
    });
    completed &= report.attempt("Subjects", |report| {
        compare_subjects(frame, save, countries, report)
    });
    completed &= report.attempt("Capitulation", |report| {
        compare_capitulation(frame, save, rosters, countries, report)
    });
    completed &= report.attempt("ScriptFlags", |report| {
        compare_flags(
            frame,
            save,
            countries,
            tokens,
            scopes.country,
            scopes.character,
            report,
        )
    });
    completed &= report.attempt("ScriptFlags characters", |report| {
        compare_character_flags(frame, characters, tokens, scopes.character, report)
    });
    completed &= report.attempt("OperationTokens", |report| {
        compare_operation_tokens(frame, save, countries, tokens, report)
    });
    completed &= report.attempt("OperativeMemberships", |report| {
        compare_operative_memberships(frame, save, countries, tokens, report)
    });
    completed &= report.attempt("OperativeMissions", |report| {
        compare_operative_missions(frame, save, countries, tokens, report)
    });
    completed &= report.attempt("CapturedOperativeReferences", |report| {
        compare_captured_operatives(frame, save, countries, report)
    });
    completed &= report.attempt("Cryptology", |report| {
        compare_cryptology(frame, save, countries, report)
    });
    completed &= report.attempt("RaidIntelInputs", |report| {
        compare_raid_intel(frame, save, countries, report)
    });
    completed &= report.attempt("IntelNetworks", |report| {
        compare_intel_networks(frame, save, countries, tokens, report)
    });
    completed &= report.attempt("IntelNetworkSummaries", |report| {
        compare_network_summaries(frame, save, countries, report)
    });
    completed &= report.attempt("IntelNetworkAuxiliaries", |report| {
        network_auxiliary::compare(frame, save, countries, tokens, report)
    });
    completed &= report.attempt("StrategicOperatives", |report| {
        compare_strategic_operatives(frame, save, countries, report)
    });
    completed &= report.attempt("PropagandaEfforts", |report| {
        compare_propaganda(frame, save, report)
    });
    completed &= report.attempt("Autonomy", |report| {
        compare_autonomy(frame, save, countries, tokens, report)
    });
    completed &= report.attempt("ScriptVariables", |report| {
        compare_variables(frame, save, characters, countries, scopes, report)
    });
    completed &= report.attempt("IncomingDiplomaticActions", |report| {
        compare_incoming_actions(frame, save, countries, report)
    });
    completed &= report.attempt("Operations", |report| {
        compare_operations(frame, save, countries, tokens, report)
    });
    completed &= report.attempt("OperationAssignments", |report| {
        compare_operation_assignments(frame, save, report)
    });
    report.limitation(
        "country tags cannot distinguish raw authenticated country-reference aliases",
        "intel, subjects, capitulation, flags, variables and operation-token country joins",
    );
    if completed {
        Ok(())
    } else {
        Err(invalid(
            "intel table comparisons are incomplete; individual failures are reported",
        ))
    }
}

pub fn compare_operation_buffers(
    frame: &DecodedFrame,
    save: &saved::Pack,
    governance: &content_reader_save::governance::Pack,
    state_ids: &BTreeSet<u32>,
    membership: Result<&content_reader_save::PartialMapMembership, &str>,
    report: &mut Report,
) -> io::Result<()> {
    let buffers = complete::<tables::AgencyOperationBuffers>(frame)?;
    if buffers.len() != 1 || buffers[0].snapshot != 0 {
        return Err(invalid(
            "agency operation buffers lack the shared client snapshot",
        ));
    }
    let mut saved_operations = BTreeMap::new();
    for country in save.countries.values() {
        for operation in country
            .operations
            .iter()
            .flat_map(|manager| manager.running.iter().flatten())
        {
            if saved_operations.insert(operation.id, operation).is_some() {
                return Err(invalid(
                    "duplicate saved operation id in buffer target comparison",
                ));
            }
        }
    }
    let buffer = &buffers[0];
    let mut compared = 0;
    let mut references = 0;
    let mut unsaved = 0;
    for (bucket, entries) in [
        ("completed", &buffer.completed),
        ("running", &buffer.running),
        ("ready", &buffer.ready),
        ("unready", &buffer.unready),
    ] {
        for (ordinal, entry) in entries.iter().enumerate() {
            references += 1;
            let Some(operation) = saved_operations.get(&entry.operation) else {
                unsaved += 1;
                continue;
            };
            let state = if let Some(province) = operation.target_province {
                let membership = membership.map_err(|why| {
                    invalid(format!(
                        "buffered operation target province needs verified map membership: {why}"
                    ))
                })?;
                match membership.provinces.get(&province) {
                    Some(content_reader_save::ProvinceMembership::Known(state)) => state.unwrap_or(0),
                    Some(content_reader_save::ProvinceMembership::Unknown(_)) => {
                        return Err(invalid(
                            "buffered operation target province has unknown state membership",
                        ));
                    }
                    None => {
                        return Err(invalid(
                            "buffered operation target province is outside verified map membership",
                        ));
                    }
                }
            } else {
                let capital = governance
                    .countries
                    .get(&operation.target)
                    .and_then(|country| country.capital)
                    .ok_or_else(|| invalid("buffered operation target has no saved capital"))?;
                if capital > 0 && state_ids.contains(&(capital as u32)) {
                    capital as u32
                } else {
                    0
                }
            };
            equal(
                report,
                &format!("AgencyOperationBuffers.{bucket}[{ordinal}].state"),
                entry.state,
                Some(state),
            );
            compared += 1;
        }
    }
    report.compared("AgencyOperationBuffers resolved targets", compared);
    report.limitation(
        "the shared agency UI operation lists are cleared at runtime reset and rebuilt on load/open/daily refresh; no save writer serializes their membership, bucket order or freshness. Resolved targets of saved instances compare against saved operands and verified province membership; references without saved instances have no save-verifiable target",
        format!("AgencyOperationBuffers: {} decoded snapshots, {references} buffered references; {compared} resolved targets compared, {unsaved} references without saved instances; membership/order/freshness runtime-only", buffers.len()),
    );
    Ok(())
}

fn compare_captured_operatives(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::<&[u8], BTreeMap<u32, u64>>::new();
    for row in complete::<tables::CapturedOperativeReferences>(frame)? {
        let tag = required_country_tag(row.country, countries)?;
        if !save.countries.contains_key(tag) {
            return Err(invalid(
                "captured-operative agency country absent from save",
            ));
        }
        if recorded
            .entry(tag)
            .or_default()
            .insert(row.ordinal, row.operative)
            .is_some()
        {
            return Err(invalid("duplicate captured-operative ordinal"));
        }
    }
    let mut matched = 0;
    for (tag, country) in report.canonical_countries(&save.countries) {
        let actual = recorded.remove(tag.as_slice()).unwrap_or_default();
        for (expected, &ordinal) in actual.keys().enumerate() {
            if ordinal as usize != expected {
                return Err(invalid(
                    "captured-operative ordinals are not contiguous from zero",
                ));
            }
        }
        let expected = country
            .agency
            .as_ref()
            .map(|agency| agency.captured.as_slice())
            .unwrap_or_default();
        equal(
            report,
            &format!("{}.captured.ordered_operatives", text(tag)),
            actual.values().copied().collect::<Vec<_>>(),
            expected
                .iter()
                .map(|entry| entry.operative)
                .collect::<Vec<_>>(),
        );
        matched += actual.len().min(expected.len());
    }
    report.compared("CapturedOperativeReferences", matched);
    report.limitation("the game saves the captured reference's stored country, not the resolved commander owner; owner and failed reference resolution have no exact save counterpart", "CapturedOperativeReferences.owner");
    Ok(())
}

fn completion_event_limitation(report: &mut Report) {
    report.limitation(
        "OperationCompletions are runtime observations; the operation writer serializes only still-existing instances and does not retain completion event history; individually refused completion descriptions retain their named reason and stored byte length but have no save comparator",
        "OperationCompletions; the Operations comparison still verifies stored outcomes of surviving instances",
    );
}

fn compare_raid_intel(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let expected: BTreeSet<_> = countries.values().map(|c| c.canonical_slot).collect();
    let mut seen = BTreeSet::new();
    for row in complete::<tables::RaidIntelInputs>(frame)? {
        let country = countries
            .get(&row.country)
            .ok_or_else(|| invalid("raid intel country unresolved"))?;
        if !seen.insert(country.canonical_slot) {
            return Err(invalid("duplicate raid intel country"));
        }
        if row.strategic_present > 1 {
            return Err(invalid("raid intel strategic presence is not boolean"));
        }
        let label = format!(
            "raid intel country {}.strategic_present",
            country.canonical_slot
        );
        let present = save
            .strategic_operatives
            .as_ref()
            .is_some_and(|rows| rows.contains_key(&country.canonical_slot));
        if (row.strategic_present != 0) != present {
            report.difference(label, row.strategic_present != 0, present);
        }
    }
    for slot in expected.difference(&seen) {
        report.difference(format!("raid intel country {slot}"), "absent", "present");
    }
    report.compared("RaidIntelInputs", seen.len());
    report.limitation("the strategic operative save writer omits the operative-reference roster; it cannot certify the counterintelligence operands selected from that roster", "RaidIntelInputs.operatives");
    report.limitation("country, operative and static modifier totals are runtime accumulators, not save fields; cryptology-owner pointer presence is not inferred from an omitted save block", "RaidIntelInputs.country_defense / crypto_strength / operatives.defense / operatives.context");
    Ok(())
}

fn compare_incoming_actions(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut by_country = BTreeMap::<&[u8], Vec<_>>::new();
    for actual in complete::<tables::IncomingDiplomaticActions>(frame)? {
        let tag = required_country_tag(actual.country, countries)?;
        by_country.entry(tag).or_default().push(actual);
    }
    let mut matched = 0;
    for (tag, expected) in report.canonical_countries(&save.countries) {
        let label = format!("{}.incoming_diplomatic_action", text(tag));
        let actual = by_country.remove(tag.as_slice()).unwrap_or_default();
        let Some(expected) = &expected.incoming_actions else {
            if !actual.is_empty() {
                report.difference(&label, actual.len(), "absent queue object");
            }
            report.limitation(
                "a country without a serialized incoming queue block has no native queue object to compare",
                label,
            );
            continue;
        };
        equal(
            report,
            &format!("{label}.count"),
            actual.len(),
            expected.len(),
        );
        let mut recorded = BTreeMap::new();
        for row in &actual {
            if recorded.insert(row.ordinal, *row).is_some() {
                return Err(invalid("duplicate recorded incoming action index"));
            }
        }
        for expected in expected {
            let label = format!("{label}[{}]", expected.ordinal);
            let Some(actual) = recorded.remove(&expected.ordinal) else {
                report.difference(label, "absent", expected.id);
                continue;
            };
            equal(report, &format!("{label}.id"), actual.handle, expected.id);
            equal(
                report,
                &format!("{label}.initiator_matters"),
                actual.action_type as u8 != 0,
                expected.initiator_matters.ok_or_else(|| {
                    invalid("incoming diplomatic action lacks saved initiator_matters")
                })?,
            );
            for (name, captured, saved) in [
                ("actor", actual.actor, &expected.actor),
                (
                    "original_actor",
                    actual.original_actor,
                    &expected.original_actor,
                ),
                ("recipient", actual.recipient, &expected.recipient),
                (
                    "original_recipient",
                    actual.original_recipient,
                    &expected.original_recipient,
                ),
            ] {
                match super::country_tag(captured, countries)? {
                    Some(tag) => equal(report, &format!("{label}.{name}"), tag, saved.as_slice()),
                    None => {
                        report.difference(
                            format!("{label}.{name}"),
                            captured,
                            String::from_utf8_lossy(saved).into_owned(),
                        );
                    }
                }
            }
            equal(
                report,
                &format!("{label}.date"),
                actual.hour,
                expected.date.engine_hour()?,
            );
            equal(
                report,
                &format!("{label}.last_command_date"),
                actual.last_command_hour,
                expected.last_command_date.engine_hour()?,
            );
            matched += 1;
        }
        for ordinal in recorded.into_keys() {
            report.difference(format!("{label}[{ordinal}]"), "present", "absent");
        }
    }
    for (tag, rows) in by_country {
        report.difference(
            format!("{}.incoming_diplomatic_action", text(tag)),
            rows.len(),
            "country absent from the save",
        );
    }
    report.limitation(
        "the game zeroes the wrapper's byte and no writer serializes it; the action body's definition name supplies its kind, which is not captured",
        "IncomingDiplomaticActions.status and kind",
    );
    report.limitation(
        "The action writer saves its body type; the historical recorded action_type retains the raw dword, whose low byte is initiator_matters",
        "IncomingDiplomaticActions.type",
    );
    report.compared("IncomingDiplomaticActions", matched);
    Ok(())
}

fn compare_operative_memberships(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    const HANDLE_TYPE: u32 = 4713;
    type SavedActive<'a> = BTreeMap<(u64, Vec<u8>), Vec<(u32, &'a saved::OperativeEntry)>>;
    let mut saved_active = SavedActive::new();
    let mut saved_recruit: BTreeMap<(u64, Vec<u8>, u8), Vec<&saved::OperativeEntry>> =
        BTreeMap::new();
    let pool_a_key = token(tokens, 0x4b44).filter(|name| !name.contains(&0));
    let mut operation_targets = BTreeMap::new();
    for (owner, country) in report.canonical_countries(&save.countries) {
        if let Some(running) = country
            .operations
            .as_ref()
            .and_then(|ops| ops.running.as_ref())
        {
            for instance in running {
                if operation_targets
                    .insert(instance.id, instance.target.as_slice())
                    .is_some()
                {
                    return Err(invalid(
                        "duplicate saved operation reference across countries",
                    ));
                }
            }
        }
        let Some(agency) = &country.agency else {
            continue;
        };
        for (ordinal, entry) in agency.operatives.iter().enumerate() {
            if (entry.id & 0xffff_ffff) != u64::from(HANDLE_TYPE) {
                return Err(invalid("saved operative handle has an unexpected type"));
            }
            saved_active
                .entry((entry.id, owner.clone()))
                .or_default()
                .push((ordinal as u32, entry));
        }
        if let Some(recruitment) = &agency.recruitment {
            let mut rosters = vec![
                (2, &recruitment.recruitable),
                (3, &recruitment.not_to_spy_master),
            ];
            for (name, entries) in &recruitment.other_rosters {
                if pool_a_key == Some(name.as_slice()) {
                    rosters.push((1, entries));
                } else {
                    report.limitation(
                        "saved recruitment roster lacks an identified recorded token; entries retained and counted",
                        format!("{}.recruitment.{}: {} entries", text(owner), text(name), entries.len()),
                    );
                }
            }
            for (kind, entries) in rosters {
                for entry in entries {
                    if (entry.id & 0xffff_ffff) != u64::from(HANDLE_TYPE) {
                        return Err(invalid("saved recruitment handle has an unexpected type"));
                    }
                    saved_recruit
                        .entry((entry.id, owner.clone(), kind))
                        .or_default()
                        .push(entry);
                }
            }
        }
    }
    if saved_recruit.values().any(|entries| entries.len() != 1) {
        return Err(invalid("duplicate saved operative membership identity"));
    }
    let mut recorded = BTreeMap::new();
    for actual in complete::<tables::OperativeMemberships>(frame)? {
        if (actual.id & 0xffff_ffff) != u64::from(HANDLE_TYPE) {
            return Err(invalid("recorded operative handle has an unexpected type"));
        }
        let roster = required_country_tag(actual.roster_country, countries)?;
        country(save, roster)?;
        if actual.owner_country != 0 {
            country_tag(actual.owner_country, countries)?;
        }
        if actual.capturer_country != 0 {
            country_tag(actual.capturer_country, countries)?;
        }
        let ordinal = report
            .recorded(
                &actual.ordinal,
                "OperativeMemberships.ordinal",
                format!(
                    "{}.roster{}.operative.{}.ordinal",
                    text(roster),
                    actual.roster_kind,
                    actual.id
                ),
            )
            .copied();
        if recorded
            .insert(
                (actual.id, roster.to_vec(), actual.roster_kind),
                (actual, ordinal),
            )
            .is_some()
        {
            return Err(invalid("duplicate recorded operative membership identity"));
        }
    }
    let assignment_names = assignment_literals(tokens);
    let nationalities_key = token(tokens, 0x3d12);
    let assignment_key = token(tokens, engine_native_intelligence::coperative_leader::TOKEN_ASSIGNMENT);
    let literals = || OperativeLiterals {
        assignment_names: assignment_names.as_ref(),
        nationalities_key,
        assignment_key,
        tokens,
        operation_targets: &operation_targets,
    };
    let mut matched = 0;
    for ((id, owner), entries) in saved_active {
        let actual = recorded.remove(&(id, owner.clone(), 0));
        let unique_position = entries.len() == 1;
        if !unique_position {
            report.limitation(
                "repeated saved operative handle has multiple serialized positions; the single recorded membership cannot certify each repeated ordinal",
                format!("{}.operative.{id}.ordinal", text(&owner)),
            );
        }
        for (occurrence, (ordinal, expected)) in entries.into_iter().enumerate() {
            let label = format!("{}.operative.{id}#{occurrence}", text(&owner));
            let Some((actual, captured_ordinal)) = actual else {
                report.difference(label, "absent", expected.id);
                continue;
            };
            matched += 1;
            if let Some(actual) = captured_ordinal.filter(|_| unique_position) {
                equal(report, &format!("{label}.ordinal"), actual, ordinal);
            }
            compare_operative_row(report, &label, actual, expected, countries, literals())?;
        }
    }
    for ((id, owner, kind), entries) in saved_recruit {
        let actual = recorded.remove(&(id, owner.clone(), kind));
        for (occurrence, expected) in entries.into_iter().enumerate() {
            let label = format!("{}.recruitment.{id}#{occurrence}", text(&owner));
            let Some((actual, _)) = actual else {
                report.difference(label, "absent", expected.id);
                continue;
            };
            matched += 1;
            compare_operative_row(report, &label, actual, expected, countries, literals())?;
        }
    }
    for ((id, roster, kind), _) in recorded {
        if kind == 1 && pool_a_key.is_none() {
            report.limitation("operative POOL_A token 0x4B44 has no exact recorded spelling; row retained and counted",
                format!("{}.roster{kind}.operative.{id}", text(&roster)));
            continue;
        }
        report.difference(
            format!("{}.roster{kind}.operative.{id}", text(&roster)),
            "present",
            "absent from the save",
        );
    }
    report.compared("OperativeMemberships", matched);
    for (id, spelling) in [
        (0x4b45_u32, b"recruitable_operatives".as_slice()),
        (
            0x4b54,
            b"recruitable_operatives_not_to_spy_master".as_slice(),
        ),
    ] {
        if token(tokens, id) != Some(spelling) {
            report.limitation(
                "operative recruitment roster token has no exact LexicalTokens spelling",
                format!("token={id} expected={}", text(spelling)),
            );
        }
    }
    if pool_a_key.is_none() {
        report.limitation(
            "operative POOL_A token 0x4B44 has no exact recorded spelling; saved named recruitment lists are retained and counted",
            "OperativeMemberships.roster_kind=1",
        );
    }
    if nationalities_key != Some(b"nationalities".as_slice()) {
        report.limitation(
            "operative nationalities literal (token 0x3D12) has no exact LexicalTokens entry; comparison verifies presence/counts and ids, not literal names",
            format!("0x3D12={:?}", nationalities_key.map(String::from_utf8_lossy)),
        );
    }
    if assignment_key != Some(b"state".as_slice()) || assignment_names.is_none() {
        report.limitation(
            "operative assignment literals (0x1B7 with 0x4A51/0x4A52/0x4A53/0x4A50/0x336C/0x394C) resolve via LexicalTokens where exact; missing or NUL entries leave state values unverified beyond presence/counts and ids",
            format!("0x1B7={:?}", assignment_key.map(String::from_utf8_lossy)),
        );
    }
    report.limitation(
        "the operative owner_country reference and the unmatched operative field have no proved save mapping beyond the joined identity; retained natively but not verified here",
        "OperativeMemberships.owner, fd0",
    );
    report.limitation(
        "mission detection trial dispatches spotted on_action without a persistent detection flag; operative serializer saves no universal detected/exposed state",
        "operative detection events are not serialized or recorded; captured assignment/capturer/date remain separate saved facts",
    );
    Ok(())
}

struct OperativeLiterals<'a> {
    assignment_names: Option<&'a BTreeMap<u32, &'a [u8]>>,
    nationalities_key: Option<&'a [u8]>,
    assignment_key: Option<&'a [u8]>,
    tokens: &'a TokenNames<'a>,
    operation_targets: &'a BTreeMap<u64, &'a [u8]>,
}

fn compare_operative_row(
    report: &mut Report,
    label: &str,
    actual: &replay_data::rows::OperativeMembershipRow,
    expected: &saved::OperativeEntry,
    countries: &BTreeMap<u32, &CountryRow>,
    literals: OperativeLiterals<'_>,
) -> io::Result<()> {
    let _ = (literals.nationalities_key, expected.nationalities.len());
    equal(
        report,
        &format!("{label}.skill"),
        actual.skill,
        expected.skill,
    );
    match &expected.experience {
        Some(decimal) => report.scalar(
            label,
            "experience",
            Some(actual.experience_raw),
            Some(decimal),
        ),
        None => equal(
            report,
            &format!("{label}.experience"),
            actual.experience_raw,
            0,
        ),
    }
    match (&expected.captured, actual.capturer_country) {
        (None, 0) => {}
        (None, raw) => report.difference(format!("{label}.capturer"), raw, "absent"),
        (Some(tag), raw) => {
            if raw == 0 {
                report.difference(format!("{label}.capturer"), 0, text(tag));
            } else {
                equal(
                    report,
                    &format!("{label}.capturer"),
                    country_tag(raw, countries)?,
                    Some(tag.as_slice()),
                );
            }
            equal(
                report,
                &format!("{label}.capture_date"),
                Some(actual.captured_hour),
                saved_hour(expected.capture_date)?,
            );
        }
    }
    compare_assignment(
        report,
        label,
        actual.assignment,
        expected,
        literals.assignment_names,
        literals.assignment_key,
    );
    if actual.assignment == engine_native_intelligence::coperative_leader::COOLDOWN_ASSIGNMENT {
        compare_cooldown(
            report,
            label,
            (
                actual
                    .cooldown_kind
                    .require("cooldown kind")
                    .ok()
                    .copied()
                    .flatten(),
                actual
                    .cooldown_start_hour
                    .require("cooldown start")
                    .ok()
                    .copied()
                    .flatten(),
                actual
                    .cooldown_end_hour
                    .require("cooldown end")
                    .ok()
                    .copied()
                    .flatten(),
            ),
            expected,
        )?;
    }
    let saved_action = expected.mission_present;
    equal(
        report,
        &format!("{label}.action_present"),
        actual.action_variant.is_some(),
        saved_action,
    );
    compare_operative_placement(report, label, actual, expected, countries, &literals)?;
    Ok(())
}

fn compare_operative_placement(
    report: &mut Report,
    label: &str,
    actual: &replay_data::rows::OperativeMembershipRow,
    expected: &saved::OperativeEntry,
    countries: &BTreeMap<u32, &CountryRow>,
    literals: &OperativeLiterals<'_>,
) -> io::Result<()> {
    if let Some(variant) = actual.action_variant {
        let id = variant
            .checked_sub(1)
            .and_then(|index| engine_native_intelligence::operative_action::SAVE_TOKENS.get(index as usize))
            .ok_or_else(|| invalid("recorded operative mission variant is unsupported"))?;
        if let Some(name) = token(literals.tokens, *id).filter(|name| !name.contains(&0)) {
            equal(
                report,
                &format!("{label}.mission_name"),
                Some(name),
                expected.mission_name.as_deref(),
            );
        } else {
            report.limitation(
                "operative mission definition token has no exact LexicalTokens entry",
                format!("{label}.variant={variant} token={id}"),
            );
        }
    }
    if let Some(operation) = report
        .recorded(&actual.operation, "OperativeMemberships.operation", label)
        .copied()
    {
        equal(
            report,
            &format!("{label}.operation"),
            operation,
            expected.operation,
        );
    }
    if token(literals.tokens, 0x1b) == Some(b"name".as_slice()) {
        if let Some(name) = report.recorded(&actual.name, "OperativeMemberships.name", label) {
            equal(
                report,
                &format!("{label}.name"),
                name.as_slice(),
                expected.name.as_deref().unwrap_or_default(),
            );
        }
    } else {
        report.limitation(
            "operative name token 0x1B has no exact LexicalTokens entry",
            label,
        );
    }
    if token(literals.tokens, 0x6b) != Some(b"target".as_slice()) {
        report.limitation(
            "operative target token 0x6B has no exact LexicalTokens entry",
            label,
        );
        return Ok(());
    }
    let Some(actual_target) = report
        .recorded(
            &actual.target_country,
            "OperativeMemberships.target_country",
            label,
        )
        .copied()
    else {
        return Ok(());
    };
    let target = match actual.assignment {
        3 if expected.mission_present => expected.mission_target.as_deref(),
        4 => {
            let Some(reference) = expected.operation else {
                report.difference(
                    format!("{label}.active_operation"),
                    "absent reference",
                    "running operation reference",
                );
                return Ok(());
            };
            let Some(target) = literals.operation_targets.get(&reference) else {
                report.difference(
                    format!("{label}.active_operation {reference}"),
                    "absent from running operations",
                    "running operation",
                );
                return Ok(());
            };
            Some(*target)
        }
        _ => None,
    };
    let actual_target = actual_target
        .map(|raw| required_country_tag(raw, countries))
        .transpose()?;
    equal(
        report,
        &format!("{label}.target_country"),
        actual_target,
        target,
    );
    Ok(())
}

fn assignment_literals<'a>(tokens: &TokenNames<'a>) -> Option<BTreeMap<u32, &'a [u8]>> {
    use engine_native_intelligence::coperative_leader as leader;
    let names: BTreeMap<u32, &[u8]> = [
        (0, leader::ASSIGNMENT_LITERAL_IDLE),
        (1, leader::ASSIGNMENT_LITERAL_TRAINING),
        (2, leader::ASSIGNMENT_LITERAL_HIDING),
        (3, leader::ASSIGNMENT_LITERAL_MISSION),
        (4, leader::ASSIGNMENT_LITERAL_OPERATION),
        (5, leader::ASSIGNMENT_LITERAL_CAPTURED),
    ]
    .into_iter()
    .filter_map(|(value, token_id)| token(tokens, token_id).map(|name| (value, name)))
    .collect();
    (names.len() == 6 && !names.values().any(|name| name.contains(&0))).then_some(names)
}

fn compare_assignment(
    report: &mut Report,
    label: &str,
    actual: u32,
    expected: &saved::OperativeEntry,
    names: Option<&BTreeMap<u32, &[u8]>>,
    key: Option<&[u8]>,
) {
    let (Some(names), Some(state), Some(_)) = (names, &expected.state, key) else {
        return;
    };
    match names.iter().find(|(_, name)| **name == state.as_slice()) {
        Some((value, _)) => equal(report, &format!("{label}.assignment"), actual, *value),
        None => report.difference(
            format!("{label}.assignment"),
            actual,
            String::from_utf8_lossy(state).into_owned(),
        ),
    }
}

fn compare_cooldown(
    report: &mut Report,
    label: &str,
    operands: (Option<u32>, Option<i32>, Option<i32>),
    expected: &saved::OperativeEntry,
) -> io::Result<()> {
    let (Some(kind), Some(start), Some(end)) = operands else {
        report.limitation("cooldown operands were not recorded in this layout", label);
        return Ok(());
    };
    let reason: &[u8] = match kind {
        1 => b"reassigned",
        2 => b"harmed",
        3 => b"forced_into_hiding",
        4 => b"deployed",
        5 => b"withdrawing",
        _ => b"",
    };
    equal(
        report,
        &format!("{label}.cooldown_reason"),
        (kind != 0).then_some(reason),
        expected.cooldown_reason.as_deref(),
    );
    if kind != 0 {
        equal(
            report,
            &format!("{label}.leader_cooldown_start_date"),
            Some(start),
            saved_hour(expected.cooldown_start_date)?,
        );
        equal(
            report,
            &format!("{label}.leader_modifier_enable_date"),
            Some(end),
            saved_hour(expected.cooldown_end_date)?,
        );
    } else {
        report.limitation("native kind0 omits both cooldown dates from saves", label);
    }
    Ok(())
}

fn compare_operative_missions(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let handle_type = u64::from(engine_native_intelligence::coperative_leader::HANDLE_TYPE);
    type SavedActive<'a> = BTreeMap<(u64, Vec<u8>), Vec<(u32, &'a saved::OperativeEntry)>>;
    let mut saved_active = SavedActive::new();
    for (owner, country) in report.canonical_countries(&save.countries) {
        let Some(agency) = &country.agency else {
            continue;
        };
        for (ordinal, entry) in agency.operatives.iter().enumerate() {
            if (entry.id & 0xffff_ffff) != handle_type {
                return Err(invalid("saved operative handle has an unexpected type"));
            }
            saved_active
                .entry((entry.id, owner.clone()))
                .or_default()
                .push((ordinal as u32, entry));
        }
    }
    let mut recorded = BTreeMap::new();
    for actual in complete::<tables::OperativeMissions>(frame)? {
        if (actual.id & 0xffff_ffff) != handle_type {
            return Err(invalid("recorded operative handle has an unexpected type"));
        }
        let roster = required_country_tag(actual.roster_country, countries)?;
        country(save, roster)?;
        if actual.owner_country != 0 {
            country_tag(actual.owner_country, countries)?;
        }
        if recorded
            .insert((actual.id, roster.to_vec()), actual)
            .is_some()
        {
            return Err(invalid("duplicate recorded operative mission identity"));
        }
    }
    let assignment_names = assignment_literals(tokens);
    let assignment_key = token(tokens, engine_native_intelligence::coperative_leader::TOKEN_ASSIGNMENT);
    let target_key = token(tokens, 0x6b) == Some(b"target".as_slice());
    let mut matched = 0;
    for ((id, owner), entries) in saved_active {
        let label = format!("{}.operative.{id}", text(&owner));
        let Some(actual) = recorded.remove(&(id, owner.clone())) else {
            report.difference(&label, "absent", "active roster entry");
            continue;
        };
        matched += 1;
        if entries.len() != 1 {
            report.difference(format!("{label}.occurrences"), 1, entries.len());
        }
        let (ordinal, expected) = entries[0];
        equal(report, &format!("{label}.ordinal"), actual.ordinal, ordinal);
        compare_assignment(
            report,
            &label,
            actual.assignment,
            expected,
            assignment_names.as_ref(),
            assignment_key,
        );
        if actual.assignment == engine_native_intelligence::coperative_leader::COOLDOWN_ASSIGNMENT {
            compare_cooldown(
                report,
                &label,
                (
                    actual.cooldown_kind,
                    actual.cooldown_start_hour,
                    actual.cooldown_end_hour,
                ),
                expected,
            )?;
        }
        equal(
            report,
            &format!("{label}.mission_present"),
            actual.variant.is_some(),
            expected.mission_present,
        );
        equal(
            report,
            &format!("{label}.gfx"),
            (!actual.portrait.is_empty()).then_some(actual.portrait.as_slice()),
            expected.gfx.as_deref(),
        );
        let Some(variant) = actual.variant else {
            continue;
        };
        let definition = variant
            .checked_sub(1)
            .and_then(|index| engine_native_intelligence::operative_action::SAVE_TOKENS.get(index as usize))
            .ok_or_else(|| invalid("recorded operative mission variant is unsupported"))?;
        match token(tokens, *definition).filter(|name| !name.contains(&0)) {
            Some(name) => equal(
                report,
                &format!("{label}.mission_name"),
                Some(name),
                expected.mission_name.as_deref(),
            ),
            None => report.limitation(
                "operative mission definition token has no exact LexicalTokens entry",
                format!("{label}.variant={variant} token={definition}"),
            ),
        }
        if target_key {
            let target = actual
                .target_country
                .filter(|&raw| (raw as i32) > 0)
                .map(|raw| required_country_tag(raw, countries))
                .transpose()?;
            equal(
                report,
                &format!("{label}.mission_target"),
                target,
                expected.mission_target.as_deref(),
            );
        } else {
            report.limitation(
                "operative target token 0x6B has no exact LexicalTokens entry",
                &label,
            );
        }
        equal(
            report,
            &format!("{label}.mission_state"),
            actual.target_state,
            expected.mission_state,
        );
        equal(
            report,
            &format!("{label}.mission_ideology"),
            (!actual.ideology.is_empty()).then_some(actual.ideology.as_slice()),
            expected.mission_ideology.as_deref(),
        );
    }
    for ((id, roster), _) in recorded {
        report.difference(
            format!("{}.operative.{id}", text(&roster)),
            "present",
            "absent from the save",
        );
    }
    report.compared("OperativeMissions", matched);
    if assignment_key != Some(b"state".as_slice()) || assignment_names.is_none() {
        report.limitation(
            "operative assignment literals (0x1B7 with 0x4A51/0x4A52/0x4A53/0x4A50/0x336C/0x394C) resolve via LexicalTokens where exact; missing or NUL entries leave state values unverified beyond presence/counts and ids",
            format!("OperativeMissions 0x1B7={:?}", assignment_key.map(String::from_utf8_lossy)),
        );
    }
    report.limitation(
        "the operative writer saves no pending mission; the mission a relocating operative carries until it arrives has no save counterpart",
        "OperativeMissions.pending_variant / pending_country / pending_state / pending_ideology",
    );
    report.limitation(
        "the operative owner_country reference has no proved save mapping beyond the joined identity; retained natively but not verified here",
        "OperativeMissions.owner_country",
    );
    report.limitation(
        "the operative writer saves no root-out-resistance operands; the owner's and the operative's effective factors and the nationality match are the hour's accumulator and roster reads",
        "OperativeMissions.root_out_country_factor / root_out_operative_factor / root_out_nationality: runtime-only; no text-save equality claim",
    );
    Ok(())
}

fn compare_cryptology(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut saved = BTreeMap::new();
    for (owner, country) in report.canonical_countries(&save.countries) {
        let Some(cryptology) = country
            .agency
            .as_ref()
            .and_then(|agency| agency.cryptology.as_ref())
        else {
            continue;
        };
        let Some(targets) = &cryptology.targets else {
            continue;
        };
        for entry in targets {
            if !save.countries.contains_key(&entry.target) {
                return Err(invalid("unknown saved cryptology target country"));
            }
            if saved
                .insert((owner.clone(), entry.target.clone()), entry)
                .is_some()
            {
                return Err(invalid("duplicate saved cryptology target"));
            }
        }
    }
    let mut recorded = BTreeMap::new();
    for actual in complete::<tables::Cryptology>(frame)? {
        let owner = required_country_tag(actual.owner_country, countries)?;
        let target = required_country_tag(actual.target_country, countries)?;
        country(save, owner)?;
        country(save, target)?;
        if recorded
            .insert((owner.to_vec(), target.to_vec()), actual)
            .is_some()
        {
            return Err(invalid("duplicate recorded cryptology target"));
        }
    }
    let mut matched = 0;
    for ((owner, target), expected) in &saved {
        let label = format!("{}.cryptology.{}", text(owner), text(target));
        let Some(actual) = recorded.remove(&(owner.clone(), target.clone())) else {
            report.difference(&label, "absent", text(target));
            continue;
        };
        matched += 1;
        report.scalar(
            &label,
            "amount",
            Some(actual.amount_raw),
            expected.amount.as_ref(),
        );
        equal(
            report,
            &format!("{label}.decryption"),
            actual.decrypting != 0,
            expected.decryption.unwrap_or(false),
        );
        if actual.decrypting > 1 {
            report.limitation(
                "yes/no decryption flag cannot certify noncanonical raw byte bits",
                &label,
            );
        }
        if actual.bonus_days != -1 || actual.bonus_active != 0 {
            report.limitation(
                "cryptology bonus_days (0x296D) and bonus_active (0x2C7E) have no proved save key in the observed live shapes (all six ENG targets omit them); retained native values are not verified here",
                format!("{label}.bonus_days={} bonus_active={}", actual.bonus_days, actual.bonus_active),
            );
        }
        if expected.date.is_some() {
            report.limitation(
                "cryptology engine-hour date (with its saved sentinel) has no recorded field; save dates are retained but not verified",
                format!("{label}.date"),
            );
        }
    }
    for ((owner, target), _) in recorded {
        report.difference(
            format!("{}.cryptology.{}", text(&owner), text(&target)),
            "present",
            "absent from the save",
        );
    }
    report.compared("Cryptology", matched);
    report.limitation(
        "cryptology flag (token 0x2A7) has no recorded field and no proved save key; retained natively but not verified",
        "Cryptology.extra_2a7",
    );
    report.limitation(
        "cryptology intel-source payload (0x4B34) has no recorded field; save countries are retained but not equated to native references",
        "Cryptology.intel_source",
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NetworkState {
    state: i32,
    strength: Option<i64>,
    selector: i32,
    local_strength: i64,
    gain: i64,
    gain_from_operatives: i64,
    gain_from_adjacency: i64,
    local_quiet: bool,
    gain_factor: Option<i64>,
    strength_target: Option<i64>,
    target_from_operatives: Option<i64>,
    target_from_counterintelligence: Option<i64>,
    subnetwork_quiet: bool,
}

impl NetworkState {
    fn recorded(row: &replay_data::rows::IntelNetworkRow) -> Self {
        let subnetwork = |value: Option<i64>| value.filter(|_| row.subnetwork_selector >= 0);
        Self {
            state: row.state_id,
            strength: subnetwork(row.strength_raw),
            selector: row.subnetwork_selector,
            local_strength: row.local_strength_raw,
            gain: row.gain_raw,
            gain_from_operatives: row.gain_from_operatives_raw,
            gain_from_adjacency: row.gain_from_adjacency_raw,
            local_quiet: row.local_quiet != 0,
            gain_factor: subnetwork(row.gain_factor_raw),
            strength_target: subnetwork(row.strength_target_raw),
            target_from_operatives: subnetwork(row.target_from_operatives_raw),
            target_from_counterintelligence: subnetwork(row.target_from_counterintelligence_raw),
            subnetwork_quiet: row.subnetwork_quiet != 0,
        }
    }
}

fn compare_intel_networks(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut saved = BTreeMap::<(u32, Vec<u8>), Vec<NetworkState>>::new();
    let slots: BTreeSet<u32> = countries.values().map(|row| row.canonical_slot).collect();
    if let Some(operatives) = &save.strategic_operatives {
        for operative in operatives.values() {
            for network in &operative.networks {
                if !save.countries.contains_key(&network.target) {
                    return Err(invalid("unknown saved intel network target"));
                }
                if !slots.contains(&operative.index) {
                    return Err(invalid("saved intel network owner slot is unknown"));
                }
                let entries = saved
                    .entry((operative.index, network.target.clone()))
                    .or_default();
                for (index, (state, selector)) in
                    network.state_ids.iter().zip(&network.selectors).enumerate()
                {
                    let subnetwork = if *selector < 0 {
                        None
                    } else {
                        Some(network.subnetworks.get(*selector as usize).ok_or_else(|| {
                            invalid("saved intel subnetwork selector outside array")
                        })?)
                    };
                    let local_quiet = usize::try_from(*selector)
                        .ok()
                        .and_then(|selector| network.local_quiet.get(selector))
                        .copied()
                        .unwrap_or(false);
                    entries.push(NetworkState {
                        state: *state,
                        strength: subnetwork.map(|sub| sub.strength.raw),
                        selector: *selector,
                        local_strength: network.strengths[index].raw,
                        gain: network.gains[index].raw,
                        gain_from_operatives: network.gains_from_operatives[index].raw,
                        gain_from_adjacency: network.gains_from_adjacency[index].raw,
                        local_quiet,
                        gain_factor: subnetwork.map(|sub| sub.gain_factor.raw),
                        strength_target: subnetwork.map(|sub| sub.strength_target.raw),
                        target_from_operatives: subnetwork
                            .map(|sub| sub.target_from_operatives.raw),
                        target_from_counterintelligence: subnetwork
                            .map(|sub| sub.target_from_counterintelligence.raw),
                        subnetwork_quiet: subnetwork.is_some_and(|sub| sub.quiet),
                    });
                }
            }
        }
    }
    for entries in saved.values_mut() {
        entries.sort_unstable_by_key(|entry| entry.state);
    }
    let mut recorded = BTreeMap::<(u32, Vec<u8>), Vec<&replay_data::rows::IntelNetworkRow>>::new();
    for actual in complete::<tables::IntelNetworks>(frame)? {
        let owner = required_country_tag(actual.owner_country, countries)?;
        let target = required_country_tag(actual.target_country, countries)?;
        country(save, owner)?;
        country(save, target)?;
        let owner_slot = countries
            .get(&actual.owner_country)
            .ok_or_else(|| invalid("recorded intel owner reference is unknown"))?
            .canonical_slot;
        recorded
            .entry((owner_slot, target.to_vec()))
            .or_default()
            .push(actual);
    }
    let mut matched = 0;
    for ((owner_slot, target), expected) in &saved {
        let label = format!("intel_network.owner_slot.{owner_slot}.{}", text(target));
        let mut actual = recorded
            .remove(&(*owner_slot, target.clone()))
            .unwrap_or_default()
            .into_iter()
            .map(NetworkState::recorded)
            .collect::<Vec<_>>();
        actual.sort_unstable_by_key(|entry| entry.state);
        if actual.len() != expected.len() {
            report.difference(format!("{label}.count"), actual.len(), expected.len());
        }
        for expected in expected {
            let state = expected.state;
            let Ok(index) = actual.binary_search_by_key(&state, |entry| entry.state) else {
                report.difference(
                    format!("{label}.state.{state}"),
                    "absent",
                    expected.selector,
                );
                continue;
            };
            let actual = &actual[index];
            let field = |name: &str| format!("{label}.state.{state}.{name}");
            equal(
                report,
                &field("local_strength"),
                actual.local_strength,
                expected.local_strength,
            );
            matched += 1;
            equal(
                report,
                &field("selector"),
                actual.selector,
                expected.selector,
            );
            equal(report, &field("gain"), actual.gain, expected.gain);
            equal(
                report,
                &field("gain_from_operatives"),
                actual.gain_from_operatives,
                expected.gain_from_operatives,
            );
            equal(
                report,
                &field("gain_from_adjacency"),
                actual.gain_from_adjacency,
                expected.gain_from_adjacency,
            );
            equal(
                report,
                &field("local_quiet"),
                actual.local_quiet,
                expected.local_quiet,
            );
            for (name, actual, expected) in [
                ("strength", actual.strength, expected.strength),
                ("gain_factor", actual.gain_factor, expected.gain_factor),
                (
                    "strength_target",
                    actual.strength_target,
                    expected.strength_target,
                ),
                (
                    "target_from_operatives",
                    actual.target_from_operatives,
                    expected.target_from_operatives,
                ),
                (
                    "target_from_counterintelligence",
                    actual.target_from_counterintelligence,
                    expected.target_from_counterintelligence,
                ),
            ] {
                if actual != expected {
                    report.difference(field(name), format!("{actual:?}"), format!("{expected:?}"));
                }
            }
            equal(
                report,
                &field("subnetwork_quiet"),
                actual.subnetwork_quiet,
                expected.subnetwork_quiet,
            );
        }
        for entry in &actual {
            let state = entry.state;
            if expected
                .binary_search_by_key(&state, |entry| entry.state)
                .is_err()
            {
                report.difference(format!("{label}.state.{state}"), "present", "absent");
            }
        }
    }
    for ((owner_slot, target), rows) in recorded {
        report.difference(
            format!("intel_network.owner_slot.{owner_slot}.{}", text(&target)),
            rows.len(),
            "absent target",
        );
    }
    let target_key = token(tokens, 0x006b);
    if target_key != Some(b"target".as_slice()) {
        report.limitation(
            "intel target literal (token 0x6B) has no exact LexicalTokens entry; comparison verifies presence/counts and ids, not the literal name",
            format!("0x6B={:?}", target_key.map(String::from_utf8_lossy)),
        );
    }
    report.compared("IntelNetworks", matched);
    Ok(())
}

fn compare_network_summaries(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::new();
    for row in complete::<tables::IntelNetworkSummaries>(frame)? {
        let owner = countries
            .get(&row.owner_country)
            .ok_or_else(|| invalid("network summary owner is unknown"))?;
        let target = required_country_tag(row.target_country, countries)?;
        if recorded
            .insert((owner.canonical_slot, target.to_vec()), row)
            .is_some()
        {
            return Err(invalid("duplicate network summary pair"));
        }
    }
    let mut matched = 0;
    for operative in save
        .strategic_operatives
        .iter()
        .flat_map(|objects| objects.values())
    {
        for (ordinal, expected) in operative.networks.iter().enumerate() {
            let label = format!(
                "intel_network.owner_slot.{}.{}",
                operative.index,
                text(&expected.target)
            );
            let Some(row) = recorded.remove(&(operative.index, expected.target.clone())) else {
                report.difference(label, "absent", "saved network summary");
                continue;
            };
            let coverage = expected
                .coverage
                .as_ref()
                .ok_or_else(|| invalid("saved network coverage absent"))?;
            equal(
                report,
                &format!("{label}.ordinal"),
                row.ordinal as usize,
                ordinal,
            );
            equal(
                report,
                &format!("{label}.coverage.core_states"),
                row.covered_core_states,
                coverage.core_states,
            );
            equal(
                report,
                &format!("{label}.coverage.controlled_states"),
                row.covered_controlled_states,
                coverage.controlled_states,
            );
            equal(
                report,
                &format!("{label}.coverage.owned_worth"),
                row.covered_worth,
                coverage.owned_worth,
            );
            report.scalar(
                &label,
                "strength_sum",
                Some(row.controlled_strength_sum),
                expected.strength_sum.as_ref(),
            );
            report.scalar(
                &label,
                "strength_sum_over_cores",
                Some(row.core_strength_sum),
                expected.strength_sum_over_cores.as_ref(),
            );
            report.scalar(
                &label,
                "state_strength_max",
                Some(row.max_state_strength),
                expected.state_strength_max.as_ref(),
            );
            matched += 1;
        }
    }
    for ((owner, target), _) in recorded {
        report.difference(
            format!("intel_network.owner_slot.{owner}.{}", text(&target)),
            "recorded network summary",
            "absent",
        );
    }
    report.compared("IntelNetworkSummaries", matched);
    Ok(())
}

fn compare_propaganda(
    frame: &DecodedFrame,
    save: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    use replay_data::rows::PropagandaEffortRow;
    let rows = complete::<tables::PropagandaEfforts>(frame)?;
    let mut actual = BTreeMap::<u32, Vec<&PropagandaEffortRow>>::new();
    for row in rows {
        actual.entry(row.slot).or_default().push(row);
    }
    let mut matched = 0;
    for (&slot, expected) in save.strategic_operatives.iter().flatten() {
        let label = format!("strategic_operative.{slot}.propaganda");
        let Some(mut entries) = actual.remove(&slot) else {
            report.difference(
                label,
                "absent",
                "saved strategic operative history and drift",
            );
            continue;
        };
        entries.sort_unstable_by_key(|row| row.sequence);
        if entries
            .windows(2)
            .any(|pair| pair[0].sequence == pair[1].sequence)
        {
            return Err(invalid("duplicate recorded propaganda sequence"));
        }
        let header = entries
            .pop()
            .filter(|row| row.sequence == u64::MAX)
            .ok_or_else(|| invalid("recorded propaganda history lacks its owner header"))?;
        if entries.iter().any(|row| row.country != header.country) {
            return Err(invalid(
                "propaganda history and header name different native owners",
            ));
        }
        equal(
            report,
            &format!("{label}.recent_propaganda_effort.len"),
            entries.len(),
            expected.recent_propaganda_effort.len(),
        );
        for (ordinal, (row, saved)) in entries
            .iter()
            .zip(&expected.recent_propaganda_effort)
            .enumerate()
        {
            equal(
                report,
                &format!("{label}.recent_propaganda_effort.{ordinal}.stability"),
                row.stability,
                saved.stability,
            );
            equal(
                report,
                &format!("{label}.recent_propaganda_effort.{ordinal}.war_support"),
                row.war_support,
                saved.war_support,
            );
        }
        let drift = PropagandaEffortRow::weekly_drift(rows, slot)
            .ok_or_else(|| invalid("propaganda weekly drift header absent"))?;
        equal(
            report,
            &format!("{label}.propaganda_weekly_drift.stability"),
            drift.0,
            expected.propaganda_weekly_drift.stability,
        );
        equal(
            report,
            &format!("{label}.propaganda_weekly_drift.war_support"),
            drift.1,
            expected.propaganda_weekly_drift.war_support,
        );
        matched += 1;
    }
    for (slot, _) in actual {
        report.difference(
            format!("strategic_operative.{slot}.propaganda"),
            "recorded",
            "absent from save",
        );
    }
    report.compared("PropagandaEfforts", matched);
    Ok(())
}

fn compare_strategic_operatives(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::new();
    for row in complete::<tables::StrategicOperatives>(frame)? {
        let owner = countries
            .get(&row.country)
            .ok_or_else(|| invalid("recorded strategic operative owner is unknown"))?;
        if owner.canonical_slot != row.slot {
            return Err(invalid(
                "recorded strategic operative owner is not its slot's country",
            ));
        }
        if recorded
            .insert(row.slot, (owner.tag.as_slice(), row))
            .is_some()
        {
            return Err(invalid("duplicate recorded strategic operative slot"));
        }
    }
    let slots: BTreeSet<u32> = countries.values().map(|row| row.canonical_slot).collect();
    let mut matched = 0;
    let mut countryless = 0_usize;
    for (slot, expected) in save.strategic_operatives.iter().flatten() {
        let Some((tag, row)) = recorded.remove(slot) else {
            if slots.contains(slot) {
                report.difference(format!("strategic_operative.{slot}"), "absent", "present");
            } else {
                countryless += 1;
            }
            continue;
        };
        matched += 1;
        let label = format!("{}.strategic_operative", text(tag));
        report.scalar(
            &label,
            "subversive_activity_level",
            Some(row.mission_activity),
            expected.mission_activity.as_ref(),
        );
        report.scalar(
            &label,
            "danger_level",
            Some(row.operation_activity),
            expected.operation_activity.as_ref(),
        );
        let trade = expected.trade_influence.as_deref().unwrap_or_default();
        equal(
            report,
            &format!("{label}.trade_influence.count"),
            row.trade_influence.len(),
            trade.len(),
        );
        for (index, (actual, (country, amount))) in
            row.trade_influence.iter().zip(trade).enumerate()
        {
            let entry = format!("{label}.trade_influence[{index}]");
            equal(
                report,
                &format!("{entry}.country"),
                country_tag(actual.owner, countries)?,
                Some(country.as_slice()),
            );
            equal(report, &format!("{entry}.amount"), actual.value, amount.raw);
        }
        let pressure = expected.diplomatic_pressure.as_deref().unwrap_or_default();
        equal(
            report,
            &format!("{label}.diplomatic_pressure.count"),
            row.diplomatic_pressure.len(),
            pressure.len(),
        );
        for (index, (actual, expected)) in row.diplomatic_pressure.iter().zip(pressure).enumerate()
        {
            let entry = format!("{label}.diplomatic_pressure[{index}]");
            equal(
                report,
                &format!("{entry}.country"),
                country_tag(actual.owner, countries)?,
                Some(expected.country.as_slice()),
            );
            equal(
                report,
                &format!("{entry}.opinion"),
                actual.acceptance,
                expected.opinion,
            );
            equal(
                report,
                &format!("{entry}.tension"),
                actual.tension,
                expected.tension,
            );
        }
    }
    for (slot, (tag, _)) in recorded {
        report.difference(
            format!("{}.strategic_operative", text(tag)),
            format!("slot {slot}"),
            "absent from the save",
        );
    }
    report.compared("StrategicOperatives", matched);
    if countryless != 0 {
        report.limitation(
            "the save keys a strategic operative by every manager slot; a slot with no live country has no recorded row",
            format!("StrategicOperatives: {countryless} saved slots without a recorded country"),
        );
    }
    Ok(())
}

fn compare_autonomy(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    _tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::new();
    for actual in complete::<tables::Autonomy>(frame)? {
        let subject = required_country_tag(actual.subject, countries)?;
        country(save, subject)?;
        if recorded.insert(subject.to_vec(), actual).is_some() {
            return Err(invalid("duplicate recorded autonomy subject"));
        }
    }
    let mut matched = 0;
    for (subject, country) in report.canonical_countries(&save.countries) {
        let Some(expected) = country.diplomacy.as_ref().and_then(|d| d.autonomy.as_ref()) else {
            continue;
        };
        let label = format!("autonomy.{}", text(subject));
        let Some(actual) = recorded.remove(subject) else {
            report.difference(&label, "absent", "present in saved diplomacy");
            continue;
        };
        matched += 1;
        report.scalar(
            &label,
            "progress",
            Some(actual.score),
            Some(&expected.progress),
        );
        equal(
            report,
            &format!("{label}.state"),
            actual.state.as_slice(),
            expected.state.as_slice(),
        );
        equal(
            report,
            &format!("{label}.previous"),
            actual.previous.as_deref(),
            expected.previous.as_deref(),
        );
        equal(
            report,
            &format!("{label}.next"),
            actual.next.as_deref(),
            expected.next.as_deref(),
        );
        equal(
            report,
            &format!("{label}.last_modified"),
            actual.last_modified_bits,
            expected.last_modified.engine_hour()? as u32,
        );
    }
    for (subject, _) in recorded {
        report.difference(
            format!("autonomy.{}", text(&subject)),
            "present",
            "absent from saved diplomacy",
        );
    }
    report.compared("Autonomy", matched);
    report.limitation(
        "autonomy lower/upper score bounds are runtime-derived and are not emitted by the game; saved path and effect records have no recorded fields",
        "Autonomy.bounds, path, effects",
    );
    Ok(())
}

fn country<'a>(save: &'a saved::Pack, tag: &[u8]) -> io::Result<&'a saved::Country> {
    save.countries.get(tag).ok_or_else(|| {
        invalid(format!(
            "intel table references unknown saved country {}",
            String::from_utf8_lossy(tag)
        ))
    })
}

fn text(tag: &[u8]) -> String {
    String::from_utf8_lossy(tag).into_owned()
}

fn equal<T: std::fmt::Debug + PartialEq>(report: &mut Report, field: &str, actual: T, expected: T) {
    if actual != expected {
        report.difference(field, actual, expected);
    }
}

fn saved_hour(date: Option<GameDate>) -> io::Result<Option<i32>> {
    date.map(GameDate::engine_hour).transpose()
}

fn token<'a>(tokens: &TokenNames<'a>, id: u32) -> Option<&'a [u8]> {
    tokens.get(&id).copied()
}

fn missing_token_reason(frame: &DecodedFrame) -> &'static str {
    if matches!(
        &*frame.tables[replay_data::rows::TableId::LexicalTokens.index()],
        replay_frame::frame::TableView::Complete(_)
    ) {
        "complete recorded lexical dictionary has no entry for a token resolved by this serializer; the dependent name and fields remain unverified"
    } else {
        "recorded lexical dictionary is unavailable; the dependent token name and fields remain unverified"
    }
}

fn compare_agencies(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for actual in complete::<tables::IntelligenceAgencies>(frame)? {
        let tag = required_country_tag(actual.country, countries)?;
        if !seen.insert(tag) {
            return Err(invalid("duplicate resolved agency country"));
        }
        let subject = format!("{}.intelligence_agency", text(tag));
        let Some(expected) = &country(save, tag)?.agency else {
            report.limitation(
                "missing serialized agency payload cannot yet prove a null native agency pointer",
                subject,
            );
            continue;
        };
        matched += 1;
        report.scalar(
            &subject,
            "upgrade_progress",
            Some(actual.upgrade_progress_raw),
            Some(&expected.upgrade_progress),
        );
        equal(
            report,
            &format!("{subject}.is_created"),
            actual.created != 0,
            expected.created,
        );
        if let Some(name) = &expected.name {
            let label = format!("{subject}.name");
            let text = super::serialized_text(report, &label, &actual.name);
            equal(report, &label, text, name.as_slice());
        } else {
            report.difference(
                format!("{subject}.name"),
                String::from_utf8_lossy(&actual.name),
                "missing serialized name",
            );
        }
        report.scalar(
            &subject,
            "defense",
            Some(actual.defense_raw),
            Some(&expected.defense),
        );
        equal(
            report,
            &format!("{subject}.active_count"),
            actual.active_count,
            expected.active_count,
        );
        equal(
            report,
            &format!("{subject}.unknown_f0"),
            actual.unknown_f0,
            expected.unknown_f0,
        );
        equal(
            report,
            &format!("{subject}.admission_ceiling"),
            actual.admission_ceiling,
            expected.admission_ceiling,
        );
    }
    for (tag, expected) in report.canonical_countries(&save.countries) {
        if expected.agency.is_some() && !seen.contains(tag.as_slice()) {
            report.difference(
                format!("{}.intelligence_agency", text(tag)),
                "missing",
                "present",
            );
        }
    }
    report.compared("IntelligenceAgencies", matched);
    report.limitation("agency max_operative_count names the opaque dword; comparison does not establish its gameplay meaning", "IntelligenceAgencies.unknown_f0");
    Ok(())
}

fn compare_agency_upgrade_levels(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut agencies = BTreeSet::new();
    for agency in complete::<tables::IntelligenceAgencies>(frame)? {
        let tag = required_country_tag(agency.country, countries)?;
        if !agencies.insert(tag) {
            return Err(invalid("duplicate resolved agency country for upgrades"));
        }
    }
    let mut recorded = BTreeMap::<&[u8], BTreeMap<u32, _>>::new();
    let mut refused = Vec::new();
    let acquired = match &*frame.tables[replay_data::rows::TableId::AgencyUpgradeLevels.index()] {
        replay_frame::frame::TableView::Complete(
            replay_data::rows::TableRows::AgencyUpgradeLevels(rows),
        )
        | replay_frame::frame::TableView::Truncated {
            rows: replay_data::rows::TableRows::AgencyUpgradeLevels(rows),
            ..
        } => rows.as_slice(),
        _ => complete::<tables::AgencyUpgradeLevels>(frame)?,
    };
    for row in acquired {
        let tag = required_country_tag(row.country, countries)?;
        if !agencies.contains(tag) {
            return Err(invalid(format!(
                "{}.upgrades[{}] has no matching agency root",
                text(tag),
                row.ordinal
            )));
        }
        if recorded
            .entry(tag)
            .or_default()
            .insert(row.ordinal, row)
            .is_some()
        {
            return Err(invalid(format!(
                "{}.upgrades has duplicate recorded ordinal {}",
                text(tag),
                row.ordinal
            )));
        }
        if row.identity_status != 0 || row.upgrade_token.is_none() || row.branch_token.is_none() {
            refused.push(format!(
                "{}.upgrades[{}] {}, definition {:#x}, branch {:#x}, level {}",
                text(tag),
                row.ordinal,
                engine_native_intelligence::agency_upgrade_levels::status_name(row.identity_status),
                row.definition_pointer,
                row.branch_pointer,
                row.level
            ));
        }
    }
    if !refused.is_empty() {
        return Err(invalid(format!(
            "{} refused acquired-upgrade identities: {}",
            refused.len(),
            refused.join("; ")
        )));
    }
    complete::<tables::AgencyUpgradeLevels>(frame)?;
    let mut matched = 0;
    for (tag, country) in report.canonical_countries(&save.countries) {
        let label = format!("{}.intelligence_agency.upgrades", text(tag));
        let actual = recorded.remove(tag.as_slice()).unwrap_or_default();
        let Some(agency) = &country.agency else {
            if !actual.is_empty() {
                report.difference(&label, actual.len(), "absent serialized agency");
            }
            if agencies.contains(tag.as_slice()) {
                report.limitation(
                    "absent serialized agency cannot certify its acquired array",
                    label,
                );
            }
            continue;
        };
        if !agencies.contains(tag.as_slice()) {
            report.difference(&label, "missing agency read", "present saved agency");
            continue;
        }
        equal(
            report,
            &format!("{label}.count"),
            actual.len(),
            agency.upgrades.len(),
        );
        for (index, expected) in agency.upgrades.iter().enumerate() {
            let ordinal = u32::try_from(index)
                .map_err(|_| invalid("saved acquired upgrade ordinal exceeds u32"))?;
            let label = format!("{label}[{ordinal}]");
            let Some(row) = actual.get(&ordinal) else {
                report.difference(label, "missing ordinal", text(&expected.token));
                continue;
            };
            matched += 1;
            equal(report, &format!("{label}.level"), row.level, expected.level);
            let id = row.upgrade_token.unwrap();
            let name = token(tokens, id).ok_or_else(|| {
                invalid(format!(
                    "{label} upgrade token {id} has no recorded lexical spelling"
                ))
            })?;
            if name.contains(&0) {
                return Err(invalid(format!(
                    "{label} upgrade token {id} contains NUL; save token formatting is unsupported"
                )));
            }
            equal(
                report,
                &format!("{label}.token"),
                name,
                expected.token.as_slice(),
            );
        }
        for (&ordinal, row) in &actual {
            if ordinal as usize >= agency.upgrades.len() {
                report.difference(
                    format!("{label}[{ordinal}]"),
                    format!(
                        "recorded token {:?}, level {}",
                        row.upgrade_token, row.level
                    ),
                    "absent saved ordinal",
                );
            }
        }
    }
    for (tag, rows) in recorded {
        report.difference(
            format!("{}.intelligence_agency.upgrades", text(tag)),
            rows.len(),
            "absent saved country",
        );
    }
    report.compared("AgencyUpgradeLevels", matched);
    report.limitation(
        "the agency writer writes ordered upgrade tokens and signed levels but no definition pointer or branch token; exact token/level agreement cannot independently certify native branch membership",
        "AgencyUpgradeLevels.definition_pointer / branch_pointer / branch_token; coordinated memory/capture assertion remains required",
    );
    Ok(())
}

fn compare_agency_upgrade_identities(
    frame: &DecodedFrame,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    intelligence: Result<&content_reader_military::intelligence::AgencyUpgrades, &str>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::AgencyUpgradeLevels>(frame)?;
    if rows.is_empty() {
        return Ok(());
    }
    let intelligence = intelligence.map_err(invalid)?;
    for row in rows {
        let country = required_country_tag(row.country, countries)?;
        let label = format!("{}.upgrades[{}]", text(country), row.ordinal);
        let name = |id: Option<u32>, kind: &str| -> io::Result<&str> {
            let id = id.ok_or_else(|| invalid(format!("{label} {kind} token is unavailable")))?;
            let bytes = token(tokens, id).ok_or_else(|| {
                invalid(format!(
                    "{label} {kind} token {id} has no recorded lexical spelling"
                ))
            })?;
            if bytes.contains(&0) {
                return Err(invalid(format!("{label} {kind} token {id} contains NUL")));
            }
            std::str::from_utf8(bytes).map_err(|_| {
                invalid(format!(
                    "{label} {kind} token {id} is not UTF-8; content identity refused"
                ))
            })
        };
        if row.identity_status != 0 {
            return Err(invalid(format!(
                "{label} {} is refused",
                engine_native_intelligence::agency_upgrade_levels::status_name(row.identity_status)
            )));
        }
        let upgrade = name(row.upgrade_token, "upgrade")?;
        let branch = name(row.branch_token, "branch")?;
        if intelligence
            .branch_upgrade(branch, upgrade)
            .map_err(invalid)?
            .is_none()
        {
            report.difference(
                format!("{label}.branch_upgrade"),
                format!("{branch}/{upgrade}"),
                "absent from recorded content stack",
            );
            continue;
        }
        let branches = intelligence.upgrade_branches(upgrade).map_err(invalid)?;
        if branches.len() > 1 {
            report.limitation(
                "saved agency upgrade token has multiple native branch-owned definitions; save format omits the captured branch identity",
                format!(
                    "{label} {upgrade}: {} branches [{}], captured {branch}",
                    branches.len(),
                    branches.iter().map(|definition| definition.branch.as_str()).collect::<Vec<_>>().join(", "),
                ),
            );
        }
        let global = intelligence
            .global_upgrade(upgrade)
            .map_err(invalid)?
            .ok_or_else(|| {
                invalid(format!(
                    "{label} {upgrade} has no global native registry definition"
                ))
            })?;
        if !branch.eq_ignore_ascii_case(&global.branch) {
            report.difference(
                format!("{label}.save_reload_branch"),
                branch,
                global.branch.as_str(),
            );
        }
    }
    Ok(())
}

fn compare_subjects(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::new();
    for actual in complete::<tables::Subjects>(frame)? {
        let first = required_country_tag(actual.overlord, countries)?;
        let second = required_country_tag(actual.subject, countries)?;
        country(save, first)?;
        country(save, second)?;
        if first == second || recorded.insert((first, second), actual).is_some() {
            return Err(invalid("self or duplicate resolved subject relation"));
        }
    }
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for (owner, country) in report.canonical_countries(&save.countries) {
        let Some(subjects) = country.diplomacy.as_ref().and_then(|d| d.subjects.as_ref()) else {
            continue;
        };
        for expected in subjects {
            let key = (expected.first.as_slice(), expected.second.as_slice());
            if expected.roster_owner != *owner
                || expected.first != *owner
                || expected.target != expected.second
                || key.0 == key.1
                || !save.countries.contains_key(key.1)
                || !seen.insert(key)
            {
                return Err(invalid(
                    "saved subject relation has inconsistent ownership, endpoint or duplicate pair",
                ));
            }
            let label = format!("{}.puppet.{}", text(key.0), text(key.1));
            let Some(actual) = recorded.remove(&key) else {
                report.difference(&label, "missing", "present");
                continue;
            };
            matched += 1;
            equal(
                report,
                &format!("{label}.state"),
                Some(actual.state.as_slice()),
                expected.state.as_deref(),
            );
            report.scalar(
                &label,
                "value",
                Some(actual.autonomy_ratio),
                Some(&expected.autonomy_ratio),
            );
            equal(
                report,
                &format!("{label}.cancel"),
                actual.cancel != 0,
                expected.cancel,
            );
            if actual.cancel > 1 {
                report.limitation("yes/no subject cancel preserves truth but cannot certify noncanonical raw flag bits", &label);
            }
            if expected.cancel {
                report.limitation(
                    "cancelled subject relation omits its retained native start-hour word",
                    &label,
                );
            } else {
                equal(
                    report,
                    &format!("{label}.start_date"),
                    Some(actual.start_hour),
                    saved_hour(expected.start_date)?,
                );
            }
            let emitted_end = (actual.start_hour < actual.end_hour).then_some(actual.end_hour);
            equal(
                report,
                &format!("{label}.end_date"),
                emitted_end,
                saved_hour(expected.end_date)?,
            );
            if expected.end_date.is_none() {
                report.limitation("subject end-date omission verifies only end_hour <= start_hour, not the retained raw end-hour value", &label);
            }
        }
    }
    for ((owner, target), _) in recorded {
        let label = format!("{}.puppet.{}", text(owner), text(target));
        if save.countries[owner]
            .diplomacy
            .as_ref()
            .and_then(|d| d.subjects.as_ref())
            .is_some()
        {
            report.difference(label, "present", "missing");
        } else {
            report.limitation("missing serialized active-relations payload does not establish a complete subject census", label);
        }
    }
    report.compared("Subjects", matched);
    Ok(())
}

fn compare_capitulation(
    frame: &DecodedFrame,
    save: &saved::Pack,
    rosters: &content_reader_save::country_rosters::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut matched = 0;
    for actual in complete::<tables::Capitulation>(frame)? {
        let tag = required_country_tag(actual.country, countries)?;
        if !seen.insert(tag) {
            return Err(invalid("duplicate resolved capitulation country"));
        }
        let label = format!("{}.diplomacy", text(tag));
        let Some(expected) = &country(save, tag)?.diplomacy else {
            report.limitation(
                "missing serialized diplomacy payload cannot verify capitulation state",
                label,
            );
            continue;
        };
        matched += 1;
        equal(
            report,
            &format!("{label}.capitulated"),
            actual.capitulated != 0,
            expected.capitulated.unwrap_or(false),
        );
        if actual.capitulated > 1 {
            report.limitation(
                "yes/no capitulation flag cannot certify noncanonical raw byte bits",
                &label,
            );
        }
        equal(
            report,
            &format!("{label}.capitulated_date"),
            (actual.capitulated != 0).then_some(actual.capitulated_hour),
            saved_hour(expected.capitulated_date)?,
        );
        if expected.capitulated_date.is_none() {
            report.limitation(
                "inactive capitulation omits its retained native capitulation-hour word",
                &label,
            );
        }
        let goals = format!("{label}.war_goal_recipients");
        if let Some(recipients) = report.recorded(
            &actual.war_goal_recipients,
            "Capitulation.war_goal_recipients",
            &goals,
        ) {
            match recipients {
                Some(recipients) => {
                    let recipients = recipients
                        .iter()
                        .map(|&raw| required_country_tag(raw, countries).map(<[u8]>::to_vec))
                        .collect::<io::Result<Vec<_>>>()?;
                    let saved = rosters
                        .owned_goals
                        .get(tag)
                        .map(Vec::as_slice)
                        .unwrap_or_default()
                        .iter()
                        .map(|handle| rosters.goals.get(handle).map(|goal| goal.recipient.clone()))
                        .collect::<Option<Vec<_>>>();
                    match saved {
                        Some(saved) => equal(report, &goals, recipients, saved),
                        None => report.difference(
                            goals,
                            "recorded",
                            "owned goal without a saved object",
                        ),
                    }
                }
                None => report.difference(goals, "refused read", "saved war goals"),
            }
        }
    }
    for (tag, country) in report.canonical_countries(&save.countries) {
        if country.diplomacy.is_some() && !seen.contains(tag.as_slice()) {
            report.difference(format!("{}.diplomacy", text(tag)), "missing", "present");
        }
    }
    report.compared("Capitulation", matched);
    Ok(())
}

fn compare_flags(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    country_scope_kind: u8,
    character_scope_kind: u8,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::<(_, _), Vec<_>>::new();
    let mut unresolved = BTreeSet::new();
    let mut raw_keys = BTreeSet::new();
    let mut country_counts = BTreeMap::<&[u8], usize>::new();
    for actual in complete::<tables::ScriptFlags>(frame)? {
        if actual.scope_kind == character_scope_kind {
            continue;
        }
        if actual.scope_kind != country_scope_kind {
            report.limitation("global and state flag save-root correspondence has not been proved; country flag coverage does not verify those scopes", format!("scope {}:{} token {}", actual.scope_kind, actual.scope_id, actual.token));
            continue;
        }
        let tag = required_country_tag(actual.scope_id, countries)?;
        country(save, tag)?;
        if !raw_keys.insert((tag, actual.token)) {
            return Err(invalid("duplicate resolved country and native flag token"));
        }
        *country_counts.entry(tag).or_default() += 1;
        let Some(name) = token(tokens, actual.token) else {
            unresolved.insert(tag);
            report.limitation(
                missing_token_reason(frame),
                format!("{} flag token {}", text(tag), actual.token),
            );
            continue;
        };
        if name.contains(&0) {
            unresolved.insert(tag);
            report.limitation(
                "flag-name formatter receives a C-string pointer, but its concrete stream behavior for interior NUL is unproved; literal identity and dependent flag fields remain unverified",
                format!("{} flag token {}", text(tag), actual.token),
            );
            continue;
        }
        recorded.entry((tag, name)).or_default().push(actual);
    }
    let mut matched = 0;
    for (tag, expected) in report.canonical_countries(&save.countries) {
        let Some(flags) = &expected.flags else {
            continue;
        };
        equal(
            report,
            &format!("{}.flags count", text(tag)),
            country_counts.get(tag.as_slice()).copied().unwrap_or(0),
            flags.len(),
        );
        let mut expected_names = BTreeMap::<&[u8], Vec<_>>::new();
        for expected in flags {
            expected_names
                .entry(&expected.name)
                .or_default()
                .push(expected);
        }
        for (name, expected) in expected_names {
            let label = format!("{}.flags.{}", text(tag), text(name));
            let Some(actual) = recorded.remove(&(tag.as_slice(), name)) else {
                if unresolved.contains(tag.as_slice()) {
                    report.limitation("saved flag cannot be joined while this country's recorded flag names are unavailable", &label);
                } else {
                    report.difference(&label, "missing", "present");
                }
                continue;
            };
            matched += compare_flag_values(
                report,
                &label,
                &actual,
                &expected,
                unresolved.contains(tag.as_slice()),
            )?;
        }
    }
    for ((tag, name), _) in recorded {
        let label = format!("{}.flags.{}", text(tag), text(name));
        if save.countries[tag].flags.is_some() {
            report.difference(label, "present", "missing");
        } else {
            report.limitation("missing serialized flag-manager payload does not yet establish an empty native scope", label);
        }
    }
    report.compared("ScriptFlags", matched);
    Ok(())
}

fn compare_flag_values(
    report: &mut Report,
    label: &str,
    actual: &[&replay_data::rows::ScriptFlagRow],
    expected: &[&saved::Flag],
    unresolved: bool,
) -> io::Result<usize> {
    if actual.len() > 1 || expected.len() > 1 {
        report.limitation(
            "distinct flag token ordinals can emit the same literal name; comparison verifies exact field multiplicities but cannot distinguish those native identities or their ordering",
            label,
        );
    }
    let mut actual_values = actual
        .iter()
        .map(|row| {
            (
                row.value,
                row.set_hour,
                (row.remaining_days > 0).then_some(row.remaining_days),
            )
        })
        .collect::<Vec<_>>();
    let mut expected_values = expected
        .iter()
        .map(|row| Ok((row.value, row.date.engine_hour()?, row.remaining_days)))
        .collect::<io::Result<Vec<_>>>()?;
    if expected.iter().any(|row| row.remaining_days.is_none()) {
        report.limitation("omitted flag duration verifies nonpositive days only; distinct retained values such as 0 and -1 cannot be distinguished", label);
    }
    actual_values.sort_unstable();
    expected_values.sort_unstable();
    if unresolved && actual_values.len() < expected_values.len() {
        for value in actual_values {
            if let Ok(index) = expected_values.binary_search(&value) {
                expected_values.remove(index);
            } else {
                report.difference(label, value, "no matching saved flag occurrence");
            }
        }
        report.limitation(
            "saved flag occurrences cannot all be joined while this scope has unresolved token names",
            label,
        );
    } else {
        equal(report, label, actual_values, expected_values);
    }
    Ok(actual.len().min(expected.len()))
}

fn compare_character_flags(
    frame: &DecodedFrame,
    manager: Option<&saved_characters::Manager>,
    tokens: &TokenNames<'_>,
    character_scope_kind: u8,
    report: &mut Report,
) -> io::Result<()> {
    let handle = |id: u32| (u64::from(id) << 32) | 73;
    let mut recorded = BTreeMap::<(u64, &[u8]), Vec<_>>::new();
    let mut unresolved = BTreeSet::new();
    let mut raw_keys = BTreeSet::new();
    let mut counts = BTreeMap::<u64, usize>::new();
    let mut rows = 0_usize;
    for actual in complete::<tables::ScriptFlags>(frame)? {
        if actual.scope_kind != character_scope_kind {
            continue;
        }
        rows += 1;
        let character = handle(actual.scope_id);
        if !raw_keys.insert((character, actual.token)) {
            return Err(invalid("duplicate character and native flag token"));
        }
        *counts.entry(character).or_default() += 1;
        let Some(name) = token(tokens, actual.token) else {
            unresolved.insert(character);
            report.limitation(
                missing_token_reason(frame),
                format!("character {character} flag token {}", actual.token),
            );
            continue;
        };
        if name.contains(&0) {
            unresolved.insert(character);
            report.limitation(
                "flag-name formatter receives a C-string pointer, but its concrete stream behavior for interior NUL is unproved; literal identity and dependent flag fields remain unverified",
                format!("character {character} flag token {}", actual.token),
            );
            continue;
        }
        recorded.entry((character, name)).or_default().push(actual);
    }
    let Some(manager) = manager else {
        if rows != 0 {
            report.limitation(
                "save lacks character_manager; recorded character flags cannot be joined",
                format!("{rows} character flag rows"),
            );
        }
        return Ok(());
    };
    let mut matched = 0;
    for (&character, saved) in &manager.characters {
        let flags = saved.flags.as_deref().unwrap_or_default();
        equal(
            report,
            &format!("character {character}.flags count"),
            counts.get(&character).copied().unwrap_or(0),
            flags.len(),
        );
        let mut expected_names = BTreeMap::<&[u8], Vec<_>>::new();
        for flag in flags {
            expected_names.entry(&flag.name).or_default().push(flag);
        }
        for (name, expected) in expected_names {
            let label = format!("character {character}.flags.{}", text(name));
            let Some(actual) = recorded.remove(&(character, name)) else {
                if unresolved.contains(&character) {
                    report.limitation("saved flag cannot be joined while this character's recorded flag names are unavailable", &label);
                } else {
                    report.difference(&label, "missing", "present");
                }
                continue;
            };
            matched += compare_flag_values(
                report,
                &label,
                &actual,
                &expected,
                unresolved.contains(&character),
            )?;
        }
    }
    for ((character, name), _) in recorded {
        report.difference(
            format!("character {character}.flags.{}", text(name)),
            "present",
            "missing",
        );
    }
    report.compared("ScriptFlags character scope", matched);
    Ok(())
}

fn multiset<'a>(values: impl IntoIterator<Item = &'a [u8]>) -> BTreeMap<&'a [u8], usize> {
    let mut counts = BTreeMap::new();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum VariableScope<'a> {
    Global,
    Country(&'a [u8]),
    State(u32),
    Character(u64),
}

impl VariableScope<'_> {
    fn label(self) -> String {
        match self {
            Self::Global => "global.variables".into(),
            Self::Country(tag) => format!("{}.variables", text(tag)),
            Self::State(id) => format!("state.{id}.variables"),
            Self::Character(character) => format!("character {character}.variables"),
        }
    }
}

fn compare_variables(
    frame: &DecodedFrame,
    save: &saved::Pack,
    characters: Option<&saved_characters::Manager>,
    countries: &BTreeMap<u32, &CountryRow>,
    scopes: ScopeKinds,
    report: &mut Report,
) -> io::Result<()> {
    let kinds = [
        scopes.global,
        scopes.country,
        scopes.state,
        scopes.character,
    ];
    if (1..kinds.len()).any(|at| kinds[..at].contains(&kinds[at])) {
        return Err(invalid("variable scope discriminants are not distinct"));
    }
    let no_variables = saved::Variables::default();
    let normalized = match &*frame.tables[TableId::ScriptVariableValues.index()] {
        TableView::NotRecorded { .. } => None,
        _ => {
            let names: BTreeMap<u32, &[u8]> = complete::<tables::ScriptVariableNames>(frame)?
                .iter()
                .map(|row| (row.id, row.name.as_slice()))
                .collect();
            let mut by_scope: BTreeMap<(u8, u32), Vec<(&[u8], i64)>> = BTreeMap::new();
            for row in complete::<tables::ScriptVariableValues>(frame)? {
                let Some(&name) = names.get(&row.var_id) else {
                    return Err(invalid(format!(
                        "script variable value references unknown variable name id {}",
                        row.var_id
                    )));
                };
                by_scope
                    .entry((row.scope_kind, row.scope_id))
                    .or_default()
                    .push((name, row.value_raw));
            }
            Some(by_scope)
        }
    };
    let mut character_rows = 0_usize;
    let mut recorded = BTreeMap::new();
    let mut seen_scopes = BTreeSet::new();
    for actual in complete::<tables::ScriptVariables>(frame)? {
        seen_scopes.insert((actual.scope_kind, actual.scope_id));
        if !actual.entries.is_empty()
            && normalized
                .as_ref()
                .is_some_and(|by_scope| !by_scope.is_empty())
        {
            return Err(invalid(
                "ScriptVariables carries both inline entries and normalized ScriptVariableValues",
            ));
        }
        let scope = if actual.scope_kind == scopes.global {
            if actual.scope_id != 0 {
                return Err(invalid("global variable scope has a nonzero id"));
            }
            VariableScope::Global
        } else if actual.scope_kind == scopes.country {
            let tag = required_country_tag(actual.scope_id, countries)?;
            country(save, tag)?;
            VariableScope::Country(tag)
        } else if actual.scope_kind == scopes.state {
            if save
                .state_variables
                .as_ref()
                .is_some_and(|states| !states.contains_key(&actual.scope_id))
            {
                return Err(invalid(format!(
                    "variable scope references unknown saved state {}",
                    actual.scope_id
                )));
            }
            VariableScope::State(actual.scope_id)
        } else if actual.scope_kind == scopes.character {
            character_rows += 1;
            VariableScope::Character((u64::from(actual.scope_id) << 32) | 73)
        } else {
            return Err(invalid(format!(
                "unknown variable scope kind {}",
                actual.scope_kind
            )));
        };
        if recorded.insert(scope, actual).is_some() {
            return Err(invalid("duplicate resolved variable scope"));
        }
    }
    if let Some(by_scope) = &normalized {
        for &(scope_kind, scope_id) in by_scope.keys() {
            if !seen_scopes.contains(&(scope_kind, scope_id)) {
                return Err(invalid(format!(
                    "script variable value references absent scope {scope_kind}:{scope_id}"
                )));
            }
        }
    }
    let mut expected_scopes =
        BTreeMap::from([(VariableScope::Global, save.global_variables.as_ref())]);
    for (tag, country) in report.canonical_countries(&save.countries) {
        expected_scopes.insert(VariableScope::Country(tag), country.variables.as_ref());
    }
    if let Some(states) = &save.state_variables {
        for (&id, variables) in states {
            expected_scopes.insert(VariableScope::State(id), variables.as_ref());
        }
    }
    match characters {
        Some(manager) => {
            for (&character, saved) in &manager.characters {
                expected_scopes.insert(
                    VariableScope::Character(character),
                    Some(saved.variables.as_ref().unwrap_or(&no_variables)),
                );
            }
        }
        None => {
            recorded.retain(|scope, _| !matches!(scope, VariableScope::Character(_)));
            if character_rows != 0 {
                report.limitation(
                    "save lacks character_manager; recorded character variables cannot be joined",
                    format!("{character_rows} character variable scopes"),
                );
            }
        }
    }
    let mut matched = 0;
    for (scope, expected) in expected_scopes {
        let actual = recorded.remove(&scope);
        let label = scope.label();
        let Some(expected) = expected else {
            if actual.is_some() {
                report.limitation(
                    "serialized variables payload is absent at its proved scope; generic pointer-writer omission behavior is not proved, so absence does not certify an empty native container",
                    label,
                );
            }
            continue;
        };
        let Some(actual) = actual else {
            report.difference(&label, "missing", "present");
            continue;
        };
        matched += 1;
        if let Some(seed) = report.recorded(&actual.seed, "ScriptVariables.seed", &label) {
            match (*seed, expected.seed) {
                (Some(actual), Some(expected)) => {
                    super::eq(report, &format!("{label} random seed"), actual, expected);
                }
                (None, _) => report.limitation(
                    "the scoped-variable RNG seed is unavailable from native capture",
                    &label,
                ),
                (_, None) => report.limitation(
                    "the variable save writer omits random metadata for an untouched seed state; this save has no emitted seed word to compare",
                    &label,
                ),
            }
        }
        let mut values = BTreeMap::<&[u8], Vec<i64>>::new();
        let mut unresolved_literal = false;
        let inline_entries = actual
            .entries
            .iter()
            .map(|entry| (entry.name.as_slice(), entry.value_raw));
        let split_entries = normalized
            .as_ref()
            .filter(|_| actual.entries.is_empty())
            .and_then(|by_scope| by_scope.get(&(actual.scope_kind, actual.scope_id)))
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .copied();
        for (index, (name, value_raw)) in inline_entries.chain(split_entries).enumerate() {
            if name.contains(&0) {
                unresolved_literal = true;
                report.limitation(
                    "variable-name formatter receives a C-string pointer, but its concrete stream behavior for interior NUL is unproved; this full native key and value remain unverified",
                    format!("{label} entry {index}"),
                );
                continue;
            }
            if name == b"random" {
                report.limitation(
                    "a native variable key emits the reserved random metadata name; its identity and value cannot be distinguished from metadata in the save",
                    format!("{label} entry {index}"),
                );
                continue;
            }
            values.entry(name).or_default().push(value_raw);
        }
        let mut expected_values = BTreeMap::<&[u8], Vec<&content_reader_save::Decimal>>::new();
        for entry in &expected.entries {
            expected_values
                .entry(&entry.name)
                .or_default()
                .push(&entry.value);
        }
        for (name, mut expected) in expected_values {
            let field = format!("{label}.{}", text(name));
            let Some(mut actual) = values.remove(name) else {
                if unresolved_literal {
                    report.limitation("saved variable occurrences cannot all be joined while this scope has unproved literal formatting", field);
                } else {
                    report.difference(field, "missing", "present");
                }
                continue;
            };
            actual.sort_unstable();
            expected.sort_unstable_by_key(|value| value.raw);
            if unresolved_literal && actual.len() < expected.len() {
                for actual in actual {
                    if let Ok(index) = expected.binary_search_by_key(&actual, |value| value.raw) {
                        report.scalar(&field, "value", Some(actual), Some(expected.remove(index)));
                    } else {
                        report.difference(&field, actual, "no matching saved variable occurrence");
                    }
                }
                report.limitation("saved variable occurrences cannot all be joined while this scope has unproved literal formatting", field);
                continue;
            }
            equal(
                report,
                &format!("{field} occurrences"),
                actual.len(),
                expected.len(),
            );
            for (actual, expected) in actual.into_iter().zip(expected) {
                report.scalar(&field, "value", Some(actual), Some(expected));
            }
        }
        for (name, values) in values {
            report.difference(format!("{label}.{}", text(name)), values, "missing");
        }
        if expected.reserved_entries != 0 {
            report.limitation(
                "scalar random entries use the variable store's reserved metadata name; their native literal keys cannot be distinguished in the save",
                format!("{label}: {} reserved entries", expected.reserved_entries),
            );
        }
    }
    for (scope, _) in recorded {
        match scope {
            VariableScope::Character(_) => {
                report.difference(scope.label(), "present", "no saved character")
            }
            _ => report.limitation(
                "serialized states root is absent; state variable save mapping is proved but this input cannot verify the native state scope",
                scope.label(),
            ),
        }
    }
    report.compared("ScriptVariables", matched);
    report.limitation(
        "variable serializer sorts a copy of literal keys; native hash-table traversal order is not verified",
        "ScriptVariables",
    );
    Ok(())
}

fn compare_operation_tokens(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::new();
    let mut owners_with_targets = BTreeSet::new();
    for actual in complete::<tables::OperationTokens>(frame)? {
        let owner = required_country_tag(actual.owner, countries)?;
        country(save, owner)?;
        owners_with_targets.insert(owner);
        let target: &[u8] = if actual.target == 0 {
            b""
        } else {
            let target = required_country_tag(actual.target, countries)?;
            country(save, target)?;
            target
        };
        let mut names = Vec::new();
        let mut missing_ids = BTreeMap::<u32, usize>::new();
        let mut name_ids = BTreeMap::<&[u8], BTreeSet<u32>>::new();
        for &id in &actual.tokens {
            if let Some(name) = token(tokens, id) {
                if name.contains(&0) {
                    report.limitation(
                        "operation-token formatter receives a C-string pointer, but its concrete stream behavior for interior NUL is unproved; this literal occurrence remains unverified",
                        format!("{}.operation_assets.{} token {id}", text(owner), text(target)),
                    );
                    continue;
                }
                names.push(name);
                name_ids.entry(name).or_default().insert(id);
            } else {
                *missing_ids.entry(id).or_default() += 1;
            }
        }
        let missing = actual.tokens.len() - names.len();
        if !missing_ids.is_empty() {
            report.limitation(
                missing_token_reason(frame),
                format!(
                    "{}.operation_assets.{}: raw id counts {missing_ids:?}",
                    text(owner),
                    text(target)
                ),
            );
        }
        for (name, ids) in name_ids {
            if ids.len() > 1 {
                report.limitation(
                    "distinct operation-token ordinals emit the same literal name; save comparison verifies name multiplicity but cannot distinguish those native identities",
                    format!("{}.operation_assets.{} name {} ids {ids:?}", text(owner), text(target), text(name)),
                );
            }
        }
        if recorded
            .insert((owner, target), (actual.tokens.len(), names, missing))
            .is_some()
        {
            return Err(invalid("duplicate resolved operation-token target"));
        }
    }
    let mut matched = 0;
    for (owner, country) in report.canonical_countries(&save.countries) {
        if let Some(manager) = &country.operation_tokens {
            equal(
                report,
                &format!("{}.operation_assets presence", text(owner)),
                owners_with_targets.contains(owner.as_slice()),
                manager.operation_assets.is_some(),
            );
        }
        let Some(expected) = country
            .operation_tokens
            .as_ref()
            .and_then(|manager| manager.operation_assets.as_ref())
        else {
            continue;
        };
        for expected in expected {
            let label = format!(
                "{}.operation_assets.{}",
                text(owner),
                text(&expected.target)
            );
            if !expected.target.is_empty() && !save.countries.contains_key(&expected.target) {
                return Err(invalid("unknown saved operation-token target country"));
            }
            let Some(actual) = recorded.remove(&(owner.as_slice(), expected.target.as_slice()))
            else {
                report.difference(&label, "missing", "present");
                continue;
            };
            let (count, names, missing) = actual;
            let expected_names = multiset(expected.names.iter().map(Vec::as_slice));
            if missing == 0 {
                matched += 1;
                equal(report, &label, multiset(names), expected_names);
            } else {
                equal(
                    report,
                    &format!("{label} count"),
                    count,
                    expected.names.len(),
                );
                for (name, count) in multiset(names) {
                    let saved_count = expected_names.get(name).copied().unwrap_or(0);
                    if count > saved_count {
                        report.difference(
                            format!("{label}.{} minimum known occurrences", text(name)),
                            count,
                            saved_count,
                        );
                    }
                }
            }
        }
    }
    for ((owner, target), (count, _, _)) in recorded {
        let label = format!("{}.operation_assets.{}", text(owner), text(target));
        if count == 0 {
            report.limitation("empty native operation-token target vectors are omitted by saves, so native map-entry presence cannot be verified", label);
        } else if save.countries[owner].operation_tokens.is_some() {
            report.difference(label, "present", "missing");
        } else {
            report.limitation("missing serialized token-manager payload cannot yet prove a null native manager pointer", label);
        }
    }
    report.compared("OperationTokens", matched);
    report.limitation("operation-token serializer sorts a copy by authored name; only exact names and multiplicities can be verified, not native vector order", "OperationTokens");
    Ok(())
}

fn compare_operations(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    const START_SENTINEL: i32 = 0x29c77f8;
    let mut recorded = BTreeMap::new();
    for actual in complete::<tables::Operations>(frame)? {
        let owner = required_country_tag(actual.owner, countries)?;
        country(save, owner)?;
        if recorded.insert(actual.id, (owner, actual)).is_some() {
            return Err(invalid("duplicate recorded operation id"));
        }
    }
    let mut matched = 0;
    for (owner, country) in report.canonical_countries(&save.countries) {
        let Some(operations) = &country.operations else {
            continue;
        };
        let label = format!("{}.operations", text(owner));
        if let Some(finished) = &operations.finished {
            report.limitation(
                "completed operation history has no recorded table; saved authored entries retained and counted",
                format!("{label}.finished: {} definition entries", finished.len()),
            );
        }
        let Some(running) = &operations.running else {
            let ids: Vec<u64> = recorded
                .iter()
                .filter(|(_, (tag, _))| *tag == owner.as_slice())
                .map(|(id, _)| *id)
                .collect();
            if !ids.is_empty() {
                report.difference(format!("{label}.running count"), ids.len(), 0);
                for id in ids {
                    recorded.remove(&id);
                }
            }
            continue;
        };
        for expected in running {
            let field = format!("{label}.running.{}", text(&expected.definition));
            let Some((tag, actual)) = recorded.remove(&expected.id) else {
                report.difference(format!("{field}.id"), "absent", expected.id);
                continue;
            };
            matched += 1;
            equal(report, &format!("{field}.owner"), tag, owner.as_slice());
            equal(
                report,
                &format!("{field}.target"),
                country_tag(actual.target, countries)?,
                Some(expected.target.as_slice()),
            );
            equal(
                report,
                &format!("{field}.target_province"),
                actual.target_province,
                expected.target_province,
            );
            match token(tokens, actual.definition_token) {
                Some(name) => {
                    if name.contains(&0) {
                        report.limitation(
                            "operation definition formatter receives a C-string pointer, but its concrete stream behavior for interior NUL is unproved; this literal occurrence remains unverified",
                            format!("{field}.definition token {}", actual.definition_token),
                        );
                    } else {
                        equal(
                            report,
                            &format!("{field}.definition"),
                            name,
                            expected.definition.as_slice(),
                        );
                    }
                }
                None => {
                    report.limitation(
                        missing_token_reason(frame),
                        format!("{field}.definition token {}", actual.definition_token),
                    );
                }
            }
            match expected.date {
                Some(date) => equal(
                    report,
                    &format!("{field}.start_hour"),
                    actual.start_hour,
                    date.engine_hour()?,
                ),
                None => equal(
                    report,
                    &format!("{field}.start_hour"),
                    actual.start_hour,
                    START_SENTINEL,
                ),
            }
            equal(
                report,
                &format!("{field}.duration"),
                actual.duration_days,
                expected.duration.unwrap_or(0),
            );
            equal(
                report,
                &format!("{field}.phase_count"),
                actual.phase_count,
                u32::try_from(expected.phases.as_ref().map(Vec::len).unwrap_or(0))
                    .map_err(|_| invalid("saved operation phase count exceeds u32"))?,
            );
            equal(
                report,
                &format!("{field}.assigned_slots"),
                actual.assigned_slots,
                u32::try_from(expected.operative_slots.as_ref().map(Vec::len).unwrap_or(0))
                    .map_err(|_| invalid("saved operation slot count exceeds u32"))?,
            );
            equal(
                report,
                &format!("{field}.completed"),
                actual.terminal != 0,
                expected.terminal,
            );
            for (name, value, expected) in [
                ("outcome_extra", &actual.bonus, expected.bonus),
                ("risk_extra", &actual.detected, expected.detected),
            ] {
                if let Some(value) = report.recorded(value, &format!("Operations.{name}"), &field) {
                    equal(report, &format!("{field}.{name}"), *value != 0, expected);
                }
            }
            if expected.terminal {
                if let Some(actual) = report.recorded(
                    &actual.effect_description,
                    "Operations.effect_description",
                    &field,
                ) {
                    match &expected.effect_description {
                        Some(description) => equal(
                            report,
                            &format!("{field}.looted"),
                            saved_effect_description(actual),
                            description.clone(),
                        ),
                        None => report.difference(
                            format!("{field}.looted"),
                            saved_effect_description(actual),
                            "ordinary terminal save omitted its description",
                        ),
                    }
                }
            } else if expected.effect_description.is_some() {
                return Err(invalid(format!(
                    "{field}.looted is present on a nonterminal saved operation"
                )));
            }
            equal(
                report,
                &format!("{field}.civilian_factories"),
                actual.preparation_current,
                expected
                    .civilian_factories
                    .as_ref()
                    .map_or(0, |value| value.raw),
            );
            equal(
                report,
                &format!("{field}.total"),
                actual.preparation_required,
                expected.total.as_ref().map_or(0, |value| value.raw),
            );
            compare_operation_phases(report, &field, actual, expected, tokens);
            compare_operation_resources(report, &field, actual, expected, tokens);
            compare_operation_equipment(report, &field, actual, expected);
        }
    }
    for (id, (tag, _)) in recorded {
        report.difference(
            format!("{}.operations.running id {id}", text(tag)),
            "present",
            "absent from the save",
        );
    }
    report.compared("Operations", matched);
    report.limitation(
        "operation-manager priority is saved by the manager writer but has no recorded table; only running instances are compared",
        "Operations.priority",
    );
    report.limitation(
        "operation flag_40 and auto_repeat words have no proved save literal in the observed live shapes (all six running instances omit them); they are retained natively but not verified here",
        "Operations.flag_40, Operations.auto_repeat",
    );
    report.limitation(
        "the operation writer saves no equipment pool archetype totals, no pool entry's archetype, no equipment requirement's preparation days and no requirement's uses-preparation-time byte",
        "Operations.equipment_totals, equipment.archetype, requirements.days (equipment requirements), requirements.uses_time: runtime-only; no text-save equality claim",
    );
    report.limitation(
        "operation definition names resolve through the recorded lexical dictionary; the exact authored-name mapping remains unproved and a missing entry leaves the definition unverified",
        "Operations.definition_token",
    );
    report.limitation(
        "the operation writer omits descriptions for nonterminal instances; description writing removes CR and text writing stops at NUL, so the save cannot independently certify those raw bytes or a nonterminal stored description",
        "Operations.effect_description compares only its terminal ordinary-save projection; exact raw bytes still require coordinated memory/capture assertion",
    );
    Ok(())
}

const FACTORY_SCALE: i64 = 100_000;

fn compare_operation_phases(
    report: &mut Report,
    field: &str,
    actual: &replay_data::rows::OperationRow,
    expected: &saved::OperationInstance,
    tokens: &TokenNames<'_>,
) {
    let names: Option<Vec<&[u8]>> = actual
        .phases
        .iter()
        .map(|&id| token(tokens, id).filter(|name| !name.contains(&0)))
        .collect();
    let Some(names) = names else {
        report.limitation(
            "operation phase definition token has no exact LexicalTokens entry",
            format!("{field}.phases"),
        );
        return;
    };
    let saved: Vec<&[u8]> = expected
        .phases
        .iter()
        .flatten()
        .map(Vec::as_slice)
        .collect();
    equal(report, &format!("{field}.phases"), names, saved);
}

fn compare_operation_resources(
    report: &mut Report,
    field: &str,
    actual: &replay_data::rows::OperationRow,
    expected: &saved::OperationInstance,
    tokens: &TokenNames<'_>,
) {
    let resources = expected.resources.as_deref().unwrap_or_default();
    equal(
        report,
        &format!("{field}.resources.count"),
        actual.requirements.len(),
        resources.len(),
    );
    for (index, (requirement, resource)) in actual.requirements.iter().zip(resources).enumerate() {
        let entry = format!("{field}.resources[{index}]");
        match (requirement.equipment != 0, resource) {
            (true, saved::OperationResource::Equipment { archetype, amount }) => {
                match token(tokens, requirement.archetype).filter(|name| !name.contains(&0)) {
                    Some(name) => equal(
                        report,
                        &format!("{entry}.archetype"),
                        name,
                        archetype.as_slice(),
                    ),
                    None => report.limitation(
                        "operation requirement archetype token has no exact LexicalTokens entry",
                        format!("{entry}.archetype token {}", requirement.archetype),
                    ),
                }
                equal(
                    report,
                    &format!("{entry}.amount"),
                    requirement.amount,
                    amount.raw,
                );
            }
            (false, saved::OperationResource::Factories { amount, days }) => {
                equal(
                    report,
                    &format!("{entry}.amount"),
                    (requirement.amount / FACTORY_SCALE) as i32,
                    *amount,
                );
                equal(report, &format!("{entry}.days"), requirement.days, *days);
            }
            (equipment, resource) => report.difference(
                format!("{entry}.kind"),
                if equipment {
                    "equipment"
                } else {
                    "civilian_factories"
                },
                format!("{resource:?}"),
            ),
        }
    }
}

fn compare_operation_equipment(
    report: &mut Report,
    field: &str,
    actual: &replay_data::rows::OperationRow,
    expected: &saved::OperationInstance,
) {
    let label = format!("{field}.equipment");
    let Some(pool) = &expected.equipment else {
        report.difference(&label, actual.equipment.len(), "missing serialized pool");
        return;
    };
    equal(
        report,
        &format!("{label}.allow_zero_entries"),
        actual.equipment_allow_zero != 0,
        pool.allow_zero,
    );
    let visible: Vec<_> = actual
        .equipment
        .iter()
        .filter(|entry| pool.allow_zero || entry.amount != 0)
        .collect();
    equal(
        report,
        &format!("{label}.count"),
        visible.len(),
        pool.entries.len(),
    );
    for (index, (entry, saved)) in visible.iter().zip(&pool.entries).enumerate() {
        equal(
            report,
            &format!("{label}[{index}].variant"),
            entry.variant,
            saved.variant,
        );
        equal(
            report,
            &format!("{label}[{index}].amount"),
            entry.amount,
            saved.amount.raw,
        );
    }
}

fn saved_effect_description(raw: &[u8]) -> Vec<u8> {
    let mut saved = Vec::new();
    for &byte in raw {
        match byte {
            0 => break,
            b'\r' => {}
            b'\n' => saved.extend_from_slice(b"\\n"),
            byte => saved.push(byte),
        }
    }
    saved
}

fn compare_operation_assignments(
    frame: &DecodedFrame,
    save: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let mut recorded = BTreeMap::new();
    for actual in complete::<tables::OperationAssignments>(frame)? {
        if recorded
            .insert((actual.operation, actual.ordinal), actual)
            .is_some()
        {
            return Err(invalid("duplicate recorded operation assignment identity"));
        }
    }
    let mut matched = 0;
    let mut mission_slots = 0usize;
    let mut empty_mission_slots = 0usize;
    let mut mission_items = 0usize;
    for (owner, country) in report.canonical_countries(&save.countries) {
        let Some(operations) = &country.operations else {
            continue;
        };
        let Some(running) = &operations.running else {
            continue;
        };
        for expected in running {
            let slots = expected.operative_slots.as_deref().unwrap_or_default();
            for (ordinal, slot) in slots.iter().enumerate() {
                if let Some(items) = &slot.mission {
                    mission_slots += 1;
                    if items.is_empty() {
                        empty_mission_slots += 1;
                    } else {
                        mission_items += items.len();
                    }
                }
                let ordinal = u32::try_from(ordinal)
                    .map_err(|_| invalid("saved operation slot ordinal exceeds u32"))?;
                let label = format!(
                    "{}.operations.running.{}.operative_slots[{ordinal}]",
                    text(owner),
                    text(&expected.definition)
                );
                let Some(actual) = recorded.remove(&(expected.id, ordinal)) else {
                    report.difference(label, "absent", slot.operative.unwrap_or(0));
                    continue;
                };
                matched += 1;
                match slot.operative {
                    Some(handle) => equal(
                        report,
                        &format!("{label}.operative"),
                        actual.operative,
                        handle,
                    ),
                    None => equal(report, &format!("{label}.operative"), actual.operative, 0),
                }
            }
        }
    }
    let mut saved_ids = BTreeSet::new();
    for (_, country) in report.canonical_countries(&save.countries) {
        if let Some(operations) = &country.operations
            && let Some(running) = &operations.running
        {
            for instance in running {
                saved_ids.insert(instance.id);
            }
        }
    }
    for ((operation, ordinal), row) in recorded {
        if !saved_ids.contains(&operation) {
            report.difference(
                format!("operation {operation}.operative_slots[{ordinal}]"),
                row.operative,
                "absent operation",
            );
        } else {
            report.difference(
                format!("operation {operation}.operative_slots[{ordinal}]"),
                "present",
                "absent from the saved slot list",
            );
        }
    }
    report.compared("OperationAssignments", matched);
    report.limitation(
        "operation selection-vs-mission semantics are unproved: comparison verifies only slot presence, positional ordinal and operative handle; recorded selection words and saved literal mission payloads are retained but not equated",
        format!(
            "OperationAssignments.selection: {mission_slots} saved mission blocks ({empty_mission_slots} empty, {mission_items} retained items)"
        ),
    );
    report.limitation(
        "a present operative_slots={} entry without operative certifies a null handle; an omitted operative_slots payload is distinct from a present null slot and cannot certify its position",
        "OperationAssignments null-slot versus missing-payload distinction",
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::Recorded, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;

    #[test]
    fn network_summaries_compare_each_saved_cache_independently_of_state_values() {
        let (_, _, countries) = operation_links();
        let countries = countries.iter().map(|(id, row)| (*id, row)).collect();
        let save = parsed(
            br#"countries={GER={} SOV={}} strategic_operatives={strategic_operative={7={
            intel_network={target="SOV" intel_network={}
                coverage={core_states=17 controlled_states=29 owned_worth=31}
                strength_sum=-0.00001 strength_sum_over_cores=2.00003 state_strength_max=4.00007
            }
        }}}"#,
        );
        let expected = replay_data::rows::IntelNetworkSummaryRow {
            owner_country: 101,
            target_country: 102,
            ordinal: 0,
            covered_core_states: 17,
            covered_controlled_states: 29,
            covered_worth: 31,
            controlled_strength_sum: -1,
            core_strength_sum: 200_003,
            max_state_strength: 400_007,
        };
        let compare = |row| {
            let mut report = Report::default();
            compare_network_summaries(
                &frame(
                    TableId::IntelNetworkSummaries,
                    TableRows::IntelNetworkSummaries(vec![row]),
                ),
                &save,
                &countries,
                &mut report,
            )
            .unwrap();
            report
        };
        let report = compare(expected);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["IntelNetworkSummaries"], 1);
        for field in 0..7 {
            let mut changed = expected;
            match field {
                0 => changed.covered_core_states += 1,
                1 => changed.covered_controlled_states += 1,
                2 => changed.covered_worth += 1,
                3 => changed.controlled_strength_sum += 1,
                4 => changed.core_strength_sum += 1,
                5 => changed.max_state_strength += 1,
                _ => changed.ordinal += 1,
            }
            assert_eq!(compare(changed).differences.len(), 1, "field {field}");
        }
    }

    #[test]
    fn captured_reference_oracle_checks_order_nulls_and_duplicates() {
        let country = CountryRow {
            id: 7,
            tag: b"AAA".to_vec(),
            ..Default::default()
        };
        let countries = BTreeMap::from([(7, &country)]);
        let save = parsed(br#"countries={AAA={intelligence_agency={
            defense=0 max_operative_count=0 usable_operative_slots=0
            captured={{operative={id=9 type=4713} intel={}} {operative={id=0 type=0} intel={}} {operative={id=9 type=4713} intel={}}}
        }}}"#);
        let rows = [
            ((9 << 32) | 4713, Some(7)),
            (0, None),
            ((9 << 32) | 4713, Some(7)),
        ]
        .into_iter()
        .enumerate()
        .map(
            |(ordinal, (operative, owner))| replay_data::rows::CapturedOperativeReferenceRow {
                country: 7,
                ordinal: ordinal as u32,
                operative,
                owner,
            },
        )
        .collect::<Vec<_>>();
        let make = |rows| {
            frame(
                TableId::CapturedOperativeReferences,
                TableRows::CapturedOperativeReferences(rows),
            )
        };
        let mut report = Report::default();
        compare_captured_operatives(&make(rows.clone()), &save, &countries, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["CapturedOperativeReferences"], 3);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("resolved commander owner"))
        );
        let mut changed = rows.clone();
        changed[0].operative = 0;
        changed[1].operative = (9 << 32) | 4713;
        let mut report = Report::default();
        compare_captured_operatives(&make(changed), &save, &countries, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        let mut missing = rows;
        missing.remove(1);
        assert!(
            compare_captured_operatives(&make(missing), &save, &countries, &mut Report::default())
                .is_err()
        );
    }

    #[test]
    fn owned_war_goal_recipients_compare_with_saved_goals() {
        let rows = [(7, b"AAA"), (8, b"BBB")].map(|(id, tag)| CountryRow {
            id,
            tag: tag.to_vec(),
            ..Default::default()
        });
        let countries = BTreeMap::from([(7, &rows[0]), (8, &rows[1])]);
        let input: &[u8] = br#"countries={AAA={diplomacy={wargoals={take_state={id={id=3 type=4713} wargoaldata_actor=AAA wargoaldata_recipient=BBB type=take_state} puppet={id={id=4 type=4713} wargoaldata_actor=AAA wargoaldata_recipient=AAA type=puppet}} available_wargoals={wargoal={id=3 type=4713}}}} BBB={diplomacy={}}}"#;
        let save = parsed(input);
        let rosters = content_reader_save::country_rosters::parse(
            &content_reader_parser::parser::parse(input).unwrap().items,
        )
        .unwrap();
        let run = |goals: [Recorded<Option<Vec<u32>>>; 2]| {
            let [first, second] = goals;
            let capitulation = |country, war_goal_recipients| replay_data::rows::CapitulationRow {
                country,
                war_goal_recipients,
                ..Default::default()
            };
            let frame = frame(
                TableId::Capitulation,
                TableRows::Capitulation(vec![capitulation(7, first), capitulation(8, second)]),
            );
            let mut report = Report::default();
            compare_capitulation(&frame, &save, &rosters, &countries, &mut report).unwrap();
            report.differences
        };
        assert!(run([Some(vec![8]).into(), Some(vec![]).into()]).is_empty());
        assert_eq!(run([Some(vec![]).into(), Some(vec![]).into()]).len(), 1);
        assert_eq!(run([None.into(), Some(vec![]).into()]).len(), 1);
        assert_eq!(
            run([Some(vec![8, 8]).into(), Some(vec![7]).into()]).len(),
            2
        );
    }

    #[test]
    fn raid_intel_oracle_checks_roster_and_reports_unsaved_operands() {
        let country = CountryRow {
            id: 101,
            canonical_slot: 3,
            ..Default::default()
        };
        let countries = BTreeMap::from([(101, &country)]);
        let save = parsed(b"countries={ABC={}} strategic_operatives={strategic_operative={3={}}}");
        let make = |present| {
            frame(
                TableId::RaidIntelInputs,
                TableRows::RaidIntelInputs(vec![replay_data::rows::RaidIntelInputRow {
                    country: 101,
                    strategic_present: present,
                    country_defense: 123456,
                    crypto_strength: Some(-1),
                    ..Default::default()
                }]),
            )
        };
        let mut report = Report::default();
        compare_raid_intel(&make(1), &save, &countries, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables["RaidIntelInputs"], 1);
        compare_raid_intel(&make(0), &save, &countries, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(compare_raid_intel(&make(2), &save, &countries, &mut Report::default()).is_err());
        let mut missing = Report::default();
        compare_raid_intel(
            &frame(TableId::RaidIntelInputs, TableRows::RaidIntelInputs(vec![])),
            &save,
            &countries,
            &mut missing,
        )
        .unwrap();
        assert_eq!(missing.differences.len(), 1);
    }

    const SCOPES: ScopeKinds = ScopeKinds {
        global: 0,
        country: 1,
        state: 2,
        character: 3,
    };

    #[test]
    fn propaganda_oracle_checks_unused_slots_logical_order_and_independent_drift() {
        use replay_data::rows::PropagandaEffortRow as Row;
        let save = parsed(
            br#"countries={} strategic_operatives={strategic_operative={0={
            recent_propaganda_effort={{stability=0.00033 war_support=-0.00039}{stability=0.00051 war_support=0.00057}}
            propaganda_weekly_drift={stability=0.03048 war_support=-0.03048}
        }}}"#,
        );
        let rows = vec![
            Row {
                slot: 0,
                country: 0,
                sequence: 7,
                stability: 11,
                war_support: -13,
                derived_drift: 0,
            },
            Row {
                slot: 0,
                country: 0,
                sequence: 8,
                stability: 17,
                war_support: 19,
                derived_drift: 0,
            },
            Row {
                slot: 0,
                country: 0,
                sequence: u64::MAX,
                stability: 999,
                war_support: -999,
                derived_drift: 0,
            },
        ];
        let check = |rows| {
            let mut report = Report::default();
            compare_propaganda(
                &frame(
                    TableId::PropagandaEfforts,
                    TableRows::PropagandaEfforts(rows),
                ),
                &save,
                &mut report,
            )
            .unwrap();
            report
        };
        assert!(check(rows.clone()).differences.is_empty());
        let mut changed = rows.clone();
        changed[0].stability += 1;
        assert_eq!(check(changed).differences.len(), 1);
        let mut derived = rows;
        derived[2].derived_drift = 1;
        derived[2].stability = 0;
        derived[2].war_support = 0;
        assert_eq!(check(derived).differences.len(), 2);
        assert_eq!(check(Vec::new()).differences.len(), 1);
    }

    fn frame(table: TableId, rows: TableRows) -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Default::default(),
        };
        *frame.tables[table.index()] = TableView::Complete(rows);
        frame
    }

    fn incoming_pack() -> saved::Pack {
        parsed(
            br#"countries={GER={incoming_diplomatic_action={
                0={ id={ id=63179 type=4713 }
                    call_allies={ type=0 initiator_matters=no actor="SOV" original_actor="SOV"
                        recipient="GER" original_recipient="GER"
                        date="1936.1.2.1" last_command_date="1936.1.2.2" } }
            }} SOV={incoming_diplomatic_action={}}}"#,
        )
    }

    fn incoming_rows(rows: Vec<replay_data::rows::IncomingDiplomaticActionRow>) -> DecodedFrame {
        frame(
            TableId::IncomingDiplomaticActions,
            TableRows::IncomingDiplomaticActions(rows),
        )
    }

    fn incoming_row() -> replay_data::rows::IncomingDiplomaticActionRow {
        replay_data::rows::IncomingDiplomaticActionRow {
            country: 1,
            ordinal: 0,
            handle: (63179 << 32) | 4713,
            status: 0,
            action_type: 0,
            actor: 2,
            original_actor: 2,
            recipient: 1,
            original_recipient: 1,
            hour: GameDate::parse(b"1936.1.2.1")
                .unwrap()
                .engine_hour()
                .unwrap(),
            last_command_hour: GameDate::parse(b"1936.1.2.2")
                .unwrap()
                .engine_hour()
                .unwrap(),
        }
    }

    #[test]
    fn incoming_flag_uses_the_serialized_byte_instead_of_the_independent_type() {
        let germany = CountryRow {
            id: 1,
            canonical_slot: 1,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let soviet = CountryRow {
            id: 2,
            canonical_slot: 2,
            tag: b"SOV".to_vec(),
            ..Default::default()
        };
        let countries = BTreeMap::from([(1, &germany), (2, &soviet)]);
        let mut pack = incoming_pack();
        let expected = &mut pack
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .incoming_actions
            .as_mut()
            .unwrap()[0];
        expected.initiator_matters = Some(true);
        let mut row = incoming_row();
        row.action_type = 0x1234_5601;
        let mut report = Report::default();
        compare_incoming_actions(
            &incoming_rows(vec![row.clone()]),
            &pack,
            &countries,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        row.action_type = 0x1234_5600;
        let mut report = Report::default();
        compare_incoming_actions(
            &incoming_rows(vec![row.clone()]),
            &pack,
            &countries,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        pack.countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .incoming_actions
            .as_mut()
            .unwrap()[0]
            .initiator_matters = None;
        assert!(
            compare_incoming_actions(
                &incoming_rows(vec![row]),
                &pack,
                &countries,
                &mut Report::default()
            )
            .is_err()
        );
    }

    #[test]
    fn a_matching_incoming_queue_compares_every_raw_field_without_differences() {
        let (germany, soviet) = (
            CountryRow {
                id: 1,
                canonical_slot: 1,
                tag: b"GER".to_vec(),
                ..CountryRow::default()
            },
            CountryRow {
                id: 2,
                canonical_slot: 2,
                tag: b"SOV".to_vec(),
                ..CountryRow::default()
            },
        );
        let countries = BTreeMap::from([(1, &germany), (2, &soviet)]);
        let mut report = Report::default();
        compare_incoming_actions(
            &incoming_rows(vec![incoming_row()]),
            &incoming_pack(),
            &countries,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["IncomingDiplomaticActions"], 1);

        let mut row = incoming_row();
        row.ordinal = 1;
        let mut report = Report::default();
        compare_incoming_actions(
            &incoming_rows(vec![row]),
            &incoming_pack(),
            &countries,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);

        for mutate in [
            (|row: &mut replay_data::rows::IncomingDiplomaticActionRow| row.handle = 1)
                as fn(&mut _),
            |row| row.action_type = 1,
            |row| row.actor = 1,
            |row| row.hour += 1,
            |row| row.last_command_hour += 1,
        ] {
            let mut row = incoming_row();
            mutate(&mut row);
            let mut report = Report::default();
            compare_incoming_actions(
                &incoming_rows(vec![row]),
                &incoming_pack(),
                &countries,
                &mut report,
            )
            .unwrap();
            assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        }

        let mut second = incoming_row();
        second.ordinal = 4;
        second.handle = 7;
        let pack = parsed(
            br#"countries={GER={incoming_diplomatic_action={
                0={ id={ id=63179 type=4713 }
                    call_allies={ type=0 initiator_matters=no actor="SOV" original_actor="SOV"
                        recipient="GER" original_recipient="GER"
                        date="1936.1.2.1" last_command_date="1936.1.2.2" } }
                4={ id={ id=0 type=7 }
                    call_allies={ type=0 initiator_matters=no actor="SOV" original_actor="SOV"
                        recipient="GER" original_recipient="GER"
                        date="1936.1.2.1" last_command_date="1936.1.2.2" } }
            }}}"#,
        );
        let mut report = Report::default();
        compare_incoming_actions(
            &incoming_rows(vec![incoming_row(), second]),
            &pack,
            &countries,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);

        let mut row = incoming_row();
        row.country = 2;
        let mut report = Report::default();
        compare_incoming_actions(
            &incoming_rows(vec![row]),
            &incoming_pack(),
            &countries,
            &mut report,
        )
        .unwrap();
        assert!(!report.differences.is_empty());
    }

    fn parsed(input: &[u8]) -> saved::Pack {
        saved::parse(&content_reader_parser::parser::parse(input).unwrap().items).unwrap()
    }

    fn germany() -> CountryRow {
        CountryRow {
            id: 101,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn character_variables_compare_with_each_saved_character() {
        let items = content_reader_parser::parser::parse(
            br#"variables={} countries={} character_manager={historical={
            character={id={id=44 type=73} token=a variables={ score=1.5 }}
            character={id={id=45 type=73} token=b}
        }}"#,
        )
        .unwrap()
        .items;
        let characters = saved_characters::parse(&items).unwrap();
        let manager = characters.manager.as_ref().unwrap();
        let pack = saved::parse(&items).unwrap();
        let row = |scope_id: u32, entries: &[(&[u8], i64)]| replay_data::rows::ScriptVariablesRow {
            scope_kind: SCOPES.character,
            scope_id,
            seed: Recorded::Value(None),
            entries: entries
                .iter()
                .map(|(name, value_raw)| replay_data::rows::ScriptVariableEntry {
                    name: name.to_vec(),
                    value_raw: *value_raw,
                })
                .collect(),
        };
        let global = replay_data::rows::ScriptVariablesRow::default();
        let run = |rows: Vec<replay_data::rows::ScriptVariablesRow>| {
            let mut report = Report::default();
            compare_variables(
                &frame(TableId::ScriptVariables, TableRows::ScriptVariables(rows)),
                &pack,
                Some(manager),
                &BTreeMap::new(),
                SCOPES,
                &mut report,
            )
            .unwrap();
            report
        };
        let report = run(vec![
            global.clone(),
            row(44, &[(b"score", 150_000)]),
            row(45, &[]),
        ]);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables.get("ScriptVariables"), Some(&3));
        let report = run(vec![
            global,
            row(44, &[(b"score", 150_000)]),
            row(45, &[(b"extra", 1)]),
        ]);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        let report = run(vec![row(44, &[(b"score", 150_000)])]);
        assert!(!report.differences.is_empty());
    }

    #[test]
    fn character_flags_compare_with_each_saved_characters_flags() {
        let items = content_reader_parser::parser::parse(
            br#"countries={} character_manager={historical={
            character={id={id=44 type=73} token=a flags={
                recently_sick={ value=1 date="1936.1.26.1" days=180 }
            }}
            character={id={id=45 type=73} token=b}
        }}"#,
        )
        .unwrap()
        .items;
        let characters = saved_characters::parse(&items).unwrap();
        let manager = characters.manager.as_ref().unwrap();
        let intel = saved::parse(&items).unwrap();
        let date = GameDate::parse(b"1936.1.26.1")
            .unwrap()
            .engine_hour()
            .unwrap();
        let row =
            |scope_id: u32, token: u32, remaining_days: i16| replay_data::rows::ScriptFlagRow {
                scope_kind: SCOPES.character,
                scope_id,
                token,
                set_hour: date,
                value: 1,
                remaining_days,
            };
        let tokens = BTreeMap::from([(600, b"recently_sick".as_slice()), (601, b"other")]);
        let run = |rows: Vec<replay_data::rows::ScriptFlagRow>| {
            let mut report = Report::default();
            compare_character_flags(
                &frame(TableId::ScriptFlags, TableRows::ScriptFlags(rows)),
                Some(manager),
                &tokens,
                SCOPES.character,
                &mut report,
            )
            .unwrap();
            report
        };
        let report = run(vec![row(44, 600, 180)]);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report.compared_tables.get("ScriptFlags character scope"),
            Some(&1)
        );
        assert!(!report.compared_tables.contains_key("ScriptFlags"));
        assert_eq!(run(vec![row(44, 600, 179)]).differences.len(), 1);
        assert_eq!(
            run(vec![row(44, 600, 180), row(45, 601, 0)])
                .differences
                .len(),
            2
        );
        assert_eq!(run(vec![]).differences.len(), 2);
        let mut report = Report::default();
        compare_flags(
            &frame(
                TableId::ScriptFlags,
                TableRows::ScriptFlags(vec![row(44, 600, 180)]),
            ),
            &intel,
            &BTreeMap::new(),
            &tokens,
            SCOPES.country,
            SCOPES.character,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty() && report.limitations.is_empty());
    }

    #[test]
    fn agency_oracle_catches_one_raw_unit_and_refuses_incomplete_tables() {
        let pack = parsed(b"countries={GER={intelligence_agency={name=CustomAgency is_created=yes defense=90071992547.40993 max_operative_count=-1 usable_operative_slots=-2}}}");
        let country = germany();
        let links = BTreeMap::from([(101, &country), (102, &country)]);
        let mut actual = replay_data::rows::IntelligenceAgencyRow {
            country: 102,
            defense_raw: 9_007_199_254_740_993,
            name: b"CustomAgency".to_vec(),
            created: 1,
            unknown_f0: u32::MAX,
            admission_ceiling: -2,
            ..Default::default()
        };
        let table = TableId::IntelligenceAgencies;
        let mut report = Report::default();
        compare_agencies(
            &frame(table, TableRows::IntelligenceAgencies(vec![actual.clone()])),
            &pack,
            &links,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        actual.name.extend_from_slice(b"\0tail");
        let mut prefix_report = Report::default();
        compare_agencies(
            &frame(table, TableRows::IntelligenceAgencies(vec![actual.clone()])),
            &pack,
            &links,
            &mut prefix_report,
        )
        .unwrap();
        assert!(
            prefix_report.differences.is_empty(),
            "{:?}",
            prefix_report.differences
        );
        assert!(
            prefix_report
                .limitations
                .values()
                .flatten()
                .any(|field| field.ends_with(".name"))
        );
        actual.name = b"CustomAgency".to_vec();
        actual.defense_raw -= 1;
        actual.name.push(b'!');
        actual.created = 0;
        let mut report = Report::default();
        compare_agencies(
            &frame(table, TableRows::IntelligenceAgencies(vec![actual])),
            &pack,
            &links,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 3);
        let mut unavailable = frame(table, TableRows::empty(table));
        *unavailable.tables[table.index()] = TableView::Unavailable {
            reason: "source read failed".into(),
        };
        assert!(compare_agencies(&unavailable, &pack, &links, &mut Report::default()).is_err());
        let mut report = Report::default();
        compare_agencies(
            &frame(table, TableRows::empty(table)),
            &pack,
            &links,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    fn acquired_upgrade_frame(rows: Vec<replay_data::rows::AgencyUpgradeLevelRow>) -> DecodedFrame {
        let mut captured = frame(
            TableId::AgencyUpgradeLevels,
            TableRows::AgencyUpgradeLevels(rows),
        );
        *captured.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(vec![
                replay_data::rows::IntelligenceAgencyRow {
                    country: 101,
                    ..Default::default()
                },
            ]));
        captured
    }

    fn acquired_upgrade_rows() -> Vec<replay_data::rows::AgencyUpgradeLevelRow> {
        [3, -2, 0]
            .into_iter()
            .enumerate()
            .map(
                |(ordinal, level)| replay_data::rows::AgencyUpgradeLevelRow {
                    country: 102,
                    ordinal: ordinal as u32,
                    definition_pointer: 0x1000 + ordinal as u64 * 0x100,
                    branch_pointer: 0x2000,
                    upgrade_token: Some(if ordinal == 1 { 12 } else { 11 }),
                    branch_token: Some(10),
                    level,
                    identity_status: 0,
                },
            )
            .collect()
    }

    #[test]
    fn agency_upgrade_oracle_compares_ordered_duplicates_empty_reset_and_refusals() {
        let country = germany();
        let countries = BTreeMap::from([(101, &country), (102, &country)]);
        let tokens = BTreeMap::from([
            (10, b"mod_branch".as_slice()),
            (11, b"mod_upgrade".as_slice()),
            (12, b"other_upgrade".as_slice()),
        ]);
        let saved = parsed(b"countries={GER={intelligence_agency={defense=0 max_operative_count=0 usable_operative_slots=0 upgrades={mod_upgrade=3 other_upgrade=-2 mod_upgrade=0}}}}");
        let run = |rows| {
            let mut report = Report::default();
            compare_agency_upgrade_levels(
                &acquired_upgrade_frame(rows),
                &saved,
                &countries,
                &tokens,
                &mut report,
            )
            .unwrap();
            report
        };
        assert!(run(acquired_upgrade_rows()).differences.is_empty());
        let mut reordered = acquired_upgrade_rows();
        reordered[0].upgrade_token = Some(12);
        assert_eq!(run(reordered).differences.len(), 1);
        let mut changed = acquired_upgrade_rows();
        changed[1].level = i32::MIN;
        assert_eq!(run(changed).differences.len(), 1);
        let mut missing = acquired_upgrade_rows();
        missing.remove(1);
        assert!(
            run(missing)
                .differences
                .iter()
                .any(|difference| difference.contains("missing ordinal"))
        );
        let mut duplicate = acquired_upgrade_rows();
        duplicate[1].ordinal = 0;
        assert!(
            compare_agency_upgrade_levels(
                &acquired_upgrade_frame(duplicate),
                &saved,
                &countries,
                &tokens,
                &mut Report::default()
            )
            .is_err()
        );
        let mut refused = acquired_upgrade_rows();
        refused[0].identity_status = 1;
        refused[0].upgrade_token = None;
        let error = compare_agency_upgrade_levels(
            &acquired_upgrade_frame(refused),
            &saved,
            &countries,
            &tokens,
            &mut Report::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("1 refused acquired-upgrade identities")
                && error.contains("GER.upgrades[0]")
        );
        let mut partial_refused = acquired_upgrade_frame(vec![]);
        let mut partial_rows = acquired_upgrade_rows();
        partial_rows[0].identity_status = engine_native_intelligence::agency_upgrade_levels::NULL_DEFINITION;
        *partial_refused.tables[TableId::AgencyUpgradeLevels.index()] = TableView::Truncated {
            rows: TableRows::AgencyUpgradeLevels(partial_rows),
            missing: 1,
        };
        let error = compare_agency_upgrade_levels(
            &partial_refused,
            &saved,
            &countries,
            &tokens,
            &mut Report::default(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("agency upgrade definition pointer is null"));
        for created in ["", "is_created=yes"] {
            let empty = parsed(format!("countries={{GER={{intelligence_agency={{defense=0 max_operative_count=0 usable_operative_slots=0 {created}}}}}}}").as_bytes());
            let mut report = Report::default();
            compare_agency_upgrade_levels(
                &acquired_upgrade_frame(vec![]),
                &empty,
                &countries,
                &tokens,
                &mut report,
            )
            .unwrap();
            assert!(report.differences.is_empty());
            let mut missing_root = acquired_upgrade_frame(vec![]);
            *missing_root.tables[TableId::IntelligenceAgencies.index()] =
                TableView::Complete(TableRows::IntelligenceAgencies(vec![]));
            let mut report = Report::default();
            compare_agency_upgrade_levels(&missing_root, &empty, &countries, &tokens, &mut report)
                .unwrap();
            assert_eq!(report.differences.len(), 1);
        }
        for view in [
            TableView::Unavailable {
                reason: "read failed".into(),
            },
            TableView::Truncated {
                rows: TableRows::AgencyUpgradeLevels(acquired_upgrade_rows()),
                missing: 1,
            },
        ] {
            let mut captured = acquired_upgrade_frame(vec![]);
            *captured.tables[TableId::AgencyUpgradeLevels.index()] = view;
            assert!(
                compare_agency_upgrade_levels(
                    &captured,
                    &saved,
                    &countries,
                    &tokens,
                    &mut Report::default()
                )
                .is_err()
            );
        }
        assert!(
            compare_agency_upgrade_levels(
                &acquired_upgrade_frame(acquired_upgrade_rows()),
                &saved,
                &countries,
                &BTreeMap::new(),
                &mut Report::default()
            )
            .unwrap_err()
            .to_string()
            .contains("recorded lexical spelling")
        );
    }

    #[test]
    fn agency_upgrade_oracle_covers_every_country_instance() {
        let mut save = saved::Pack::default();
        let template = parsed(b"countries={XAA={intelligence_agency={defense=0 max_operative_count=0 usable_operative_slots=0 upgrades={mod_upgrade=-3}}}}").countries.remove(b"XAA".as_slice()).unwrap();
        let countries = (1..=3)
            .map(|id| {
                (
                    id,
                    CountryRow {
                        id,
                        canonical_slot: id,
                        tag: format!("country_{id}").into_bytes(),
                        ..Default::default()
                    },
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut rows = Vec::new();
        let mut agencies = Vec::new();
        for (&id, country) in &countries {
            save.countries.insert(country.tag.clone(), template.clone());
            rows.push(replay_data::rows::AgencyUpgradeLevelRow {
                country: id,
                ordinal: 0,
                upgrade_token: Some(11),
                branch_token: Some(10),
                level: -3,
                ..Default::default()
            });
            agencies.push(replay_data::rows::IntelligenceAgencyRow {
                country: id,
                ..Default::default()
            });
        }
        let mut captured = acquired_upgrade_frame(rows);
        *captured.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(agencies));
        let links = countries
            .iter()
            .map(|(&id, country)| (id, country))
            .collect();
        let tokens = BTreeMap::from([
            (11, b"mod_upgrade".as_slice()),
            (10, b"mod_branch".as_slice()),
        ]);
        let mut report = Report::default();
        compare_agency_upgrade_levels(&captured, &save, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["AgencyUpgradeLevels"], 3);
        let replaced = countries.values().next().unwrap().tag.clone();
        save.countries.remove(&replaced);
        let mut report = Report::default();
        compare_agency_upgrade_levels(&captured, &save, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn agency_upgrade_oracle_reports_ambiguous_branch_and_save_reload_discrepancy() {
        let scratch = std::env::var_os("BATTLEPLAN_TEST_SCRATCH")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let root = scratch.join(format!(
            "agency_upgrade_identity_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let file = root.join("common/intelligence_agency_upgrades/synthetic.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, b"first_branch={mod_upgrade={picture=first}} second_branch={mod_upgrade={picture=second}}").unwrap();
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let intelligence = content_reader_military::intelligence::read_agency_upgrades(&layers).unwrap();
        let country = germany();
        let countries = BTreeMap::from([(101, &country), (102, &country)]);
        let tokens = BTreeMap::from([
            (10, b"second_branch".as_slice()),
            (11, b"mod_upgrade".as_slice()),
        ]);
        let row = acquired_upgrade_rows().remove(0);
        let mut report = Report::default();
        compare_agency_upgrade_identities(
            &acquired_upgrade_frame(vec![row]),
            &countries,
            &tokens,
            Ok(&intelligence),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("save_reload_branch"));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("multiple native branch-owned"))
        );
        assert!(root.starts_with(&scratch));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn independent_tables_continue_after_failures_and_preserve_partial_comparisons() {
        let pack = parsed(b"countries={GER={intelligence_agency={defense=0.00001 max_operative_count=0 usable_operative_slots=0}}}");
        let country = germany();
        let links = BTreeMap::from([(101, &country), (102, &country)]);
        let first = replay_data::rows::IntelligenceAgencyRow {
            country: 101,
            defense_raw: 2,
            ..Default::default()
        };
        let duplicate = replay_data::rows::IntelligenceAgencyRow {
            country: 102,
            ..first.clone()
        };
        let mut frame = frame(
            TableId::IntelligenceAgencies,
            TableRows::IntelligenceAgencies(vec![first, duplicate]),
        );
        for table in [
            TableId::IntelNetworks,
            TableId::ScriptVariables,
            TableId::OperationAssignments,
        ] {
            *frame.tables[table.index()] = TableView::Unavailable {
                reason: format!("{table:?} source failed"),
            };
        }
        let mut report = Report::default();
        assert!(
            compare(
                &frame,
                &pack,
                &Default::default(),
                None,
                &links,
                &BTreeMap::new(),
                Err("no acquired upgrades in this fixture"),
                SCOPES,
                &mut report
            )
            .is_err()
        );
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        assert!(
            report
                .differences
                .iter()
                .any(|s| s.contains("missing serialized name"))
        );
        assert!(
            report
                .differences
                .iter()
                .any(|s| s.contains("raid intel country "))
        );
        assert_eq!(report.scalar_checks, 2);
        for table in [
            "IntelligenceAgencies",
            "IntelNetworks",
            "ScriptVariables",
            "OperationAssignments",
        ] {
            assert!(report.failed_comparisons.contains_key(table), "{table}");
        }
        assert_eq!(report.compared_tables.get("Operations"), Some(&0));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("operation-manager priority"))
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("country-reference aliases"))
        );
    }

    #[test]
    fn flag_duration_omission_checks_predicate_without_certifying_sentinel() {
        let pack = parsed(b"countries={GER={flags={x={value=-32768 date=1936.1.1.1}}}}");
        let country = germany();
        let links = BTreeMap::from([(101, &country)]);
        let tokens = BTreeMap::from([(u32::MAX, b"x".as_slice())]);
        let date = GameDate::parse(b"1936.1.1.1")
            .unwrap()
            .engine_hour()
            .unwrap();
        for days in [i16::MIN, -1, 0, 1] {
            let mut report = Report::default();
            let recorded = frame(
                TableId::ScriptFlags,
                TableRows::ScriptFlags(vec![replay_data::rows::ScriptFlagRow {
                    scope_kind: 9,
                    scope_id: 101,
                    token: u32::MAX,
                    value: i16::MIN,
                    set_hour: date,
                    remaining_days: days,
                }]),
            );
            compare_flags(
                &recorded,
                &pack,
                &links,
                &tokens,
                9,
                SCOPES.character,
                &mut report,
            )
            .unwrap();
            assert_eq!(report.differences.len(), usize::from(days > 0));
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("nonpositive days only"))
            );
        }
    }

    #[test]
    fn token_comparison_preserves_multiplicity_and_literal_bytes() {
        let expected = multiset([b"a".as_slice(), b"\xff".as_slice(), b"a".as_slice()]);
        assert_eq!(
            expected,
            multiset([b"\xff".as_slice(), b"a".as_slice(), b"a".as_slice()])
        );
        assert_ne!(expected, multiset([b"\xff".as_slice(), b"a".as_slice()]));
        let tokens = BTreeMap::from([(u32::MAX, b"\xff".as_slice())]);
        assert_eq!(token(&tokens, u32::MAX).unwrap(), b"\xff");
        assert!(token(&tokens, 0).is_none());
    }

    #[test]
    fn repeated_flag_names_preserve_field_multisets_and_limit_unproved_nul_names() {
        let pack = parsed(b"countries={GER={flags={x={value=-3 date=1936.1.1.1} x={value=4 date=1936.1.2.1 days=5} y={value=9 date=1936.1.3.1}}}}");
        let ger = germany();
        let links = BTreeMap::from([(101, &ger)]);
        let names = BTreeMap::from([
            (0, b"x".as_slice()),
            (1, b"x".as_slice()),
            (2, b"y\0unproved".as_slice()),
        ]);
        let hour = |date| GameDate::parse(date).unwrap().engine_hour().unwrap();
        let mut rows = vec![
            replay_data::rows::ScriptFlagRow {
                scope_kind: SCOPES.country,
                scope_id: 101,
                token: 0,
                value: -3,
                set_hour: hour(b"1936.1.1.1"),
                remaining_days: -1,
            },
            replay_data::rows::ScriptFlagRow {
                scope_kind: SCOPES.country,
                scope_id: 101,
                token: 1,
                value: 4,
                set_hour: hour(b"1936.1.2.1"),
                remaining_days: 5,
            },
            replay_data::rows::ScriptFlagRow {
                scope_kind: SCOPES.country,
                scope_id: 101,
                token: 2,
                value: -999,
                set_hour: -99,
                remaining_days: -9,
            },
        ];
        rows.reverse();
        let mut report = Report::default();
        compare_flags(
            &frame(TableId::ScriptFlags, TableRows::ScriptFlags(rows.clone())),
            &pack,
            &links,
            &names,
            SCOPES.country,
            SCOPES.character,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptFlags"), Some(&2));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("distinct flag token ordinals"))
        );
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("concrete stream behavior for interior NUL is unproved")
        }));
        rows[1].value -= 1;
        let mut report = Report::default();
        compare_flags(
            &frame(TableId::ScriptFlags, TableRows::ScriptFlags(rows)),
            &pack,
            &links,
            &names,
            SCOPES.country,
            SCOPES.character,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn token_dictionary_gaps_are_distinct_from_unavailability_and_reserved_ids() {
        let pack = parsed(b"countries={GER={flags={\"\"={value=0 date=1936.1.1.1} known={value=1 date=1936.1.1.1}}}}");
        let ger = germany();
        let links = BTreeMap::from([(101, &ger)]);
        let names = BTreeMap::from([(0, b"".as_slice()), (1, b"known".as_slice())]);
        let date = GameDate::parse(b"1936.1.1.1")
            .unwrap()
            .engine_hour()
            .unwrap();
        let rows = [0, 1]
            .into_iter()
            .map(|token| replay_data::rows::ScriptFlagRow {
                scope_kind: SCOPES.country,
                scope_id: 101,
                token,
                value: token as i16,
                set_hour: date,
                remaining_days: 0,
            })
            .collect();
        let mut recorded = frame(TableId::ScriptFlags, TableRows::ScriptFlags(rows));
        let mut report = Report::default();
        compare_flags(
            &recorded,
            &pack,
            &links,
            &names,
            SCOPES.country,
            SCOPES.character,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptFlags"), Some(&2));

        let partial_names = BTreeMap::from([(0, b"".as_slice())]);
        let mut report = Report::default();
        compare_flags(
            &recorded,
            &pack,
            &links,
            &partial_names,
            SCOPES.country,
            SCOPES.character,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptFlags"), Some(&1));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("complete recorded lexical dictionary has no entry"))
        );

        *recorded.tables[TableId::LexicalTokens.index()] = TableView::Unavailable {
            reason: "source failed".into(),
        };
        let mut report = Report::default();
        compare_flags(
            &recorded,
            &pack,
            &links,
            &BTreeMap::new(),
            SCOPES.country,
            SCOPES.character,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptFlags"), Some(&0));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("recorded lexical dictionary is unavailable"))
        );
    }

    #[test]
    fn missing_lexical_names_limit_value_joins_but_still_check_counts_and_known_values() {
        let pack = parsed(b"countries={GER={flags={x={value=1 date=1936.1.1.1}} tokens={operation_assets={FRA={b b}}}} FRA={}}");
        let ger = germany();
        let fra = CountryRow {
            id: 201,
            tag: b"FRA".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(101, &ger), (201, &fra)]);
        let flags = frame(
            TableId::ScriptFlags,
            TableRows::ScriptFlags(vec![replay_data::rows::ScriptFlagRow {
                scope_kind: 1,
                scope_id: 101,
                token: 7,
                set_hour: -100,
                value: -9,
                remaining_days: 3,
            }]),
        );
        let mut report = Report::default();
        compare_flags(
            &flags,
            &pack,
            &links,
            &BTreeMap::new(),
            1,
            SCOPES.character,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("dependent name and fields remain unverified"))
        );
        let empty = frame(TableId::ScriptFlags, TableRows::ScriptFlags(vec![]));
        let mut report = Report::default();
        compare_flags(
            &empty,
            &pack,
            &links,
            &BTreeMap::new(),
            1,
            SCOPES.character,
            &mut report,
        )
        .unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("flags count"))
        );
        let rows = frame(
            TableId::OperationTokens,
            TableRows::OperationTokens(vec![replay_data::rows::OperationTokensRow {
                owner: 101,
                target: 201,
                tokens: vec![1, 2],
            }]),
        );
        let mut report = Report::default();
        compare_operation_tokens(&rows, &pack, &links, &BTreeMap::new(), &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("dependent name and fields remain unverified"))
        );
        let mut report = Report::default();
        let partial = BTreeMap::from([(1, b"a".as_slice())]);
        compare_operation_tokens(&rows, &pack, &links, &partial, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("minimum known occurrences"));
    }

    #[test]
    fn operation_token_oracle_checks_outer_presence_and_empty_target_omission() {
        let pack = parsed(b"countries={GER={tokens={operation_assets={FRA={a a z}}}} FRA={}}");
        let ger = germany();
        let fra = CountryRow {
            id: 201,
            tag: b"FRA".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(101, &ger), (201, &fra)]);
        let names = BTreeMap::from([(1, b"a".as_slice()), (2, b"z".as_slice())]);
        let table = TableId::OperationTokens;
        let mut row = replay_data::rows::OperationTokensRow {
            owner: 101,
            target: 201,
            tokens: vec![2, 1, 1],
        };
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row.clone()])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        row.tokens.pop();
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row.clone()])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        row.tokens.clear();
        let pack = parsed(b"countries={GER={tokens={operation_assets={}}} FRA={}}");
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row.clone()])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("empty native operation-token target"))
        );
        let pack = parsed(b"countries={GER={tokens={}} FRA={}}");
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn operation_tokens_held_against_no_target_compare_with_the_saved_empty_key() {
        let pack = saved::parse(&content_reader_parser::parser::parse_save(b"countries={GER={tokens={operation_assets={ ={z a} }}} FRA={}}").unwrap().items).unwrap();
        let ger = germany();
        let links = BTreeMap::from([(101, &ger)]);
        let names = BTreeMap::from([(1, b"a".as_slice()), (2, b"z".as_slice())]);
        let row = replay_data::rows::OperationTokensRow { owner: 101, target: 0, tokens: vec![1, 2] };
        let mut report = Report::default();
        let table = TableId::OperationTokens;
        compare_operation_tokens(&frame(table, TableRows::OperationTokens(vec![row.clone()])), &pack, &links, &names, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables.get("OperationTokens"), Some(&1));
        let mut report = Report::default();
        let fewer = replay_data::rows::OperationTokensRow { tokens: vec![1], ..row };
        compare_operation_tokens(&frame(table, TableRows::OperationTokens(vec![fewer])), &pack, &links, &names, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn operation_token_aliases_and_unproved_nul_names_keep_known_multiplicities() {
        let pack = parsed(b"countries={GER={tokens={operation_assets={FRA={x x y}}}} FRA={}}");
        let ger = germany();
        let fra = CountryRow {
            id: 201,
            tag: b"FRA".to_vec(),
            ..Default::default()
        };
        let links = BTreeMap::from([(101, &ger), (201, &fra)]);
        let names = BTreeMap::from([
            (0, b"x".as_slice()),
            (1, b"x".as_slice()),
            (2, b"y\0unproved".as_slice()),
        ]);
        let table = TableId::OperationTokens;
        let mut row = replay_data::rows::OperationTokensRow {
            owner: 101,
            target: 201,
            tokens: vec![1, 2, 0],
        };
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row.clone()])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("distinct operation-token ordinals"))
        );
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("concrete stream behavior for interior NUL is unproved")
        }));
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("lexical dictionary has no entry"))
        );
        row.tokens[1] = 0;
        let mut report = Report::default();
        compare_operation_tokens(
            &frame(table, TableRows::OperationTokens(vec![row])),
            &pack,
            &links,
            &names,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn variables_check_exact_literals_and_limit_unproved_nul_formatting() {
        let pack = parsed(b"countries={GER={variables={random=1 bar=-0.00001 foo=90071992547.40993 \"\"=0 \"\xff\"=-1.00001 unproved=999}}}");
        let ger = germany();
        let links = BTreeMap::from([(101, &ger), (102, &ger)]);
        let table = TableId::ScriptVariables;
        let mut row = replay_data::rows::ScriptVariablesRow {
            scope_kind: 1,
            scope_id: 102,
            seed: Recorded::Value(None),
            entries: [
                (b"foo".as_slice(), 9_007_199_254_740_993),
                (b"bar".as_slice(), -1),
                (b"".as_slice(), 0),
                (b"random".as_slice(), 1234),
                (b"unproved\0hidden".as_slice(), -5678),
                (b"\xff".as_slice(), -100_001),
            ]
            .into_iter()
            .map(|(name, value_raw)| replay_data::rows::ScriptVariableEntry {
                name: name.to_vec(),
                value_raw,
            })
            .collect(),
        };
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(vec![row.clone()])),
            &pack,
            None,
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 4);
        assert_eq!(report.compared_tables.get("ScriptVariables"), Some(&1));
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("concrete stream behavior for interior NUL is unproved")
        }));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("reserved random metadata name"))
        );
        row.entries.reverse();
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(vec![row.clone()])),
            &pack,
            None,
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        row.entries.last_mut().unwrap().value_raw -= 1;
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(vec![row])),
            &pack,
            None,
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        let mut unavailable = frame(table, TableRows::empty(table));
        *unavailable.tables[table.index()] = TableView::Unavailable {
            reason: "native variable source failed".into(),
        };
        assert!(
            compare_variables(
                &unavailable,
                &pack,
                None,
                &links,
                SCOPES,
                &mut Report::default()
            )
            .is_err()
        );
    }

    #[test]
    fn variable_seeds_compare_all_scope_kinds_and_name_missing_operands() {
        let items = content_reader_parser::parser::parse(
            br#"variables={random={11 2147483648}}
            countries={GER={variables={random={12 99}}}}
            states={1={variables={random={13 0}}}}
            character_manager={historical={character={
                id={id=44 type=73} token=a variables={random={14 4294967295}}
            }}}"#,
        )
        .unwrap()
        .items;
        let pack = saved::parse(&items).unwrap();
        let characters = saved_characters::parse(&items).unwrap();
        let manager = characters.manager.as_ref().unwrap();
        let ger = germany();
        let links = BTreeMap::from([(101, &ger)]);
        let table = TableId::ScriptVariables;
        let mut rows = [
            (SCOPES.global, 0, 0x8000_0000),
            (SCOPES.country, 101, 99),
            (SCOPES.state, 1, 0),
            (SCOPES.character, 44, u32::MAX),
        ]
        .into_iter()
        .map(
            |(scope_kind, scope_id, seed)| replay_data::rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                seed: Recorded::Value(Some(seed)),
                entries: Vec::new(),
            },
        )
        .collect::<Vec<_>>();
        let compare = |rows: Vec<replay_data::rows::ScriptVariablesRow>| {
            let mut report = Report::default();
            compare_variables(
                &frame(table, TableRows::ScriptVariables(rows)),
                &pack,
                Some(manager),
                &links,
                SCOPES,
                &mut report,
            )
            .unwrap();
            report
        };
        let report = compare(rows.clone());
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.scalar_checks, 4);
        rows[2].seed = Recorded::Value(Some(1));
        let report = compare(rows.clone());
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("random seed"));
        for row in &mut rows {
            row.seed = Recorded::Value(None);
        }
        let report = compare(rows.clone());
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 0);
        let missing = report
            .limitations
            .iter()
            .find(|(reason, _)| reason.starts_with("the scoped-variable RNG seed is unavailable"))
            .unwrap()
            .1;
        assert_eq!(missing.len(), 4);
        for row in &mut rows {
            row.seed = Recorded::NotRecorded;
        }
        let report = compare(rows.clone());
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 0);
        assert_eq!(
            report.limitations["ScriptVariables.seed not recorded"].len(),
            4
        );
        assert_eq!(report.compared_tables["ScriptVariables"], 4);
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("unavailable from native capture"))
        );
        for row in &mut rows {
            row.seed = Recorded::Value(Some(1));
        }
        let omitted =
            parsed(b"variables={} countries={GER={variables={}}} states={1={variables={}}}");
        let mut omitted_manager = manager.clone();
        for character in omitted_manager.characters.values_mut() {
            character.variables = None;
        }
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(rows)),
            &omitted,
            Some(&omitted_manager),
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 0);
        let missing = report
            .limitations
            .iter()
            .find(|(reason, _)| {
                reason.contains("omits random metadata for an untouched seed state")
            })
            .unwrap()
            .1;
        assert_eq!(missing.len(), 4);
    }

    #[test]
    fn absent_variable_payloads_do_not_certify_empty_containers() {
        let table = TableId::ScriptVariables;
        let rows = [(SCOPES.global, 0), (SCOPES.country, 101), (SCOPES.state, 7)]
            .into_iter()
            .map(
                |(scope_kind, scope_id)| replay_data::rows::ScriptVariablesRow {
                    scope_kind,
                    scope_id,
                    seed: Recorded::Value(None),
                    entries: vec![],
                },
            )
            .collect();
        let recorded = frame(table, TableRows::ScriptVariables(rows));
        let ger = germany();
        let links = BTreeMap::from([(101, &ger)]);
        let absent = parsed(b"countries={GER={}}");
        let mut report = Report::default();
        compare_variables(&recorded, &absent, None, &links, SCOPES, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptVariables"), Some(&0));
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("serialized variables payload is absent at its proved scope")
        }));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("serialized states root is absent"))
        );

        let empty =
            parsed(b"variables={} countries={GER={variables={}}} states={7={variables={}}}");
        let mut report = Report::default();
        compare_variables(&recorded, &empty, None, &links, SCOPES, &mut report).unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.compared_tables.get("ScriptVariables"), Some(&3));
    }

    #[test]
    fn variable_scope_kinds_separate_global_country_and_full_width_state_ids() {
        let pack = parsed(b"variables={same=-0.00001} countries={GER={variables={same=0.00002}}} states={0={variables={same=90071992547.40993}} 4294967295={variables={same=-90071992547.40993}} 7={variables={}}}");
        let ger = germany();
        let links = BTreeMap::from([(101, &ger), (102, &ger)]);
        let table = TableId::ScriptVariables;
        let mut rows = [
            (SCOPES.global, 0, Some(-1)),
            (SCOPES.country, 102, Some(2)),
            (SCOPES.state, 0, Some(9_007_199_254_740_993)),
            (SCOPES.state, u32::MAX, Some(-9_007_199_254_740_993)),
            (SCOPES.state, 7, None),
        ]
        .into_iter()
        .map(
            |(scope_kind, scope_id, value)| replay_data::rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                seed: Recorded::Value(None),
                entries: value
                    .into_iter()
                    .map(|value_raw| replay_data::rows::ScriptVariableEntry {
                        name: b"same".to_vec(),
                        value_raw,
                    })
                    .collect(),
            },
        )
        .collect::<Vec<_>>();
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(rows.clone())),
            &pack,
            None,
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        assert_eq!(report.scalar_checks, 4);
        assert_eq!(report.compared_tables.get("ScriptVariables"), Some(&5));
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("save-root correspondence has not been proved"))
        );

        rows[2].entries[0].value_raw -= 1;
        rows[3].entries[0].value_raw += 1;
        rows.pop();
        let mut report = Report::default();
        compare_variables(
            &frame(table, TableRows::ScriptVariables(rows.clone())),
            &pack,
            None,
            &links,
            SCOPES,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 3);

        rows[0].scope_id = 1;
        assert!(
            compare_variables(
                &frame(table, TableRows::ScriptVariables(rows.clone())),
                &pack,
                None,
                &links,
                SCOPES,
                &mut Report::default()
            )
            .is_err()
        );
        rows[0].scope_id = 0;
        rows[3].scope_id = 123;
        assert!(
            compare_variables(
                &frame(table, TableRows::ScriptVariables(rows)),
                &pack,
                None,
                &links,
                SCOPES,
                &mut Report::default()
            )
            .is_err()
        );
    }

    const OPERATION_SAVE: &[u8] = br#"countries={
        GER={operations={
            finished={} priority=1
            running={
                operation_rescue_operative={
                    id={ id=69023 type=4713 }
                    date="1942.12.6.10" duration=68 target="SOV"
                    operative_slots={
                        { operative={ id=9831 type=4713 } mission={ mission=build_intel_network target_country="SOV" target_state=219 } }
                    }
                    phases={ "infiltration_border" "free_operative_liberate_camp" }
                    resources={ infantry_equipment=180 }
                    equipment={ allow_zero_entries=no }
                }
            }
        }}
        SOV={operations={ finished={} priority=1 }}
    }"#;

    #[test]
    fn buffered_targets_compare_saved_capitals_in_every_bucket_and_reject_mutations() {
        use replay_data::{rows::AgencyBufferedOperation, rows::AgencyOperationBuffersRow};
        let pack = parsed(OPERATION_SAVE);
        let governance = content_reader_save::governance::Pack {
            countries: [(
                b"SOV".to_vec(),
                content_reader_save::governance::Country {
                    capital: Some(219),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let states = BTreeSet::from([219, 777]);
        let entry = AgencyBufferedOperation {
            operation: operation_row().id,
            state: Some(219),
        };
        let buffers = AgencyOperationBuffersRow {
            completed: vec![entry.clone()],
            running: vec![entry.clone()],
            ready: vec![entry.clone()],
            unready: vec![entry],
            ..Default::default()
        };
        let run = |buffers| {
            let mut report = Report::default();
            compare_operation_buffers(
                &frame(
                    TableId::AgencyOperationBuffers,
                    TableRows::AgencyOperationBuffers(vec![buffers]),
                ),
                &pack,
                &governance,
                &states,
                Err("no province context needed for a capital"),
                &mut report,
            )
            .unwrap();
            report
        };
        let report = run(buffers.clone());
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report.compared_tables["AgencyOperationBuffers resolved targets"],
            4
        );
        for bucket in 0..4 {
            for wrong in [Some(777), Some(0), None] {
                let mut changed = buffers.clone();
                [
                    &mut changed.completed,
                    &mut changed.running,
                    &mut changed.ready,
                    &mut changed.unready,
                ][bucket][0]
                    .state = wrong;
                let report = run(changed);
                assert_eq!(
                    report.differences.len(),
                    1,
                    "bucket {bucket}, {wrong:?}: {:?}",
                    report.differences
                );
            }
        }
        let report = run(AgencyOperationBuffersRow {
            running: vec![AgencyBufferedOperation {
                operation: 0,
                state: None,
            }],
            ..Default::default()
        });
        assert!(report.differences.is_empty());
        assert_eq!(
            report.compared_tables["AgencyOperationBuffers resolved targets"],
            0
        );
    }

    #[test]
    fn buffered_province_targets_take_precedence_and_capital_absence_means_no_state() {
        use content_reader_save::{PartialMapMembership, ProvinceMembership};
        use replay_data::{rows::AgencyBufferedOperation, rows::AgencyOperationBuffersRow};
        let mut pack = parsed(OPERATION_SAVE);
        let operation = pack
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .operations
            .as_mut()
            .unwrap()
            .running
            .as_mut()
            .unwrap()
            .first_mut()
            .unwrap();
        operation.target_province = Some(333);
        let governance = content_reader_save::governance::Pack {
            countries: [(
                b"SOV".to_vec(),
                content_reader_save::governance::Country {
                    capital: Some(219),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let membership = PartialMapMembership {
            provinces: [(333, ProvinceMembership::Known(Some(444)))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        let run = |pack: &saved::Pack, state, membership: &PartialMapMembership| {
            let mut report = Report::default();
            let buffers = AgencyOperationBuffersRow {
                running: vec![AgencyBufferedOperation {
                    operation: operation_row().id,
                    state,
                }],
                ..Default::default()
            };
            let result = compare_operation_buffers(
                &frame(
                    TableId::AgencyOperationBuffers,
                    TableRows::AgencyOperationBuffers(vec![buffers]),
                ),
                pack,
                &governance,
                &BTreeSet::from([444]),
                Ok(membership),
                &mut report,
            );
            (result, report)
        };
        assert!(run(&pack, Some(444), &membership).1.differences.is_empty());
        assert_eq!(run(&pack, Some(219), &membership).1.differences.len(), 1);
        let unknown = PartialMapMembership {
            provinces: [(333, ProvinceMembership::Unknown(BTreeSet::from([444])))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert!(run(&pack, Some(444), &unknown).0.is_err());
        let unassigned = PartialMapMembership {
            provinces: [(333, ProvinceMembership::Known(None))]
                .into_iter()
                .collect(),
            ..Default::default()
        };
        assert!(run(&pack, Some(0), &unassigned).1.differences.is_empty());
        assert_eq!(run(&pack, Some(444), &unassigned).1.differences.len(), 1);
        pack.countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .operations
            .as_mut()
            .unwrap()
            .running
            .as_mut()
            .unwrap()[0]
            .target_province = None;
        assert!(run(&pack, Some(0), &membership).1.differences.is_empty());
        assert_eq!(run(&pack, Some(219), &membership).1.differences.len(), 1);
    }

    fn operation_tokens() -> TokenNames<'static> {
        [
            (7_u32, b"operation_rescue_operative".as_slice()),
            (21, b"infiltration_border".as_slice()),
            (22, b"free_operative_liberate_camp".as_slice()),
            (31, b"infantry_equipment".as_slice()),
        ]
        .into_iter()
        .collect()
    }

    fn operation_links() -> (CountryRow, CountryRow, BTreeMap<u32, CountryRow>) {
        let ger = CountryRow {
            id: 101,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let sov = CountryRow {
            id: 102,
            canonical_slot: 8,
            tag: b"SOV".to_vec(),
            ..Default::default()
        };
        let map = BTreeMap::from([(101, ger.clone()), (102, sov.clone())]);
        (ger, sov, map)
    }

    fn operation_row() -> replay_data::rows::OperationRow {
        replay_data::rows::OperationRow {
            id: (69023 << 32) | 4713,
            owner: 101,
            target: 102,
            target_province: None,
            definition_token: 7,
            preparation_current: 0,
            preparation_required: 0,
            flag_40: 0,
            auto_repeat: 0,
            terminal: 0,
            bonus: Recorded::Value(0),
            detected: Recorded::Value(0),
            effect_description: Recorded::Value(vec![]),
            start_hour: GameDate::parse(b"1942.12.6.10")
                .unwrap()
                .engine_hour()
                .unwrap(),
            duration_days: 68,
            phase_count: 2,
            assigned_slots: 1,
            phases: vec![21, 22],
            requirements: vec![replay_data::rows::OperationRequirement {
                amount: 18_000_000,
                archetype: 31,
                equipment: 1,
                ..Default::default()
            }],
            equipment: Vec::new(),
            equipment_totals: Vec::new(),
            equipment_allow_zero: 0,
        }
    }

    fn operation_frame(rows: Vec<replay_data::rows::OperationRow>) -> DecodedFrame {
        frame(TableId::Operations, TableRows::Operations(rows))
    }

    #[test]
    fn operation_outcome_oracle_compares_flags_and_native_description_save_transform() {
        let (_, _, countries) = operation_links();
        let owned = countries
            .iter()
            .map(|(&id, country)| (id, country))
            .collect();
        let tokens = BTreeMap::from([(7, b"mod_operation".as_slice())]);
        let save = parsed(br#"countries={GER={operations={running={mod_operation={id={id=69023 type=4713} target=SOV equipment={allow_zero_entries=no} completed=yes outcome_extra=yes risk_extra=yes looted="line\\n\"quote\"\\literal"}}}} SOV={}}"#);
        let mut row = operation_row();
        row.start_hour = 0x29c77f8;
        row.duration_days = 0;
        row.phase_count = 0;
        row.assigned_slots = 0;
        row.phases.clear();
        row.requirements.clear();
        row.terminal = 1;
        row.bonus = Recorded::Value(1);
        row.detected = Recorded::Value(1);
        row.effect_description =
            Recorded::Value(b"line\r\n\"quote\"\\literal\0unserialized suffix".to_vec());
        let run = |row| {
            let mut report = Report::default();
            compare_operations(
                &operation_frame(vec![row]),
                &save,
                &owned,
                &tokens,
                &mut report,
            )
            .unwrap();
            report
        };
        assert!(run(row.clone()).differences.is_empty());
        let mut no_bonus = row.clone();
        no_bonus.bonus = Recorded::Value(0);
        assert_eq!(run(no_bonus).differences.len(), 1);
        let mut no_detection = row.clone();
        no_detection.detected = Recorded::Value(0);
        assert_eq!(run(no_detection).differences.len(), 1);
        let mut no_terminal = row.clone();
        no_terminal.terminal = 0;
        assert_eq!(run(no_terminal).differences.len(), 1);
        let mut wrong_description = row.clone();
        wrong_description
            .effect_description
            .require_mut("test")
            .unwrap()[0] = b'L';
        assert_eq!(run(wrong_description).differences.len(), 1);
        let empty_description = parsed(br#"countries={GER={operations={running={mod_operation={id={id=69023 type=4713} target=SOV equipment={allow_zero_entries=no} completed=yes looted=""}}}} SOV={}}"#);
        row.bonus = Recorded::Value(0);
        row.detected = Recorded::Value(0);
        row.effect_description.require_mut("test").unwrap().clear();
        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![row.clone()]),
            &empty_description,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
        let missing_description = parsed(br#"countries={GER={operations={running={mod_operation={id={id=69023 type=4713} target=SOV equipment={allow_zero_entries=no} completed=yes}}}} SOV={}}"#);
        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![row.clone()]),
            &missing_description,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("omitted its description"));
        let byte_description = parsed(b"countries={GER={operations={running={mod_operation={id={id=69023 type=4713} target=SOV equipment={allow_zero_entries=no} completed=yes looted=\"\xfe\"}}}} SOV={}}");
        row.effect_description = Recorded::Value(vec![0xfe]);
        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![row]),
            &byte_description,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty());
    }

    #[test]
    fn operations_present_and_omitted_compare_with_gated_emission() {
        let (_, _, countries) = operation_links();
        let owned: BTreeMap<u32, &CountryRow> =
            countries.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(OPERATION_SAVE);
        let tokens = operation_tokens();
        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![operation_row()]),
            &pack,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["Operations"], 1);
        for text in [
            "operation-manager priority",
            "no proved save literal",
            "lexical dictionary",
        ] {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains(text)),
                "{text}: {:?}",
                report.limitations.keys()
            );
        }
        assert!(
            !report
                .limitations
                .values()
                .flatten()
                .any(|field| field.contains("target_province")),
            "{:?}",
            report.limitations
        );

        type Change = fn(&mut replay_data::rows::OperationRow);
        let mutate: [(&str, Change); 8] = [
            ("owner", |row| row.owner = 102),
            ("target", |row| row.target = 101),
            ("target_province", |row| row.target_province = Some(219)),
            ("definition", |row| row.definition_token = 8),
            ("start_hour", |row| row.start_hour += 1),
            ("duration", |row| row.duration_days += 1),
            ("phase_count", |row| row.phase_count += 1),
            ("assigned_slots", |row| row.assigned_slots += 1),
        ];
        for (name, change) in mutate {
            let mut row = operation_row();
            let tokens = if name == "definition" {
                [(8_u32, b"other_operation".as_slice())]
                    .into_iter()
                    .collect()
            } else {
                tokens.clone()
            };
            change(&mut row);
            let mut report = Report::default();
            compare_operations(
                &operation_frame(vec![row]),
                &pack,
                &owned,
                &tokens,
                &mut report,
            )
            .unwrap();
            assert_eq!(
                report.differences.len(),
                1,
                "{name}: {:?}",
                report.differences
            );
            assert!(report.differences[0].contains(name));
        }

        let omitted = parsed(
            br#"countries={GER={operations={ running={
                operation_rescue_operative={ id={ id=69023 type=4713 } target="SOV" equipment={ allow_zero_entries=no } }
            } }} SOV={operations={ finished={} priority=1 }}}"#,
        );
        let mut row = operation_row();
        row.start_hour = 0x29c77f8;
        row.duration_days = 0;
        row.phase_count = 0;
        row.assigned_slots = 0;
        row.phases.clear();
        row.requirements.clear();
        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![row]),
            &omitted,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);

        let mut row = operation_row();
        row.start_hour = 0x29c77f8;
        row.duration_days = 0;
        row.phase_count = 0;
        row.assigned_slots = 0;
        row.phases.clear();
        row.requirements.clear();
        row.start_hour += 1;
        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![row]),
            &omitted,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("start_hour"));

        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![]),
            &pack,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        let mut row = operation_row();
        row.owner = 102;
        row.id = (99999 << 32) | 4713;
        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![row]),
            &pack,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    }

    #[test]
    fn operations_missing_tokens_limit_definition_but_still_check_counts() {
        let (_, _, countries) = operation_links();
        let owned: BTreeMap<u32, &CountryRow> =
            countries.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(OPERATION_SAVE);
        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![operation_row()]),
            &pack,
            &owned,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("lexical dictionary"))
        );
        let mut row = operation_row();
        row.duration_days += 1;
        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![row]),
            &pack,
            &owned,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    const PREPARED_SAVE: &[u8] = br#"countries={
        GER={operations={ priority=1 running={ operation_rescue_operative={
            id={ id=69023 type=4713 } target="SOV"
            civilian_factories=1.5 total=12
            phases={ "infiltration_border" "infiltration_border" }
            resources={ infantry_equipment=180 civilian_factories={ amount=2 days=30 } }
            equipment={ allow_zero_entries=no equipment={ id={ id=12 type=70 } amount=100 } }
        } } }}
        SOV={}
    }"#;

    fn prepared_row() -> replay_data::rows::OperationRow {
        replay_data::rows::OperationRow {
            start_hour: 0x29c77f8,
            duration_days: 0,
            assigned_slots: 0,
            preparation_current: 150_000,
            preparation_required: 1_200_000,
            phases: vec![21, 21],
            requirements: vec![
                replay_data::rows::OperationRequirement {
                    amount: 18_000_000,
                    days: 5,
                    archetype: 31,
                    equipment: 1,
                    uses_time: 1,
                },
                replay_data::rows::OperationRequirement {
                    amount: 299_999,
                    days: 30,
                    ..Default::default()
                },
            ],
            equipment: vec![
                replay_data::rows::OperationEquipment {
                    variant: (12 << 32) | 70,
                    archetype: 31,
                    amount: 10_000_000,
                },
                replay_data::rows::OperationEquipment {
                    variant: (13 << 32) | 70,
                    archetype: 31,
                    amount: 0,
                },
            ],
            equipment_totals: vec![replay_data::rows::OperationEquipmentTotal {
                archetype: 31,
                amount: 10_000_000,
            }],
            ..operation_row()
        }
    }

    fn compare_prepared(row: replay_data::rows::OperationRow, save: &[u8]) -> Report {
        let (_, _, countries) = operation_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let mut report = Report::default();
        compare_operations(
            &operation_frame(vec![row]),
            &parsed(save),
            &owned,
            &operation_tokens(),
            &mut report,
        )
        .unwrap();
        report
    }

    #[test]
    fn operation_preparation_phases_resources_and_pool_compare_under_their_own_labels() {
        let report = compare_prepared(prepared_row(), PREPARED_SAVE);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report.limitations.values().flatten().any(|subject| subject
                == "Operations.equipment_totals, equipment.archetype, requirements.days (equipment requirements), requirements.uses_time: runtime-only; no text-save equality claim"),
            "{:?}",
            report.limitations
        );
        type Change = fn(&mut replay_data::rows::OperationRow);
        let changes: [(&str, Change); 14] = [
            ("civilian_factories", |row| row.preparation_current += 1),
            ("total", |row| row.preparation_required += 1),
            ("phases", |row| row.phases[1] = 22),
            ("resources.count", |row| {
                row.requirements.push(Default::default())
            }),
            ("resources[0].archetype", |row| {
                row.requirements[0].archetype = 22
            }),
            ("resources[0].amount", |row| row.requirements[0].amount += 1),
            ("resources[1].amount", |row| {
                row.requirements[1].amount = 300_000
            }),
            ("resources[1].days", |row| row.requirements[1].days = 31),
            ("resources[1].kind", |row| row.requirements[1].equipment = 1),
            ("equipment.allow_zero_entries", |row| {
                row.equipment_allow_zero = 1;
                row.equipment.pop();
            }),
            ("equipment.count", |row| {
                row.equipment.push(replay_data::rows::OperationEquipment {
                    variant: (14 << 32) | 70,
                    archetype: 31,
                    amount: 1,
                })
            }),
            ("equipment[0].variant", |row| {
                row.equipment[0].variant = (13 << 32) | 70
            }),
            ("equipment[0].amount", |row| row.equipment[0].amount += 1),
            ("phase_count", |row| row.phase_count = 3),
        ];
        let prefix = "GER.operations.running.operation_rescue_operative";
        for (name, change) in changes {
            let mut row = prepared_row();
            change(&mut row);
            let report = compare_prepared(row, PREPARED_SAVE);
            assert_eq!(
                report.differences.len(),
                1,
                "{name}: {:?}",
                report.differences
            );
            assert!(
                report.differences[0].starts_with(&format!("{prefix}.{name}:")),
                "{name}: {:?}",
                report.differences
            );
        }
        let mut unsaved = prepared_row();
        unsaved.requirements[0].days = 0;
        unsaved.requirements[0].uses_time = 0;
        unsaved.requirements[1].uses_time = 1;
        unsaved.equipment_totals.clear();
        unsaved.equipment[0].archetype = 32;
        assert!(
            compare_prepared(unsaved, PREPARED_SAVE)
                .differences
                .is_empty()
        );
        let save = std::str::from_utf8(PREPARED_SAVE).unwrap().replace(
            "equipment={ allow_zero_entries=no equipment={ id={ id=12 type=70 } amount=100 } }",
            "",
        );
        let report = compare_prepared(prepared_row(), save.as_bytes());
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with(&format!("{prefix}.equipment:")));
        let save = std::str::from_utf8(PREPARED_SAVE)
            .unwrap()
            .replace("civilian_factories=1.5 total=12", "");
        let mut row = prepared_row();
        row.preparation_current = 0;
        row.preparation_required = 0;
        assert!(
            compare_prepared(row, save.as_bytes())
                .differences
                .is_empty()
        );
        let mut row = prepared_row();
        row.phases[0] = 99;
        let report = compare_prepared(row, PREPARED_SAVE);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("operation phase definition token"))
        );
    }

    const STRATEGIC_SAVE: &str = r#"countries={GER={} SOV={}}
        strategic_operatives={ strategic_operative={
            0={ subversive_activity_level={ value=0 } danger_level={ value=0 } }
            7={
                subversive_activity_level={ value=1.23456 } danger_level={ value=-0.5 }
                trade_influence_modifier_of_buyers={ { "SOV" 2.5 } { "GER" 0 } }
                diplomatic_pressure_from_countries={ { "SOV" { opinion=32768 tension=-16384 } } }
            }
            8={ subversive_activity_level={ value=0 } danger_level={ value=0 } }
        } }"#;

    fn strategic_rows() -> Vec<replay_data::rows::StrategicOperativeRow> {
        vec![
            replay_data::rows::StrategicOperativeRow {
                country: 101,
                slot: 7,
                mission_activity: 123_456,
                operation_activity: -50_000,
                trade_influence: vec![
                    replay_data::rows::TradeInfluence {
                        owner: 102,
                        value: 250_000,
                    },
                    replay_data::rows::TradeInfluence {
                        owner: 101,
                        value: 0,
                    },
                ],
                diplomatic_pressure: vec![replay_data::rows::DiplomaticPressure {
                    owner: 102,
                    acceptance: 32_768,
                    tension: -16_384,
                }],
                ..Default::default()
            },
            replay_data::rows::StrategicOperativeRow {
                country: 102,
                slot: 8,
                ..Default::default()
            },
        ]
    }

    fn compare_strategic(
        rows: Vec<replay_data::rows::StrategicOperativeRow>,
        save: &str,
    ) -> io::Result<Report> {
        let (_, _, countries) = operation_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let mut report = Report::default();
        compare_strategic_operatives(
            &frame(
                TableId::StrategicOperatives,
                TableRows::StrategicOperatives(rows),
            ),
            &parsed(save.as_bytes()),
            &owned,
            &mut report,
        )?;
        Ok(report)
    }

    #[test]
    fn strategic_operatives_compare_activity_and_influence_under_their_own_labels() {
        let report = compare_strategic(strategic_rows(), STRATEGIC_SAVE).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["StrategicOperatives"], 2);
        assert!(
            report.limitations.values().flatten().any(|subject| subject
                == "StrategicOperatives: 1 saved slots without a recorded country"),
            "{:?}",
            report.limitations
        );
        type Change = fn(&mut [replay_data::rows::StrategicOperativeRow]);
        let changes: [(&str, Change); 9] = [
            ("subversive_activity_level", |rows| {
                rows[0].mission_activity += 1
            }),
            ("danger_level", |rows| rows[0].operation_activity += 1),
            ("trade_influence.count", |rows| {
                rows[0].trade_influence.push(Default::default())
            }),
            ("trade_influence[0].country", |rows| {
                rows[0].trade_influence[0].owner = 101
            }),
            ("trade_influence[1].amount", |rows| {
                rows[0].trade_influence[1].value = 1
            }),
            ("diplomatic_pressure.count", |rows| {
                rows[0].diplomatic_pressure.clear()
            }),
            ("diplomatic_pressure[0].country", |rows| {
                rows[0].diplomatic_pressure[0].owner = 101
            }),
            ("diplomatic_pressure[0].opinion", |rows| {
                rows[0].diplomatic_pressure[0].acceptance += 1
            }),
            ("diplomatic_pressure[0].tension", |rows| {
                rows[0].diplomatic_pressure[0].tension += 1
            }),
        ];
        for (name, change) in changes {
            let mut rows = strategic_rows();
            change(&mut rows);
            let report = compare_strategic(rows, STRATEGIC_SAVE).unwrap();
            assert_eq!(
                report.differences.len(),
                1,
                "{name}: {:?}",
                report.differences
            );
            assert!(
                report.differences[0].starts_with(&format!("GER.strategic_operative.{name}:")),
                "{name}: {:?}",
                report.differences
            );
        }
        let report = compare_strategic(
            strategic_rows(),
            &STRATEGIC_SAVE.replace(
                "8={ subversive",
                "8={ trade_influence_modifier_of_buyers={ { \"GER\" 1 } } subversive",
            ),
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(
            report.differences[0].starts_with("SOV.strategic_operative.trade_influence.count:")
        );
    }

    #[test]
    fn strategic_operatives_keep_membership_and_always_saved_activity() {
        let mut rows = strategic_rows();
        rows.pop();
        let report = compare_strategic(rows, STRATEGIC_SAVE).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("strategic_operative.8:"));
        let report = compare_strategic(
            strategic_rows(),
            &STRATEGIC_SAVE.replace(
                "8={ subversive_activity_level={ value=0 } danger_level={ value=0 } }",
                "",
            ),
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].starts_with("SOV.strategic_operative:"));
        assert!(report.differences[0].contains("absent from the save"));
        let report = compare_strategic(
            strategic_rows(),
            &STRATEGIC_SAVE.replace("8={ subversive_activity_level={ value=0 } ", "8={ "),
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(
            report.differences[0].starts_with("SOV.strategic_operative.subversive_activity_level:")
        );
        let mut rows = strategic_rows();
        rows[1].slot = 9;
        assert!(compare_strategic(rows, STRATEGIC_SAVE).is_err());
        let mut rows = strategic_rows();
        rows.push(rows[0].clone());
        assert!(compare_strategic(rows, STRATEGIC_SAVE).is_err());
    }

    fn assignment_frame(rows: Vec<replay_data::rows::OperationAssignmentRow>) -> DecodedFrame {
        frame(
            TableId::OperationAssignments,
            TableRows::OperationAssignments(rows),
        )
    }

    fn assignment_rows() -> Vec<replay_data::rows::OperationAssignmentRow> {
        vec![
            replay_data::rows::OperationAssignmentRow {
                operation: (69023 << 32) | 4713,
                ordinal: 0,
                operative: (9831 << 32) | 4713,
                selection: 0,
            },
            replay_data::rows::OperationAssignmentRow {
                operation: (69023 << 32) | 4713,
                ordinal: 1,
                operative: 0,
                selection: 0,
            },
        ]
    }

    const ASSIGNMENT_SAVE: &[u8] = br#"countries={
        GER={operations={ running={
            operation_rescue_operative={
                id={ id=69023 type=4713 } target="SOV"
                operative_slots={
                    { operative={ id=9831 type=4713 } mission={ mission=build_intel_network } }
                    { }
                }
            }
        } }}
    }"#;

    #[test]
    fn operation_assignments_join_positionally_and_keep_empty_slots() {
        let pack = parsed(ASSIGNMENT_SAVE);
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(assignment_rows()), &pack, &mut report)
            .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["OperationAssignments"], 2);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("selection-vs-mission"))
        );

        let mut rows = assignment_rows();
        rows[0].operative = 1;
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("operative"));

        let mut rows = assignment_rows();
        rows[0].ordinal = 5;
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);

        let mut rows = assignment_rows();
        rows[1].operative = (111 << 32) | 4713;
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut rows = assignment_rows();
        rows.push(replay_data::rows::OperationAssignmentRow {
            operation: (69023 << 32) | 4713,
            ordinal: 2,
            operative: 0,
            selection: 0,
        });
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut rows = assignment_rows();
        rows[1].ordinal = 2;
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);

        let mut bad = assignment_frame(assignment_rows());
        *bad.tables[TableId::OperationAssignments.index()] =
            replay_frame::frame::TableView::Unavailable {
                reason: "source failed".into(),
            };
        assert!(compare_operation_assignments(&bad, &pack, &mut Report::default()).is_err());
    }

    #[test]
    fn operation_assignments_report_retained_mission_payloads_without_equating_selection() {
        let pack = parsed(
            br#"countries={
                GER={operations={ running={
                    operation_rescue_operative={
                        id={ id=69023 type=4713 } target="SOV"
                        operative_slots={
                            { operative={ id=9831 type=4713 } mission={ mission=build_intel_network target_country="SOV" } }
                            { mission={ mission=boost_ideology target_country="FRA" target_state=16 } }
                            { mission={} }
                            { }
                        }
                    }
                } }}
            }"#,
        );
        let rows = vec![
            replay_data::rows::OperationAssignmentRow {
                operation: (69023 << 32) | 4713,
                ordinal: 0,
                operative: (9831 << 32) | 4713,
                selection: 7,
            },
            replay_data::rows::OperationAssignmentRow {
                operation: (69023 << 32) | 4713,
                ordinal: 1,
                operative: 0,
                selection: 19,
            },
            replay_data::rows::OperationAssignmentRow {
                operation: (69023 << 32) | 4713,
                ordinal: 2,
                operative: 0,
                selection: 0,
            },
            replay_data::rows::OperationAssignmentRow {
                operation: (69023 << 32) | 4713,
                ordinal: 3,
                operative: 0,
                selection: 3,
            },
        ];
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["OperationAssignments"], 4);
        let subjects = report
            .limitations
            .iter()
            .find(|(reason, _)| reason.contains("selection-vs-mission"))
            .map(|(_, subjects)| subjects)
            .expect("selection-vs-mission limitation");
        assert!(
            subjects
                .iter()
                .any(|subject| subject.contains("3 saved mission blocks (1 empty, 5 retained items)")),
            "{subjects:?}"
        );
    }

    fn operative_links() -> (CountryRow, CountryRow, BTreeMap<u32, CountryRow>) {
        let ger = CountryRow {
            id: 101,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let eng = CountryRow {
            id: 102,
            canonical_slot: 8,
            tag: b"ENG".to_vec(),
            ..Default::default()
        };
        let map = BTreeMap::from([(101, ger.clone()), (102, eng.clone())]);
        (ger, eng, map)
    }

    fn operative_tokens() -> TokenNames<'static> {
        BTreeMap::from([
            (0x1b_u32, b"name".as_slice()),
            (0x6b_u32, b"target".as_slice()),
            (0x3d12_u32, b"nationalities".as_slice()),
            (0x01b7_u32, b"state".as_slice()),
            (0x4a51_u32, b"idle".as_slice()),
            (0x4a52_u32, b"training".as_slice()),
            (0x4a53_u32, b"hiding".as_slice()),
            (0x394c_u32, b"on_mission".as_slice()),
            (0x4a50_u32, b"on_operation".as_slice()),
            (0x336c_u32, b"on_capture".as_slice()),
            (0x4b44_u32, b"synthetic_pool".as_slice()),
        ])
    }

    const OPERATIVE_SAVE: &[u8] = br#"countries={
        GER={operations={ running={ operation_test={ id={ id=69023 type=4713 } target=ENG } } } intelligence_agency={
            defense=0 max_operative_count=2 usable_operative_slots=2
            operative={ name="" id={ id=9831 type=4713 } skill=2 experience=453.456 nationalities={ ENG } state=on_operation operation={ id=69023 type=4713 } }
            recruitment={ recruitable_operatives={ { name="" id={ id=1043 type=4713 } skill=1 nationalities={ ENG } state=on_mission } } }
        }}
        ENG={intelligence_agency={ defense=0 max_operative_count=0 usable_operative_slots=0 }}
    }"#;

    fn operative_rows() -> Vec<replay_data::rows::OperativeMembershipRow> {
        vec![
            replay_data::rows::OperativeMembershipRow {
                id: (9831 << 32) | 4713,
                owner_country: 0,
                roster_country: 101,
                roster_kind: 0,
                legacy_description_length: 0,
                assignment: 4,
                experience_raw: 45_345_600,
                skill: Some(2),
                capturer_country: 0,
                captured_hour: 0,
                action_variant: None,
                name: Recorded::Value(vec![]),
                target_country: Recorded::Value(Some(102)),
                operation: Recorded::Value(Some((69023 << 32) | 4713)),
                ordinal: Recorded::Value(0),
                ..Default::default()
            },
            replay_data::rows::OperativeMembershipRow {
                id: (1043 << 32) | 4713,
                owner_country: 0,
                roster_country: 101,
                roster_kind: 2,
                legacy_description_length: 0,
                assignment: 3,
                experience_raw: 0,
                skill: Some(1),
                capturer_country: 0,
                captured_hour: 0,
                action_variant: None,
                name: Recorded::Value(vec![]),
                target_country: Recorded::Value(None),
                operation: Recorded::Value(None),
                ordinal: Recorded::Value(0),
                ..Default::default()
            },
        ]
    }

    #[test]
    fn operative_memberships_join_by_handle_roster_and_kind_group() {
        let (_, _, countries) = operative_links();
        let owned: BTreeMap<u32, &CountryRow> =
            countries.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(OPERATIVE_SAVE);
        let tokens = operative_tokens();
        let table = TableId::OperativeMemberships;
        let frame = |rows| frame(table, TableRows::OperativeMemberships(rows));
        let mut report = Report::default();
        let rows = operative_rows();
        compare_operative_memberships(&frame(rows.clone()), &pack, &owned, &tokens, &mut report)
            .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["OperativeMemberships"], 2);
        let text = "no universal detected/exposed state";
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains(text)),
            "{text}: {:?}",
            report.limitations.keys()
        );

        let mut row = rows[0].clone();
        row.skill = Some(3);
        let mut report = Report::default();
        compare_operative_memberships(
            &frame(vec![row, rows[1].clone()]),
            &pack,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("skill"));

        let mut row = rows[0].clone();
        row.experience_raw += 1;
        let mut report = Report::default();
        compare_operative_memberships(
            &frame(vec![row, rows[1].clone()]),
            &pack,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut report = Report::default();
        compare_operative_memberships(&frame(vec![]), &pack, &owned, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        let mut rows = operative_rows();
        rows.push(replay_data::rows::OperativeMembershipRow {
            id: (99999 << 32) | 4713,
            roster_country: 101,
            roster_kind: 0,
            ..Default::default()
        });
        let mut report = Report::default();
        compare_operative_memberships(&frame(rows), &pack, &owned, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut rows = operative_rows();
        rows[1].roster_kind = 1;
        let mut report = Report::default();
        compare_operative_memberships(&frame(rows), &pack, &owned, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);

        let mut rows = operative_rows();
        rows.push(rows[0].clone());
        assert!(
            compare_operative_memberships(
                &frame(rows),
                &pack,
                &owned,
                &tokens,
                &mut Report::default()
            )
            .is_err()
        );
        let mut bad = frame(operative_rows());
        *bad.tables[table.index()] = replay_frame::frame::TableView::Unavailable {
            reason: "source failed".into(),
        };
        assert!(
            compare_operative_memberships(&bad, &pack, &owned, &tokens, &mut Report::default())
                .is_err()
        );
    }

    #[test]
    fn operative_experience_omission_proves_exact_zero_independently_of_explicit_zero() {
        let (_, _, countries) = operative_links();
        let owned: BTreeMap<u32, &CountryRow> =
            countries.iter().map(|(id, row)| (*id, row)).collect();
        let tokens = operative_tokens();
        let run = |saved_experience: &str, recorded_experience: i64| {
            let source = format!(
                r#"countries={{ GER={{ intelligence_agency={{
                    defense=0 max_operative_count=1 usable_operative_slots=1
                    operative={{ name="" id={{ id=1043 type=4713 }} skill=1 {saved_experience} nationalities={{ ENG }} state=idle }}
                }} }} ENG={{}} }}"#
            );
            let pack = parsed(source.as_bytes());
            let mut row = operative_rows().remove(1);
            row.roster_kind = 0;
            row.assignment = 0;
            row.experience_raw = recorded_experience;
            let captured = frame(
                TableId::OperativeMemberships,
                TableRows::OperativeMemberships(vec![row]),
            );
            let mut report = Report::default();
            compare_operative_memberships(&captured, &pack, &owned, &tokens, &mut report).unwrap();
            report
        };

        let absent_zero = run("", 0);
        assert!(
            absent_zero.differences.is_empty(),
            "{:?}",
            absent_zero.differences
        );
        assert!(
            !absent_zero
                .limitations
                .keys()
                .any(|reason| reason.contains("experience")),
            "{:?}",
            absent_zero.limitations
        );
        assert!(
            !absent_zero
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.contains("OperativeMemberships.experience")),
            "{:?}",
            absent_zero.limitations
        );
        for wrong in [1, -1] {
            let report = run("", wrong);
            assert_eq!(report.differences.len(), 1, "{wrong}: {:?}", report.differences);
            assert!(
                report.differences[0].contains("experience"),
                "{wrong}: {:?}",
                report.differences
            );
        }

        let explicit_zero = run("experience=0", 0);
        assert!(
            explicit_zero.differences.is_empty(),
            "{:?}",
            explicit_zero.differences
        );
        for wrong in [1, -1] {
            let report = run("experience=0", wrong);
            assert_eq!(report.differences.len(), 1, "{wrong}: {:?}", report.differences);
            assert!(
                report.differences[0].contains("experience"),
                "{wrong}: {:?}",
                report.differences
            );
        }

        let explicit_negative = run("experience=-0.00001", -1);
        assert!(
            explicit_negative.differences.is_empty(),
            "{:?}",
            explicit_negative.differences
        );
        let negative_mismatch = run("experience=-0.00001", 0);
        assert_eq!(
            negative_mismatch.differences.len(),
            1,
            "{:?}",
            negative_mismatch.differences
        );
        assert!(negative_mismatch.differences[0].contains("experience"));
    }

    #[test]
    fn operative_placements_compare_all_mission_definitions_names_targets_and_own_references() {
        let (_, _, countries) = operative_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let definitions: [&[u8]; 8] = [b"m1", b"m2", b"m3", b"m4", b"m5", b"m6", b"m7", b"m8"];
        let mut tokens = operative_tokens();
        for (id, name) in engine_native_intelligence::operative_action::SAVE_TOKENS
            .into_iter()
            .zip(definitions)
        {
            tokens.insert(id, name);
        }
        for (index, definition) in definitions.into_iter().enumerate() {
            let source = format!(
                r#"countries={{ GER={{ intelligence_agency={{
                defense=0 max_operative_count=1 usable_operative_slots=1
                operative={{ id={{id=9831 type=4713}} name="Renamed literal" skill=2 experience=453.456
                    state=on_mission operation={{id=69023 type=4713}} mission={{{}={{target=ENG state=999}}}} }}
            }} }} ENG={{}} }}"#,
                text(definition)
            );
            let pack = parsed(source.as_bytes());
            let mut row = operative_rows().remove(0);
            row.name = Recorded::Value(b"Renamed literal".to_vec());
            row.assignment = 3;
            row.action_variant = Some(index as u32 + 1);
            let make_frame = |row| {
                frame(
                    TableId::OperativeMemberships,
                    replay_data::rows::TableRows::OperativeMemberships(vec![row]),
                )
            };
            let mut report = Report::default();
            compare_operative_memberships(
                &make_frame(row.clone()),
                &pack,
                &owned,
                &tokens,
                &mut report,
            )
            .unwrap();
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            for field in ["name", "target_country", "operation", "capturer"] {
                let mut changed = row.clone();
                match field {
                    "name" => changed.name = Recorded::Value(b"Renamed literal!".to_vec()),
                    "target_country" => changed.target_country = Recorded::Value(Some(101)),
                    "operation" => changed.operation = Recorded::Value(None),
                    "capturer" => changed.capturer_country = 101,
                    _ => unreachable!(),
                }
                let mut report = Report::default();
                compare_operative_memberships(
                    &make_frame(changed),
                    &pack,
                    &owned,
                    &tokens,
                    &mut report,
                )
                .unwrap();
                assert_eq!(
                    report.differences.len(),
                    1,
                    "{field}: {:?}",
                    report.differences
                );
            }
            let absent = parsed(source.replace("target=ENG", "").as_bytes());
            let mut no_target = row.clone();
            no_target.target_country = Recorded::Value(None);
            let mut report = Report::default();
            compare_operative_memberships(
                &make_frame(no_target),
                &absent,
                &owned,
                &tokens,
                &mut report,
            )
            .unwrap();
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            row.action_variant = Some((index as u32 + 1) % 8 + 1);
            let mut report = Report::default();
            compare_operative_memberships(&make_frame(row), &pack, &owned, &tokens, &mut report)
                .unwrap();
            assert_eq!(report.differences.len(), 1);
            assert!(report.differences[0].contains("mission_name"));
        }
        let pack = parsed(OPERATIVE_SAVE);
        let mut rows = operative_rows();
        rows[0].target_country = Recorded::Value(Some(101));
        let mut report = Report::default();
        compare_operative_memberships(
            &frame(
                TableId::OperativeMemberships,
                replay_data::rows::TableRows::OperativeMemberships(rows),
            ),
            &pack,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("target_country"));
    }

    const MISSION_SAVE: &str = r#"countries={
        GER={intelligence_agency={
            defense=0 max_operative_count=2 usable_operative_slots=2
            operative={ name="" id={ id=9831 type=4713 } gfx="GFX_spy" state=on_mission mission={ m5={ target=ENG state=219 ideology=fascism } } }
            operative={ name="" id={ id=1043 type=4713 } state=training cooldown_reason=harmed leader_cooldown_start_date=1938.4.3.7 leader_modifier_enable_date=1938.4.8.19 }
        }}
        ENG={intelligence_agency={ defense=0 max_operative_count=0 usable_operative_slots=0 }}
    }"#;

    fn mission_tokens() -> TokenNames<'static> {
        let mut tokens = operative_tokens();
        let definitions: [&[u8]; 8] = [b"m1", b"m2", b"m3", b"m4", b"m5", b"m6", b"m7", b"m8"];
        for (id, name) in engine_native_intelligence::operative_action::SAVE_TOKENS
            .into_iter()
            .zip(definitions)
        {
            tokens.insert(id, name);
        }
        tokens
    }

    fn mission_rows(pack: &saved::Pack) -> Vec<replay_data::rows::OperativeMissionRow> {
        let saved = &pack.countries[b"GER".as_slice()]
            .agency
            .as_ref()
            .unwrap()
            .operatives[1];
        vec![
            replay_data::rows::OperativeMissionRow {
                roster_country: 101,
                id: (9831 << 32) | 4713,
                ordinal: 0,
                assignment: 3,
                variant: Some(engine_native_intelligence::operative_action::BOOST_IDEOLOGY),
                target_country: Some(102),
                target_state: Some(219),
                ideology: b"fascism".to_vec(),
                pending_variant: Some(engine_native_intelligence::operative_action::BUILD_NETWORK),
                pending_country: Some(101),
                pending_state: Some(7),
                portrait: b"GFX_spy".to_vec(),
                ..Default::default()
            },
            replay_data::rows::OperativeMissionRow {
                roster_country: 101,
                id: (1043 << 32) | 4713,
                ordinal: 1,
                assignment: engine_native_intelligence::coperative_leader::COOLDOWN_ASSIGNMENT,
                cooldown_kind: Some(2),
                cooldown_start_hour: saved_hour(saved.cooldown_start_date).unwrap(),
                cooldown_end_hour: saved_hour(saved.cooldown_end_date).unwrap(),
                ..Default::default()
            },
        ]
    }

    fn compare_missions(
        rows: Vec<replay_data::rows::OperativeMissionRow>,
        pack: &saved::Pack,
    ) -> io::Result<Report> {
        let (_, _, countries) = operative_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let mut report = Report::default();
        compare_operative_missions(
            &frame(
                TableId::OperativeMissions,
                TableRows::OperativeMissions(rows),
            ),
            pack,
            &owned,
            &mission_tokens(),
            &mut report,
        )?;
        Ok(report)
    }

    #[test]
    fn operative_missions_compare_every_saved_field_under_its_own_label() {
        let pack = parsed(MISSION_SAVE.as_bytes());
        let rows = mission_rows(&pack);
        let report = compare_missions(rows.clone(), &pack).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["OperativeMissions"], 2);
        let subject = "OperativeMissions.pending_variant / pending_country / pending_state / pending_ideology";
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|entry| entry == subject),
            "{subject}: {:?}",
            report.limitations
        );
        type Change = fn(&mut [replay_data::rows::OperativeMissionRow]);
        let changes: [(usize, &str, Change); 13] = [
            (0, "gfx", |rows| rows[0].portrait = b"GFX_other".to_vec()),
            (1, "gfx", |rows| rows[1].portrait = b"GFX_other".to_vec()),
            (0, "ordinal", |rows| rows[0].ordinal = 1),
            (0, "assignment", |rows| rows[0].assignment = 4),
            (1, "cooldown_reason", |rows| rows[1].cooldown_kind = Some(3)),
            (1, "leader_cooldown_start_date", |rows| {
                rows[1].cooldown_start_hour = rows[1].cooldown_start_hour.map(|hour| hour + 1)
            }),
            (1, "leader_modifier_enable_date", |rows| {
                rows[1].cooldown_end_hour = rows[1].cooldown_end_hour.map(|hour| hour + 1)
            }),
            (0, "mission_present", |rows| rows[0].variant = None),
            (0, "mission_name", |rows| {
                rows[0].variant = Some(engine_native_intelligence::operative_action::PROPAGANDA)
            }),
            (0, "mission_target", |rows| {
                rows[0].target_country = Some(101)
            }),
            (0, "mission_target", |rows| {
                rows[0].target_country = Some(0x8000_0000)
            }),
            (0, "mission_state", |rows| rows[0].target_state = None),
            (0, "mission_ideology", |rows| {
                rows[0].ideology = b"communism".to_vec()
            }),
        ];
        for (index, field, change) in changes {
            let label = format!("GER.operative.{}.{field}:", rows[index].id);
            let mut changed = rows.clone();
            change(&mut changed);
            let report = compare_missions(changed, &pack).unwrap();
            assert_eq!(
                report.differences.len(),
                1,
                "{label} {:?}",
                report.differences
            );
            assert!(
                report.differences[0].starts_with(&label),
                "{label} {:?}",
                report.differences
            );
        }
        let mut unsaved = rows;
        unsaved[0].pending_variant = None;
        unsaved[0].pending_state = Some(8);
        unsaved[1].pending_ideology = b"democratic".to_vec();
        assert!(
            compare_missions(unsaved, &pack)
                .unwrap()
                .differences
                .is_empty()
        );
    }

    #[test]
    fn root_out_operands_are_reported_runtime_only_and_never_differ() {
        let pack = parsed(
            MISSION_SAVE
                .replace(
                    "m5={ target=ENG state=219 ideology=fascism }",
                    "m4={ target=ENG state=219 }",
                )
                .as_bytes(),
        );
        let mut rows = mission_rows(&pack);
        rows[0].variant = Some(engine_native_intelligence::operative_action::ROOT_OUT_RESISTANCE);
        rows[0].ideology = Vec::new();
        for (country, operative, nationality) in [
            (Some(25_000), Some(-7_000), Some(1)),
            (Some(0), Some(0), Some(0)),
        ] {
            rows[0].root_out_country_factor = country;
            rows[0].root_out_operative_factor = operative;
            rows[0].root_out_nationality = nationality;
            let report = compare_missions(rows.clone(), &pack).unwrap();
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            assert!(
                report.limitations.values().flatten().any(|subject| subject
                    == "OperativeMissions.root_out_country_factor / root_out_operative_factor / root_out_nationality: runtime-only; no text-save equality claim"),
                "{:?}",
                report.limitations
            );
        }
    }

    #[test]
    fn operative_missions_keep_save_omissions_and_roster_membership() {
        let source = MISSION_SAVE;
        let omitted = parsed(
            source
                .replace("target=ENG state=219 ideology=fascism", "")
                .as_bytes(),
        );
        let mut rows = mission_rows(&omitted);
        rows[0].target_country = Some(0);
        rows[0].target_state = None;
        rows[0].ideology = Vec::new();
        assert!(
            compare_missions(rows.clone(), &omitted)
                .unwrap()
                .differences
                .is_empty()
        );
        rows[0].target_country = Some(u32::MAX);
        assert!(
            compare_missions(rows, &omitted)
                .unwrap()
                .differences
                .is_empty()
        );
        let country = parsed(
            source
                .replace(
                    "m5={ target=ENG state=219 ideology=fascism }",
                    "m3={ target=ENG }",
                )
                .as_bytes(),
        );
        let mut rows = mission_rows(&country);
        rows[0].variant = Some(engine_native_intelligence::operative_action::COUNTER_INTELLIGENCE);
        rows[0].target_state = None;
        rows[0].ideology = Vec::new();
        assert!(
            compare_missions(rows.clone(), &country)
                .unwrap()
                .differences
                .is_empty()
        );
        rows[0].target_state = Some(219);
        let report = compare_missions(rows, &country).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains(".mission_state"));

        let pack = parsed(source.as_bytes());
        let mut rows = mission_rows(&pack);
        rows.pop();
        let report = compare_missions(rows, &pack).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(
            report.differences[0]
                .starts_with(&format!("GER.operative.{}:", (1043_u64 << 32) | 4713)),
            "{:?}",
            report.differences
        );
        let mut rows = mission_rows(&pack);
        rows.push(replay_data::rows::OperativeMissionRow {
            roster_country: 101,
            id: (77 << 32) | 4713,
            ordinal: 2,
            ..Default::default()
        });
        let report = compare_missions(rows, &pack).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("absent from the save"));
        let mut rows = mission_rows(&pack);
        rows.push(rows[0].clone());
        assert!(compare_missions(rows, &pack).is_err());
        let (_, _, countries) = operative_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let mut bad = frame(
            TableId::OperativeMissions,
            TableRows::OperativeMissions(mission_rows(&pack)),
        );
        *bad.tables[TableId::OperativeMissions.index()] = TableView::Unavailable {
            reason: "source failed".into(),
        };
        assert!(
            compare_operative_missions(
                &bad,
                &pack,
                &owned,
                &mission_tokens(),
                &mut Report::default()
            )
            .is_err()
        );
    }

    #[test]
    fn operative_pool_a_uses_recorded_spelling_and_counts_unidentified_saved_rosters() {
        let (_, _, countries) = operative_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let tokens = operative_tokens();
        let source = String::from_utf8_lossy(OPERATIVE_SAVE);
        let pack = parsed(
            source
                .replace("recruitable_operatives=", "synthetic_pool=")
                .as_bytes(),
        );
        let mut rows = operative_rows();
        rows[1].roster_kind = 1;
        let make_frame = |rows| {
            frame(
                TableId::OperativeMemberships,
                TableRows::OperativeMemberships(rows),
            )
        };
        let mut report = Report::default();
        compare_operative_memberships(
            &make_frame(rows.clone()),
            &pack,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["OperativeMemberships"], 2);

        rows.pop();
        let mut report = Report::default();
        compare_operative_memberships(
            &make_frame(rows.clone()),
            &pack,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("recruitment"));

        let mut no_spelling = tokens.clone();
        no_spelling.remove(&0x4b44);
        let mut report = Report::default();
        compare_operative_memberships(&make_frame(rows), &pack, &owned, &no_spelling, &mut report)
            .unwrap();
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|entry| entry.contains("synthetic_pool: 1 entries")),
            "{:?}",
            report.limitations
        );
    }

    #[test]
    fn operative_omitted_name_compares_as_empty_and_stale_operation_is_a_difference() {
        let (_, _, countries) = operative_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let tokens = operative_tokens();
        let source = String::from_utf8_lossy(OPERATIVE_SAVE);
        let omitted_names = parsed(source.replace("name=\"\"", "").as_bytes());
        let make_frame = |rows| {
            frame(
                TableId::OperativeMemberships,
                TableRows::OperativeMemberships(rows),
            )
        };
        let mut report = Report::default();
        compare_operative_memberships(
            &make_frame(operative_rows()),
            &omitted_names,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("saved name is omitted"))
        );

        let mut rows = operative_rows();
        rows[0].name = Recorded::Value(b"not empty".to_vec());
        let mut report = Report::default();
        compare_operative_memberships(
            &make_frame(rows),
            &omitted_names,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains(".name"));

        for stale_save in [
            source.replace("operation={ id=69023 type=4713 }", ""),
            source.replace(
                "running={ operation_test={ id={ id=69023 type=4713 } target=ENG } }",
                "finished={ operation_test={ ENG 1 } }",
            ),
        ] {
            let mut rows = operative_rows();
            rows[1].skill = Some(9);
            let mut report = Report::default();
            compare_operative_memberships(
                &make_frame(rows),
                &parsed(stale_save.as_bytes()),
                &owned,
                &tokens,
                &mut report,
            )
            .unwrap();
            assert!(
                report
                    .differences
                    .iter()
                    .any(|difference| difference.contains(".active_operation"))
            );
            assert!(
                report
                    .differences
                    .iter()
                    .any(|difference| difference.contains(".skill"))
            );
            assert_eq!(report.compared_tables["OperativeMemberships"], 2);
        }
    }

    #[test]
    fn active_operative_ordinals_compare_save_vector_order_and_keep_old_absence() {
        let (_, _, countries) = operative_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(OPERATIVE_SAVE);
        let tokens = operative_tokens();
        let run = |ordinal| {
            let mut rows = operative_rows();
            rows[0].ordinal = ordinal;
            let frame = frame(
                TableId::OperativeMemberships,
                TableRows::OperativeMemberships(rows),
            );
            let mut report = Report::default();
            compare_operative_memberships(&frame, &pack, &owned, &tokens, &mut report).unwrap();
            report
        };
        assert!(run(Recorded::Value(0)).differences.is_empty());
        let report = run(Recorded::Value(1));
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("ordinal"));
        let report = run(Recorded::NotRecorded);
        assert!(report.differences.is_empty());
        assert_eq!(
            report.limitations["OperativeMemberships.ordinal not recorded"].len(),
            1
        );
    }

    #[test]
    fn historical_operative_ordinals_preserve_membership_payload_comparisons() {
        let (_, _, countries) = operative_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(OPERATIVE_SAVE);
        let mut rows = operative_rows();
        for row in &mut rows {
            row.ordinal = Recorded::NotRecorded;
        }
        rows[0].experience_raw += 1;
        let frame = frame(
            TableId::OperativeMemberships,
            TableRows::OperativeMemberships(rows),
        );
        let mut report = Report::default();
        compare_operative_memberships(&frame, &pack, &owned, &operative_tokens(), &mut report)
            .unwrap();
        assert_eq!(report.compared_tables["OperativeMemberships"], 2);
        assert_eq!(report.differences.len(), 1);
        assert!(report.differences[0].contains("experience"));
        assert_eq!(
            report.limitations["OperativeMemberships.ordinal not recorded"].len(),
            2
        );
    }

    #[test]
    fn repeated_saved_operatives_do_not_compare_one_row_with_multiple_ordinals() {
        let (_, _, countries) = operative_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let mut pack = parsed(OPERATIVE_SAVE);
        let agency = pack
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .agency
            .as_mut()
            .unwrap();
        agency.operatives.push(agency.operatives[0].clone());
        let mut rows = operative_rows();
        rows[0].ordinal = Recorded::Value(0);
        let captured = frame(
            TableId::OperativeMemberships,
            TableRows::OperativeMemberships(rows),
        );
        let mut report = Report::default();
        compare_operative_memberships(&captured, &pack, &owned, &operative_tokens(), &mut report)
            .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("repeated ordinal"))
        );
    }

    const CRYPTOLOGY_SAVE: &[u8] = br#"countries={
        GER={intelligence_agency={ defense=0 max_operative_count=0 usable_operative_slots=0 cryptology={ intel_source={ country="GER" pool=2 id=1 discriminant=0 } } }}
        ENG={intelligence_agency={ defense=0 max_operative_count=0 usable_operative_slots=0 cryptology={
            targets={ { target="SOV" decryption=yes amount=27625 } { target="ITA" amount=12000 date="1940.1.15.1" } }
            intel_source={ country="ENG" pool=2 id=1 discriminant=0 } } }}
        SOV={} ITA={}
    }"#;

    fn cryptology_rows() -> Vec<replay_data::rows::CryptologyRow> {
        vec![
            replay_data::rows::CryptologyRow {
                owner_country: 102,
                target_country: 201,
                amount_raw: 2_762_500_000,
                bonus_days: -1,
                bonus_active: 0,
                decrypting: 1,
                ordinal: 0.into(),
            },
            replay_data::rows::CryptologyRow {
                owner_country: 102,
                target_country: 202,
                amount_raw: 1_200_000_000,
                bonus_days: -1,
                bonus_active: 0,
                decrypting: 0,
                ordinal: 0.into(),
            },
        ]
    }

    fn cryptology_links() -> (CountryRow, BTreeMap<u32, CountryRow>) {
        let eng = CountryRow {
            id: 102,
            tag: b"ENG".to_vec(),
            ..Default::default()
        };
        let sov = CountryRow {
            id: 201,
            tag: b"SOV".to_vec(),
            ..Default::default()
        };
        let ita = CountryRow {
            id: 202,
            tag: b"ITA".to_vec(),
            ..Default::default()
        };
        let ger = CountryRow {
            id: 101,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        let map = BTreeMap::from([(101, ger), (102, eng.clone()), (201, sov), (202, ita)]);
        (eng, map)
    }

    #[test]
    fn cryptology_targets_join_by_owner_and_target_with_gated_scalars() {
        let (_, map) = cryptology_links();
        let owned: BTreeMap<u32, &CountryRow> = map.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(CRYPTOLOGY_SAVE);
        let table = TableId::Cryptology;
        let frame = |rows| frame(table, TableRows::Cryptology(rows));
        let mut report = Report::default();
        compare_cryptology(&frame(cryptology_rows()), &pack, &owned, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["Cryptology"], 2);
        for text in ["0x2A7", "intel-source", "engine-hour date"] {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains(text)),
                "{text}: {:?}",
                report.limitations.keys()
            );
        }

        let mut rows = cryptology_rows();
        rows[0].amount_raw += 1;
        let mut report = Report::default();
        compare_cryptology(&frame(rows), &pack, &owned, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut rows = cryptology_rows();
        rows[0].decrypting = 0;
        let mut report = Report::default();
        compare_cryptology(&frame(rows), &pack, &owned, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut report = Report::default();
        compare_cryptology(&frame(vec![]), &pack, &owned, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        let mut rows = cryptology_rows();
        rows.push(replay_data::rows::CryptologyRow {
            owner_country: 101,
            target_country: 201,
            ..Default::default()
        });
        let mut report = Report::default();
        compare_cryptology(&frame(rows), &pack, &owned, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut bad = frame(cryptology_rows());
        *bad.tables[table.index()] = replay_frame::frame::TableView::Unavailable {
            reason: "source failed".into(),
        };
        assert!(compare_cryptology(&bad, &pack, &owned, &mut Report::default()).is_err());
    }

    const NETWORK_SAVE: &[u8] = br#"countries={ENG={} SOV={} AUS={}}
        strategic_operatives={ strategic_operative={
            2={ intel_network={ target="SOV" intel_network={ state_id={ 219 253 } gain={ 0.85 -1.75 } gain_breakdown={ { operatives=0.425 adjacencies=0.2125 } { operatives=0 adjacencies=0 } } strength={ 71 0 } sub_network_id={ 0 -1 } sub_network_is_quiet={ yes } } sub_intel_networks={ { modifiers={ intel_network_gain_factor=1 } sub_network_strength_target=57.5 sub_network_strength_target_from_operatives=15 sub_network_strength_target_from_counterintelligence=-47.5 strength=72.5 sub_network_is_quiet=yes } } } }
        }}"#;

    fn network_links() -> BTreeMap<u32, CountryRow> {
        BTreeMap::from([
            (
                101,
                CountryRow {
                    id: 101,
                    canonical_slot: 2,
                    tag: b"ENG".to_vec(),
                    ..Default::default()
                },
            ),
            (
                201,
                CountryRow {
                    id: 201,
                    tag: b"SOV".to_vec(),
                    ..Default::default()
                },
            ),
            (
                202,
                CountryRow {
                    id: 202,
                    tag: b"AUS".to_vec(),
                    ..Default::default()
                },
            ),
        ])
    }

    fn network_rows() -> Vec<replay_data::rows::IntelNetworkRow> {
        vec![
            replay_data::rows::IntelNetworkRow {
                owner_country: 101,
                target_country: 201,
                state_id: 219,
                strength_raw: Some(7_250_000),
                subnetwork_selector: 0,
                local_strength_raw: 7_100_000,
                gain_raw: 85_000,
                gain_from_operatives_raw: 42_500,
                gain_from_adjacency_raw: 21_250,
                local_quiet: 1,
                gain_factor_raw: Some(100_000),
                strength_target_raw: Some(5_750_000),
                target_from_operatives_raw: Some(1_500_000),
                target_from_counterintelligence_raw: Some(-4_750_000),
                subnetwork_quiet: 1,
            },
            replay_data::rows::IntelNetworkRow {
                owner_country: 101,
                target_country: 201,
                state_id: 253,
                strength_raw: None,
                subnetwork_selector: -1,
                local_strength_raw: 0,
                gain_raw: -175_000,
                ..Default::default()
            },
        ]
    }

    #[test]
    fn intel_networks_compare_gains_quiet_bytes_and_subnetwork_targets() {
        let map = network_links();
        let owned: BTreeMap<u32, &CountryRow> = map.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(NETWORK_SAVE);
        let tokens: TokenNames<'_> = [(0x006b_u32, b"target".as_slice())].into_iter().collect();
        let run = |rows: Vec<replay_data::rows::IntelNetworkRow>| {
            let mut report = Report::default();
            compare_intel_networks(
                &frame(TableId::IntelNetworks, TableRows::IntelNetworks(rows)),
                &pack,
                &owned,
                &tokens,
                &mut report,
            )
            .unwrap();
            report
                .differences
                .iter()
                .map(|line| line.split(':').next().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert!(run(network_rows()).is_empty());
        type Change = fn(&mut replay_data::rows::IntelNetworkRow);
        let changes: [(&str, usize, Change); 10] = [
            ("gain", 0, |row| row.gain_raw += 1),
            ("gain", 1, |row| row.gain_raw = 0),
            ("gain_from_operatives", 0, |row| {
                row.gain_from_operatives_raw = 0
            }),
            ("gain_from_adjacency", 0, |row| {
                row.gain_from_adjacency_raw = 0
            }),
            ("local_quiet", 0, |row| row.local_quiet = 0),
            ("gain_factor", 0, |row| row.gain_factor_raw = Some(50_000)),
            ("strength_target", 0, |row| row.strength_target_raw = None),
            ("target_from_operatives", 0, |row| {
                row.target_from_operatives_raw = Some(0)
            }),
            ("target_from_counterintelligence", 0, |row| {
                row.target_from_counterintelligence_raw = Some(0)
            }),
            ("subnetwork_quiet", 0, |row| row.subnetwork_quiet = 0),
        ];
        for (name, index, change) in changes {
            let mut rows = network_rows();
            change(&mut rows[index]);
            let state = rows[index].state_id;
            assert_eq!(
                run(rows),
                [format!(
                    "intel_network.owner_slot.2.SOV.state.{state}.{name}"
                )],
                "{name}"
            );
        }
    }

    #[test]
    fn intel_networks_join_positionally_with_negative_selector_normalization() {
        let map = network_links();
        let owned: BTreeMap<u32, &CountryRow> = map.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(NETWORK_SAVE);
        let tokens: TokenNames<'_> = [(0x006b_u32, b"target".as_slice())].into_iter().collect();
        let table = TableId::IntelNetworks;
        let frame = |rows| frame(table, TableRows::IntelNetworks(rows));
        let mut report = Report::default();
        compare_intel_networks(&frame(network_rows()), &pack, &owned, &tokens, &mut report)
            .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["IntelNetworks"], 2);
        assert!(
            report
                .limitations
                .keys()
                .all(|reason| !reason.contains("have no recorded fields"))
        );

        let mut rows = network_rows();
        rows[0].subnetwork_selector = 1;
        let mut report = Report::default();
        compare_intel_networks(&frame(rows), &pack, &owned, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut report = Report::default();
        compare_intel_networks(&frame(vec![]), &pack, &owned, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);

        let mut bad = frame(network_rows());
        *bad.tables[table.index()] = replay_frame::frame::TableView::Unavailable {
            reason: "source failed".into(),
        };
        assert!(
            compare_intel_networks(&bad, &pack, &owned, &tokens, &mut Report::default()).is_err()
        );
    }

    #[test]
    fn intel_networks_keep_distinct_owners_of_the_same_target_state() {
        let mut map = network_links();
        map.get_mut(&202).unwrap().canonical_slot = 3;
        let owned = map.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(br#"countries={ENG={} SOV={} AUS={}}
            strategic_operatives={ strategic_operative={
                2={ intel_network={ target="SOV" intel_network={ state_id={ 219 } gain={ 0.85 } gain_breakdown={ { operatives=0.425 adjacencies=0.2125 } } strength={ 11 } sub_network_id={ 0 } sub_network_is_quiet={ yes } } sub_intel_networks={ { modifiers={ intel_network_gain_factor=1 } sub_network_strength_target=57.5 sub_network_strength_target_from_operatives=15 sub_network_strength_target_from_counterintelligence=-47.5 strength=12 sub_network_is_quiet=yes } } } }
                3={ intel_network={ target="SOV" intel_network={ state_id={ 219 } gain={ 0.85 } gain_breakdown={ { operatives=0.425 adjacencies=0.2125 } } strength={ 21 } sub_network_id={ 0 } sub_network_is_quiet={ yes } } sub_intel_networks={ { modifiers={ intel_network_gain_factor=1 } sub_network_strength_target=57.5 sub_network_strength_target_from_operatives=15 sub_network_strength_target_from_counterintelligence=-47.5 strength=22 sub_network_is_quiet=yes } } } }
            }}"#);
        let tokens = TokenNames::new();
        let mut rows = vec![network_rows()[0].clone(); 2];
        rows[0].strength_raw = Some(1_200_000);
        rows[0].local_strength_raw = 1_100_000;
        rows[1].owner_country = 202;
        rows[1].strength_raw = Some(2_200_000);
        rows[1].local_strength_raw = 2_100_000;
        let mut report = Report::default();
        compare_intel_networks(
            &frame(
                TableId::IntelNetworks,
                TableRows::IntelNetworks(rows.clone()),
            ),
            &pack,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["IntelNetworks"], 2);

        rows[1].strength_raw = rows[0].strength_raw;
        let mut report = Report::default();
        compare_intel_networks(
            &frame(TableId::IntelNetworks, TableRows::IntelNetworks(rows)),
            &pack,
            &owned,
            &tokens,
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    }

    const AUTONOMY_SAVE: &[u8] = br#"countries={
        ENG={diplomacy={active_relations={ AST={ puppet={ first=ENG second=AST autonomy_state="autonomy_dominion" value=0.2 start_date="1936.1.1.12" } } }}}
        AST={diplomacy={active_relations={} autonomy_state={ progress=4312.86618 current_state="autonomy_colony" prev_state="autonomy_puppet" next_state="autonomy_dominion" last_modified_autonomy_date="1943.4.9.5" }}}
    }"#;

    #[test]
    fn autonomy_compares_its_country_root_and_all_saved_recorded_fields() {
        let eng = CountryRow {
            id: 101,
            tag: b"ENG".to_vec(),
            ..Default::default()
        };
        let ast = CountryRow {
            id: 102,
            tag: b"AST".to_vec(),
            ..Default::default()
        };
        let map = BTreeMap::from([(101, eng), (102, ast)]);
        let owned: BTreeMap<u32, &CountryRow> = map.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(AUTONOMY_SAVE);
        let table = TableId::Autonomy;
        let rows = vec![replay_data::rows::AutonomyRow {
            subject: 102,
            score: 431_286_618,
            lower_score: 0,
            upper_score: 0,
            last_modified_bits: GameDate::parse(b"1943.4.9.5")
                .unwrap()
                .engine_hour()
                .unwrap() as u32,
            state: b"autonomy_colony".to_vec(),
            previous: Some(b"autonomy_puppet".to_vec()),
            next: Some(b"autonomy_dominion".to_vec()),
        }];
        let mut report = Report::default();
        compare_autonomy(
            &frame(table, TableRows::Autonomy(rows.clone())),
            &pack,
            &owned,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["Autonomy"], 1);
        for text in ["bounds", "path", "effect"] {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains(text)),
                "{text}: {:?}",
                report.limitations.keys()
            );
        }

        let mut rows = rows;
        rows[0].state = b"autonomy_puppet".to_vec();
        let mut report = Report::default();
        compare_autonomy(
            &frame(table, TableRows::Autonomy(rows)),
            &pack,
            &owned,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut report = Report::default();
        compare_autonomy(
            &frame(table, TableRows::Autonomy(vec![])),
            &pack,
            &owned,
            &BTreeMap::new(),
            &mut report,
        )
        .unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut bad = frame(table, TableRows::Autonomy(vec![]));
        *bad.tables[table.index()] = replay_frame::frame::TableView::Unavailable {
            reason: "source failed".into(),
        };
        assert!(
            compare_autonomy(
                &bad,
                &pack,
                &owned,
                &BTreeMap::new(),
                &mut Report::default()
            )
            .is_err()
        );
    }
}
