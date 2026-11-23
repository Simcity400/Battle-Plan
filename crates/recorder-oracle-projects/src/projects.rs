use super::{Report, complete, country_tag, invalid, required_country_tag};
use content_reader_save::{characters as characters_saved, projects as saved};
use replay_data::{rows::CountryRow, rows::RaidScope, rows::RaidSource, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub type TokenNames<'a> = BTreeMap<u32, &'a [u8]>;

fn equal<T: std::fmt::Debug + PartialEq>(
    report: &mut Report,
    label: impl AsRef<str>,
    actual: T,
    expected: T,
) {
    if actual != expected {
        report.difference(label, actual, expected);
    }
}
fn token<'a>(
    tokens: &TokenNames<'a>,
    value: u32,
    report: &mut Report,
    label: &str,
) -> Option<&'a [u8]> {
    if value == engine_native::cpdx_lexer::FIXED15_TOKEN {
        report.limitation("the token writer formats reserved lexical token 0x167 as fixed15 numeric text; its raw dictionary literal cannot certify this saved field", label);
        return None;
    }
    let Some(result) = tokens.get(&value).copied() else {
        report.limitation("recorded lexical token spelling is unavailable; this field cannot be compared to its saved literal", format!("{label} (token {value})"));
        return None;
    };
    if result.contains(&0) {
        report.limitation("recorded lexical string contains an interior NUL; concrete text-sink emission is unproved, so this field's saved literal cannot be compared", format!("{label} (token {value})"));
        return None;
    }
    Some(result)
}
fn compare_token(
    report: &mut Report,
    label: &str,
    tokens: &TokenNames<'_>,
    actual: u32,
    expected: &[u8],
) {
    if let Some(actual) = token(tokens, actual, report, label) {
        equal(report, label, actual, expected);
    }
}
fn compare_optional_token(
    report: &mut Report,
    label: &str,
    tokens: &TokenNames<'_>,
    actual: Option<u32>,
    expected: Option<&[u8]>,
) {
    equal(
        report,
        format!("{label}.present"),
        actual.is_some(),
        expected.is_some(),
    );
    if let (Some(actual), Some(expected)) = (actual, expected) {
        compare_token(report, label, tokens, actual, expected);
    }
}
fn status<'a>(pack: &'a saved::Pack, tag: &[u8]) -> io::Result<&'a saved::ProgramStatus> {
    pack.countries
        .get(tag)
        .and_then(|country| country.status.as_ref())
        .ok_or_else(|| {
            invalid(format!(
                "save country {} lacks program_status",
                String::from_utf8_lossy(tag)
            ))
        })
}
fn signed_country<'a>(
    raw: i32,
    countries: &BTreeMap<u32, &'a CountryRow>,
) -> io::Result<Option<&'a [u8]>> {
    if raw <= 0 {
        Ok(None)
    } else {
        required_country_tag(raw as u32, countries).map(Some)
    }
}
fn missing<T: std::fmt::Debug>(report: &mut Report, table: &str, id: T, side: &str) {
    report.difference(
        format!("{table} {id:?} membership"),
        side,
        "matching saved/captured row",
    );
}

