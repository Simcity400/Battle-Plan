
use super::{Report, TokenNames, complete, invalid, required_country_tag};
use content_reader_military::leader_traits::LeaderTraits;
use content_reader_parser::parser::c_string;
use content_reader_save::characters as saved;
use replay_data::{rows::CountryRow, rows::tables};
use replay_frame::frame::DecodedFrame;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

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

fn handle(kind: u32, id: u32) -> u64 {
    (u64::from(id) << 32) | u64::from(kind)
}

fn token_value(
    report: &mut Report,
    label: &str,
    tokens: &TokenNames<'_>,
    id: i32,
    expected: &[u8],
) {
    if let Some(actual) = tokens.get(&(id as u32)) {
        text_value(report, label, actual, expected);
    } else {
        report.limitation(
            "lexer literal is not recorded; this field's token spelling is unverified and neither native string slot is substituted",
            format!("{label} (native token {id})"),
        );
    }
}

fn trait_token_value(
    report: &mut Report,
    label: &str,
    tokens: &TokenNames<'_>,
    id: i32,
    expected: &[u8],
) {
    if tokens
        .get(&(id as u32))
        .is_some_and(|literal| literal.contains(&0))
    {
        report.limitation(
            "trait lexer literal contains an interior NUL; the text stream's projection is unproved, so this token spelling is unverified",
            label,
        );
    } else {
        token_value(report, label, tokens, id, expected);
    }
}

fn text_value(report: &mut Report, label: &str, actual: &[u8], expected: &[u8]) {
    equal(report, label, c_string(actual), expected);
    if c_string(actual).len() != actual.len() {
        report.limitation(
            "text serialization stops at the first NUL; captured suffix bytes are not comparable",
            label,
        );
    }
}

fn optional_text(report: &mut Report, label: &str, actual: &[u8], expected: Option<&[u8]>) {
    match (actual.is_empty(), expected) {
        (true, None) => {}
        (false, Some(expected)) => text_value(report, label, actual, expected),
        (_, expected) => report.difference(
            label,
            (!actual.is_empty()).then_some(c_string(actual)),
            expected,
        ),
    }
}

fn country<'a>(raw: u32, links: &BTreeMap<u32, &'a CountryRow>) -> io::Result<Option<&'a [u8]>> {
    if (raw as i32) <= 0 {
        Ok(None)
    } else {
        required_country_tag(raw, links).map(Some)
    }
}

