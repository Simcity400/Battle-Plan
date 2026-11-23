
use super::{
    Report, complete, country_tag, current_slice_owners, invalid, required_country_tag,
    slice_owner_limitation,
};
use content_reader_save::{Save, characters as saved, governance};
use replay_data::{rows::CountryRow, rows::PartyLeaderRow, rows::TableId, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

pub fn compare(
    frame: &DecodedFrame,
    save: &Save,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) {
    report.attempt_table(TableId::CountryCharacters, |report| {
        character_records::<tables::CountryCharacters>(
            frame,
            &save.characters.character_status,
            links,
            report,
        )
    });
    report.attempt_table(TableId::CountryRetiredCharacters, |report| {
        character_records::<tables::CountryRetiredCharacters>(
            frame,
            &save.characters.retired_character_status,
            links,
            report,
        )
    });
    report.attempt_table(TableId::PartyLeaders, |report| {
        party_leaders(frame, &save.governance, &save.characters, links, report)
    });
    report.attempt_table(TableId::CountryCivilWars, |report| {
        civil_wars(frame, &save.countries, links, report)
    });
    report.attempt_table(TableId::AdvisorTokens, |report| {
        advisor_tokens(frame, &save.characters, report)
    });
    report.attempt_table(TableId::AvailableAdvisors, |report| {
        available_advisors(frame, &save.characters, links, report)
    });
    report.attempt_table(TableId::FactionAdvisors, |report| {
        faction_advisors(frame, &save.factions, &save.characters, report)
    });
}

fn available_advisors(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::AvailableAdvisors>(frame)?;
    let manager = pack
        .manager
        .as_ref()
        .ok_or_else(|| invalid("save lacks character_manager"))?;
    let current = current_slice_owners(frame, TableId::AvailableAdvisors, report)?;
    slice_owner_limitation(frame, TableId::AvailableAdvisors, &current, report);
    let current: Option<BTreeSet<&[u8]>> = match &current {
        None => None,
        Some(owners) => {
            let mut tags = BTreeSet::new();
            for &owner in owners {
                if let Some(tag) = country_tag(owner, links)? {
                    tags.insert(tag);
                }
            }
            Some(tags)
        }
    };
    let read_now = |tag: &[u8]| current.as_ref().is_none_or(|tags| tags.contains(tag));
    let mut recorded = BTreeMap::<Vec<u8>, BTreeSet<(u32, Vec<u8>)>>::new();
    let mut compared = 0;
    for row in rows {
        let tag = required_country_tag(row.country, links)?;
        if !read_now(tag) {
            continue;
        }
        compared += 1;
        recorded.entry(tag.to_vec()).or_default().insert((
            row.character,
            content_reader_parser::parser::c_string(&row.idea).to_vec(),
        ));
    }
    for (tag, records) in report.canonical_countries(&pack.character_status) {
        if !read_now(tag.as_slice()) {
            continue;
        }
        let mut expected = BTreeSet::new();
        let mut missing = false;
        for record in records {
            match record.advisor {
                Some(false) => continue,
                None => {
                    missing = true;
                    report.limitation(
                        "saved character_status omits advisor flag; available-advisor membership cannot be established",
                        format!(
                            "{} character {:?}",
                            String::from_utf8_lossy(tag),
                            record.character,
                        ),
                    );
                    continue;
                }
                Some(true) => {}
            }
            let handle = record.character;
            if handle == 0 {
                return Err(invalid("saved advisor status has a null character"));
            }
            let id = (handle >> 32) as u32;
            let character = manager
                .characters
                .get(&handle)
                .ok_or_else(|| invalid("saved advisor status character missing from manager"))?;
            for role in &character.advisors {
                expected.insert((id, role.idea.clone().unwrap_or_default()));
            }
        }
        let actual = recorded.remove(tag).unwrap_or_default();
        let mut candidates = BTreeMap::<Vec<u8>, BTreeSet<(u32, Vec<u8>)>>::new();
        for item in &expected {
            candidates
                .entry(item.1.to_ascii_lowercase())
                .or_default()
                .insert(item.clone());
        }
        for (idea, members) in &candidates {
            if !idea.is_ascii() {
                report.limitation(
                    "the save cannot certify non-ASCII _stricmp equivalence or the primary comparison word, so available-advisor representative completeness is unverified",
                    format!("{}.AvailableAdvisors {members:?}", String::from_utf8_lossy(tag)),
                );
                continue;
            }
            let present = if members.len() == 1 && !missing {
                actual.contains(members.first().unwrap())
            } else {
                actual
                    .iter()
                    .any(|item| item.1.to_ascii_lowercase() == *idea)
            };
            if !present {
                report.difference(
                    format!(
                        "{}.AvailableAdvisors idea {:?}",
                        String::from_utf8_lossy(tag),
                        String::from_utf8_lossy(idea),
                    ),
                    "absent",
                    "present",
                );
            }
            if members.len() > 1 {
                report.limitation(
                    "the game inserts only comparator-distinct ideas; the save omits the primary key word and cannot certify the representative or cardinality of shared idea keys",
                    format!("{}.AvailableAdvisors {members:?}", String::from_utf8_lossy(tag)),
                );
            }
        }
        if !missing {
            for item in actual.difference(&expected) {
                report.difference(
                    format!(
                        "{}.AvailableAdvisors {item:?}",
                        String::from_utf8_lossy(tag)
                    ),
                    "present",
                    "absent",
                );
            }
        }
    }
    for (tag, members) in recorded {
        report.difference(
            format!("{}.AvailableAdvisors", String::from_utf8_lossy(&tag)),
            members.len(),
            "country absent from save",
        );
    }
    report.compared("AvailableAdvisors", compared);
    report.limitation(
        "the game saves advisor status and roles but not the available-advisor vector order or duplicate multiplicity; non-ASCII case-insensitive comparison equivalence is not certified by the saved idea bytes",
        "AvailableAdvisors.ordinal",
    );
    Ok(())
}

fn faction_advisors(
    frame: &DecodedFrame,
    factions: &BTreeMap<u64, content_reader_save::Faction>,
    characters: &saved::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::FactionAdvisors>(frame)?;
    let mut recorded = BTreeMap::<u64, BTreeMap<u32, &[u8]>>::new();
    for row in rows {
        if recorded
            .entry(row.faction)
            .or_default()
            .insert(row.ordinal, row.token.as_slice())
            .is_some()
        {
            return Err(invalid("FactionAdvisors records a slot twice"));
        }
    }
    let manager = characters.manager.as_ref();
    let mut by_token = BTreeMap::<&[u8], Vec<&saved::Character>>::new();
    for character in manager
        .iter()
        .flat_map(|manager| manager.characters.values())
    {
        by_token
            .entry(character.token.as_slice())
            .or_default()
            .push(character);
    }
    let mut ambiguous = Vec::new();
    for (&id, faction) in factions {
        let label = format!("FactionAdvisors {}", id >> 32);
        let mut actual = recorded.remove(&id).unwrap_or_default();
        for (ordinal, saved) in faction.advisor_slots.iter().enumerate() {
            let label = format!("{label}[{ordinal}]");
            let row = actual.remove(&(ordinal as u32));
            report.scalar_checks += 1;
            let (row, advisor) = match (row, saved) {
                (None, None) => continue,
                (Some(row), Some(advisor)) => (row, advisor),
                (row, advisor) => {
                    report.difference(
                        label,
                        row.map(|token| String::from_utf8_lossy(token).into_owned()),
                        advisor
                            .as_ref()
                            .map(|token| String::from_utf8_lossy(token).into_owned()),
                    );
                    continue;
                }
            };
            let ideas: Vec<&[u8]> = match by_token.get(advisor.as_slice()).map(Vec::as_slice) {
                Some([character]) => character
                    .advisors
                    .iter()
                    .map(|role| role.idea.as_deref().unwrap_or_default())
                    .collect(),
                _ => {
                    report.difference(
                        format!("{label} advisor character"),
                        String::from_utf8_lossy(row).into_owned(),
                        format!(
                            "no single saved character {}",
                            String::from_utf8_lossy(advisor)
                        ),
                    );
                    continue;
                }
            };
            let matches = match ideas.as_slice() {
                [idea] => *idea == row,
                ideas => {
                    ambiguous.push(label.clone());
                    ideas.contains(&row)
                }
            };
            if !matches {
                report.difference(
                    format!("{label}.token"),
                    String::from_utf8_lossy(row).into_owned(),
                    ideas
                        .iter()
                        .map(|idea| String::from_utf8_lossy(idea).into_owned())
                        .collect::<Vec<_>>(),
                );
            }
        }
        for (ordinal, token) in actual {
            report.difference(
                format!("{label}[{ordinal}]"),
                String::from_utf8_lossy(token).into_owned(),
                "no saved slot",
            );
        }
    }
    for (id, slots) in recorded {
        report.difference(
            format!("FactionAdvisors {}", id >> 32),
            slots.len(),
            "faction absent from save",
        );
    }
    if !ambiguous.is_empty() {
        report.limitation(
            "the save names a faction slot's advisor by its character, and that character has several advisor roles, so the slot's idea token is only checked to be one of them",
            ambiguous.join(", "),
        );
    }
    report.compared("FactionAdvisors", rows.len());
    Ok(())
}

fn advisor_token_key(token: &[u8]) -> (u32, Vec<u8>) {
    let hash = token.iter().fold(0x811c_9dc5_u32, |hash, byte| {
        (hash ^ u32::from(byte.to_ascii_lowercase())).wrapping_mul(0x0100_0193)
    });
    (
        hash,
        content_reader_parser::parser::c_string(token).to_ascii_lowercase(),
    )
}

fn advisor_tokens(frame: &DecodedFrame, pack: &saved::Pack, report: &mut Report) -> io::Result<()> {
    let rows = complete::<tables::AdvisorTokens>(frame)?;
    let mut recorded = BTreeMap::new();
    for row in rows {
        if recorded
            .insert(advisor_token_key(&row.token), row.token.as_slice())
            .is_some()
        {
            return Err(invalid("AdvisorTokens records a token twice"));
        }
    }
    let manager = pack
        .manager
        .as_ref()
        .ok_or_else(|| invalid("save lacks character_manager"))?;
    let mut expected = BTreeMap::new();
    let mut listed = BTreeSet::new();
    let lists = [
        ("character_status", &pack.character_status, false),
        (
            "retired_character_status",
            &pack.retired_character_status,
            true,
        ),
    ];
    for (list, countries, retired) in lists {
        for (tag, statuses) in report.canonical_countries(countries) {
            for status in statuses {
                listed.insert(status.character);
                match (retired, status.advisor) {
                    (true, _) | (false, Some(true)) => {}
                    (false, Some(false)) => continue,
                    (false, None) => {
                        return Err(invalid(format!(
                            "{} {list} lacks its advisor flag",
                            String::from_utf8_lossy(tag)
                        )));
                    }
                }
                let character = manager.characters.get(&status.character).ok_or_else(|| {
                    invalid(format!(
                        "{} {list} names an unsaved character {}",
                        String::from_utf8_lossy(tag),
                        status.character >> 32
                    ))
                })?;
                for role in &character.advisors {
                    let idea = role.idea.as_deref().unwrap_or_default();
                    expected.entry(advisor_token_key(idea)).or_insert(idea);
                }
            }
        }
    }
    report.scalar_checks += 1;
    for (key, missing) in &expected {
        if recorded.contains_key(key) {
            continue;
        }
        report.difference(
            "AdvisorTokens",
            "absent",
            String::from_utf8_lossy(missing).into_owned(),
        );
    }
    let mut recorded_roles = None;
    for (key, extra) in &recorded {
        if expected.contains_key(key) {
            continue;
        }
        let runtime_only = !report.load_hour && {
            let roles = match recorded_roles {
                Some(roles) => roles,
                None => *recorded_roles.insert(complete::<tables::AdvisorRoles>(frame)?),
            };
            let holders = manager
                .characters
                .iter()
                .filter(|(_, character)| {
                    character.advisors.iter().any(|role| {
                        advisor_token_key(role.idea.as_deref().unwrap_or_default()) == *key
                    })
                })
                .map(|(&handle, _)| handle)
                .chain(
                    roles
                        .iter()
                        .filter(|role| advisor_token_key(&role.idea) == *key)
                        .map(|role| (u64::from(role.character) << 32) | 73),
                )
                .collect::<BTreeSet<_>>();
            !holders.is_empty() && holders.is_disjoint(&listed)
        };
        let token = String::from_utf8_lossy(extra).into_owned();
        if runtime_only {
            report.limitation(
                "runtime-only advisor token: character not in the saved rosters",
                format!("AdvisorTokens {token}"),
            );
        } else {
            report.difference("AdvisorTokens", token, "absent");
        }
    }
    report.limitation(
        "no writer saves the advisor idea-token set; it is compared with the set loading the save builds (the country-character loader): the idea tokens of the saved advisor roles of every character_status marked advisor=yes (advisor-status updating) and of every retired_character_status character, using the native ASCII-folded hash and case-insensitive key; role strings are saved exactly, but set spelling depends on unsaved insertion history; exactly at a load hour all membership mismatches are differences, elsewhere a recorded token whose holders are in neither list is a runtime-only limitation (dynamic-country resetting drops a country's lists without an erase); the game omits empty idea strings, but the game inserts their empty key",
        "AdvisorTokens",
    );
    report.compared("AdvisorTokens", rows.len());
    Ok(())
}

fn civil_wars(
    frame: &DecodedFrame,
    countries: &BTreeMap<Vec<u8>, content_reader_save::Country>,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::CountryCivilWars>(frame)?;
    let mut recorded = by_country(rows, links, "CountryCivilWars", |row| {
        (row.country, row.ordinal)
    })?;
    for (tag, country) in report.canonical_countries(countries) {
        let label = format!("CountryCivilWars {}", String::from_utf8_lossy(tag));
        let mut actual = Vec::new();
        for row in recorded.remove(tag.as_slice()).unwrap_or_default() {
            actual.push(
                country_tag(row.target, links)?
                    .ok_or_else(|| invalid(format!("{label} records a null reference")))?,
            );
        }
        let expected: Vec<&[u8]> = country
            .civil_war_targets
            .iter()
            .map(Vec::as_slice)
            .collect();
        report.scalar_checks += 1;
        if actual != expected {
            let names = |tags: &[&[u8]]| {
                tags.iter()
                    .map(|tag| String::from_utf8_lossy(tag).into_owned())
                    .collect::<Vec<_>>()
            };
            report.difference(label, names(&actual), names(&expected));
        }
    }
    for (tag, list) in recorded {
        report.difference(
            format!("CountryCivilWars {}", String::from_utf8_lossy(tag)),
            list.len(),
            "country absent from save",
        );
    }
    report.compared("CountryCivilWars", rows.len());
    Ok(())
}

fn character_handle(id: u32) -> u64 {
    (u64::from(id) << 32) | u64::from(engine_native::ccharacter::REFERENCE_TYPE)
}

fn by_country<'a, R>(
    rows: &'a [R],
    links: &BTreeMap<u32, &'a CountryRow>,
    table: &str,
    key: impl Fn(&R) -> (u32, u32),
) -> io::Result<BTreeMap<&'a [u8], Vec<&'a R>>> {
    let mut grouped = BTreeMap::<&[u8], (u32, Vec<&R>)>::new();
    for row in rows {
        let (country, ordinal) = key(row);
        let tag = required_country_tag(country, links)?;
        let (raw, list) = grouped.entry(tag).or_insert((country, Vec::new()));
        if *raw != country {
            return Err(invalid(format!(
                "{table} records two country references for {}",
                String::from_utf8_lossy(tag)
            )));
        }
        if ordinal as usize != list.len() {
            return Err(invalid(format!(
                "{table} ordinals are not contiguous from zero within a country"
            )));
        }
        list.push(row);
    }
    Ok(grouped
        .into_iter()
        .map(|(tag, (_, list))| (tag, list))
        .collect())
}