pub fn compare(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut succeeded = report.attempt("projects Nukes", |report| {
        compare_nukes(frame, pack, countries, report)
    });
    succeeded &= report.attempt("projects RaidFacilities", |report| {
        compare_facilities(frame, pack, countries, tokens, report)
    });
    use replay_data::rows::TableId;
    let [programs, projects, history, breakthroughs] = [
        TableId::ProjectPrograms,
        TableId::Projects,
        TableId::ProjectHistory,
        TableId::ProjectBreakthroughs,
    ]
    .map(|table| {
        let current = report.refresh_is_current(table, frame.hour);
        if !current {
            report.limitation(
                &format!("Projects {}", replay_data::cadence::STAGGER_LIMITATION),
                format!(
                    "{table:?} refreshes on {} (oracle hour {})",
                    replay_data::cadence::refresh_text(table),
                    frame.hour
                ),
            );
        }
        current
    });
    succeeded &= report.attempt("projects ProjectPrograms", |report| {
        if !programs {
            return Ok(());
        }
        compare_programs(frame, pack, countries, tokens, report)
    });
    succeeded &= report.attempt("projects Projects", |report| {
        if !projects {
            return Ok(());
        }
        compare_projects(frame, pack, countries, tokens, report)
    });
    succeeded &= report.attempt("projects ProjectHistory", |report| {
        if !history {
            return Ok(());
        }
        compare_history(frame, pack, countries, tokens, report)
    });
    succeeded &= report.attempt("projects ProjectBreakthroughs", |report| {
        if !breakthroughs {
            return Ok(());
        }
        compare_breakthroughs(frame, pack, countries, tokens, report)
    });
    succeeded &= report.attempt("projects FacilityMapFlags", |report| {
        compare_facility_flags(frame, pack, report)
    });
    succeeded &= report.attempt("projects RaidCountries", |report| {
        compare_raid_countries(frame, pack, countries, tokens, report)
    });
    succeeded &= report.attempt("projects Raids", |report| {
        compare_raids(frame, pack, countries, tokens, report)
    });
    succeeded &= report.attempt("projects RaidEquipment", |report| {
        compare_raid_equipment(frame, pack, report)
    });
    succeeded &= report.attempt("projects RaidSpeedInputs", |report| {
        compare_raid_speed_inputs(frame, pack, report)
    });
    succeeded &= report.attempt("projects RaidPlayerAllowedTypes", |report| {
        compare_raid_player(frame, report)
    });
    succeeded &= report.attempt("projects RaidTargets", |report| {
        compare_raid_targets(frame, pack, tokens, report)
    });
    succeeded &= report.attempt("projects RaidCooldowns", |report| {
        compare_raid_cooldowns(frame, pack, tokens, report)
    });
    succeeded &= report.attempt("projects RaidTargetStatuses", |report| {
        compare_raid_target_statuses(frame, pack, tokens, report)
    });
    succeeded &= report.attempt("projects PeaceConferences", |report| {
        compare_conferences(frame, pack, report)
    });
    report.limitation("saved country tags cannot distinguish native country-reference aliases; omitted signed-nonpositive raid handles do not preserve their exact raw sentinel", "project, nuke and raid country joins");
    if succeeded {
        Ok(())
    } else {
        Err(invalid(
            "project comparisons incomplete; see individual table failures",
        ))
    }
}