fn membership<T: std::fmt::Debug>(report: &mut Report, table: &str, key: T, present: bool) {
    report.difference(format!("{table} {key:?} membership"), present, !present);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SavedRoster {
    Written { unloadable: usize },
    Loaded { unloadable: usize },
}

pub fn fit_saved_roster(
    frame: &DecodedFrame,
    manager: &mut saved::Manager,
    load_hour: bool,
) -> Option<SavedRoster> {
    let unloadable: Vec<u64> = manager
        .characters
        .values()
        .filter(|character| !character.survives_load())
        .map(|character| character.id)
        .collect();
    if unloadable.is_empty() {
        return None;
    }
    let loaded = load_hour
        || complete::<tables::Characters>(frame).is_ok_and(|rows| {
            let recorded: BTreeSet<u64> = rows.iter().map(|row| handle(73, row.id)).collect();
            unloadable.iter().all(|id| !recorded.contains(id))
        });
    let unloadable = unloadable.len();
    Some(if loaded {
        *manager = manager.loaded();
        SavedRoster::Loaded { unloadable }
    } else {
        SavedRoster::Written { unloadable }
    })
}

pub fn note_saved_roster(report: &mut Report, roster: SavedRoster) {
    let (reason, unloadable) = match roster {
        SavedRoster::Written { unloadable } => (
            "the save holds characters without a name, which loading destroys; the frame holds them, so its memory wrote the save and every saved character is compared",
            unloadable,
        ),
        SavedRoster::Loaded { unloadable } => (
            "the save holds characters without a name, which loading destroys; the frame's memory loaded the save (a load hour, or the frame holds none of them), so they and their roles are not expected and later roster positions close up",
            unloadable,
        ),
    };
    report.limitation(
        reason,
        format!("Characters and every character-keyed table: {unloadable} characters"),
    );
}

pub fn compare(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    country_leader_traits: Result<&LeaderTraits, &str>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete = true;
    if let Some(manager) = &pack.manager {
        complete &= report.attempt("Characters", |report| {
            compare_characters(frame, manager, links, tokens, report)
        });
        complete &= report.attempt("CountryLeaderRoles", |report| {
            compare_country_leader_roles(frame, manager, links, tokens, report)
        });
        complete &= report.attempt("AdvisorRoles", |report| {
            compare_advisors(frame, manager, links, tokens, country_leader_traits, report)
        });
        let mut unrepresented = None;
        complete &= report.attempt("UnitLeaders", |report| {
            unrepresented = Some(compare_leaders(
                frame,
                manager,
                &pack.character_status,
                links,
                report,
            )?);
            Ok(())
        });
        complete &= report.attempt("UnitLeaderTraits", |report| {
            compare_traits(frame, manager, tokens, unrepresented.as_ref(), report)
        });
        complete &= report.attempt("UnitLeaderTraitExperience", |report| {
            compare_trait_progress(frame, manager, tokens, report)
        });
        complete &= report.attempt("CharacterRoleTraits", |report| {
            compare_role_traits(frame, manager, tokens, country_leader_traits, report)
        });
        complete &= report.attempt("CharacterOperatives", |report| {
            compare_operatives(frame, manager, report)
        });
    } else {
        complete &= report.attempt("character manager", |_| {
            Err(invalid("save lacks character_manager; character, role and trait comparisons cannot treat missing coverage as an empty roster"))
        });
    }
    complete &= report.attempt("ActiveAbilities", |report| {
        compare_abilities(frame, pack, links, report)
    });
    report.limitation("saved country tags do not preserve native country-reference aliases; omitted character/ability scope references prove only signed-nonpositive values", "Characters, CountryLeaderRoles, AdvisorRoles, ActiveAbilities");
    if complete {
        Ok(())
    } else {
        Err(invalid(
            "one or more character or ability table comparisons failed; independent results and individual errors are retained",
        ))
    }
}

fn compare_characters(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut ordinals = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::Characters>(frame)? {
        let id = handle(73, row.id);
        if !seen.insert(id) || row.roster > 1 || !ordinals.insert((row.roster, row.ordinal)) {
            return Err(invalid(
                "duplicate character identity or invalid native roster position",
            ));
        }
        let Some(expected) = manager.characters.get(&id) else {
            membership(report, "character", id, true);
            continue;
        };
        count += 1;
        let label = format!("character {id}");
        equal(
            report,
            format!("{label}.roster"),
            row.roster,
            expected.roster,
        );
        equal(
            report,
            format!("{label}.ordinal"),
            row.ordinal,
            expected.ordinal,
        );
        equal(
            report,
            format!("{label}.country"),
            country(row.country, links)?,
            expected.country.as_deref(),
        );
        optional_text(
            report,
            &format!("{label}.name"),
            &row.token,
            expected.name.as_deref(),
        );
        token_value(
            report,
            &format!("{label}.token"),
            tokens,
            row.name_token_id,
            &expected.token,
        );
        let gender: &[u8] = match row.gender {
            1 => b"male",
            2 => b"female",
            _ => b"undefined",
        };
        if let Some(expected) = &expected.gender {
            text_value(report, &format!("{label}.gender"), gender, expected);
        } else {
            report.difference(
                format!("{label}.gender"),
                String::from_utf8_lossy(gender),
                "missing serialized gender",
            );
        }
        match (row.template_present != 0, &expected.template) {
            (false, None) => {}
            (true, Some(template)) => token_value(
                report,
                &format!("{label}.template"),
                tokens,
                row.template_tag as i32,
                template,
            ),
            (present, saved) => report.difference(
                format!("{label}.template presence"),
                present,
                saved.is_some(),
            ),
        }
    }
    for id in manager.characters.keys().filter(|id| !seen.contains(id)) {
        membership(report, "character", id, false);
    }
    report.compared("Characters", count);
    report.limitation("the captured character name is not serialized; the saved name comes from the captured token field", "Characters.name");
    report.limitation("the character writer saves only its template's tag; the template's source file and line and generated byte are runtime-only", "Characters.template_file, template_line, template_generated");
    Ok(())
}

fn compare_country_leader_roles(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut counts = BTreeMap::<u64, usize>::new();
    let mut roles = BTreeMap::<u64, BTreeMap<i32, _>>::new();
    for row in complete::<tables::CountryLeaderRoles>(frame)? {
        if !seen.insert((row.character, row.ideology)) {
            return Err(invalid("duplicate country leader role key"));
        }
        let character = handle(73, row.character);
        *counts.entry(character).or_default() += 1;
        roles
            .entry(character)
            .or_default()
            .insert(row.ideology, row);
        if let Some(expected) = manager.characters.get(&character) {
            equal(
                report,
                format!("country leader {character}/{}.country", row.ideology),
                country(row.country, links)?,
                expected.country.as_deref(),
            );
        } else {
            membership(report, "country leader character", character, true);
        }
    }
    let mut count = 0;
    for (&id, character) in &manager.characters {
        let actual = counts.remove(&id).unwrap_or(0);
        equal(
            report,
            format!("country leader character {id} role count"),
            actual,
            character.country_leaders.len(),
        );
        count += actual.min(character.country_leaders.len());
        if actual != character.country_leaders.len() {
            report.limitation(
                "current leader ideologies cannot be joined while the signed role-map census differs",
                format!("CountryLeaderRoles character {id}"),
            );
        } else if let Some(roles) = roles.get(&id) {
            for (row, saved) in roles.values().zip(&character.country_leaders) {
                let label = format!("country leader {id}/{}.current_ideology", row.ideology);
                if let Some(current) = report.recorded(
                    &row.current_ideology,
                    "CountryLeaderRoles.current_ideology",
                    &label,
                ) {
                    match current {
                        Some(Some(token)) => {
                            token_value(report, &label, tokens, *token as i32, &saved.ideology);
                        }
                        Some(None) => {
                            report.difference(&label, "native null ideology", &saved.ideology)
                        }
                        None => report.limitation(
                            "current country-leader ideology unavailable from native capture",
                            label,
                        ),
                    }
                }
            }
        }
    }
    report.compared("CountryLeaderRoles (per-character census)", count);
    report.limitation("country leader serializer does not write numeric role-map keys; the oracle compares the per-character census and joins trait and party-leader rows by signed map order", "CountryLeaderRoles.ideology, PartyLeaders.role");
    report.limitation("persistent country leader and advisor roles do not assert ruling status or cabinet appointment; those require same-hour political membership", "CountryLeaderRoles, AdvisorRoles");
    Ok(())
}

fn compare_advisors(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    traits: Result<&LeaderTraits, &str>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut count = 0;
    let role_traits = frame.complete::<tables::CharacterRoleTraits>().map(|rows| {
        rows.iter()
            .filter(|row| row.kind == replay_data::rows::CharacterRoleTraitRow::ADVISOR)
            .map(|row| ((row.character, row.role), row))
            .collect::<BTreeMap<_, _>>()
    });
    for row in complete::<tables::AdvisorRoles>(frame)? {
        let key = (handle(73, row.character), row.ordinal);
        if !seen.insert(key) {
            return Err(invalid("duplicate advisor role position"));
        }
        let label = format!("advisor {key:?}");
        if let Some(keys) = report.recorded(
            &row.tooltip_keys,
            "AdvisorRoles.tooltip_keys",
            format!("{label}.tooltip_keys"),
        ) {
            match keys {
                Some(keys) => {
                    report.limitation("the advisor save writer omits the tooltip string table; the structural content check cannot certify its occupied-slot order or refresh history", format!("{label}.tooltip_keys"));
                    compare_advisor_tooltip_keys(
                        row,
                        keys,
                        role_traits
                            .as_ref()
                            .map(|rows| rows.get(&(row.character, row.ordinal)).copied()),
                        tokens,
                        traits,
                        report,
                    );
                }
                None => report.limitation(
                    "AdvisorRoles.tooltip_keys is explicitly unavailable",
                    format!("{label}.tooltip_keys"),
                ),
            }
        }
        if let Some(cache) = report.recorded(
            &row.modifier_cache,
            "AdvisorRoles.modifier_cache",
            format!("{label}.modifier_cache"),
        ) {
            if cache.is_some() {
                report.limitation(
                    "the advisor save writer saves traits and the own modifier block, not the independently rebuilt modifier cache",
                    format!("{label}.modifier_cache.entries"),
                );
            } else {
                report.limitation(
                    "AdvisorRoles.modifier_cache is explicitly unavailable",
                    format!("{label}.modifier_cache"),
                );
            }
        }
        let Some(character) = manager.characters.get(&key.0) else {
            membership(report, "advisor character", key.0, true);
            continue;
        };
        let Some(expected) = character.advisors.get(row.ordinal as usize) else {
            membership(report, "advisor role", key, true);
            continue;
        };
        count += 1;
        equal(
            report,
            format!("{label}.country"),
            country(row.country, links)?,
            character.country.as_deref(),
        );
        optional_text(
            report,
            &format!("{label}.slot"),
            &row.slot,
            expected.slot.as_deref(),
        );
        optional_text(
            report,
            &format!("{label}.idea_token"),
            &row.idea,
            expected.idea.as_deref(),
        );
    }
    for (&id, character) in &manager.characters {
        for ordinal in 0..character.advisors.len() {
            let ordinal =
                u32::try_from(ordinal).map_err(|_| invalid("saved advisor ordinal exceeds u32"))?;
            if !seen.contains(&(id, ordinal)) {
                membership(report, "advisor role", (id, ordinal), false);
            }
        }
    }
    report.compared("AdvisorRoles", count);
    report.limitation("advisor serializer emits slot and idea_token from the role object, not the native map key; ordered role values are compared but map-key bytes are not", "AdvisorRoles.key");
    Ok(())
}

fn compare_advisor_tooltip_keys(
    row: &replay_data::rows::AdvisorRoleRow,
    keys: &[Vec<u8>],
    trait_row: Result<Option<&replay_data::rows::CharacterRoleTraitRow>, &io::Error>,
    tokens: &TokenNames<'_>,
    traits: Result<&LeaderTraits, &str>,
    report: &mut Report,
) {
    let label = format!("advisor {}/{}.tooltip_keys", row.character, row.ordinal);
    let Ok(traits) = traits else {
        report.limitation(
            "advisor tooltip structural check lacks the recorded content's trait definitions",
            label,
        );
        return;
    };
    let trait_row = match trait_row {
        Ok(row) => row,
        Err(reason) => {
            report.limitation(
                &format!("advisor tooltip structural check lacks CharacterRoleTraits: {reason}"),
                label,
            );
            return;
        }
    };
    let mut authored = BTreeSet::new();
    if let Some(trait_row) = trait_row {
        if advisor_trait_names(trait_row, tokens, Ok(traits)).is_err() {
            report.limitation(
                "advisor tooltip structural check cannot identify every recorded trait",
                label,
            );
            return;
        }
        for &ordinal in &trait_row.definitions {
            let definition = traits.by_ordinal(ordinal).unwrap();
            authored.extend(
                definition
                    .modifiers
                    .tooltips()
                    .into_iter()
                    .map(String::into_bytes),
            );
        }
    }
    for (ordinal, key) in keys.iter().enumerate() {
        if !authored.contains(key) {
            report.difference(
                format!("{label}[{ordinal}]"),
                key,
                "key in a recorded trait's content",
            );
        }
    }
    report.compared("AdvisorRoles.tooltip_keys (content membership)", keys.len());
}

pub fn leader_role_keys(frame: &DecodedFrame) -> io::Result<BTreeMap<u64, Vec<i32>>> {
    let mut keys = BTreeMap::<u64, Vec<i32>>::new();
    for row in complete::<tables::CountryLeaderRoles>(frame)? {
        keys.entry(handle(73, row.character))
            .or_default()
            .push(row.ideology);
    }
    for keys in keys.values_mut() {
        keys.sort_unstable();
    }
    Ok(keys)
}

fn compare_role_traits(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    tokens: &TokenNames<'_>,
    country_leader_traits: Result<&LeaderTraits, &str>,
    report: &mut Report,
) -> io::Result<()> {
    use replay_data::rows::CharacterRoleTraitRow as Row;
    let rows = complete::<tables::CharacterRoleTraits>(frame)?;
    let mut recorded = BTreeMap::<(u64, u8, u32), &Row>::new();
    for row in rows {
        recorded.insert((handle(73, row.character), row.kind, row.role), row);
    }
    let mut leader_keys = leader_role_keys(frame)?;
    let mut unnamed = Vec::new();
    let mut compared = 0;
    for (&id, character) in &manager.characters {
        let mut expected = BTreeMap::<(u8, u32), &[Vec<u8>]>::new();
        let keys = leader_keys.remove(&id).unwrap_or_default();
        if keys.len() == character.country_leaders.len() {
            for (key, role) in keys.iter().zip(&character.country_leaders) {
                if !role.traits.is_empty() {
                    expected.insert((Row::COUNTRY_LEADER, *key as u32), &role.traits);
                }
            }
        } else {
            for role in &character.country_leaders {
                if !role.traits.is_empty() {
                    report.difference(
                        format!(
                            "CharacterRoleTraits {id} country leader {}",
                            String::from_utf8_lossy(&role.ideology)
                        ),
                        format!("{} recorded roles", keys.len()),
                        format!("{} saved roles", character.country_leaders.len()),
                    );
                }
            }
        }
        for (ordinal, role) in character.advisors.iter().enumerate() {
            if !role.traits.is_empty() {
                let ordinal = u32::try_from(ordinal)
                    .map_err(|_| invalid("saved advisor ordinal exceeds u32"))?;
                expected.insert((Row::ADVISOR, ordinal), &role.traits);
            }
        }
        if let Some(traits) = character
            .scientist
            .as_ref()
            .and_then(|scientist| scientist.traits.as_deref())
        {
            expected.insert((Row::SCIENTIST, 0), traits);
        }
        let recorded_keys: Vec<(u8, u32)> = recorded
            .range((id, 0, 0)..=(id, u8::MAX, u32::MAX))
            .map(|(&(_, kind, role), _)| (kind, role))
            .collect();
        for key in recorded_keys {
            let actual = recorded.remove(&(id, key.0, key.1)).unwrap();
            let label = format!("CharacterRoleTraits {id}/{}/{}", key.0, key.1);
            let Some(saved) = expected.remove(&key) else {
                membership(report, "CharacterRoleTraits", (id, key), true);
                continue;
            };
            let names = if key.0 == Row::ADVISOR {
                advisor_trait_names(actual, tokens, country_leader_traits)
            } else {
                actual
                    .tokens
                    .iter()
                    .map(|token| {
                        tokens
                            .get(token)
                            .map(|name| c_string(name))
                            .ok_or_else(|| Unnamed::Limitation(format!("token {token}")))
                    })
                    .collect()
            };
            let names = match names {
                Ok(names) => names,
                Err(Unnamed::Limitation(why)) => {
                    unnamed.push(format!("{label} {why}"));
                    continue;
                }
                Err(Unnamed::Difference {
                    ordinal,
                    token,
                    key,
                }) => {
                    report.difference(
                        format!("{label} definition {ordinal}: recorded token, content key"),
                        token,
                        key,
                    );
                    continue;
                }
            };
            compared += 1;
            report.scalar_checks += 1;
            let saved: Vec<&[u8]> = saved.iter().map(Vec::as_slice).collect();
            if names != saved {
                let text = |names: &[&[u8]]| {
                    names
                        .iter()
                        .map(|name| String::from_utf8_lossy(name).into_owned())
                        .collect::<Vec<_>>()
                };
                report.difference(label, text(&names), text(&saved));
            }
        }
        for key in expected.into_keys() {
            membership(report, "CharacterRoleTraits", (id, key), false);
        }
    }
    for (id, kind, role) in recorded.into_keys() {
        membership(
            report,
            "CharacterRoleTraits character",
            (id, kind, role),
            true,
        );
    }
    if !unnamed.is_empty() {
        report.limitation(
            "role traits the recorded lexical dictionary, or for an advisor the content's country-leader traits, cannot name are not compared",
            unnamed.join(", "),
        );
    }
    report.compared("CharacterRoleTraits", compared);
    report.limitation(
        "the country-leader role writer saves traits by token rather than database ordinal, so the recorded country-leader definitions are not compared; an advisor row's definitions select the keys used for its saved trait names",
        "CharacterRoleTraits.definitions",
    );
    Ok(())
}

enum Unnamed {
    Limitation(String),
    Difference {
        ordinal: u32,
        token: String,
        key: String,
    },
}
impl From<String> for Unnamed {
    fn from(why: String) -> Self {
        Self::Limitation(why)
    }
}

fn advisor_trait_names<'a>(
    row: &replay_data::rows::CharacterRoleTraitRow,
    tokens: &TokenNames<'_>,
    content: Result<&'a LeaderTraits, &str>,
) -> Result<Vec<&'a [u8]>, Unnamed> {
    let content =
        content.map_err(|why| format!("(content's country-leader traits unavailable: {why})"))?;
    if row.definitions.len() != row.tokens.len() {
        return Err(format!(
            "({} definitions for {} tokens)",
            row.definitions.len(),
            row.tokens.len()
        )
        .into());
    }
    let mut names = Vec::with_capacity(row.tokens.len());
    for (&token, &ordinal) in row.tokens.iter().zip(&row.definitions) {
        let key = content
            .by_ordinal(ordinal)
            .ok_or_else(|| format!("definition {ordinal} (not determined by the content)"))?
            .token
            .as_bytes();
        let refused = key
            .first()
            .is_none_or(|&first| first == b'-' || first.is_ascii_digit());
        let literal = tokens.get(&token).map(|literal| c_string(literal));
        let made = if refused {
            token == 0
        } else {
            literal
                .ok_or_else(|| format!("token {token}"))?
                .eq_ignore_ascii_case(key)
        };
        if !made {
            return Err(Unnamed::Difference {
                ordinal,
                token: match literal {
                    Some(literal) => format!("{token} {}", String::from_utf8_lossy(literal)),
                    None => token.to_string(),
                },
                key: String::from_utf8_lossy(key).into_owned(),
            });
        }
        names.push(key);
    }
    Ok(names)
}

