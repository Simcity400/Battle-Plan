//! Save comparisons for the intel pack's proved serialized fields.

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use installed_content::save::{GameDate, characters as saved_characters, intel as saved};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, tables},
};

use super::{Report, TokenNames, complete, country_tag, invalid, required_country_tag};

#[derive(Clone, Copy)]
pub(super) struct ScopeKinds {
    pub global: u8,
    pub country: u8,
    pub state: u8,
    /// A character's own flags and script variables, keyed by its id.
    pub character: u8,
}

// Keep independently available save and content inputs explicit at this boundary.
#[allow(clippy::too_many_arguments)]
pub(super) fn compare(
    frame: &DecodedFrame,
    save: &saved::Pack,
    characters: Option<&saved_characters::Manager>,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    intelligence: Result<&installed_content::intelligence::AgencyUpgrades, &str>,
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
        compare_capitulation(frame, save, countries, report)
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

/// The save writes captured references in native roster order.
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
    report.limitation("the game saves the captured reference's stored country, not the resolved CUnitLeader owner; owner and failed reference resolution have no exact save counterpart", "CapturedOperativeReferences.owner");
    Ok(())
}

fn completion_event_limitation(report: &mut Report) {
    report.limitation(
        "OperationCompletions are runtime observations; the operation writer serializes only still-existing instances and does not retain completion event history; individually refused completion descriptions retain their named reason and stored byte length but have no save comparator",
        "OperationCompletions; the Operations comparison still verifies stored outcomes of surviving instances",
    );
}

/// The game serializes each strategic object under its country-array
/// index. The game omits the mission roster, and the effective
/// country/operative/static modifier totals are rebuilt after load.
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

/// The per-country incoming diplomatic-action queue.
///
/// The writer walks the queue's pointer span for its signed
/// count, skips null slots without renumbering and keys every surviving
/// wrapper by its decimal native index, so the recorded ordinal is directly
/// comparable. Each wrapper writes only its own `id`; its status byte
/// has no serialized form. The body is written under its definition name, which
/// is the only place its kind appears. The action writer emits the low
/// byte of the historical `action_type` dword as `initiator_matters`;
/// saved `type` is an independent, uncaptured signed integer.
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
        // Both sides carry the writer's own native index, so align on it: one
        // removed slot must not shift every later row into a difference.
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
                // The game resolves these raw references only while they
                // are positive; a nonpositive one has no tag to compare.
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