pub fn compare_scientists(
    frame: &DecodedFrame,
    characters: &characters_saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let manager = characters
        .manager
        .as_ref()
        .ok_or_else(|| invalid("save lacks the character manager roster"))?;
    let mut expected = BTreeMap::new();
    for (&handle, character) in &manager.characters {
        let Some(scientist) = &character.scientist else {
            continue;
        };
        if handle as u32 != 73 {
            return Err(invalid("saved character handle is not a type-73 reference"));
        }
        if expected
            .insert((handle >> 32) as u32, (character, scientist))
            .is_some()
        {
            return Err(invalid("duplicate saved scientist character id"));
        }
    }
    let mut recorded = BTreeMap::new();
    for row in complete::<tables::Scientists>(frame)? {
        if recorded.insert(row.character, row).is_some() {
            return Err(invalid("duplicate recorded scientist character id"));
        }
    }
    let daily = if report.full_refresh()
        || report.refresh_is_current(replay_data::rows::TableId::ScientistDaily, frame.hour)
    {
        Some(complete::<tables::ScientistDaily>(frame)?)
    } else {
        None
    };
    let mut skills = BTreeMap::<u32, Vec<_>>::new();
    for row in complete::<tables::ScientistSkills>(frame)? {
        let records = skills.entry(row.character).or_default();
        if row.ordinal as usize != records.len() {
            return Err(invalid(
                "recorded scientist specialization ordinals are not contiguous from zero",
            ));
        }
        records.push(row);
    }
    if let Some(character) = skills.keys().find(|id| !recorded.contains_key(id)) {
        return Err(invalid(format!(
            "recorded specialization records reference unrostered scientist {character}"
        )));
    }
    let mut compared = 0;
    let mut compared_skills = 0;
    let mut nonpositive_levels = 0;
    let mut nonpositive_experience = 0;
    for (&id, actual) in &recorded {
        let label = format!("scientist {id}");
        let Some((character, saved)) = expected.get(&id) else {
            report.difference(&label, "present", "absent or not a scientist");
            continue;
        };
        compared += 1;
        if let Some(daily) = daily {
            match daily
                .iter()
                .find(|row| row.character == id && row.country == actual.country)
            {
                Some(observation) => {
                    if let Some(injury) = report.recorded(
                        &observation.injury_days,
                        "ScientistDaily.injury_days",
                        format!("{label}.injury days"),
                    ) {
                        match injury {
                            Some(days) => equal(
                                report,
                                format!("{label}.injury days"),
                                (*days > 0).then_some(*days),
                                saved.injury_days,
                            ),
                            None => report.difference(
                                format!("{label}.injury days"),
                                "capture refused",
                                saved.injury_days,
                            ),
                        }
                    }
                }
                None => report.difference(
                    format!("{label}.daily observation"),
                    "absent or different owner",
                    "present",
                ),
            }
        }
        if let Some(active) = report.recorded(
            &actual.active,
            "Scientists.active",
            format!("{label}.active"),
        ) {
            match active {
                Some(active) => {
                    equal(
                        report,
                        format!("{label}.active"),
                        *active != 0,
                        saved.active,
                    );
                    if *active > 1 {
                        report.limitation("the scientist save writer saves the active flag as Boolean truth; nonzero byte magnitude is not serialized", format!("{label}.active"));
                    }
                }
                None => {
                    report.difference(format!("{label}.active"), "capture refused", saved.active)
                }
            }
        }
        equal(
            report,
            format!("{label}.roster country"),
            country_tag(actual.country, links)?,
            character.country.as_deref(),
        );
        equal(
            report,
            format!("{label}.trait_count"),
            actual.trait_count,
            saved
                .traits
                .as_ref()
                .map_or(0, |traits| traits.len() as i32),
        );
        equal(
            report,
            format!("{label}.skill_count"),
            actual.skill_count,
            saved.skills.len() as i32,
        );
        let records = skills.get(&id).map(Vec::as_slice).unwrap_or_default();
        equal(
            report,
            format!("{label}.specialization records"),
            records.len(),
            actual.skill_count.max(0) as usize,
        );
        for (ordinal, expected_skill) in saved.skills.iter().enumerate() {
            let label = format!("{label}.specialization[{ordinal}]");
            let Some(actual) = records.get(ordinal) else {
                report.difference(label, "absent", &expected_skill.name);
                continue;
            };
            compare_token(report, &label, tokens, actual.token, &expected_skill.name);
            match expected_skill.level {
                Some(level) => equal(report, format!("{label}.level"), actual.level, level),
                None if actual.level > 0 => report.difference(
                    format!("{label}.level"),
                    actual.level,
                    "omitted nonpositive level",
                ),
                None => nonpositive_levels += 1,
            }
            match &expected_skill.experience {
                Some(experience) => equal(
                    report,
                    format!("{label}.experience"),
                    actual.experience_raw,
                    experience.raw,
                ),
                None if actual.experience_raw > 0 => report.difference(
                    format!("{label}.experience"),
                    actual.experience_raw,
                    "omitted nonpositive experience",
                ),
                None => nonpositive_experience += 1,
            }
            compared_skills += 1;
        }
        for (ordinal, extra) in records.iter().enumerate().skip(saved.skills.len()) {
            report.difference(
                format!("{label}.specialization[{ordinal}]"),
                extra.token,
                "absent",
            );
        }
    }
    for (&id, (character, _)) in &expected {
        if character.country.is_some() && !recorded.contains_key(&id) {
            report.difference(format!("scientist {id}"), "absent", "present");
        }
    }
    if nonpositive_levels != 0 || nonpositive_experience != 0 {
        report.limitation(
            "the game emits a specialization level and its experience only while each is strictly positive; an omitted field proves a nonpositive raw value but cannot distinguish zero from a negative one",
            format!(
                "{nonpositive_levels} omitted specialization levels and {nonpositive_experience} omitted experience values"
            ),
        );
    }
    report.limitation(
        "no writer serializes the country scientist roster itself, so its native membership order is recorded but has no saved counterpart; saved scientists without a positive country reference cannot belong to a roster and are not required to be recorded",
        "Scientists.ordinal and unowned saved scientists",
    );
    report.compared("Scientists", compared);
    report.compared("ScientistSkills", compared_skills);
    Ok(())
}

