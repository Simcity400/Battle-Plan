//! Exact comparisons for saved project, nuclear, raid and conference fields.
use super::{Report, complete, country_tag, invalid, required_country_tag};
use installed_content::save::{characters as characters_saved, projects as saved};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, tables},
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub(super) type TokenNames<'a> = BTreeMap<u32, &'a [u8]>;

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
    if value == 0x167 {
        report.limitation("24AAC50 formats reserved lexical token 0x167 as fixed15 numeric text; its raw dictionary literal cannot certify this saved field", label);
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

pub(super) fn compare(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut succeeded = report.attempt("projects Nukes", |report| {
        compare_nukes(frame, pack, countries, report)
    });
    succeeded &= report.attempt("projects ProjectPrograms", |report| {
        compare_programs(frame, pack, countries, tokens, report)
    });
    succeeded &= report.attempt("projects Projects", |report| {
        compare_projects(frame, pack, countries, tokens, report)
    });
    succeeded &= report.attempt("projects ProjectHistory", |report| {
        compare_history(frame, pack, countries, tokens, report)
    });
    succeeded &= report.attempt("projects ProjectBreakthroughs", |report| {
        compare_breakthroughs(frame, pack, countries, tokens, report)
    });
    succeeded &= report.attempt("projects RaidCountries", |report| {
        compare_raid_countries(frame, pack, countries, report)
    });
    succeeded &= report.attempt("projects Raids", |report| {
        compare_raids(frame, pack, countries, report)
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

/// Scientist roster membership and its specialization records.
///
/// The roster itself is `CCountry +0xff0`, which no writer serializes: the save
/// carries a scientist only as the `scientist={...}` block of its own
/// `character={...}`, whose `country=` is `CCharacter +0x68`. F913A0 emits that
/// tag only while the raw reference is strictly positive, so a saved scientist
/// without one has no country at all and cannot be in a country roster. Every
/// recorded roster entry is therefore compared against its saved character, and
/// every saved scientist that does carry a country is required to be recorded.
pub(super) fn compare_scientists(
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
        // The roster country and CCharacter +0x68 are two different native
        // fields; the save carries only the second one.
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
            // 14144E480 guards the record's i64 at +0x10 the same way.
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
            "14144E480 emits a specialization level and its experience only while each is strictly positive; an omitted field proves a nonpositive raw value but cannot distinguish zero from a negative one",
            format!(
                "{nonpositive_levels} omitted specialization levels and {nonpositive_experience} omitted experience values"
            ),
        );
    }
    report.limitation(
        "no writer serializes the CCountry +0xff0 scientist roster itself, so its native membership order is recorded but has no saved counterpart; saved scientists without a positive country reference cannot belong to a roster and are not required to be recorded",
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
    for (tag, country) in &pack.countries {
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

fn compare_programs(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut expected = BTreeMap::new();
    for country in pack.countries.values() {
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
        count += 1;
    }
    for id in expected.keys() {
        if !seen.contains(id) {
            missing(report, "ProjectPrograms", id, "absent from replay");
        }
    }
    report.compared("ProjectPrograms", count);
    report.limitation("program scientist is the primary reference only; the separately saved scientist history/list and timers are outside this table", "ProjectPrograms");
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
        count += 1;
    }
    for (tag, country) in &pack.countries {
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
    for (tag, country) in &pack.countries {
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
    for (tag, country) in &pack.countries {
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

fn compare_raid_countries(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let expected = pack
        .raids
        .as_ref()
        .ok_or_else(|| invalid("save lacks raids root; absence does not certify empty graph"))?;
    let mut parents = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::RaidCountries>(frame)? {
        if !parents.insert(row.canonical_slot) {
            return Err(invalid("duplicate raid parent slot"));
        }
        let Some(saved) = expected.get(row.canonical_slot as usize) else {
            missing(
                report,
                "RaidCountries",
                row.canonical_slot,
                "absent from save",
            );
            continue;
        };
        let label = format!("RaidCountry {}", row.canonical_slot);
        equal(
            report,
            format!("{label}.country"),
            signed_country(row.country, countries)?,
            saved.country.as_deref(),
        );
        equal(
            report,
            format!("{label}.priority"),
            row.priority,
            saved.priority,
        );
        equal(
            report,
            format!("{label}.child_count"),
            u64::from(row.child_count),
            saved.instances.len() as u64,
        );
        count += 1;
    }
    for parent in expected {
        if !parents.contains(&parent.ordinal) {
            missing(
                report,
                "RaidCountries",
                parent.ordinal,
                "absent from replay",
            );
        }
    }
    report.compared("RaidCountries", count);
    Ok(())
}

fn compare_raids(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    countries: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let expected = pack
        .raids
        .as_ref()
        .ok_or_else(|| invalid("save lacks raids root; absence does not certify empty graph"))?;
    let mut children = BTreeSet::new();
    let mut count = 0;
    let expected_children: BTreeMap<_, _> = expected
        .iter()
        .flat_map(|parent| {
            parent
                .instances
                .iter()
                .enumerate()
                .map(move |(ordinal, raid)| (raid.id, (parent.ordinal, ordinal, raid)))
        })
        .collect();
    for row in complete::<tables::Raids>(frame)? {
        let id = (u64::from(row.id) << 32) | u64::from(row.reference_type);
        if !children.insert(id) {
            return Err(invalid("duplicate raid full reference"));
        }
        let Some((parent, ordinal, saved)) = expected_children.get(&id) else {
            missing(report, "Raids", id, "absent from save");
            continue;
        };
        let label = format!("Raid {id}");
        equal(report, format!("{label}.parent"), row.parent_slot, *parent);
        equal(
            report,
            format!("{label}.ordinal"),
            u64::from(row.ordinal),
            *ordinal as u64,
        );
        equal(
            report,
            format!("{label}.preparation_hours"),
            row.preparation_hours,
            saved.preparation_hours,
        );
        equal(report, format!("{label}.phase"), row.phase, saved.phase);
        equal(
            report,
            format!("{label}.outcome"),
            row.outcome,
            saved.outcome,
        );
        equal(
            report,
            format!("{label}.end_date_present"),
            row.end_date_present,
            u8::from(saved.end_date.is_some()),
        );
        equal(
            report,
            format!("{label}.victim"),
            signed_country(row.victim_country, countries)?,
            saved.victim_country.as_deref(),
        );
        if row.source.kind_present > 1
            || (row.source.kind_present != 0) != row.source.kind.is_some()
        {
            return Err(invalid("raid source kind payload disagrees with presence"));
        }
        equal(
            report,
            format!("{label}.source_kind"),
            row.source.kind,
            saved.source.as_ref().map(|source| source.kind),
        );
        if let Some(source) = &saved.source {
            if source.province == Some(0) {
                return Err(invalid(
                    "saved raid source has zero id for nonnull province pointer",
                ));
            }
            equal(
                report,
                format!("{label}.source_province"),
                row.source.province,
                source.province.unwrap_or(0),
            );
            equal(
                report,
                format!("{label}.source_ship"),
                (u64::from(row.source.ship_reference_id) << 32)
                    | u64::from(row.source.ship_reference_type),
                source.ship.unwrap_or(0),
            );
            equal(
                report,
                format!("{label}.source_country"),
                signed_country(row.source.country, countries)?,
                source.country.as_deref(),
            );
        } else {
            report.limitation("absent raid source kind suppresses the whole saved source; province, ship and country raw fields cannot be checked", label);
        }
        count += 1;
    }
    for id in expected_children.keys() {
        if !children.contains(id) {
            missing(report, "Raids", id, "absent from replay");
        }
    }
    report.compared("Raids", count);
    report.limitation("raid rows capture end-date presence only; dummy payloads, target/unit/equipment/building payloads and auxiliary raid arrays are outside this field set", "Raids and RaidCountries");
    Ok(())
}

fn reconstructed_turn(saved: &saved::Conference) -> Option<i32> {
    let first = saved.history_turns.values().next().copied().unwrap_or(0);
    saved
        .history_turns
        .values()
        .all(|&count| count == first)
        .then_some(first)
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
mod tests {
    use super::*;
    use replay::{
        frame::TableView,
        rows::{
            NukeRow, ProjectBreakthroughRow, ProjectContext, ProjectHistoryRow, ProjectProgramRow,
            ProjectRow, ProjectState, RaidCountryRow, RaidRow, RaidSource, TableId, TableRows,
        },
    };

    fn scientist_pack() -> characters_saved::Pack {
        characters_saved::parse(
            &installed_content::parser::parse(
                br#"character_manager={ historical={
                    character={ id={ id=265 type=73 } token="a" country="GER"
                        scientist={ traits={ "t1" "t2" }
                            skills={ specialization_nuclear={ level=2 }
                                     specialization_naval={ experience=219.18 } } } }
                    character={ id={ id=266 type=73 } token="b"
                        scientist={ skills={ specialization_nuclear={ } } } }
                }} countries={}"#,
            )
            .unwrap()
            .items,
        )
        .unwrap()
    }

    fn scientist_frame() -> DecodedFrame {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            }),
            metrics: Default::default(),
        };
        frame.tables[TableId::Scientists.index()] =
            TableView::Complete(TableRows::Scientists(vec![replay::rows::ScientistRow {
                character: 265,
                country: 7,
                ordinal: 0,
                trait_count: 2,
                skill_count: 2,
            }]));
        frame.tables[TableId::ScientistSkills.index()] =
            TableView::Complete(TableRows::ScientistSkills(vec![
                replay::rows::ScientistSkillRow {
                    character: 265,
                    ordinal: 0,
                    token: 0x56c6,
                    level: 2,
                    experience_raw: 0,
                },
                replay::rows::ScientistSkillRow {
                    character: 265,
                    ordinal: 1,
                    token: 0x56c7,
                    level: 0,
                    experience_raw: 21_918_000,
                },
            ]));
        frame
    }

    fn run_scientists(frame: &DecodedFrame) -> Report {
        let germany = CountryRow {
            id: 7,
            canonical_slot: 1,
            tag: b"GER".to_vec(),
            ..CountryRow::default()
        };
        let links = BTreeMap::from([(7, &germany)]);
        let tokens: TokenNames<'_> = BTreeMap::from([
            (0x56c6, b"specialization_nuclear".as_slice()),
            (0x56c7, b"specialization_naval"),
        ]);
        let mut report = Report::default();
        compare_scientists(frame, &scientist_pack(), &links, &tokens, &mut report).unwrap();
        report
    }

    #[test]
    fn a_matching_scientist_roster_compares_counts_tokens_and_levels() {
        let report = run_scientists(&scientist_frame());
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["Scientists"], 1);
        assert_eq!(report.compared_tables["ScientistSkills"], 2);
        // The naval record's omitted level proves only a nonpositive value.
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("14144E480"))
        );
    }

    #[test]
    fn count_token_and_level_disagreements_are_each_reported() {
        for (mutate, expected) in [
            (
                (|frame: &mut DecodedFrame| {
                    let TableView::Complete(TableRows::Scientists(rows)) =
                        &mut frame.tables[TableId::Scientists.index()]
                    else {
                        unreachable!()
                    };
                    rows[0].trait_count = 1;
                }) as fn(&mut DecodedFrame),
                "trait_count",
            ),
            (
                |frame| {
                    let TableView::Complete(TableRows::ScientistSkills(rows)) =
                        &mut frame.tables[TableId::ScientistSkills.index()]
                    else {
                        unreachable!()
                    };
                    rows[0].level = 3;
                },
                "level",
            ),
            (
                |frame| {
                    let TableView::Complete(TableRows::ScientistSkills(rows)) =
                        &mut frame.tables[TableId::ScientistSkills.index()]
                    else {
                        unreachable!()
                    };
                    rows[1].level = 1;
                },
                "level",
            ),
            (
                |frame| {
                    let TableView::Complete(TableRows::ScientistSkills(rows)) =
                        &mut frame.tables[TableId::ScientistSkills.index()]
                    else {
                        unreachable!()
                    };
                    rows[1].token = 0x56c6;
                },
                "specialization[1]",
            ),
            (
                |frame| {
                    let TableView::Complete(TableRows::ScientistSkills(rows)) =
                        &mut frame.tables[TableId::ScientistSkills.index()]
                    else {
                        unreachable!()
                    };
                    rows[1].experience_raw = 1;
                },
                "experience",
            ),
            (
                |frame| {
                    let TableView::Complete(TableRows::ScientistSkills(rows)) =
                        &mut frame.tables[TableId::ScientistSkills.index()]
                    else {
                        unreachable!()
                    };
                    // A positive experience where the writer omitted the field.
                    rows[0].experience_raw = 5;
                },
                "experience",
            ),
            (
                |frame| {
                    let TableView::Complete(TableRows::Scientists(rows)) =
                        &mut frame.tables[TableId::Scientists.index()]
                    else {
                        unreachable!()
                    };
                    // Zero is the engine's own "no country", which the save omits.
                    rows[0].country = 0;
                },
                "roster country",
            ),
        ] {
            let mut frame = scientist_frame();
            mutate(&mut frame);
            let report = run_scientists(&frame);
            let text = report.differences.join("\n");
            assert!(text.contains(expected), "{expected} missing from {text}");
        }
    }

    #[test]
    fn a_saved_scientist_with_a_country_must_be_recorded_and_an_extra_row_is_reported() {
        // Character 266 has no saved country, so no roster row is required.
        let report = run_scientists(&scientist_frame());
        assert!(report.differences.is_empty());
        // Recording it anyway is compared against its saved character, which
        // has no country tag at all.
        let mut frame = scientist_frame();
        let TableView::Complete(TableRows::Scientists(rows)) =
            &mut frame.tables[TableId::Scientists.index()]
        else {
            unreachable!()
        };
        rows.push(replay::rows::ScientistRow {
            character: 266,
            country: 7,
            ordinal: 1,
            trait_count: 0,
            skill_count: 1,
        });
        let TableView::Complete(TableRows::ScientistSkills(skills)) =
            &mut frame.tables[TableId::ScientistSkills.index()]
        else {
            unreachable!()
        };
        skills.push(replay::rows::ScientistSkillRow {
            character: 266,
            ordinal: 0,
            token: 0x56c6,
            level: 0,
            experience_raw: 0,
        });
        let report = run_scientists(&frame);
        assert!(
            report
                .differences
                .iter()
                .any(|line| line.contains("scientist 266.roster country"))
        );
    }

    #[test]
    fn an_unresolved_roster_country_reference_fails_the_group_instead_of_comparing() {
        let mut frame = scientist_frame();
        let TableView::Complete(TableRows::Scientists(rows)) =
            &mut frame.tables[TableId::Scientists.index()]
        else {
            unreachable!()
        };
        rows[0].country = 9;
        let germany = CountryRow {
            id: 7,
            canonical_slot: 1,
            tag: b"GER".to_vec(),
            ..CountryRow::default()
        };
        let links = BTreeMap::from([(7, &germany)]);
        let mut report = Report::default();
        assert!(
            compare_scientists(
                &frame,
                &scientist_pack(),
                &links,
                &BTreeMap::new(),
                &mut report
            )
            .is_err()
        );
    }

    fn raid_fixture() -> (DecodedFrame, saved::Pack, CountryRow) {
        let mut frame = DecodedFrame {
            branch: 1,
            hour: 0,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            }),
            metrics: Default::default(),
        };
        let source = br"countries={GER={nukes={{amount=90071992547.40993 nukes_ready=-1}{amount=0 nukes_ready=0}} program_status={project_pool={project={}} breakthrough={}}}}
            raids={countries={{tag=GER priority=-1 raid_instance={{id={id=4294967295 type=84} phase=PREPARED prep_time=-2}}}}} peace_conference={}";
        let pack = saved::parse(&installed_content::parser::parse(source).unwrap().items).unwrap();
        frame.tables[TableId::Nukes.index()] = TableView::Complete(TableRows::Nukes(vec![
            NukeRow {
                country: 102,
                nuke_type: 0,
                amount_raw: 9_007_199_254_740_993,
                ready: -1,
            },
            NukeRow {
                country: 102,
                nuke_type: 1,
                amount_raw: 0,
                ready: 0,
            },
        ]));
        frame.tables[TableId::RaidCountries.index()] =
            TableView::Complete(TableRows::RaidCountries(vec![RaidCountryRow {
                canonical_slot: 0,
                country: 102,
                priority: -1,
                child_count: 1,
            }]));
        frame.tables[TableId::Raids.index()] =
            TableView::Complete(TableRows::Raids(vec![RaidRow {
                reference_type: 84,
                id: u32::MAX,
                parent_slot: 0,
                ordinal: 0,
                preparation_hours: -2,
                phase: 3,
                outcome: 0,
                end_date_present: 0,
                victim_country: -1,
                source: RaidSource {
                    kind: None,
                    kind_present: 0,
                    province: 123,
                    ship_reference_type: 51,
                    ship_reference_id: u32::MAX,
                    country: 102,
                },
            }]));
        let country = CountryRow {
            id: 102,
            canonical_slot: 0,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        (frame, pack, country)
    }

    #[test]
    fn comparison_keeps_fixed_precision_and_does_not_certify_omitted_raid_source() {
        let (mut frame, pack, country) = raid_fixture();
        let countries = BTreeMap::from([(102, &country)]);
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("suppresses the whole saved source"))
        );
        let TableView::Complete(TableRows::Nukes(rows)) = &mut frame.tables[TableId::Nukes.index()]
        else {
            unreachable!()
        };
        rows[0].amount_raw -= 1;
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        frame.tables[TableId::Raids.index()] = TableView::Unavailable {
            reason: "source unavailable".into(),
        };
        assert!(
            compare(
                &frame,
                &pack,
                &countries,
                &BTreeMap::new(),
                &mut Report::default()
            )
            .is_err()
        );
    }

    #[test]
    fn unavailable_tables_keep_independent_comparisons_and_differences() {
        let (mut frame, pack, country) = raid_fixture();
        let countries = BTreeMap::from([(102, &country)]);
        for table in [TableId::Nukes, TableId::Projects, TableId::RaidCountries] {
            frame.tables[table.index()] = TableView::Unavailable {
                reason: format!("{table:?} source unavailable"),
            };
        }
        let TableView::Complete(TableRows::Raids(rows)) = &mut frame.tables[TableId::Raids.index()]
        else {
            unreachable!()
        };
        rows[0].preparation_hours += 1;
        let mut report = Report::default();
        assert!(compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).is_err());
        assert_eq!(report.failed_comparisons.len(), 3);
        for table in ["Nukes", "Projects", "RaidCountries"] {
            assert!(
                report
                    .failed_comparisons
                    .contains_key(&format!("projects {table}"))
            );
            assert!(!report.compared_tables.contains_key(table));
        }
        for (table, count) in [
            ("ProjectPrograms", 0),
            ("ProjectHistory", 0),
            ("ProjectBreakthroughs", 0),
            ("Raids", 1),
            ("PeaceConferences", 0),
        ] {
            assert_eq!(report.compared_tables.get(table), Some(&count), "{table}");
        }
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("preparation_hours"));
    }

    #[test]
    fn raid_type_word_changes_membership_and_present_source_changes_fail() {
        let (mut frame, mut pack, country) = raid_fixture();
        let countries = BTreeMap::from([(102, &country)]);
        pack.raids.as_mut().unwrap()[0].instances[0].source = Some(saved::RaidSource {
            kind: 3,
            province: Some(9),
            ship: Some(51),
            country: Some(b"GER".to_vec()),
        });
        let TableView::Complete(TableRows::Raids(rows)) = &mut frame.tables[TableId::Raids.index()]
        else {
            unreachable!()
        };
        let row = &mut rows[0];
        row.source = RaidSource {
            kind: Some(3),
            kind_present: 1,
            province: 9,
            ship_reference_type: 51,
            ship_reference_id: 0,
            country: 102,
        };
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let TableView::Complete(TableRows::Raids(rows)) = &mut frame.tables[TableId::Raids.index()]
        else {
            unreachable!()
        };
        let row = &mut rows[0];
        row.source.province = 10;
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        let TableView::Complete(TableRows::Raids(rows)) = &mut frame.tables[TableId::Raids.index()]
        else {
            unreachable!()
        };
        let row = &mut rows[0];
        row.reference_type ^= 1;
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
    }

    #[test]
    fn missing_tokens_limit_fields_while_presence_census_and_native_values_still_compare() {
        let (mut frame, mut pack, country) = raid_fixture();
        let countries = BTreeMap::from([(102, &country)]);
        let program_id = (7_u64 << 32) | 83;
        let status = pack
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .status
            .as_mut()
            .unwrap();
        status.programs.insert(
            program_id,
            saved::Program {
                id: program_id,
                country: Some(b"GER".to_vec()),
                province: 1,
                active_project: Some(b"definition".to_vec()),
                scientist: None,
                building: b"building".to_vec(),
                unread_rewards: -2,
            },
        );
        status
            .breakthroughs
            .insert(b"specialization".to_vec(), i32::MIN);
        frame.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
                id: 7,
                owner: 102,
                province: 1,
                active_project: Some(100),
                scientist: 0,
                unread_rewards: -2,
                building: b"building".to_vec(),
            }]));
        frame.tables[TableId::ProjectBreakthroughs.index()] =
            TableView::Complete(TableRows::ProjectBreakthroughs(vec![
                ProjectBreakthroughRow {
                    owner: 102,
                    specialization: 101,
                    points: i32::MIN,
                },
            ]));
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .values()
                .flatten()
                .any(|subject| subject.contains("active_project"))
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("points cannot be matched"))
        );
        let TableView::Complete(TableRows::ProjectPrograms(rows)) =
            &mut frame.tables[TableId::ProjectPrograms.index()]
        else {
            unreachable!()
        };
        rows[0].unread_rewards = -1;
        rows[0].active_project = None;
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        let tokens = BTreeMap::from([
            (100, b"definition".as_slice()),
            (101, b"wrong_specialization".as_slice()),
        ]);
        let mut report = Report::default();
        compare_breakthroughs(&frame, &pack, &countries, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        frame.tables[TableId::ProjectBreakthroughs.index()] =
            TableView::Complete(TableRows::ProjectBreakthroughs(Vec::new()));
        let mut report = Report::default();
        compare_breakthroughs(&frame, &pack, &countries, &BTreeMap::new(), &mut report).unwrap();
        assert!(
            report
                .differences
                .iter()
                .any(|difference| difference.contains("count"))
        );
    }

    #[test]
    fn populated_dictionary_checks_project_names_and_breakthrough_points() {
        let (mut frame, mut pack, country) = raid_fixture();
        let countries = BTreeMap::from([(102, &country)]);
        let source = br"countries={GER={program_status={
            program={id={id=7 type=83} country=GER province=1 scientist={}
                building=building unread_prototype_rewards=-2 project=definition}
            project_pool={project={definition={id={id=9 type=86}
                progress={current_state=1 prototype_state={phase_progress=-0.00001 project_progress=-2 iterations=-3} stopping_state={phase_progress=0}}
                start=yes completed=no program={id=7 type=83}
                history={{context={province=-1 progress=-2} option=option prototype_reward=reward date=1936.1.1.1}}
            }}}
            breakthrough={specialization=-2147483648}
        }}}";
        let mut parsed = saved::parse(&installed_content::parser::parse(source).unwrap().items).unwrap();
        pack.countries.get_mut(b"GER".as_slice()).unwrap().status =
            parsed.countries.remove(b"GER".as_slice()).unwrap().status;
        let definition = 0x1234;
        let reward = 0x1235;
        let option = 0x1236;
        let specialization = i32::MAX as u32;
        frame.tables[TableId::ProjectPrograms.index()] =
            TableView::Complete(TableRows::ProjectPrograms(vec![ProjectProgramRow {
                id: 7,
                owner: 102,
                province: 1,
                active_project: Some(definition),
                scientist: 0,
                unread_rewards: -2,
                building: b"building".to_vec(),
            }]));
        frame.tables[TableId::Projects.index()] =
            TableView::Complete(TableRows::Projects(vec![ProjectRow {
                owner: 102,
                local_id: 9,
                definition_id: definition,
                program_reference: (7_u64 << 32) | 83,
                started: 1,
                completed: 0,
                state: ProjectState {
                    current_offset: 0x18,
                    state_enums: [1, 2, 3, 4],
                    not_completed: 1,
                    phase_progress: -1,
                    phase_target: -4,
                    prototype_progress: -2,
                    iterations: -3,
                },
            }]));
        frame.tables[TableId::ProjectHistory.index()] =
            TableView::Complete(TableRows::ProjectHistory(vec![ProjectHistoryRow {
                owner: 102,
                project: 9,
                ordinal: 0,
                prototype_reward_handle: reward,
                option_handle: option,
                date: installed_content::save::GameDate::parse(b"1936.1.1.1")
                    .unwrap()
                    .engine_hour()
                    .unwrap(),
                context: ProjectContext {
                    province: -1,
                    progress: -2,
                    ..Default::default()
                },
            }]));
        frame.tables[TableId::ProjectBreakthroughs.index()] =
            TableView::Complete(TableRows::ProjectBreakthroughs(vec![
                ProjectBreakthroughRow {
                    owner: 102,
                    specialization,
                    points: i32::MIN,
                },
            ]));
        let tokens = BTreeMap::from([
            (definition, b"definition".as_slice()),
            (reward, b"reward".as_slice()),
            (option, b"option".as_slice()),
            (specialization, b"specialization".as_slice()),
        ]);
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            !report
                .limitations
                .keys()
                .any(|reason| reason.contains("token"))
        );
        for table in [
            "ProjectPrograms",
            "Projects",
            "ProjectHistory",
            "ProjectBreakthroughs",
        ] {
            assert_eq!(report.compared_tables.get(table), Some(&1), "{table}");
        }
        for (token, differences, field) in [
            (definition, 2, ".definition"),
            (reward, 1, ".reward"),
            (option, 1, ".option"),
            (specialization, 2, "membership"),
        ] {
            let mut wrong = tokens.clone();
            wrong.insert(token, b"wrong_literal");
            let mut report = Report::default();
            compare(&frame, &pack, &countries, &wrong, &mut report).unwrap();
            assert_eq!(
                report.differences.len(),
                differences,
                "{:?}",
                report.differences
            );
            assert!(
                report
                    .differences
                    .iter()
                    .any(|difference| difference.contains(field))
            );
        }
        let TableView::Complete(TableRows::ProjectBreakthroughs(rows)) =
            &mut frame.tables[TableId::ProjectBreakthroughs.index()]
        else {
            unreachable!()
        };
        rows[0].points += 1;
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("ProjectBreakthrough"));
        let mut limited = tokens.clone();
        limited.insert(option, b"option\0suffix");
        let mut report = Report::default();
        compare(&frame, &pack, &countries, &limited, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("ProjectBreakthrough"));
        assert_eq!(report.compared_tables.get("ProjectHistory"), Some(&1));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("interior NUL"))
        );
    }

    #[test]
    fn optional_token_presence_is_not_inferred_from_zero_or_empty_literal() {
        let tokens = BTreeMap::from([(0, b"".as_slice())]);
        let mut report = Report::default();
        compare_optional_token(&mut report, "active_project", &tokens, Some(0), Some(b""));
        compare_optional_token(&mut report, "active_project", &tokens, None, None);
        assert!(report.differences.is_empty());
        assert!(report.limitations.is_empty());
        compare_optional_token(&mut report, "active_project", &tokens, Some(0), None);
        compare_optional_token(&mut report, "active_project", &tokens, None, Some(b""));
        assert_eq!(report.differences.len(), 2);
        assert!(
            report
                .differences
                .iter()
                .all(|difference| difference.contains(".present"))
        );
    }

    #[test]
    fn exceptional_lexical_tokens_do_not_claim_a_saved_text_mapping() {
        let tokens = BTreeMap::from([
            (1, b"prefix\0suffix".as_slice()),
            (0x167, b"32768".as_slice()),
        ]);
        let mut report = Report::default();
        compare_token(&mut report, "nul", &tokens, 1, b"prefix");
        compare_token(&mut report, "reserved", &tokens, 0x167, b"32768");
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.len(), 2);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("interior NUL"))
        );
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("fixed15"))
        );
        assert_eq!(tokens[&1], b"prefix\0suffix");
    }

    #[test]
    fn peace_turn_reconstruction_never_picks_arbitrary_tag_order() {
        let mut conference = saved::Conference {
            id: 1,
            ordinal: 0,
            finalized: false,
            history_turns: BTreeMap::new(),
        };
        assert_eq!(reconstructed_turn(&conference), Some(0));
        conference.history_turns.insert(b"GER".to_vec(), 4);
        conference.history_turns.insert(b"ENG".to_vec(), 4);
        assert_eq!(reconstructed_turn(&conference), Some(4));
        conference.history_turns.insert(b"SOV".to_vec(), 5);
        assert_eq!(reconstructed_turn(&conference), None);
    }
}