/// Active and recruitment operative memberships.
///
/// Operative saves contain nationalities, capturer
/// references, the unmatched operative field, action presence,
/// operation references and the assignment enum;
/// loading preserves the same fields and omission rules.
/// Agency loading reads the operative-reference roster.
/// Join is by full handle (low-u32 type 4713), agency
/// owner tag to roster country, kind 0 to operative blocks and kinds 1/2/3 to
/// the identified recruitment lists (POOL_A save spelling unproved). Literal names resolve via
/// LexicalTokens where exact, else presence/counts with limitation-printed names.
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
    // Recruitment writes its fields under token 0x4B44.
    // Resolve its saved name from this recording, never a guessed spelling.
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
        // The operative-reference roster is written directly in vector order.
        for (ordinal, entry) in agency.operatives.iter().enumerate() {
            // Handles are (id<<32)|type with native IDENTITY type 4713.
            // Repeated memberships are preserved; each block is one entry.
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
    // Assignment literal names via the lexical dictionary where exact.
    let assignment_names: BTreeMap<u32, &[u8]> = [
        (0, 0x4a51_u32),
        (1, 0x4a52),
        (2, 0x4a53),
        (3, 0x394c),
        (4, 0x4a50),
        (5, 0x336c),
    ]
    .into_iter()
    .filter_map(|(value, token_id)| token(tokens, token_id).map(|name| (value, name)))
    .collect();
    let assignment_exact =
        assignment_names.len() == 6 && !assignment_names.values().any(|name| name.contains(&0));
    let nationalities_key = token(tokens, 0x3d12);
    let assignment_key = token(tokens, 0x01b7);
    let literals = || OperativeLiterals {
        assignment_names: assignment_exact.then_some(&assignment_names),
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
    // Recruitment saves the two fields under 0x4B45 and 0x4B54.
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
    if assignment_key != Some(b"state".as_slice()) || !assignment_exact {
        report.limitation(
            "operative assignment literals (0x1B7 with 0x4A51/0x4A52/0x4A53/0x4A50/0x336C/0x394C) resolve via LexicalTokens where exact; missing or NUL entries leave state values unverified beyond presence/counts and ids",
            format!("0x1B7={:?}", assignment_key.map(String::from_utf8_lossy)),
        );
    }
    report.limitation(
        "operative experience omission is compared as zero from live recruitment shapes; the writer's omission rule for experience is unproved",
        "OperativeMemberships.experience",
    );
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
    actual: &replay::rows::OperativeMembershipRow,
    expected: &saved::OperativeEntry,
    countries: &BTreeMap<u32, &CountryRow>,
    literals: OperativeLiterals<'_>,
) -> io::Result<()> {
    // Native nationality vector width is unproved and unrecorded; the saved
    // count is retained by the parser and the literal key is proved only when
    // the lexical entry is exact (checked by the caller).
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
        // Live recruitment blocks omit experience while native holds zero;
        // the writer's omission rule is unproved, so omission compares as zero.
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
    match (
        literals.assignment_names,
        &expected.state,
        literals.assignment_key,
    ) {
        (Some(names), Some(state), Some(_)) => {
            let mut found = None;
            for (value, name) in names {
                if *name == state.as_slice() {
                    found = Some(*value);
                    break;
                }
            }
            match found {
                Some(value) => equal(
                    report,
                    &format!("{label}.assignment"),
                    actual.assignment,
                    value,
                ),
                None => report.difference(
                    format!("{label}.assignment"),
                    actual.assignment,
                    String::from_utf8_lossy(state).into_owned(),
                ),
            }
        }
        _ => {
            // Literal mapping unproved; presence already verified by the census.
        }
    }
    if actual.assignment == engine::coperative_leader::COOLDOWN_ASSIGNMENT {
        match (
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
        ) {
            (Some(kind), Some(start), Some(end)) => {
                // Saved cooldown status spellings. The inherited
                // writer emits reason/start/end only for nonzero kind.
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
            }
            _ => report.limitation("cooldown operands were not recorded in this layout", label),
        }
    }
    // The game writes mission iff the game sees a nonzero
    // action discriminator; its separate operation handle is not that action.
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
    actual: &replay::rows::OperativeMembershipRow,
    expected: &saved::OperativeEntry,
    countries: &BTreeMap<u32, &CountryRow>,
    literals: &OperativeLiterals<'_>,
) -> io::Result<()> {
    // The game writes the own reference regardless of outer assignment;
    // The stored name is written, including literal renames.
    if let Some(variant) = actual.action_variant {
        let id = variant
            .checked_sub(1)
            .and_then(|index| engine::operative_action::SAVE_TOKENS.get(index as usize))
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
        // The unit leader save writer writes token 0x1B only for a
        // nonzero stored length: its omission proves an empty name.
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
                // Operation hover null-checks the operation-instance lookup;
                // one stale assignment must not abort the census.
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

/// Cryptology targets.
///
/// Cryptology saves the 0x3D75 list iff its count is nonzero, with
/// stride 0x38 plus the 0x4B34 source; loading mirrors them.
/// Each entry saves target (0x6B), decrypting (0x32F2),
/// amount (0x1A1), bonus_active (0x2C7E), bonus_days (0x296D) and date (0x284A);
/// loading mirrors them plus 0x2A7. Join is by owner and
/// target tags; amount compares as i64/100000 and decrypting as yes/no.
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
    // Empty-omitted census: a country with no targets key must have no rows.
    // (Verified by the leftover-recorded pass above; no separate loop needed.)
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

/// Intel subnetworks by target with positional state join.
///
/// The writer saves target (0x6B), local net (0x3D1D), subnetworks
/// (0x4B6D, when their count is nonzero), records (0x4C00, when their
/// count is nonzero), static source (0x4B34), coverage (0x30B5) and strength
/// sums; loading mirrors them. Rows join positionally on state_id lists with
/// strength None iff selector negative (strength 0 / sub_network_id -1).
/// The game keys strategic operatives by the same country-array slot
/// authenticated by the native walker; owners must remain separate.
fn compare_intel_networks(
    frame: &DecodedFrame,
    save: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut saved = BTreeMap::<(u32, Vec<u8>), Vec<(i32, Option<i64>, i32, i64)>>::new();
    // Built once: a per-network scan of every link is quadratic in countries.
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
                for ((state, selector), local_strength) in network
                    .state_ids
                    .iter()
                    .zip(&network.selectors)
                    .zip(&network.strengths)
                {
                    let strength = if *selector < 0 {
                        None
                    } else {
                        Some(
                            network
                                .subnetwork_strengths
                                .get(*selector as usize)
                                .ok_or_else(|| {
                                    invalid("saved intel subnetwork selector outside array")
                                })?
                                .raw,
                        )
                    };
                    entries.push((*state, strength, *selector, local_strength.raw));
                }
            }
        }
    }
    for entries in saved.values_mut() {
        entries.sort_unstable_by_key(|(state, _, _, _)| *state);
    }
    let mut recorded = BTreeMap::<(u32, Vec<u8>), Vec<&replay::rows::IntelNetworkRow>>::new();
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
        let actual = recorded
            .remove(&(*owner_slot, target.clone()))
            .unwrap_or_default();
        let mut actual_values = actual
            .iter()
            .map(|row| {
                (
                    row.state_id,
                    row.strength_raw,
                    row.subnetwork_selector,
                    row.local_strength_raw,
                )
            })
            .collect::<Vec<_>>();
        actual_values.sort_unstable();
        let mut expected_values = expected
            .iter()
            .map(|(state, strength, selector, local)| (*state, *strength, *selector, *local))
            .collect::<Vec<_>>();
        expected_values.sort_unstable();
        // Negative selectors carry no strength: save uses strength 0 with
        // sub_network_id -1, native uses None. Normalize before comparing.
        let normalize = |values: Vec<(i32, Option<i64>, i32, i64)>| {
            values
                .into_iter()
                .map(|(state, strength, selector, local)| {
                    if selector < 0 {
                        (state, None, selector, local)
                    } else {
                        (state, strength, selector, local)
                    }
                })
                .collect::<Vec<_>>()
        };
        let actual_norm = normalize(actual_values);
        let expected_norm = normalize(expected_values);
        if actual_norm.len() != expected_norm.len() {
            report.difference(
                format!("{label}.count"),
                actual_norm.len(),
                expected_norm.len(),
            );
        }
        for (state, strength, selector, local) in &expected_norm {
            match actual_norm
                .binary_search_by_key(state, |(state, _, _, _)| *state)
                .ok()
                .map(|index| &actual_norm[index])
            {
                None => {
                    report.difference(format!("{label}.state.{state}"), "absent", *selector);
                }
                Some((_, actual_strength, actual_selector, actual_local)) => {
                    equal(
                        report,
                        &format!("{label}.state.{state}.local_strength"),
                        *actual_local,
                        *local,
                    );
                    matched += 1;
                    equal(
                        report,
                        &format!("{label}.state.{state}.selector"),
                        *actual_selector,
                        *selector,
                    );
                    match (actual_strength, strength) {
                        (None, None) => {}
                        (Some(actual), Some(expected)) => {
                            if actual != expected {
                                report.difference(
                                    format!("{label}.state.{state}.strength"),
                                    *actual,
                                    *expected,
                                );
                            }
                        }
                        _ => {
                            report.difference(
                                format!("{label}.state.{state}.strength"),
                                *actual_strength,
                                *strength,
                            );
                        }
                    }
                }
            }
        }
        for (state, _, _, _) in &actual_norm {
            if expected_norm
                .binary_search_by_key(state, |(state, _, _, _)| *state)
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
    report.limitation(
        "intel coverage (0x30B5), records (0x4C00), static source (0x4B34), strength sums and the list (0x3D1E) have no recorded field; retained natively but not verified here",
        "IntelNetworks.coverage, records, source, sums, extra_3d1e",
    );
    report.compared("IntelNetworks", matched);
    Ok(())
}

/// Compare the country-rooted CCurrentAutonomyStatus serializer, not the
/// historically stored CSubjectRelation definition in active_relations.
/// The game emits score, current/adjacent names and the date's
/// persistent base, independently of its current hour.
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
        &*frame.tables[replay::rows::TableId::LexicalTokens.index()],
        replay::frame::TableView::Complete(_)
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
        // The game writes is_created=yes exactly when the byte is nonzero.
        equal(
            report,
            &format!("{subject}.is_created"),
            actual.created != 0,
            expected.created,
        );
        if let Some(name) = &expected.name {
            // The agency save writer emits its name
            // unconditionally in ordinary text mode, including empty text.
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
    // Refused rows survive a truncated read. Name them before refusing the
    // table's coverage, without comparing partial rows against a full save.
    let acquired = match &*frame.tables[replay::rows::TableId::AgencyUpgradeLevels.index()] {
        replay::frame::TableView::Complete(replay::rows::TableRows::AgencyUpgradeLevels(rows))
        | replay::frame::TableView::Truncated {
            rows: replay::rows::TableRows::AgencyUpgradeLevels(rows),
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
                engine::agency_upgrade_levels::status_name(row.identity_status),
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
    intelligence: Result<&installed_content::intelligence::AgencyUpgrades, &str>,
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
                engine::agency_upgrade_levels::status_name(row.identity_status)
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
        // Loading selects the first global token pointer.
        // Exact saved token/level agreement cannot waive a different live branch.
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
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    const DATE_SENTINEL: i32 = 0x29c77f8;
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
        equal(
            report,
            &format!("{label}.last_surrender_date"),
            (actual.last_surrender_hour != DATE_SENTINEL).then_some(actual.last_surrender_hour),
            saved_hour(expected.last_surrender_date)?,
        );
        equal(
            report,
            &format!("{label}.faction_join_date"),
            (actual.faction_join_hour != DATE_SENTINEL).then_some(actual.faction_join_hour),
            saved_hour(expected.faction_join_date)?,
        );
        if let Some(legitimacy) = &expected.legitimacy {
            report.scalar(
                &label,
                "legitimacy",
                Some(actual.legitimacy_raw),
                Some(legitimacy),
            );
        } else {
            report.limitation("legitimacy is serialized only for a positive exile host; omission cannot verify its native qword", &label);
        }
        equal(
            report,
            &format!("{label}.exile_army_leaders"),
            Some(actual.exile_army_leaders),
            expected.exile_army_leaders,
        );
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
            // `compare_character_flags` joins them with the characters.
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

/// One scope's recorded flags of one literal name against its saved flags of
/// that name. Returns the joined occurrences.
fn compare_flag_values(
    report: &mut Report,
    label: &str,
    actual: &[&replay::rows::ScriptFlagRow],
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

/// Character flags (ScriptFlags' character scope) against each saved
/// character's own `flags`. The character writer writes that
/// block from the character's flag store exactly when its count is
/// positive, so a character without it has no flags.
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
    /// A character's store, by its full reference (`id << 32 | 73`).
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

/// ScriptVariables against the save's `variables` blocks: the game state's,
/// each country's, each state's and each saved character's. The character
/// writer omits the block only for a store with no entry
/// and an untouched random state, so a saved character
/// without it has no variables.
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
    let mut character_rows = 0_usize;
    let mut recorded = BTreeMap::new();
    for actual in complete::<tables::ScriptVariables>(frame)? {
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
                    super::pass_inputs::eq(report, &format!("{label} random seed"), actual, expected);
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
        for (index, entry) in actual.entries.iter().enumerate() {
            // The variable writer passes a C-string pointer. The concrete stream target is
            // not proved, so do not infer a prefix projection from that ABI.
            if entry.name.contains(&0) {
                unresolved_literal = true;
                report.limitation(
                    "variable-name formatter receives a C-string pointer, but its concrete stream behavior for interior NUL is unproved; this full native key and value remain unverified",
                    format!("{label} entry {index}"),
                );
                continue;
            }
            let name = entry.name.as_slice();
            if name == b"random" {
                report.limitation(
                    "a native variable key emits the reserved random metadata name; its identity and value cannot be distinguished from metadata in the save",
                    format!("{label} entry {index}"),
                );
                continue;
            }
            values.entry(name).or_default().push(entry.value_raw);
        }
        let mut expected_values = BTreeMap::<&[u8], Vec<&installed_content::save::Decimal>>::new();
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
                "scalar random entries use CVariables' reserved metadata name; their native literal keys cannot be distinguished in the save",
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
    let mut null_target_owners = BTreeSet::new();
    for actual in complete::<tables::OperationTokens>(frame)? {
        let owner = required_country_tag(actual.owner, countries)?;
        country(save, owner)?;
        owners_with_targets.insert(owner);
        if actual.target == 0 {
            if !null_target_owners.insert(owner) {
                return Err(invalid("duplicate resolved null operation-token target"));
            }
            report.limitation(
                "operation-token native null-target serialization is not proved",
                format!("{} tokens {:?}", text(owner), actual.tokens),
            );
            continue;
        }
        let target = required_country_tag(actual.target, countries)?;
        country(save, target)?;
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
            // The asset writer emits operation_assets for a nonempty target map;
            // its callback omits each empty token vector independently.
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
            if !save.countries.contains_key(&expected.target) {
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

/// Persistent espionage-operation instances.
///
/// The operation save writer always emits the target. Start date and duration
/// are emitted only for nonzero duration, and the start-date sentinel is omitted.
/// Assigned slots and phases are emitted only for nonzero signed counts; the
/// province reference is emitted only when nonzero. Loading restores the same
/// fields. The manager writer always emits finished and priority lists, and
/// emits the running list only when its count is nonzero.
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
            // The manager writer omits running iff its count is zero. Report
            // one count difference and consume those rows so the leftover
            // pass below does not double-count the same extra rows.
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
            // The operation writer omits each boolean only when its byte is zero.
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
        "operation preparation, flag_40 and auto_repeat words have no proved save literal in the observed live shapes (all six running instances omit them); they are retained natively but not verified here",
        "Operations.preparation_current, Operations.preparation_required, Operations.flag_40, Operations.auto_repeat",
    );
    report.limitation(
        "operation target-province literal `province` is inferred from live omission shapes (all six omit it with native null); a present case is unproved",
        "Operations.target_province",
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

fn saved_effect_description(raw: &[u8]) -> Vec<u8> {
    // Description serialization removes CR and replaces LF with literal
    // backslash-n. The text writer emits its C-string prefix;
    // save parser decodes the writer's quote/backslash escapes afterwards.
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

/// Assigned-operative slots, joined positionally by (operation, ordinal).
///
/// Assigned-slot writer omits the 0x3D13 ref for a both-zero
/// handle but retains the slot, omits 0x4C11 for zero selection, and
/// omits 0x2CBA for a null optional. Both sides carry the writer's own
/// positional ordinal (copy of the incoming-action ordinal pattern in
/// compare_incoming_actions:135-142), so one removed slot must not shift later
/// rows into differences. Selection-vs-mission semantics are unproved, so only
/// presence, ordinal and handle are compared.
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
    // No dense-ordinal refusal here: like incoming actions, the ordinal is
    // identity, so a moved row joins as missing plus extra differences rather
    // than an error. Duplicates above are still refused.
    let mut matched = 0;
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
            // A recorded assignment for this operation beyond the saved list
            // length is reported with the leftovers below; nothing more to do
            // here besides the per-ordinal join above.
        }
    }
    // Validate that every recorded assignment's operation id exists in the save
    // census (and that its owner link resolves), so stray rows are differences,
    // not silent passes.
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
        "operation selection-vs-mission semantics are unproved: comparison verifies only slot presence, positional ordinal and operative handle; recorded selection words and saved mission contents are retained but not equated",
        "OperationAssignments.selection",
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
    use replay::frame::TableView;
    use replay::rows::{Recorded, TableId, TableRows};

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
            |(ordinal, (operative, owner))| replay::rows::CapturedOperativeReferenceRow {
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
                .any(|reason| reason.contains("resolved CUnitLeader owner"))
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
    fn operation_completion_events_are_named_save_limitation() {
        let mut report = Report::default();
        completion_event_limitation(&mut report);
        assert!(report.differences.is_empty());
        assert!(report.compared_tables.is_empty());
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("does not retain completion event history"))
        );
        assert!(report.limitations.keys().any(|reason| {
            reason.contains("individually refused completion descriptions")
                && reason.contains("no save comparator")
        }));
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
                TableRows::RaidIntelInputs(vec![replay::rows::RaidIntelInputRow {
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

    fn incoming_rows(rows: Vec<replay::rows::IncomingDiplomaticActionRow>) -> DecodedFrame {
        frame(
            TableId::IncomingDiplomaticActions,
            TableRows::IncomingDiplomaticActions(rows),
        )
    }

    fn incoming_row() -> replay::rows::IncomingDiplomaticActionRow {
        replay::rows::IncomingDiplomaticActionRow {
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

        // The native index is identity, not payload: changing it moves the row.
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

        // A changed handle, tag or date is its own single difference.
        for mutate in [
            (|row: &mut replay::rows::IncomingDiplomaticActionRow| row.handle = 1) as fn(&mut _),
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

        // One removed slot must not shift the rows after it: the second action
        // still aligns on its own native index.
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

        // A recorded action for a country whose queue block is empty is a
        // difference, not a silently accepted extra row.
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
        saved::parse(&installed_content::parser::parse(input).unwrap().items).unwrap()
    }

    fn germany() -> CountryRow {
        CountryRow {
            id: 101,
            canonical_slot: 7,
            tag: b"GER".to_vec(),
            ..Default::default()
        }
    }

    /// Character flags join each saved character's own `flags` by id and
    /// literal name; a character without the block has none, and the country
    /// comparison neither counts nor limits them.
    /// Each saved character's `variables` block, and no block for an
    /// empty store.
    #[test]
    fn character_variables_compare_with_each_saved_character() {
        let items = installed_content::parser::parse(
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
        let row = |scope_id: u32, entries: &[(&[u8], i64)]| replay::rows::ScriptVariablesRow {
            scope_kind: SCOPES.character,
            scope_id,
            seed: Recorded::Value(None),
            entries: entries
                .iter()
                .map(|(name, value_raw)| replay::rows::ScriptVariableEntry {
                    name: name.to_vec(),
                    value_raw: *value_raw,
                })
                .collect(),
        };
        let global = replay::rows::ScriptVariablesRow::default();
        let run = |rows: Vec<replay::rows::ScriptVariablesRow>| {
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
        // A recorded key the save lacks, and a missing store, differ.
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
        let items = installed_content::parser::parse(
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
        let row = |scope_id: u32, token: u32, remaining_days: i16| replay::rows::ScriptFlagRow {
            scope_kind: SCOPES.character,
            scope_id,
            token,
            set_hour: date,
            value: 1,
            remaining_days,
        };
        let tokens = BTreeMap::from([(600, b"recently_sick".as_slice()), (601, b"other")]);
        let run = |rows: Vec<replay::rows::ScriptFlagRow>| {
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
        // A different countdown, a flag the save lacks, a missing flag.
        assert_eq!(run(vec![row(44, 600, 179)]).differences.len(), 1);
        assert_eq!(
            run(vec![row(44, 600, 180), row(45, 601, 0)])
                .differences
                .len(),
            2
        );
        assert_eq!(run(vec![]).differences.len(), 2);
        // The country comparison leaves character rows to this one.
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
        let mut actual = replay::rows::IntelligenceAgencyRow {
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

    fn acquired_upgrade_frame(rows: Vec<replay::rows::AgencyUpgradeLevelRow>) -> DecodedFrame {
        let mut captured = frame(
            TableId::AgencyUpgradeLevels,
            TableRows::AgencyUpgradeLevels(rows),
        );
        *captured.tables[TableId::IntelligenceAgencies.index()] =
            TableView::Complete(TableRows::IntelligenceAgencies(vec![
                replay::rows::IntelligenceAgencyRow {
                    country: 101,
                    ..Default::default()
                },
            ]));
        captured
    }

    fn acquired_upgrade_rows() -> Vec<replay::rows::AgencyUpgradeLevelRow> {
        [3, -2, 0]
            .into_iter()
            .enumerate()
            .map(|(ordinal, level)| replay::rows::AgencyUpgradeLevelRow {
                country: 102, // authenticated alias of the same current country
                ordinal: ordinal as u32,
                definition_pointer: 0x1000 + ordinal as u64 * 0x100,
                branch_pointer: 0x2000,
                upgrade_token: Some(if ordinal == 1 { 12 } else { 11 }),
                branch_token: Some(10),
                level,
                identity_status: 0,
            })
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
        partial_rows[0].identity_status = engine::agency_upgrade_levels::NULL_DEFINITION;
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
    fn agency_upgrade_oracle_covers_five_thousand_country_instances() {
        let mut save = saved::Pack::default();
        let template = parsed(b"countries={XAA={intelligence_agency={defense=0 max_operative_count=0 usable_operative_slots=0 upgrades={mod_upgrade=-3}}}}").countries.remove(b"XAA".as_slice()).unwrap();
        let countries = (1..=5000)
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
            rows.push(replay::rows::AgencyUpgradeLevelRow {
                country: id,
                ordinal: 0,
                upgrade_token: Some(11),
                branch_token: Some(10),
                level: -3,
                ..Default::default()
            });
            agencies.push(replay::rows::IntelligenceAgencyRow {
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
        assert_eq!(report.compared_tables["AgencyUpgradeLevels"], 5000);
        let replaced = countries.values().next().unwrap().tag.clone();
        save.countries.remove(&replaced);
        let mut report = Report::default();
        compare_agency_upgrade_levels(&captured, &save, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn agency_upgrade_oracle_reports_ambiguous_branch_and_save_reload_discrepancy() {
        let scratch = std::env::var_os("battle-plan_TEST_SCRATCH")
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
        let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let intelligence = installed_content::intelligence::read_agency_upgrades(&layers).unwrap();
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
        // The fixture has a unique task-owned absolute path outside source.
        assert!(root.starts_with(&scratch));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn independent_tables_continue_after_failures_and_preserve_partial_comparisons() {
        let pack = parsed(b"countries={GER={intelligence_agency={defense=0.00001 max_operative_count=0 usable_operative_slots=0}}}");
        let country = germany();
        let links = BTreeMap::from([(101, &country), (102, &country)]);
        let first = replay::rows::IntelligenceAgencyRow {
            country: 101,
            defense_raw: 2,
            ..Default::default()
        };
        let duplicate = replay::rows::IntelligenceAgencyRow {
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
                TableRows::ScriptFlags(vec![replay::rows::ScriptFlagRow {
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
            replay::rows::ScriptFlagRow {
                scope_kind: SCOPES.country,
                scope_id: 101,
                token: 0,
                value: -3,
                set_hour: hour(b"1936.1.1.1"),
                remaining_days: -1,
            },
            replay::rows::ScriptFlagRow {
                scope_kind: SCOPES.country,
                scope_id: 101,
                token: 1,
                value: 4,
                set_hour: hour(b"1936.1.2.1"),
                remaining_days: 5,
            },
            replay::rows::ScriptFlagRow {
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
            .map(|token| replay::rows::ScriptFlagRow {
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
            TableRows::ScriptFlags(vec![replay::rows::ScriptFlagRow {
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
            TableRows::OperationTokens(vec![replay::rows::OperationTokensRow {
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
        let mut row = replay::rows::OperationTokensRow {
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
        let mut row = replay::rows::OperationTokensRow {
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
        let mut row = replay::rows::ScriptVariablesRow {
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
            .map(|(name, value_raw)| replay::rows::ScriptVariableEntry {
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
        let items = installed_content::parser::parse(
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
            |(scope_kind, scope_id, seed)| replay::rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                seed: Recorded::Value(Some(seed)),
                entries: Vec::new(),
            },
        )
        .collect::<Vec<_>>();
        let compare = |rows: Vec<replay::rows::ScriptVariablesRow>| {
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
            .map(|(scope_kind, scope_id)| replay::rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                seed: Recorded::Value(None),
                entries: vec![],
            })
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
            |(scope_kind, scope_id, value)| replay::rows::ScriptVariablesRow {
                scope_kind,
                scope_id,
                seed: Recorded::Value(None),
                entries: value
                    .into_iter()
                    .map(|value_raw| replay::rows::ScriptVariableEntry {
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
                }
            }
        }}
        SOV={operations={ finished={} priority=1 }}
    }"#;

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

    fn operation_row() -> replay::rows::OperationRow {
        replay::rows::OperationRow {
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
        }
    }

    fn operation_frame(rows: Vec<replay::rows::OperationRow>) -> DecodedFrame {
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
        let save = parsed(br#"countries={GER={operations={running={mod_operation={id={id=69023 type=4713} target=SOV completed=yes outcome_extra=yes risk_extra=yes looted="line\\n\"quote\"\\literal"}}}} SOV={}}"#);
        let mut row = operation_row();
        row.start_hour = 0x29c77f8;
        row.duration_days = 0;
        row.phase_count = 0;
        row.assigned_slots = 0;
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
        let empty_description = parsed(br#"countries={GER={operations={running={mod_operation={id={id=69023 type=4713} target=SOV completed=yes looted=""}}}} SOV={}}"#);
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
        let missing_description = parsed(br#"countries={GER={operations={running={mod_operation={id={id=69023 type=4713} target=SOV completed=yes}}}} SOV={}}"#);
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
        let byte_description = parsed(b"countries={GER={operations={running={mod_operation={id={id=69023 type=4713} target=SOV completed=yes looted=\"\xfe\"}}}} SOV={}}");
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

    /// Gated operation emission: present values compare exactly, omitted
    /// values prove sentinel/zero/empty, and any mismatch is a real
    /// difference rather than a printed limitation (except the three proved
    /// limitation groups: definition-token mapping, manager history/priority,
    /// plus the unproved prep/flag and inferred province literals).
    #[test]
    fn operations_present_and_omitted_compare_with_gated_emission() {
        let (_, _, countries) = operation_links();
        let owned: BTreeMap<u32, &CountryRow> =
            countries.iter().map(|(id, row)| (*id, row)).collect();
        let pack = parsed(OPERATION_SAVE);
        let tokens: TokenNames<'_> = [(7_u32, b"operation_rescue_operative".as_slice())]
            .into_iter()
            .collect();
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
        // Unrecorded manager history/priority, prep/flag, province-inference
        // and definition-token limitations are always printed.
        for text in [
            "operation-manager priority",
            "no proved save literal",
            "inferred from live omission",
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

        // Every exactly-compared field can differ on its own.
        type Change = fn(&mut replay::rows::OperationRow);
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
            // A definition token with no lexical entry is a limitation, not a
            // definition difference; give it an entry so the change is visible.
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

        // Omitted saved date/duration/phases/slots prove sentinel/zero.
        let omitted = parsed(
            br#"countries={GER={operations={ running={
                operation_rescue_operative={ id={ id=69023 type=4713 } target="SOV" }
            } }} SOV={operations={ finished={} priority=1 }}}"#,
        );
        let mut row = operation_row();
        row.start_hour = 0x29c77f8;
        row.duration_days = 0;
        row.phase_count = 0;
        row.assigned_slots = 0;
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

        // A recorded start/duration/count against an omitted saved field is a
        // real difference, not a waived omission.
        let mut row = operation_row();
        row.start_hour = 0x29c77f8;
        row.duration_days = 0;
        row.phase_count = 0;
        row.assigned_slots = 0;
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

        // GER shape (no running key) with a recorded row for that owner is a
        // difference; with no recorded rows it matches.
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
        // One missing (saved id absent from recorded) plus one extra
        // (recorded id absent from save).
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
        // A changed scalar still fails even when the definition is unverified.
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

    fn assignment_frame(rows: Vec<replay::rows::OperationAssignmentRow>) -> DecodedFrame {
        frame(
            TableId::OperationAssignments,
            TableRows::OperationAssignments(rows),
        )
    }

    fn assignment_rows() -> Vec<replay::rows::OperationAssignmentRow> {
        vec![
            replay::rows::OperationAssignmentRow {
                operation: (69023 << 32) | 4713,
                ordinal: 0,
                operative: (9831 << 32) | 4713,
                selection: 0,
            },
            replay::rows::OperationAssignmentRow {
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

    /// Positional slot join: the ordinal is identity (like incoming actions),
    /// so one removed slot must not shift later rows. Only presence, ordinal
    /// and handle compare exactly; selection-vs-mission is limitation-printed.
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

        // A changed handle is its own single difference.
        let mut rows = assignment_rows();
        rows[0].operative = 1;
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("operative"));

        // The native ordinal is identity, not payload: changing it moves the row.
        let mut rows = assignment_rows();
        rows[0].ordinal = 5;
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        // Saved slot 0 is now absent and recorded (op,5) is extra.
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);

        // An omitted saved operative (both-zero slot retained) requires a zero
        // recorded handle; a present saved operative requires that handle.
        let mut rows = assignment_rows();
        rows[1].operative = (111 << 32) | 4713;
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        // A recorded assignment beyond the saved list length is a difference.
        let mut rows = assignment_rows();
        rows.push(replay::rows::OperationAssignmentRow {
            operation: (69023 << 32) | 4713,
            ordinal: 2,
            operative: 0,
            selection: 0,
        });
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        // One removed slot must not shift later rows: with saved slots [0,1]
        // and recorded rows [(op,0),(op,2)], slot 1 is missing and (op,2) is
        // extra, while slot 0 still aligns on its own ordinal.
        let mut rows = assignment_rows();
        rows[1].ordinal = 2;
        let mut report = Report::default();
        compare_operation_assignments(&assignment_frame(rows), &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);

        let mut bad = assignment_frame(assignment_rows());
        *bad.tables[TableId::OperationAssignments.index()] =
            replay::frame::TableView::Unavailable {
                reason: "source failed".into(),
            };
        assert!(compare_operation_assignments(&bad, &pack, &mut Report::default()).is_err());
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

    fn operative_rows() -> Vec<replay::rows::OperativeMembershipRow> {
        vec![
            replay::rows::OperativeMembershipRow {
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
            replay::rows::OperativeMembershipRow {
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

    /// Present values compare exactly, omitted saved entries prove absence, and
    /// unproved literals stay limitation-printed with exact mapping names.
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
        // An operation handle and an on_mission assignment do not imply
        // a mission action discriminator. Both fixtures correctly have None.
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

        // Every exactly-compared field can differ on its own.
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

        // Omitted saved entry is a difference; extra recorded row is a difference.
        let mut report = Report::default();
        compare_operative_memberships(&frame(vec![]), &pack, &owned, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        let mut rows = operative_rows();
        rows.push(replay::rows::OperativeMembershipRow {
            id: (99999 << 32) | 4713,
            roster_country: 101,
            roster_kind: 0,
            ..Default::default()
        });
        let mut report = Report::default();
        compare_operative_memberships(&frame(rows), &pack, &owned, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        // POOL_A cannot impersonate a saved recruitable membership.
        let mut rows = operative_rows();
        rows[1].roster_kind = 1;
        let mut report = Report::default();
        compare_operative_memberships(&frame(rows), &pack, &owned, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);

        // Duplicate recorded identity is refused; unavailable tables error.
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
        *bad.tables[table.index()] = replay::frame::TableView::Unavailable {
            reason: "source failed".into(),
        };
        assert!(
            compare_operative_memberships(&bad, &pack, &owned, &tokens, &mut Report::default())
                .is_err()
        );
    }

    #[test]
    fn operative_placements_compare_all_mission_definitions_names_targets_and_own_references() {
        let (_, _, countries) = operative_links();
        let owned = countries.iter().map(|(id, row)| (*id, row)).collect();
        let definitions: [&[u8]; 8] = [b"m1", b"m2", b"m3", b"m4", b"m5", b"m6", b"m7", b"m8"];
        let mut tokens = operative_tokens();
        for (id, name) in engine::operative_action::SAVE_TOKENS
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
                    replay::rows::TableRows::OperativeMemberships(vec![row]),
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
            // Native country-tag serialization omits a null target.
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
            // A different concrete definition must not pass on presence alone.
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
                replay::rows::TableRows::OperativeMemberships(rows),
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
            // Continue through later operative rows after a stale assignment.
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

    fn cryptology_rows() -> Vec<replay::rows::CryptologyRow> {
        vec![
            replay::rows::CryptologyRow {
                owner_country: 102,
                target_country: 201,
                amount_raw: 2_762_500_000,
                bonus_days: -1,
                bonus_active: 0,
                decrypting: 1,
            },
            replay::rows::CryptologyRow {
                owner_country: 102,
                target_country: 202,
                amount_raw: 1_200_000_000,
                bonus_days: -1,
                bonus_active: 0,
                decrypting: 0,
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

    /// Present targets compare amount and decryption exactly; omitted GER
    /// targets prove empty; bonus/date shapes stay limitation-printed.
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

        // Omitted saved target is a difference; GER empty omission matches.
        let mut report = Report::default();
        compare_cryptology(&frame(vec![]), &pack, &owned, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        let mut rows = cryptology_rows();
        rows.push(replay::rows::CryptologyRow {
            owner_country: 101,
            target_country: 201,
            ..Default::default()
        });
        let mut report = Report::default();
        compare_cryptology(&frame(rows), &pack, &owned, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut bad = frame(cryptology_rows());
        *bad.tables[table.index()] = replay::frame::TableView::Unavailable {
            reason: "source failed".into(),
        };
        assert!(compare_cryptology(&bad, &pack, &owned, &mut Report::default()).is_err());
    }

    const NETWORK_SAVE: &[u8] = br#"countries={ENG={} SOV={} AUS={}}
        strategic_operatives={ strategic_operative={
            2={ intel_network={ target="SOV" intel_network={ state_id={ 219 253 } strength={ 71 0 } sub_network_id={ 0 -1 } } sub_intel_networks={ { strength=72.5 } } } }
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

    fn network_rows() -> Vec<replay::rows::IntelNetworkRow> {
        vec![
            replay::rows::IntelNetworkRow {
                owner_country: 101,
                target_country: 201,
                state_id: 219,
                strength_raw: Some(7_250_000),
                subnetwork_selector: 0,
                local_strength_raw: 7_100_000,
            },
            replay::rows::IntelNetworkRow {
                owner_country: 101,
                target_country: 201,
                state_id: 253,
                strength_raw: None,
                subnetwork_selector: -1,
                local_strength_raw: 0,
            },
        ]
    }

    /// Positional state join within each canonical owner's target network.
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
        for text in ["coverage", "records", "target literal"]
            .into_iter()
            .take(2)
        {
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains(text)),
                "{text}: {:?}",
                report.limitations.keys()
            );
        }

        let mut rows = network_rows();
        rows[0].subnetwork_selector = 1;
        let mut report = Report::default();
        compare_intel_networks(&frame(rows), &pack, &owned, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);

        let mut report = Report::default();
        compare_intel_networks(&frame(vec![]), &pack, &owned, &tokens, &mut report).unwrap();
        // One count difference plus one absent difference per saved state.
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);

        let mut bad = frame(network_rows());
        *bad.tables[table.index()] = replay::frame::TableView::Unavailable {
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
                2={ intel_network={ target="SOV" intel_network={ state_id={ 219 } strength={ 11 } sub_network_id={ 0 } } sub_intel_networks={ { strength=12 } } } }
                3={ intel_network={ target="SOV" intel_network={ state_id={ 219 } strength={ 21 } sub_network_id={ 0 } } sub_intel_networks={ { strength=22 } } } }
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

    /// Current autonomy can differ from the old puppet-relation definition.
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
        let rows = vec![replay::rows::AutonomyRow {
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
        *bad.tables[table.index()] = replay::frame::TableView::Unavailable {
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