fn compare_nukes(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::Nukes>(frame)? {
        let tag = required_country_tag(row.country, countries)?;
        let entries = pack
            .countries
            .get(tag)
            .and_then(|country| country.nukes.as_ref())
            .ok_or_else(|| invalid("save lacks nuclear stockpile array"))?;
        if entries.len() != 2 {
            return Err(invalid(
                "saved nuclear stockpile does not contain both native entries",
            ));
        }
        let expected = entries
            .get(row.nuke_type as usize)
            .ok_or_else(|| invalid("captured nuke type outside native two-entry array"))?;
        if !seen.insert((tag.to_vec(), row.nuke_type)) {
            return Err(invalid("duplicate resolved nuclear stockpile key"));
        }
        let label = format!(
            "Nukes {} type {}",
            String::from_utf8_lossy(tag),
            row.nuke_type
        );
        equal(
            report,
            format!("{label}.amount_raw"),
            row.amount_raw,
            expected.amount.raw,
        );
        equal(
            report,
            format!("{label}.ready_launches_this_hour"),
            row.ready,
            expected.ready,
        );
        count += 1;
    }
    for (tag, country) in report.canonical_countries(&pack.countries) {
        let entries = country
            .nukes
            .as_ref()
            .ok_or_else(|| invalid("save country lacks nuclear stockpile array"))?;
        if entries.len() != 2 {
            return Err(invalid("saved nuclear stockpile array has wrong extent"));
        }
        for ordinal in 0..2 {
            if !seen.contains(&(tag.clone(), ordinal)) {
                missing(report, "Nukes", (tag, ordinal), "absent from replay");
            }
        }
    }
    report.compared("Nukes", count);
    Ok(())
}

fn compare_facilities(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut expected = BTreeMap::new();
    for (tag, country) in report.canonical_countries(&pack.countries) {
        let status = country
            .status
            .as_ref()
            .ok_or_else(|| invalid("save country lacks program status"))?;
        for (ordinal, id) in status.program_order.iter().enumerate() {
            let program = status
                .programs
                .get(id)
                .ok_or_else(|| invalid("saved program order references absent program"))?;
            expected.insert((tag.as_slice(), ordinal as u32), program);
        }
    }
    let rows = complete::<tables::RaidFacilities>(frame)?;
    for row in rows {
        let tag = required_country_tag(row.country, countries)?;
        let key = (tag, row.ordinal);
        let Some(saved) = expected.remove(&key) else {
            missing(report, "RaidFacilities", key, "absent from save");
            continue;
        };
        let label = format!("RaidFacilities {}:{}", row.country, row.ordinal);
        equal(
            report,
            format!("{label}.id"),
            (u64::from(row.id) << 32)
                | u64::from(engine_projects::nproject_cprogram::REFERENCE_TYPE),
            saved.id,
        );
        equal(
            report,
            format!("{label}.owner"),
            Some(required_country_tag(row.owner, countries)?),
            saved.country.as_deref(),
        );
        equal(
            report,
            format!("{label}.province"),
            row.province,
            saved.province,
        );
        compare_optional_token(
            report,
            &format!("{label}.active_project"),
            tokens,
            row.active_project,
            saved.active_project.as_deref(),
        );
        let owner_tag = required_country_tag(row.owner, countries)?;
        let definitions = &status(pack, owner_tag)?.projects;
        let definition = saved.active_project.as_deref().filter(|key| {
            definitions
                .values()
                .any(|project| project.definition == *key)
        });
        compare_optional_token(
            report,
            &format!("{label}.project_definition"),
            tokens,
            row.project_definition,
            definition,
        );
    }
    for key in expected.keys() {
        missing(report, "RaidFacilities", key, "absent from replay");
    }
    report.compared("RaidFacilities", rows.len());
    Ok(())
}