fn compare_operatives(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    report: &mut Report,
) -> io::Result<()> {
    let rows = complete::<tables::CharacterOperatives>(frame)?;
    let mut recorded: BTreeMap<u64, u64> = rows
        .iter()
        .map(|row| (handle(73, row.character), row.reference))
        .collect();
    for (&id, character) in &manager.characters {
        let actual = recorded.remove(&id);
        report.scalar_checks += 1;
        equal(
            report,
            format!("CharacterOperatives {id}.reference"),
            actual,
            character.operative,
        );
    }
    for id in recorded.into_keys() {
        membership(report, "CharacterOperatives character", id, true);
    }
    report.compared("CharacterOperatives", rows.len());
    report.limitation(
        "the resolved unit leader's category, owner, skill level and trait definitions are not saved with the character",
        "CharacterOperatives.resolved, category, owner, skill_level, trait_definitions",
    );
    Ok(())
}

pub fn compare_codenames(
    frame: &DecodedFrame,
    pack: &content_reader_save::intel::Pack,
    report: &mut Report,
) -> io::Result<()> {
    let mut saved = BTreeMap::new();
    for (_, country) in report.canonical_countries(&pack.countries) {
        let Some(agency) = &country.agency else {
            continue;
        };
        let recruitment = agency.recruitment.iter().flat_map(|recruitment| {
            recruitment
                .recruitable
                .iter()
                .chain(&recruitment.not_to_spy_master)
        });
        for entry in agency.operatives.iter().chain(recruitment) {
            if let Some(previous) = saved.insert(entry.id, entry)
                && previous.codename != entry.codename
            {
                return Err(invalid("conflicting codename for repeated saved operative"));
            }
        }
    }
    let mut complete_tables = report.attempt("CharacterOperatives codenames", |report| {
        let mut compared = 0;
        for row in complete::<tables::CharacterOperatives>(frame)? {
            let name_label = format!("CharacterOperatives {}.name", row.character);
            if let Some(name) = report.recorded(
                &row.name,
                "CharacterOperatives.name",
                name_label.clone(),
            ) {
                if name.is_some() {
                    report.limitation(
                        "the unit-leader writer saves the stored name, while the captured localized string is not serialized by the game",
                        name_label,
                    );
                } else {
                    report.limitation(
                        "CharacterOperatives.name is explicitly unavailable",
                        name_label,
                    );
                }
            }
            let label = format!("CharacterOperatives {} codename", row.character);
            let Some(actual) = report.recorded(
                &row.codename,
                "CharacterOperatives.codename",
                label.clone(),
            ) else {
                continue;
            };
            let Some(actual) = actual.as_ref() else {
                report.limitation(
                    "CharacterOperatives.codename is explicitly unavailable",
                    label,
                );
                continue;
            };
            if !row.resolved {
                continue;
            }
            compared += usize::from(codename(
                report,
                &label,
                actual,
                saved.get(&row.reference).copied(),
            ));
        }
        report.compared("CharacterOperatives codenames", compared);
        Ok(())
    });
    complete_tables &= report.attempt("UnitLeaders codenames", |report| {
        let mut compared = 0;
        for row in complete::<tables::UnitLeaders>(frame)?
            .iter()
            .filter(|row| engine_native::cunit_leader::has_codename(row.kind))
        {
            let label = format!("UnitLeaders {} codename", row.id);
            let Some(actual) =
                report.recorded(&row.codename, "UnitLeaders.codename", label.clone())
            else {
                continue;
            };
            let Some(actual) = actual.as_ref() else {
                report.limitation("UnitLeaders.codename is explicitly unavailable", label);
                continue;
            };
            compared += usize::from(codename(
                report,
                &label,
                actual,
                saved.get(&handle(row.reference_type, row.id)).copied(),
            ));
        }
        report.compared("UnitLeaders codenames", compared);
        Ok(())
    });
    report.limitation(
        "the game omits the codename member's group key",
        "CharacterOperatives.codename.group, UnitLeaders.codename.group",
    );
    if complete_tables {
        Ok(())
    } else {
        Err(invalid("operative codename table comparison failed"))
    }
}