fn character_records<T>(
    frame: &DecodedFrame,
    saved: &BTreeMap<Vec<u8>, Vec<saved::CharacterStatus>>,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()>
where
    T: replay_data::rows::TableSchema<Row = replay_data::rows::CountryCharacterRow>,
{
    let name = format!("{:?}", T::ID);
    let rows = complete::<T>(frame)?;
    let mut recorded = by_country(rows, links, &name, |row| (row.country, row.ordinal))?;
    for (tag, expected) in saved {
        let label = format!("{name} {}", String::from_utf8_lossy(tag));
        let actual = recorded.remove(tag.as_slice()).unwrap_or_default();
        let mut set = BTreeSet::new();
        for row in &actual {
            match row.character {
                Some(id) => {
                    set.insert(character_handle(id));
                }
                None => report.difference(
                    format!("{label}[{}]", row.ordinal),
                    "null record",
                    "the writer dereferences every record",
                ),
            }
        }
        let expected: BTreeSet<u64> = expected.iter().map(|status| status.character).collect();
        for missing in expected.difference(&set) {
            report.difference(format!("{label} character"), "absent", missing >> 32);
        }
        for extra in set.difference(&expected) {
            report.difference(format!("{label} character"), extra >> 32, "absent");
        }
    }
    for (tag, list) in recorded {
        report.difference(
            format!("{name} {}", String::from_utf8_lossy(tag)),
            list.len(),
            "country absent from save",
        );
    }
    report.limitation(
        "each record list is written through a map keyed by the character reference (the country character save writer), so the native record order (sorted by name token) and duplicate records are not compared; the records compare as a set",
        name.as_str(),
    );
    report.compared(&name, rows.len());
    Ok(())
}

fn party_leaders(
    frame: &DecodedFrame,
    governance: &governance::Pack,
    characters: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::PartyLeaders>(frame)?;
    let mut tokens = BTreeMap::new();
    for party in complete::<tables::PoliticalParties>(frame)? {
        if tokens
            .insert((party.country, party.id), party.token.as_slice())
            .is_some()
        {
            return Err(invalid("duplicate PoliticalParties key"));
        }
    }
    let mut recorded = BTreeMap::<(&[u8], &[u8]), Vec<&PartyLeaderRow>>::new();
    let mut raws = BTreeMap::<&[u8], u32>::new();
    for row in rows {
        let tag = required_country_tag(row.country, links)?;
        if *raws.entry(tag).or_insert(row.country) != row.country {
            return Err(invalid(format!(
                "PartyLeaders records two country references for {}",
                String::from_utf8_lossy(tag)
            )));
        }
        let token = tokens.get(&(row.country, row.party)).ok_or_else(|| {
            invalid(format!(
                "PartyLeaders party {} of {} has no PoliticalParties row",
                row.party,
                String::from_utf8_lossy(tag)
            ))
        })?;
        let list = recorded.entry((tag, token)).or_default();
        if row.ordinal as usize != list.len() {
            return Err(invalid(
                "PartyLeaders ordinals are not contiguous from zero within a party",
            ));
        }
        list.push(row);
    }
    let roles = characters.manager.as_ref();
    let keys = super::characters::leader_role_keys(frame)?;
    let mut unjoined = Vec::new();
    let mut compared = 0;
    for (tag, country) in report.canonical_countries(&governance.countries) {
        let Some(politics) = &country.politics else {
            continue;
        };
        for (party, expected) in &politics.leaders {
            let label = format!(
                "PartyLeaders {} {}",
                String::from_utf8_lossy(tag),
                String::from_utf8_lossy(party)
            );
            let actual = recorded
                .remove(&(tag.as_slice(), party.as_slice()))
                .unwrap_or_default();
            if actual.len() != expected.len() {
                report.difference(format!("{label} count"), actual.len(), expected.len());
            }
            for (ordinal, (row, saved)) in actual.iter().zip(expected).enumerate() {
                compared += 1;
                let label = format!("{label}[{ordinal}]");
                let reference = (saved.character != (0, 0)).then_some(saved.character);
                let recorded_reference = row
                    .character
                    .map(|id| (id, engine_native::ccharacter::REFERENCE_TYPE));
                if recorded_reference != reference {
                    report.difference(
                        format!("{label}.character"),
                        row.character,
                        reference.map(|(id, _)| id),
                    );
                }
                let role = reference.and_then(|(id, _)| {
                    let character = roles?.characters.get(&character_handle(id))?;
                    let keys = keys.get(&character_handle(id))?;
                    if keys.len() != character.country_leaders.len() {
                        return None;
                    }
                    let position = keys.binary_search(&row.role?).ok()?;
                    character.country_leaders.get(position)
                });
                if let Some(role) = role {
                    if role.ideology != saved.ideology {
                        report.difference(format!("{label}.role"), &role.ideology, &saved.ideology);
                    }
                    if row.leader_id != role.id.unwrap_or(-1) {
                        report.difference(
                            format!("{label}.leader_id"),
                            row.leader_id,
                            role.id.unwrap_or(-1),
                        );
                    }
                } else if reference.is_some() {
                    report.difference(
                        format!("{label}.role"),
                        row.role,
                        "no matching saved role position",
                    );
                } else {
                    unjoined.push(format!("{label}.leader_id"));
                }
            }
            for (ordinal, extra) in actual.iter().enumerate().skip(expected.len()) {
                report.difference(format!("{label}[{ordinal}]"), extra.character, "absent");
            }
            for (ordinal, missing) in expected.iter().enumerate().skip(actual.len()) {
                report.difference(format!("{label}[{ordinal}]"), "absent", missing.character.0);
            }
        }
    }
    for ((tag, party), list) in recorded {
        report.difference(
            format!(
                "PartyLeaders {} {}",
                String::from_utf8_lossy(tag),
                String::from_utf8_lossy(party)
            ),
            list.len(),
            "party absent from save",
        );
    }
    if !unjoined.is_empty() {
        report.limitation(
            "the party writer saves no leader id and a null character has no serialized role, so its leader_id is unverified",
            unjoined.join(", "),
        );
    }
    report.compared("PartyLeaders", compared);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_data::{rows::CountryCharacterRow, rows::PoliticalPartyRow, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};

    fn frame() -> DecodedFrame {
        DecodedFrame {
            branch: 1,
            hour: 1,
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Metrics::default(),
        }
    }

    fn germany() -> CountryRow {
        CountryRow {
            id: 102,
            tag: b"GER".to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn available_advisors_compare_saved_status_membership_not_native_order() {
        let source = br#"
                character_manager={historical={
                    character={id={id=1 type=73} token="a" advisors={
                        advisor={idea_token="a1"} advisor={idea_token="a2"}
                    }}
                    character={id={id=2 type=73} token="b" advisors={advisor={idea_token="b1"}}}
                }}
                countries={GER={characters={
                    character_status={character={id=1 type=73} advisor=yes}
                    character_status={character={id=2 type=73} advisor=no}
                }}}
            "#;
        let pack = saved::parse(&content_reader_parser::parser::parse(source).unwrap().items).unwrap();
        let germany = germany();
        let links = BTreeMap::from([(102, &germany)]);
        let mut frame = frame();
        let make = |ordinal, idea: &[u8]| replay_data::rows::AvailableAdvisorRow {
            country: 102,
            ordinal,
            character: 1,
            idea: idea.to_vec(),
        };
        *frame.tables[TableId::AvailableAdvisors.index()] =
            TableView::Complete(TableRows::AvailableAdvisors(vec![
                make(0, b"a2"),
                make(1, b"a1"),
            ]));
        let mut report = Report::default();
        available_advisors(&frame, &pack, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["AvailableAdvisors"], 2);
        *frame.tables[TableId::AvailableAdvisors.index()] =
            TableView::Complete(TableRows::AvailableAdvisors(vec![
                make(0, b"a1"),
                make(1, b"b1"),
            ]));
        let mut report = Report::default();
        available_advisors(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        *frame.tables[TableId::AvailableAdvisors.index()] =
            TableView::Complete(TableRows::AvailableAdvisors(Vec::new()));
        let mut report = Report::default();
        available_advisors(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
        *frame.tables[TableId::AvailableAdvisors.index()] = TableView::Unavailable {
            reason: "source failed".into(),
        };
        assert!(available_advisors(&frame, &pack, &links, &mut Report::default()).is_err());
    }

    #[test]
    fn available_advisors_compare_only_pools_read_at_the_oracle_hour() {
        let source = br#"
                character_manager={historical={
                    character={id={id=1 type=73} token="a" advisors={advisor={idea_token="a1"}}}
                }}
                countries={GER={characters={
                    character_status={character={id=1 type=73} advisor=yes}
                }}}
            "#;
        let pack = saved::parse(&content_reader_parser::parser::parse(source).unwrap().items).unwrap();
        let germany = germany();
        let links = BTreeMap::from([(102, &germany)]);
        let mut frame = frame();
        *frame.tables[TableId::AvailableAdvisors.index()] =
            TableView::Complete(TableRows::AvailableAdvisors(vec![
                replay_data::rows::AvailableAdvisorRow {
                    country: 102,
                    ordinal: 0,
                    character: 1,
                    idea: b"old".to_vec(),
                },
            ]));
        let read_at = |hour| {
            TableView::Complete(TableRows::SliceReads(vec![
                replay_data::rows::SliceReadRow {
                    table: TableId::AvailableAdvisors.index() as u32,
                    owner: 102,
                    sample_hour: hour,
                },
            ]))
        };
        *frame.tables[TableId::SliceReads.index()] = read_at(frame.hour - 3);
        let mut report = Report::default();
        available_advisors(&frame, &pack, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["AvailableAdvisors"], 0);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("owner decision 2026-10-11"))
        );
        *frame.tables[TableId::SliceReads.index()] = read_at(frame.hour);
        let mut report = Report::default();
        available_advisors(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert_eq!(report.compared_tables["AvailableAdvisors"], 1);
    }

    #[test]
    fn advisor_rosters_and_tokens_retain_the_omitted_empty_idea() {
        let source = br#"
            character_manager={historical={
                character={id={id=1 type=73} token="a" advisors={advisor={slot="theorist"}}}
            }}
            countries={GER={characters={
                character_status={character={id=1 type=73} advisor=yes}
            }}}
        "#;
        let pack = saved::parse(&content_reader_parser::parser::parse(source).unwrap().items).unwrap();
        let germany = germany();
        let links = BTreeMap::from([(102, &germany)]);
        let mut frame = frame();
        *frame.tables[TableId::AvailableAdvisors.index()] =
            TableView::Complete(TableRows::AvailableAdvisors(vec![
                replay_data::rows::AvailableAdvisorRow {
                    country: 102,
                    character: 1,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::AdvisorTokens.index()] =
            TableView::Complete(TableRows::AdvisorTokens(vec![
                replay_data::rows::AdvisorTokenRow::default(),
            ]));
        let mut report = Report::default();
        available_advisors(&frame, &pack, &links, &mut report).unwrap();
        advisor_tokens(&frame, &pack, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        *frame.tables[TableId::AdvisorTokens.index()] =
            TableView::Complete(TableRows::AdvisorTokens(Vec::new()));
        let mut report = Report::default();
        advisor_tokens(&frame, &pack, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    }

    #[test]
    fn available_advisors_accept_a_saved_same_idea_representative_and_name_the_ambiguity() {
        let source = br#"
            character_manager={historical={
                character={id={id=1 type=73} token="a"
                    advisors={advisor={idea_token="shared"}}}
                character={id={id=2 type=73} token="b"
                    advisors={advisor={idea_token="SHARED"}}}
            }}
            countries={GER={characters={
                character_status={character={id=1 type=73} advisor=yes}
                character_status={character={id=2 type=73} advisor=yes}
            }}}
        "#;
        let pack = saved::parse(&content_reader_parser::parser::parse(source).unwrap().items).unwrap();
        let country = germany();
        let links = BTreeMap::from([(102, &country)]);
        let mut frame = frame();
        for (character, idea) in [(1, b"shared".as_slice()), (2, b"SHARED".as_slice())] {
            let row = replay_data::rows::AvailableAdvisorRow {
                country: 102,
                ordinal: 0,
                character,
                idea: idea.to_vec(),
            };
            *frame.tables[TableId::AvailableAdvisors.index()] =
                TableView::Complete(TableRows::AvailableAdvisors(vec![
                    row.clone(),
                    replay_data::rows::AvailableAdvisorRow { ordinal: 1, ..row },
                ]));
            let mut report = Report::default();
            available_advisors(&frame, &pack, &links, &mut report).unwrap();
            assert!(report.differences.is_empty(), "{:?}", report.differences);
            assert!(
                report
                    .limitations
                    .keys()
                    .any(|reason| reason.contains("representative"))
            );
        }
        *frame.tables[TableId::AvailableAdvisors.index()] =
            TableView::Complete(TableRows::AvailableAdvisors(vec![
                replay_data::rows::AvailableAdvisorRow {
                    country: 102,
                    character: 3,
                    idea: b"shared".to_vec(),
                    ..Default::default()
                },
            ]));
        let mut report = Report::default();
        available_advisors(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        *frame.tables[TableId::AvailableAdvisors.index()] =
            TableView::Complete(TableRows::AvailableAdvisors(Vec::new()));
        let mut report = Report::default();
        available_advisors(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    fn characters_frame(characters: &[Option<u32>]) -> DecodedFrame {
        let mut frame = frame();
        *frame.tables[TableId::CountryCharacters.index()] =
            TableView::Complete(TableRows::CountryCharacters(
                characters
                    .iter()
                    .enumerate()
                    .map(|(ordinal, &character)| CountryCharacterRow {
                        country: 102,
                        ordinal: ordinal as u32,
                        character,
                    })
                    .collect(),
            ));
        frame
    }

    fn status(handles: &[u32]) -> saved::Pack {
        saved::Pack {
            character_status: BTreeMap::from([
                (
                    b"GER".to_vec(),
                    handles
                        .iter()
                        .map(|&id| saved::CharacterStatus {
                            character: character_handle(id),
                            advisor: Some(false),
                            unit_leader: None,
                        })
                        .collect(),
                ),
                (b"ENG".to_vec(), Vec::new()),
            ]),
            ..Default::default()
        }
    }

    fn run(frame: &DecodedFrame, pack: &saved::Pack) -> Report {
        let germany = germany();
        let links = BTreeMap::from([(102, &germany)]);
        let mut report = Report::default();
        report.attempt_table(TableId::CountryCharacters, |report| {
            character_records::<tables::CountryCharacters>(
                frame,
                &pack.character_status,
                &links,
                report,
            )
        });
        report
    }

    #[test]
    fn retired_records_compare_as_a_set_with_retired_character_status() {
        let mut frame = frame();
        *frame.tables[TableId::CountryRetiredCharacters.index()] =
            TableView::Complete(TableRows::CountryRetiredCharacters(
                [9, 4]
                    .into_iter()
                    .enumerate()
                    .map(|(ordinal, id)| CountryCharacterRow {
                        country: 102,
                        ordinal: ordinal as u32,
                        character: Some(id),
                    })
                    .collect(),
            ));
        let germany = germany();
        let links = BTreeMap::from([(102, &germany)]);
        let run = |pack: &saved::Pack| {
            let mut report = Report::default();
            report.attempt_table(TableId::CountryRetiredCharacters, |report| {
                character_records::<tables::CountryRetiredCharacters>(
                    &frame,
                    &pack.retired_character_status,
                    &links,
                    report,
                )
            });
            report
        };
        let mut pack = status(&[]);
        pack.retired_character_status = std::mem::take(&mut status(&[4, 9]).character_status);
        let report = run(&pack);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryRetiredCharacters"], 2);
        pack.retired_character_status = std::mem::take(&mut status(&[4]).character_status);
        assert_eq!(run(&pack).differences.len(), 1);
    }

    #[test]
    fn active_records_compare_as_a_set_with_character_status() {
        let report = run(
            &characters_frame(&[Some(9), Some(4), Some(9)]),
            &status(&[4, 9]),
        );
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(report.compared_tables["CountryCharacters"], 3);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("compare as a set"))
        );
    }

    #[test]
    fn missing_extra_and_null_records_are_differences() {
        let report = run(
            &characters_frame(&[Some(4), None, Some(7)]),
            &status(&[4, 9]),
        );
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        assert!(report.differences.iter().any(|d| d.contains("null record")));
        let report = run(&characters_frame(&[]), &status(&[4]));
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        let mut pack = status(&[4]);
        pack.character_status.remove(b"GER".as_slice());
        let report = run(&characters_frame(&[Some(4)]), &pack);
        assert!(
            report
                .differences
                .iter()
                .any(|d| d.contains("country absent from save"))
        );
        let mut frame = characters_frame(&[Some(4)]);
        *frame.tables[TableId::CountryCharacters.index()] =
            TableView::Complete(TableRows::CountryCharacters(vec![CountryCharacterRow {
                country: 102,
                ordinal: 1,
                character: Some(4),
            }]));
        let report = run(&frame, &status(&[4]));
        assert!(report.failed_comparisons.contains_key("CountryCharacters"));
    }

    fn leader_frame(leaders: &[(u32, u32, Option<u32>, i32)]) -> DecodedFrame {
        let mut frame = frame();
        *frame.tables[TableId::PoliticalParties.index()] =
            TableView::Complete(TableRows::PoliticalParties(
                [(3, b"fascism".as_slice()), (5, b"democratic")]
                    .into_iter()
                    .map(|(id, token)| PoliticalPartyRow {
                        country: 102,
                        id,
                        token: token.to_vec(),
                        ..Default::default()
                    })
                    .collect(),
            ));
        *frame.tables[TableId::CountryLeaderRoles.index()] =
            TableView::Complete(TableRows::CountryLeaderRoles(vec![
                replay_data::rows::CountryLeaderRoleRow {
                    character: 1482,
                    country: 102,
                    ideology: -7,
                    ..Default::default()
                },
                replay_data::rows::CountryLeaderRoleRow {
                    character: 1482,
                    country: 102,
                    ideology: 9,
                    ..Default::default()
                },
                replay_data::rows::CountryLeaderRoleRow {
                    character: 1451,
                    country: 102,
                    ideology: -7,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::PartyLeaders.index()] =
            TableView::Complete(TableRows::PartyLeaders(
                leaders
                    .iter()
                    .map(|&(party, ordinal, character, leader_id)| PartyLeaderRow {
                        country: 102,
                        party,
                        ordinal,
                        character,
                        role: character.map(|_| if party == 5 { 9 } else { -7 }),
                        leader_id,
                    })
                    .collect(),
            ));
        frame
    }

    fn saved_leaders() -> (governance::Pack, saved::Pack) {
        let characters = saved::parse(
            &content_reader_parser::parser::parse(
                br#"character_manager={ historical={
                    character={ id={ id=1482 type=73 } token="a"
                        country_leaders={ country_leader={ ideology=nazism id=12 }
                                          country_leader={ ideology=conservatism } } }
                    character={ id={ id=1451 type=73 } token="b"
                        country_leaders={ country_leader={ ideology=nazism } } }
                }} countries={ }"#,
            )
            .unwrap()
            .items,
        )
        .unwrap();
        let leader = |ideology: &[u8], character| governance::PartyLeader {
            ideology: ideology.to_vec(),
            character,
        };
        let mut pack = governance::Pack::default();
        pack.countries.insert(
            b"GER".to_vec(),
            governance::Country {
                politics: Some(governance::Politics {
                    parties: BTreeMap::new(),
                    leaders: BTreeMap::from([
                        (
                            b"fascism".to_vec(),
                            vec![leader(b"nazism", (1482, 73)), leader(b"nazism", (0, 0))],
                        ),
                        (
                            b"democratic".to_vec(),
                            vec![leader(b"conservatism", (1482, 73))],
                        ),
                    ]),
                    names: BTreeMap::new(),
                    ideas: Vec::new(),
                    timed_ideas: Vec::new(),
                    ruling_party: None,
                    last_election: None,
                    election_frequency: None,
                    elections_allowed: None,
                }),
                ..Default::default()
            },
        );
        (pack, characters)
    }

    fn run_leaders(frame: &DecodedFrame) -> Report {
        let (pack, characters) = saved_leaders();
        let germany = germany();
        let links = BTreeMap::from([(102, &germany)]);
        let mut report = Report::default();
        report.attempt_table(TableId::PartyLeaders, |report| {
            party_leaders(frame, &pack, &characters, &links, report)
        });
        report
    }

    #[test]
    fn party_leaders_compare_order_characters_and_role_ids() {
        let frame = leader_frame(&[
            (3, 0, Some(1482), 12),
            (3, 1, None, -1),
            (5, 0, Some(1482), -1),
        ]);
        let report = run_leaders(&frame);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(report.failed_comparisons.is_empty());
        assert_eq!(report.compared_tables["PartyLeaders"], 3);
        assert!(
            report
                .limitations
                .values()
                .any(|subjects| subjects.iter().any(|s| s.contains("fascism[1].leader_id")))
        );
    }

    #[test]
    fn party_leader_role_keys_join_saved_map_order_after_ideology_changes() {
        let (mut pack, mut characters) = saved_leaders();
        for role in &mut characters
            .manager
            .as_mut()
            .unwrap()
            .characters
            .get_mut(&character_handle(1482))
            .unwrap()
            .country_leaders
        {
            role.ideology = b"changed".to_vec();
        }
        for leaders in pack
            .countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .politics
            .as_mut()
            .unwrap()
            .leaders
            .values_mut()
        {
            leaders[0].ideology = b"changed".to_vec();
        }
        let mut frame = leader_frame(&[
            (3, 0, Some(1482), 12),
            (3, 1, None, -1),
            (5, 0, Some(1482), -1),
        ]);
        let germany = germany();
        let links = BTreeMap::from([(102, &germany)]);
        let run = |frame: &DecodedFrame| {
            let mut report = Report::default();
            party_leaders(frame, &pack, &characters, &links, &mut report).unwrap();
            report
        };
        assert!(run(&frame).differences.is_empty());
        let mut rows = frame.complete::<tables::PartyLeaders>().unwrap().to_vec();
        rows[0].role = Some(9);
        *frame.tables[TableId::PartyLeaders.index()] =
            TableView::Complete(TableRows::PartyLeaders(rows));
        assert!(
            run(&frame)
                .differences
                .iter()
                .any(|difference| difference.contains(".leader_id"))
        );
    }

    #[test]
    fn party_leader_order_identity_id_and_membership_differences_are_reported() {
        let report = run_leaders(&leader_frame(&[
            (3, 0, None, -1),
            (3, 1, Some(1482), 12),
            (5, 0, Some(1482), -1),
        ]));
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        let report = run_leaders(&leader_frame(&[
            (3, 0, Some(1482), 7),
            (5, 0, Some(1482), -1),
            (5, 1, Some(1451), -1),
        ]));
        assert!(report.differences.iter().any(|d| d.contains(".leader_id")));
        assert!(
            report
                .differences
                .iter()
                .any(|d| d.contains("fascism count"))
        );
        assert!(
            report
                .differences
                .iter()
                .any(|d| d.contains("democratic[1]"))
        );
        let report = run_leaders(&leader_frame(&[(4, 0, Some(1482), 12)]));
        assert!(report.failed_comparisons.contains_key("PartyLeaders"));
    }

    fn run_civil_wars(targets: &[u32], saved: &[&[u8]]) -> Report {
        let mut frame = frame();
        *frame.tables[TableId::CountryCivilWars.index()] =
            TableView::Complete(TableRows::CountryCivilWars(
                targets
                    .iter()
                    .enumerate()
                    .map(|(ordinal, &target)| replay_data::rows::CountryCivilWarRow {
                        country: 102,
                        ordinal: ordinal as u32,
                        target,
                    })
                    .collect(),
            ));
        let mut countries = BTreeMap::new();
        for tag in [b"GER".as_slice(), b"ITA", b"D01"] {
            countries.insert(
                tag.to_vec(),
                content_reader_save::Country {
                    tag: tag.to_vec(),
                    ..Default::default()
                },
            );
        }
        countries
            .get_mut(b"GER".as_slice())
            .unwrap()
            .civil_war_targets = saved.iter().map(|tag| tag.to_vec()).collect();
        let [germany, italy, dynamic] =
            [(102, b"GER"), (103, b"ITA"), (104, b"D01")].map(|(id, tag)| CountryRow {
                id,
                tag: tag.to_vec(),
                ..Default::default()
            });
        let links = BTreeMap::from([(102, &germany), (103, &italy), (104, &dynamic)]);
        let mut report = Report::default();
        report.attempt_table(TableId::CountryCivilWars, |report| {
            civil_wars(&frame, &countries, &links, report)
        });
        report
    }

    fn run_tokens(tokens: &[&[u8]], advisor: Option<bool>) -> Report {
        run_retired_tokens(tokens, advisor, None, false)
    }

    fn run_retired_tokens(
        tokens: &[&[u8]],
        advisor: Option<bool>,
        retired: Option<Option<bool>>,
        load_hour: bool,
    ) -> Report {
        let mut pack = saved::parse(
            &content_reader_parser::parser::parse(
                br#"character_manager={ historical={
                    character={ id={ id=1482 type=73 } token="a"
                        advisors={ advisor={ slot="political_advisor" idea_token="GER_a" }
                                   advisor={ slot="theorist" idea_token="GER_a_theorist" } } }
                    character={ id={ id=1451 type=73 } token="b"
                        advisors={ advisor={ slot="high_command" idea_token="GER_b" } } }
                    character={ id={ id=1523 type=73 } token="c"
                        advisors={ advisor={ slot="political_advisor" idea_token="SOV_c" }
                                   advisor={ slot="theorist" idea_token="SOV_c_theorist" } } }
                }} countries={ }"#,
            )
            .unwrap()
            .items,
        )
        .unwrap();
        pack.character_status.insert(
            b"GER".to_vec(),
            vec![
                saved::CharacterStatus {
                    character: character_handle(1482),
                    advisor,
                    unit_leader: None,
                },
                saved::CharacterStatus {
                    character: character_handle(1451),
                    advisor: Some(false),
                    unit_leader: None,
                },
            ],
        );
        if let Some(advisor) = retired {
            pack.retired_character_status.insert(
                b"SOV".to_vec(),
                vec![saved::CharacterStatus {
                    character: character_handle(1523),
                    advisor,
                    unit_leader: None,
                }],
            );
        }
        run_token_pack(tokens, &pack, load_hour)
    }

    fn run_token_pack(tokens: &[&[u8]], pack: &saved::Pack, load_hour: bool) -> Report {
        let mut frame = frame();
        *frame.tables[TableId::AdvisorTokens.index()] =
            TableView::Complete(TableRows::AdvisorTokens(
                tokens
                    .iter()
                    .enumerate()
                    .map(|(ordinal, token)| replay_data::rows::AdvisorTokenRow {
                        ordinal: ordinal as u32,
                        token: token.to_vec(),
                    })
                    .collect(),
            ));
        let mut report = Report {
            load_hour,
            ..Default::default()
        };
        report.attempt_table(TableId::AdvisorTokens, |report| {
            advisor_tokens(&frame, pack, report)
        });
        report
    }

    #[test]
    fn advisor_tokens_merge_case_variants_from_different_mod_files() {
        content_reader_script::test_game::enter();
        let names = content_reader_script::key_names::KeyNames::default();
        let mut authored = content_reader_key_names::characters::Characters::default();
        for file in [
            b"characters={AAA_first={advisor={slot=chief idea_token=mixed_advisor}}}".as_slice(),
            b"characters={AAA_second={advisor={slot=chief idea_token=MiXeD_Advisor}}}".as_slice(),
        ] {
            authored.merge(content_reader_key_names::characters::parse(file, &names).unwrap());
        }
        let (tokens, gap) = authored.advisor_tokens();
        assert!(gap.is_none());
        let tokens = tokens.collect::<Vec<_>>();
        assert_eq!(tokens, ["mixed_advisor"]);
        let mut pack = saved::parse(&content_reader_parser::parser::parse(br#"
            character_manager={historical={
                character={id={id=1 type=73} token=first advisors={advisor={idea_token=mixed_advisor}}}
                character={id={id=2 type=73} token=second advisors={advisor={idea_token=MiXeD_Advisor}}}
            }} countries={AAA={characters={
                character_status={character={id=1 type=73} advisor=yes}
                character_status={character={id=2 type=73} advisor=yes}
            }}}
        "#).unwrap().items).unwrap();
        for load_hour in [false, true] {
            for spelling in [b"mixed_advisor".as_slice(), b"MiXeD_Advisor"] {
                let report = run_token_pack(&[spelling], &pack, load_hour);
                assert!(
                    report.failed_comparisons.is_empty(),
                    "{:?}",
                    report.failed_comparisons
                );
                assert!(report.differences.is_empty(), "{:?}", report.differences);
            }
            let report = run_token_pack(&[b"mixed_advisor", b"MiXeD_Advisor"], &pack, load_hour);
            assert!(report.failed_comparisons.contains_key("AdvisorTokens"));
            assert_eq!(run_token_pack(&[], &pack, load_hour).differences.len(), 1);
        }
        let retired = pack.character_status.remove(b"AAA".as_slice()).unwrap();
        pack.retired_character_status
            .insert(b"AAA".to_vec(), retired);
        assert!(
            run_token_pack(&[b"MIXED_ADVISOR"], &pack, true)
                .differences
                .is_empty()
        );
    }

    #[test]
    fn advisor_token_case_variants_keep_runtime_only_holder_rules() {
        let all: [&[u8]; 4] = [b"ger_A", b"GER_A_THEORIST", b"sov_C", b"SOV_C_THEORIST"];
        let report = run_retired_tokens(&all, Some(true), None, false);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report.limitations["runtime-only advisor token: character not in the saved rosters"]
                .len(),
            2
        );
        let report = run_retired_tokens(&all, Some(true), None, true);
        assert_eq!(report.differences.len(), 2);
    }

    #[test]
    fn advisor_tokens_compare_with_the_registered_roles_of_advisor_characters() {
        let report = run_tokens(&[b"GER_a", b"GER_a_theorist"], Some(true));
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["AdvisorTokens"], 2);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("no writer saves"))
        );
        assert_eq!(
            run_tokens(&[b"GER_a", b"GER_a_theorist"], Some(false))
                .differences
                .len(),
            2
        );
        let report = run_tokens(&[b"GER_a", b"GER_b"], Some(true));
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        let report = run_tokens(&[b"GER_a", b"GER_a"], Some(true));
        assert!(report.failed_comparisons.contains_key("AdvisorTokens"));
        let report = run_tokens(&[], None);
        assert!(report.failed_comparisons.contains_key("AdvisorTokens"));
    }

    #[test]
    fn advisor_tokens_keep_every_role_of_a_retired_character() {
        let all: [&[u8]; 4] = [b"GER_a", b"GER_a_theorist", b"SOV_c", b"SOV_c_theorist"];
        for load_hour in [true, false] {
            for flag in [Some(true), Some(false), None] {
                let report = run_retired_tokens(&all, Some(true), Some(flag), load_hour);
                assert!(report.differences.is_empty(), "{:?}", report.differences);
                assert_eq!(report.compared_tables["AdvisorTokens"], 4);
            }
            assert_eq!(
                run_retired_tokens(
                    &[b"GER_a", b"GER_a_theorist"],
                    Some(true),
                    Some(Some(true)),
                    load_hour
                )
                .differences
                .len(),
                2
            );
        }
    }

    #[test]
    fn a_token_of_a_character_in_no_saved_list_is_runtime_only_except_at_a_load_hour() {
        const RUNTIME_ONLY: &str = "runtime-only advisor token: character not in the saved rosters";
        let all: [&[u8]; 4] = [b"GER_a", b"GER_a_theorist", b"SOV_c", b"SOV_c_theorist"];
        let report = run_retired_tokens(&all, Some(true), None, true);
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(!report.limitations.contains_key(RUNTIME_ONLY));
        let report = run_retired_tokens(&all, Some(true), None, false);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.limitations[RUNTIME_ONLY].len(), 2);
        assert_eq!(report.compared_tables["AdvisorTokens"], 4);
        for load_hour in [true, false] {
            let report = run_retired_tokens(&[b"GER_a"], Some(true), None, load_hour);
            assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
            let report = run_retired_tokens(
                &[b"GER_a", b"GER_a_theorist", b"XXX_unknown"],
                Some(true),
                None,
                load_hour,
            );
            assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
            let report = run_retired_tokens(
                &[b"GER_a", b"GER_a_theorist", b"GER_b"],
                Some(true),
                None,
                load_hour,
            );
            assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
            assert!(!report.limitations.contains_key(RUNTIME_ONLY));
        }
    }

    fn run_faction_advisors(rows: &[(u32, &[u8])], slots: Vec<Option<&[u8]>>) -> Report {
        let characters = saved::parse(
            &content_reader_parser::parser::parse(
                br#"character_manager={ historical={
                    character={ id={ id=1482 type=73 } token="GER_a"
                        advisors={ advisor={ slot="political_advisor" idea_token="GER_a_idea" }
                                   advisor={ slot="theorist" idea_token="GER_a_theorist" } } }
                    character={ id={ id=1451 type=73 } token="GER_b"
                        advisors={ advisor={ slot="spymaster" idea_token="GER_b_idea" } } }
                }} countries={ }"#,
            )
            .unwrap()
            .items,
        )
        .unwrap();
        let id = (1_u64 << 32) | 88;
        let factions = BTreeMap::from([(
            id,
            content_reader_save::Faction {
                id,
                goals: Default::default(),
                companions: Default::default(),
                name: None,
                members: Vec::new(),
                icon: None,
                template: None,
                color: None,
                color_overridden: None,
                theatres: Vec::new(),
                advisor_slots: slots
                    .into_iter()
                    .map(|slot| slot.map(<[u8]>::to_vec))
                    .collect(),
            },
        )]);
        let mut frame = frame();
        *frame.tables[TableId::FactionAdvisors.index()] =
            TableView::Complete(TableRows::FactionAdvisors(
                rows.iter()
                    .map(|&(ordinal, token)| replay_data::rows::FactionAdvisorRow {
                        faction: id,
                        ordinal,
                        token: token.to_vec(),
                    })
                    .collect(),
            ));
        let mut report = Report::default();
        report.attempt_table(TableId::FactionAdvisors, |report| {
            faction_advisors(&frame, &factions, &characters, report)
        });
        report
    }

    #[test]
    fn faction_advisor_slots_compare_occupancy_and_the_characters_roles() {
        let slots = || vec![Some(b"GER_b".as_slice()), None, Some(b"GER_a")];
        let report = run_faction_advisors(&[(0, b"GER_b_idea"), (2, b"GER_a_theorist")], slots());
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["FactionAdvisors"], 2);
        assert!(
            report
                .limitations
                .values()
                .any(|subjects| subjects.iter().any(|s| s.ends_with("[2]")))
        );
        let report = run_faction_advisors(&[(0, b"GER_a_idea"), (3, b"GER_b_idea")], slots());
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        let report = run_faction_advisors(&[(0, b"GER_b_idea")], vec![Some(b"GER_c")]);
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
    }

    #[test]
    fn civil_wars_compare_tags_in_order() {
        let report = run_civil_wars(&[103, 104], &[b"ITA", b"D01"]);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CountryCivilWars"], 2);
        assert_eq!(
            run_civil_wars(&[104, 103], &[b"ITA", b"D01"])
                .differences
                .len(),
            1
        );
        assert_eq!(run_civil_wars(&[], &[b"ITA"]).differences.len(), 1);
        assert_eq!(run_civil_wars(&[103], &[]).differences.len(), 1);
        let report = run_civil_wars(&[999], &[]);
        assert!(report.failed_comparisons.contains_key("CountryCivilWars"));
    }
}