fn compare_programs(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut expected = BTreeMap::new();
    for (_, country) in report.canonical_countries(&pack.countries) {
        let status = country
            .status
            .as_ref()
            .ok_or_else(|| invalid("save country lacks project program status"))?;
        for (&id, program) in &status.programs {
            if let Some(previous) = expected.insert(id, program)
                && previous != program
            {
                return Err(invalid("conflicting program across country status rosters"));
            }
        }
    }
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::ProjectPrograms>(frame)? {
        let id = (u64::from(row.id) << 32) | 83;
        if !seen.insert(id) {
            return Err(invalid("duplicate project program"));
        }
        let Some(saved) = expected.get(&id) else {
            missing(report, "ProjectPrograms", id, "absent from save");
            continue;
        };
        let label = format!("ProjectProgram {id}");
        equal(
            report,
            format!("{label}.country"),
            Some(required_country_tag(row.owner, countries)?),
            saved.country.as_deref(),
        );
        equal(
            report,
            format!("{label}.province"),
            row.province,
            saved.province,
        );
        compare_optional_token(
            report,
            &format!("{label}.active_project"),
            tokens,
            row.active_project,
            saved.active_project.as_deref(),
        );
        equal(
            report,
            format!("{label}.scientist"),
            row.scientist,
            saved.scientist.unwrap_or(0),
        );
        equal(
            report,
            format!("{label}.building"),
            row.building.as_slice(),
            saved.building.as_slice(),
        );
        equal(
            report,
            format!("{label}.unread_rewards"),
            row.unread_rewards,
            saved.unread_rewards,
        );
        if let Some(assignments) = report.recorded(
            &row.assignments,
            "ProjectPrograms.assignments",
            format!("{label}.assignments"),
        ) {
            match assignments {
                Some(assignments) => equal(
                    report,
                    format!("{label}.assignments"),
                    Some(
                        assignments
                            .iter()
                            .map(|assignment| {
                                (
                                    assignment.scientist,
                                    assignment.target,
                                    assignment.current,
                                    assignment.timed != 0,
                                )
                            })
                            .collect::<Vec<_>>(),
                    ),
                    Some(
                        saved
                            .assignments
                            .iter()
                            .map(|assignment| {
                                (
                                    assignment.scientist,
                                    assignment.target,
                                    assignment.current,
                                    assignment.timed,
                                )
                            })
                            .collect::<Vec<_>>(),
                    ),
                ),
                None => report.difference(
                    format!("{label}.assignments"),
                    "capture refused",
                    &saved.assignments,
                ),
            }
        }
        count += 1;
    }
    for id in expected.keys() {
        if !seen.contains(id) {
            missing(report, "ProjectPrograms", id, "absent from replay");
        }
    }
    report.compared("ProjectPrograms", count);
    Ok(())
}