fn codename(
    report: &mut Report,
    label: &str,
    actual: &replay_data::rows::NameOperands,
    entry: Option<&content_reader_save::intel::OperativeEntry>,
) -> bool {
    let Some(entry) = entry else {
        report.limitation(
            "the operative has no saved agency or recruitment entry; captured-only and unsaved holding vectors cannot certify its codename",
            label,
        );
        return false;
    };
    let Some(expected) = &entry.codename else {
        report.difference(label, "present", "absent");
        return true;
    };
    report.scalar_checks += 4;
    equal(report, format!("{label} type"), actual.kind, expected.kind);
    equal(
        report,
        format!("{label} name_order"),
        actual.order,
        expected.order.unwrap_or(0),
    );
    equal(
        report,
        format!("{label} is_name_ordered"),
        actual.ordered != 0,
        expected.ordered.unwrap_or(true),
    );
    equal(
        report,
        format!("{label} override_set_programmatically"),
        actual.programmatic != 0,
        expected.programmatic.unwrap_or(false),
    );
    if actual.programmatic == 0 || !actual.custom.is_empty() {
        optional_text(
            report,
            &format!("{label} override"),
            &actual.custom,
            expected.custom.as_deref(),
        );
    } else {
        equal(
            report,
            format!("{label} override"),
            actual.custom.as_slice(),
            b"".as_slice(),
        );
        report.limitation(
            "the game hides a programmatic override for a resolved type-two group; the save retains the stored override regardless, so the omitted displayed bytes cannot certify the stored override",
            label,
        );
    }
    true
}