fn compare_projects(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::Projects>(frame)? {
        let tag = required_country_tag(row.owner, countries)?;
        let id = (u64::from(row.local_id) << 32) | 86;
        if !seen.insert((tag.to_vec(), id)) {
            return Err(invalid("duplicate scoped project handle"));
        }
        let Some(saved) = status(pack, tag)?.projects.get(&id) else {
            missing(report, "Projects", (tag, id), "absent from save");
            continue;
        };
        let label = format!("Project {} {id}", String::from_utf8_lossy(tag));
        compare_token(
            report,
            &format!("{label}.definition"),
            tokens,
            row.definition_id,
            saved.definition.as_slice(),
        );
        equal(
            report,
            format!("{label}.program"),
            row.program_reference,
            saved.program.unwrap_or(0),
        );
        equal(
            report,
            format!("{label}.started"),
            row.started != 0,
            saved.started,
        );
        equal(
            report,
            format!("{label}.completed"),
            row.completed != 0,
            saved.completed,
        );
        let index = match row.state.current_offset {
            0x18 => 0,
            0xe8 => 1,
            0x168 => 2,
            0x1e8 => 3,
            _ => {
                return Err(invalid(
                    "project current state offset outside inline states",
                ));
            }
        };
        equal(
            report,
            format!("{label}.current_state"),
            row.state.state_enums[index] as i32,
            saved.current_state,
        );
        equal(
            report,
            format!("{label}.prototype_progress"),
            row.state.prototype_progress,
            saved.prototype_progress,
        );
        equal(
            report,
            format!("{label}.iterations"),
            row.state.iterations,
            saved.iterations,
        );
        match index {
            0 => equal(report, format!("{label}.phase_progress"), row.state.phase_progress, saved.prototype_phase_progress.raw),
            2 => equal(report, format!("{label}.phase_progress"), row.state.phase_progress, saved.stopping_phase_progress.raw),
            _ => report.limitation("project serializer saves prototype and stopping progress only; active first/third simple-state progress has no saved counterpart", label.clone()),
        }
        if let Some(runtime) =
            report.recorded(&row.runtime, "Projects.runtime", format!("{label}.runtime"))
        {
            if runtime.pending_known {
                compare_optional_token(
                    report,
                    &format!("{label}.pending reward"),
                    tokens,
                    runtime.pending_reward,
                    saved.pending_reward.as_deref(),
                );
            } else {
                report.difference(
                    format!("{label}.pending reward"),
                    "capture refused",
                    &saved.pending_reward,
                );
            }
            if let Some(variables) = &runtime.variables {
                equal(
                    report,
                    format!("{label}.variable seed"),
                    variables.seed,
                    saved.variables.seed.unwrap_or(1),
                );
                let mut actual = variables
                    .entries
                    .iter()
                    .map(|entry| {
                        (
                            content_reader_parser::parser::c_string(&entry.name),
                            entry.value_raw,
                        )
                    })
                    .collect::<Vec<_>>();
                let mut expected = saved
                    .variables
                    .entries
                    .iter()
                    .map(|entry| (entry.name.as_slice(), entry.value.raw))
                    .collect::<Vec<_>>();
                actual.sort();
                expected.sort();
                equal(report, format!("{label}.variables"), actual, expected);
                if saved.variables.reserved_entries != 0 {
                    return Err(invalid(
                        "project variable reserved-name collision cannot establish complete coverage",
                    ));
                }
            } else {
                report.difference(
                    format!("{label}.variables"),
                    "capture refused",
                    &saved.variables,
                );
            }
        }
        count += 1;
    }
    for (tag, country) in report.canonical_countries(&pack.countries) {
        let status = country
            .status
            .as_ref()
            .ok_or_else(|| invalid("save country lacks project status"))?;
        for id in status.projects.keys() {
            if !seen.contains(&(tag.clone(), *id)) {
                missing(report, "Projects", (tag, id), "absent from replay");
            }
        }
    }
    report.compared("Projects", count);
    report.limitation("project saves do not directly preserve current pointer offset, inactive state enum words, not_completed byte, or whole-unit phase target; started/completed serialize truth rather than noncanonical byte bits", "Projects");
    Ok(())
}

fn compare_history(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::ProjectHistory>(frame)? {
        let tag = required_country_tag(row.owner, countries)?;
        let id = (u64::from(row.project) << 32) | 86;
        let key = (tag.to_vec(), id, row.ordinal);
        if !seen.insert(key.clone()) {
            return Err(invalid("duplicate scoped project history ordinal"));
        }
        let Some(saved) = status(pack, tag)?
            .projects
            .get(&id)
            .and_then(|project| project.history.get(row.ordinal as usize))
        else {
            missing(report, "ProjectHistory", key, "absent from save");
            continue;
        };
        let label = format!(
            "ProjectHistory {} {id}/{}",
            String::from_utf8_lossy(tag),
            row.ordinal
        );
        compare_token(
            report,
            &format!("{label}.reward"),
            tokens,
            row.prototype_reward_handle,
            saved.reward.as_slice(),
        );
        compare_token(
            report,
            &format!("{label}.option"),
            tokens,
            row.option_handle,
            saved.option.as_slice(),
        );
        equal(
            report,
            format!("{label}.date"),
            row.date,
            saved.date.engine_hour()?,
        );
        equal(
            report,
            format!("{label}.province"),
            row.context.province,
            saved.context.province,
        );
        equal(
            report,
            format!("{label}.progress"),
            row.context.progress,
            saved.context.progress,
        );
        for (field, presence, actual, expected) in [
            (
                "country",
                row.context.country_present,
                row.context.country_reference,
                saved.context.country,
            ),
            (
                "scientist",
                row.context.scientist_present,
                row.context.scientist_reference,
                saved.context.scientist,
            ),
            (
                "state",
                row.context.state_present,
                row.context.state_reference,
                saved.context.state,
            ),
        ] {
            if (presence != 0) != actual.is_some() {
                return Err(invalid(
                    "project context payload disagrees with native presence",
                ));
            }
            equal(report, format!("{label}.{field}"), actual, expected);
        }
        if let Some(character) = saved.context.character {
            equal(
                report,
                format!("{label}.character"),
                row.context.character_reference,
                character,
            );
        } else {
            report.limitation("project history character omission means null or unresolvable; no saved value proves the raw reference", label);
        }
        count += 1;
    }
    for (tag, country) in report.canonical_countries(&pack.countries) {
        let status = country
            .status
            .as_ref()
            .ok_or_else(|| invalid("save country lacks project status"))?;
        for (&id, project) in &status.projects {
            for ordinal in 0..project.history.len() {
                let ordinal = u32::try_from(ordinal)
                    .map_err(|_| invalid("saved history ordinal overflow"))?;
                if !seen.contains(&(tag.clone(), id, ordinal)) {
                    missing(
                        report,
                        "ProjectHistory",
                        (tag, id, ordinal),
                        "absent from replay",
                    );
                }
            }
        }
    }
    report.compared("ProjectHistory", count);
    report.limitation("project context saves encode presence as field emission and cannot distinguish noncanonical nonzero presence bytes", "ProjectHistory");
    Ok(())
}

fn compare_breakthroughs(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut country_counts = BTreeMap::<Vec<u8>, usize>::new();
    let mut unresolved = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::ProjectBreakthroughs>(frame)? {
        let tag = required_country_tag(row.owner, countries)?;
        if !keys.insert((tag.to_vec(), row.specialization)) {
            return Err(invalid("duplicate raw breakthrough specialization"));
        }
        *country_counts.entry(tag.to_vec()).or_default() += 1;
        let label = format!(
            "ProjectBreakthrough {} specialization",
            String::from_utf8_lossy(tag)
        );
        let Some(name) = token(tokens, row.specialization, report, &label) else {
            unresolved.insert(tag.to_vec());
            report.limitation("breakthrough points cannot be matched without the specialization's saved literal; raw points are not paired by ordinal or value", format!("{} token {}", String::from_utf8_lossy(tag), row.specialization));
            continue;
        };
        if !seen.insert((tag.to_vec(), name.to_vec())) {
            return Err(invalid("duplicate resolved breakthrough specialization"));
        }
        let Some(saved) = status(pack, tag)?.breakthroughs.get(name) else {
            missing(
                report,
                "ProjectBreakthroughs",
                (tag, name),
                "absent from save",
            );
            continue;
        };
        equal(
            report,
            format!(
                "ProjectBreakthrough {} {}",
                String::from_utf8_lossy(tag),
                String::from_utf8_lossy(name)
            ),
            row.points,
            *saved,
        );
        count += 1;
    }
    for (tag, country) in report.canonical_countries(&pack.countries) {
        let status = country
            .status
            .as_ref()
            .ok_or_else(|| invalid("save country lacks project status"))?;
        equal(
            report,
            format!(
                "ProjectBreakthroughs {} count",
                String::from_utf8_lossy(tag)
            ),
            country_counts.get(tag).copied().unwrap_or(0),
            status.breakthroughs.len(),
        );
        for name in status.breakthroughs.keys() {
            if !seen.contains(&(tag.clone(), name.clone())) {
                if unresolved.contains(tag) {
                    report.limitation("saved breakthrough membership cannot be matched while captured specialization literals are unavailable or their text emission is unproved", format!("{} {}", String::from_utf8_lossy(tag), String::from_utf8_lossy(name)));
                } else {
                    missing(
                        report,
                        "ProjectBreakthroughs",
                        (tag, name),
                        "absent from replay",
                    );
                }
            }
        }
    }
    report.compared("ProjectBreakthroughs", count);
    Ok(())
}