fn compare_leaders(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    rosters: &BTreeMap<Vec<u8>, Vec<saved::CharacterStatus>>,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<BTreeSet<u64>> {
    let mut rostered = BTreeMap::<u64, Option<&[u8]>>::new();
    for (tag, statuses) in rosters {
        for status in statuses {
            rostered
                .entry(status.character)
                .and_modify(|owner| {
                    if *owner != Some(tag.as_slice()) {
                        *owner = None;
                    }
                })
                .or_insert(Some(tag.as_slice()));
        }
    }
    let mut seen = BTreeSet::new();
    let mut parents = BTreeSet::new();
    let mut unrepresented = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::UnitLeaders>(frame)? {
        let id = handle(row.reference_type, row.id);
        let character = handle(row.character_type, row.character_id);
        if id == 0 || !seen.insert(id) || !parents.insert(character) {
            return Err(invalid(
                "null/duplicate leader identity or repeated character unit role",
            ));
        }
        if !manager.characters.contains_key(&character) {
            membership(report, "unit leader character", character, true);
        }
        let Some(expected) = manager.unit_leaders.get(&id) else {
            if !(0..=2).contains(&row.category) {
                unrepresented.insert(id);
                report.limitation("character writer serializes unit roles only for categories 0, 1 and 2; other categories and their trait state have no manager-save comparison", format!("leader {id} category {}", row.category));
            } else {
                membership(report, "unit leader", id, true);
            }
            continue;
        };
        count += 1;
        let label = format!("unit leader {id}");
        equal(
            report,
            format!("{label}.character"),
            character,
            expected.character,
        );
        equal(
            report,
            format!("{label}.category"),
            row.category,
            expected.category,
        );
        optional_text(
            report,
            &format!("{label}.name"),
            &row.inherited_text_a,
            expected.name.as_deref(),
        );
        if row.skill_status == 0 {
            report.difference(
                format!("{label}.skill"),
                "null native skill pointer",
                expected.skill,
            );
        } else {
            equal(
                report,
                format!("{label}.skill"),
                row.skill_level,
                expected.skill,
            );
        }
        equal(
            report,
            format!("{label}.components"),
            row.components,
            expected.components,
        );
        report.scalar(
            &label,
            "experience",
            Some(row.experience),
            Some(&expected.experience),
        );
        if row.kind == engine_native::cunit_leader::KIND_ARMY {
            equal(
                report,
                format!("{label}.captured"),
                row.captured != 0,
                expected.captured,
            );
        } else if row.captured != 0 {
            report.difference(
                format!("{label}.captured"),
                row.captured,
                "no army leader byte",
            );
        }
        if let Some(&exile_country) = report.recorded(
            &row.exile_country,
            "UnitLeaders.exile_country",
            format!("{label}.government_in_exile_tag"),
        ) {
            let exile = if exile_country > 0 {
                country(exile_country as u32, links)?
            } else {
                None
            };
            equal(
                report,
                format!("{label}.government_in_exile_tag"),
                exile,
                expected.government_in_exile_tag.as_deref(),
            );
            if exile_country < 0 {
                report.limitation("nonpositive exile-country references are omitted by the game; only the absent-tag predicate is certified", format!("{label}.exile_country"));
            }
        }
        let owner = country(row.owner, links)?;
        match rostered.get(&expected.character) {
            Some(Some(tag)) => equal(report, format!("{label}.owner"), owner, Some(*tag)),
            Some(None) => report.limitation(
                "the leader's character is on several countries' rosters; the roster loaded last sets the owner, and the save does not keep their load order",
                format!("{label}.owner"),
            ),
            None if owner.is_some()
                && owner
                    == manager
                        .characters
                        .get(&expected.character)
                        .and_then(|saved| saved.country.as_deref()) =>
            {
                report.limitation(
                    "a leader built in this run takes its off-roster character's country, which loading leaves without an owner; the save cannot tell the two apart",
                    format!("{label}.owner"),
                );
            }
            None => equal(report, format!("{label}.owner"), owner, None),
        }
    }
    for id in manager.unit_leaders.keys().filter(|id| !seen.contains(id)) {
        membership(report, "unit leader", id, false);
    }
    report.compared("UnitLeaders", count);
    report.limitation("unit leader secondary inherited text, native class discriminator, skill validity byte, skill definition category and required experience are not serialized by the compared writers", "UnitLeaders.inherited_text_b/kind/skill_valid/skill_category/skill_required_experience");
    report.limitation(
        "unit-leader visibility inputs (choice presence and visibility state) are not serialized",
        "UnitLeaders.visibility_choice/template_visibility",
    );
    report.limitation("the unit leader save writer and the army leader save writer omit the active formation reference. The army writer's optional formation token reads a distinct reference. Saved group membership cannot certify the exact active reference used by ability late-join and communications gates; a matched-hour native comparison remains necessary",
        "UnitLeaders.formation_reference");
    Ok(unrepresented)
}

fn compare_traits(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    tokens: &TokenNames<'_>,
    unrepresented: Option<&BTreeSet<u64>>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut count = 0;
    let mut formatted_tokens = Vec::new();
    for row in complete::<tables::UnitLeaderTraits>(frame)? {
        let key = (row.leader_reference, row.ordinal);
        if !seen.insert(key) {
            return Err(invalid("duplicate assigned trait position"));
        }
        if !trait_owner_comparable(manager, unrepresented, row.leader_reference, report) {
            continue;
        }
        let Some(expected) = manager
            .unit_leaders
            .get(&row.leader_reference)
            .and_then(|leader| leader.traits.get(row.ordinal as usize))
        else {
            membership(report, "assigned trait", key, true);
            continue;
        };
        count += 1;
        if row.token_id == engine_native_memory::cpdx_lexer::FIXED15_TOKEN as i32 {
            formatted_tokens.push(key);
        } else {
            trait_token_value(
                report,
                &format!("leader trait {key:?}.token"),
                tokens,
                row.token_id,
                expected,
            );
        }
    }
    for (&id, leader) in &manager.unit_leaders {
        for ordinal in 0..leader.traits.len() {
            let ordinal =
                u32::try_from(ordinal).map_err(|_| invalid("saved trait ordinal exceeds u32"))?;
            if !seen.contains(&(id, ordinal)) {
                membership(report, "assigned trait", (id, ordinal), false);
            }
        }
    }
    report.compared("UnitLeaderTraits", count);
    report.limitation("trait serializer resolves the native lexer ID; captured definition ordinals and separate definition-string bytes are not serialized", "UnitLeaderTraits");
    if formatted_tokens.is_empty() {
        Ok(())
    } else {
        Err(invalid(format!(
            "assigned traits use fixed15 formatter token 0x167, not a directly serialized lexical spelling: {formatted_tokens:?}"
        )))
    }
}

fn compare_trait_progress(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let rows = complete::<tables::UnitLeaderTraitExperience>(frame)?;
    for row in rows {
        let key = (row.leader_reference, row.list, row.ordinal);
        if !seen.insert(key) {
            return Err(invalid("duplicate training position"));
        }
        let Some(expected) = manager
            .unit_leaders
            .get(&row.leader_reference)
            .and_then(|leader| {
                match row.list {
                    0 => &leader.trait_experience,
                    1 => &leader.trait_factors,
                    _ => return None,
                }
                .get(row.ordinal as usize)
            })
        else {
            membership(report, "leader training", key, true);
            continue;
        };
        trait_token_value(
            report,
            &format!("leader training {key:?}.token"),
            tokens,
            row.token_id,
            &expected.token,
        );
        report.scalar(
            &format!("leader training {key:?}"),
            "experience",
            Some(row.experience),
            Some(&expected.experience),
        );
    }
    for (&id, leader) in &manager.unit_leaders {
        for (list, entries) in [(0, &leader.trait_experience), (1, &leader.trait_factors)] {
            for ordinal in 0..entries.len() {
                let ordinal =
                    u32::try_from(ordinal).map_err(|_| invalid("training ordinal exceeds u32"))?;
                if !seen.contains(&(id, list, ordinal)) {
                    membership(report, "leader training", (id, list, ordinal), false);
                }
            }
        }
    }

    report.compared("UnitLeaderTraitExperience", rows.len());
    Ok(())
}

fn trait_owner_comparable(
    manager: &saved::Manager,
    unrepresented: Option<&BTreeSet<u64>>,
    leader: u64,
    report: &mut Report,
) -> bool {
    if manager.unit_leaders.contains_key(&leader) {
        return true;
    }
    if let Some(unrepresented) = unrepresented {
        return !unrepresented.contains(&leader);
    }
    report.limitation(
        "unit leader comparison failed and this trait owner has no saved unit role; its native category cannot establish whether the save omits the role, so no extra-role membership is asserted",
        format!("leader {leader}"),
    );
    false
}

fn compare_abilities(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::ActiveAbilities>(frame)? {
        let owner = required_country_tag(row.country, links)?;
        let key = (owner.to_vec(), row.ordinal);
        if !seen.insert(key.clone()) {
            return Err(invalid("duplicate country ability position"));
        }
        let Some(expected) = pack
            .abilities
            .get(owner)
            .and_then(|rows| rows.get(row.ordinal as usize))
        else {
            membership(report, "active ability", &key, true);
            continue;
        };
        count += 1;
        let label = format!("active ability {key:?}");
        text_value(report, &format!("{label}.id"), &row.token, &expected.token);
        equal(
            report,
            format!("{label}.start_date"),
            row.activation_hour,
            expected.start_date.engine_hour()?,
        );
        equal(
            report,
            format!("{label}.country"),
            country(row.scope_country, links)?,
            expected.country.as_deref(),
        );
        equal(
            report,
            format!("{label}.leader"),
            row.caster,
            expected.leader,
        );
        equal(
            report,
            format!("{label}.num_units"),
            row.caster.map(|_| row.division_count),
            expected.num_units,
        );
        equal(
            report,
            format!("{label}.strategic_region"),
            row.strategic_region.map(|id| id as i32),
            expected.strategic_region,
        );
        match (row.allocated_command_power, &expected.cost) {
            (actual, Some(expected)) => report.scalar(&label, "cost", Some(actual), Some(expected)),
            (actual, None) if actual > 0 => report.difference(
                format!("{label}.cost"),
                actual,
                "missing positive serialized cost",
            ),
            (_, None) => {}
        }
    }
    for (owner, rows) in report.canonical_countries(&pack.abilities) {
        for ordinal in 0..rows.len() {
            let ordinal =
                u32::try_from(ordinal).map_err(|_| invalid("saved ability ordinal exceeds u32"))?;
            if !seen.contains(&(owner.clone(), ordinal)) {
                membership(report, "active ability", (owner, ordinal), false);
            }
        }
    }
    report.compared("ActiveAbilities", count);
    report.limitation("ability division count is initialized and captured for every record, but the country writer omits it when no caster resolves", "ActiveAbilities.division_count");
    report.limitation("ability phase, allocation flag, duration, cooldown and target kind are not serialized by the country writer; omitted allocated cost proves only nonpositive raw value", "ActiveAbilities.phase/allocate_cost/duration_hours/cooldown_hours/target_kind");
    report.limitation("ability ordinal is a native vector position reused by swap removal; the oracle compares current state, not ability lifetime identity or instantaneous effects", "ActiveAbilities");
    Ok(())
}

#[cfg(test)]
mod tests;