mod raids;
#[cfg(test)]
use raids::{PathVerdict, compare_scope, path_verdict};
pub use raids::{RaidGraph, compare_raid_paths, compare_raid_tooltip_operands};
use raids::{
    compare_raid_cooldowns, compare_raid_countries, compare_raid_equipment, compare_raid_player,
    compare_raid_speed_inputs, compare_raid_target_statuses, compare_raid_targets, compare_raids,
};

fn reconstructed_turn(saved: &saved::Conference) -> Option<i32> {
    let first = saved.history_turns.values().next().copied().unwrap_or(0);
    saved
        .history_turns
        .values()
        .all(|&count| count == first)
        .then_some(first)
}
fn compare_facility_flags(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let programs: BTreeSet<u32> = pack
        .countries
        .values()
        .filter_map(|country| country.status.as_ref())
        .flat_map(|status| status.programs.values().map(|program| program.province))
        .filter(|&province| province != 0)
        .collect();
    let rows = complete::<tables::FacilityMapFlags>(frame)?;
    for row in rows {
        if !programs.contains(&row.province) {
            report.difference(
                format!("FacilityMapFlag {}", row.province),
                "flagged",
                "no saved program at the province",
            );
        }
    }
    let unflagged = programs
        .iter()
        .filter(|&&province| {
            rows.binary_search_by_key(&province, |row| row.province)
                .is_err()
        })
        .count();
    report.compared("FacilityMapFlags", rows.len());
    report.limitation(
        "the flag is client map state the save does not hold; only its program membership is compared",
        format!("FacilityMapFlags: {unflagged} saved program provinces unflagged"),
    );
    Ok(())
}

fn compare_conferences(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let expected = pack
        .conferences
        .as_ref()
        .ok_or_else(|| invalid("save lacks peace conference manager"))?;
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::PeaceConferences>(frame)? {
        if !seen.insert(row.id) {
            return Err(invalid("duplicate peace conference full reference"));
        }
        let Some(saved) = expected.get(&row.id) else {
            missing(report, "PeaceConferences", row.id, "absent from save");
            continue;
        };
        let label = format!("PeaceConference {}", row.id);
        equal(
            report,
            format!("{label}.ordinal"),
            row.ordinal,
            saved.ordinal,
        );
        equal(
            report,
            format!("{label}.finalized"),
            row.finalized != 0,
            saved.finalized,
        );
        if let Some(turn) = reconstructed_turn(saved) {
            equal(
                report,
                format!("{label}.turn_from_saved_history"),
                row.current_turn,
                turn,
            );
        } else {
            report.limitation("peace turn is rebuilt from the first native history-map node; unequal saved counts and tag-only keys do not identify that native node", label);
        }
        count += 1;
    }
    for id in expected.keys() {
        if !seen.contains(id) {
            missing(report, "PeaceConferences", id, "absent from replay");
        }
    }
    report.compared("PeaceConferences", count);
    report.limitation("peace current turn is reconstructed from bidding history, not a directly serialized scalar; completed saves truth rather than noncanonical finalized-byte bits; intermediate paused-hour turns and removed conferences are not event history", "PeaceConferences");
    Ok(())
}

#[cfg(test)]
mod tests;
