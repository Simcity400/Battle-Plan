//! Character and persistent-ability comparisons against native text serialization.

use super::{Report, TokenNames, complete, invalid, required_country_tag};
use installed_content::save::characters as saved;
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, TableId, tables},
};
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
        // AAC50 forwards c_str to stream slot +30. The concrete text target
        // has not established how an interior NUL affects the saved token.
        report.limitation(
            "trait lexer literal contains an interior NUL; the text stream's projection is unproved, so this token spelling is unverified",
            label,
        );
    } else {
        token_value(report, label, tokens, id, expected);
    }
}

fn c_string(value: &[u8]) -> &[u8] {
    &value[..value
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(value.len())]
}

fn text_value(report: &mut Report, label: &str, actual: &[u8], expected: &[u8]) {
    // ADF40 -> AF5A0 consumes c_str; bytes after the first NUL are not saved.
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

pub(super) fn compare(
    frame: &DecodedFrame,
    pack: &saved::Pack,
    links: &BTreeMap<u32, &CountryRow>,
    tokens: &TokenNames<'_>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete = true;
    let tables = [
        TableId::Characters,
        TableId::CountryLeaderRoles,
        TableId::AdvisorRoles,
        TableId::UnitLeaders,
        TableId::UnitLeaderTraits,
        TableId::UnitLeaderTraitExperience,
    ];
    let current = tables.map(|table| report.refresh_is_current(table, frame.hour));
    for (table, current) in tables.into_iter().zip(current) {
        if !current {
            report.limitation(
                &format!("Characters family {}", replay::cadence::STAGGER_LIMITATION),
                format!(
                    "{table:?} refreshes on {} (oracle hour {})",
                    replay::cadence::refresh_text(table),
                    frame.hour
                ),
            );
        }
    }
    if let Some(manager) = &pack.manager {
        complete &= report.attempt("Characters", |report| {
            if !current[0] {
                return Ok(());
            }
            compare_characters(frame, manager, links, tokens, report)
        });
        complete &= report.attempt("CountryLeaderRoles", |report| {
            if !current[1] {
                return Ok(());
            }
            compare_country_leader_roles(frame, manager, links, report)
        });
        complete &= report.attempt("AdvisorRoles", |report| {
            if !current[2] {
                return Ok(());
            }
            compare_advisors(frame, manager, links, report)
        });
        let mut unrepresented = None;
        complete &= report.attempt("UnitLeaders", |report| {
            if !current[3] {
                return Ok(());
            }
            unrepresented = Some(compare_leaders(frame, manager, report)?);
            Ok(())
        });
        complete &= report.attempt("UnitLeaderTraits", |report| {
            if !current[4] {
                return Ok(());
            }
            compare_traits(frame, manager, tokens, unrepresented.as_ref(), report)
        });
        complete &= report.attempt("UnitLeaderTraitExperience", |report| {
            if !current[5] {
                return Ok(());
            }
            compare_trait_experience(frame, manager, tokens, unrepresented.as_ref(), report)
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
        // The frozen row names follow the harvested layout. F913A0 emits +48
        // (row.token) as name, and resolves +18 separately for saved token.
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
    }
    for id in manager.characters.keys().filter(|id| !seen.contains(id)) {
        membership(report, "character", id, false);
    }
    report.compared("Characters", count);
    report.limitation("CCharacter +28 (captured name field) is not serialized; saved name comes from captured +48 token field", "Characters.name");
    Ok(())
}

fn compare_country_leader_roles(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut counts = BTreeMap::<u64, usize>::new();
    for row in complete::<tables::CountryLeaderRoles>(frame)? {
        if !seen.insert((row.character, row.ideology)) {
            return Err(invalid("duplicate country leader role key"));
        }
        let character = handle(73, row.character);
        *counts.entry(character).or_default() += 1;
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
    }
    report.compared("CountryLeaderRoles (per-character census)", count);
    report.limitation("country leader serializer writes the role's ideology definition; the captured native map key is not serialized directly and its runtime equivalence is unproved; only per-character role census and country are compared", "CountryLeaderRoles.ideology");
    report.limitation("persistent country leader and advisor roles do not assert ruling status or cabinet appointment; those require same-hour political membership", "CountryLeaderRoles, AdvisorRoles");
    Ok(())
}

fn compare_advisors(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    links: &BTreeMap<u32, &CountryRow>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for row in complete::<tables::AdvisorRoles>(frame)? {
        let key = (handle(73, row.character), row.ordinal);
        if !seen.insert(key) {
            return Err(invalid("duplicate advisor role position"));
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
        let label = format!("advisor {key:?}");
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

fn compare_leaders(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    report: &mut Report,
) -> io::Result<BTreeSet<u64>> {
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
        report.scalar(
            &label,
            "experience",
            Some(row.experience),
            Some(&expected.experience),
        );
        equal(
            report,
            format!("{label}.components"),
            row.components,
            expected.components,
        );
    }
    for id in manager.unit_leaders.keys().filter(|id| !seen.contains(id)) {
        membership(report, "unit leader", id, false);
    }
    report.compared("UnitLeaders", count);
    report.limitation("unit leader secondary inherited text, native class discriminator, skill validity byte, skill definition category and required experience are not serialized by the compared writers", "UnitLeaders.inherited_text_b/kind/skill_valid/skill_category/skill_required_experience");
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
        // AAC50 formats reserved token 0x167 as fixed15 even in a trait
        // position. Its dictionary spelling is not the serialized value.
        if row.token_id == 0x167 {
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

fn compare_trait_experience(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    tokens: &TokenNames<'_>,
    unrepresented: Option<&BTreeSet<u64>>,
    report: &mut Report,
) -> io::Result<()> {
    let mut seen = BTreeSet::new();
    let mut count = 0;
    let mut formatted_tokens = Vec::new();
    for row in complete::<tables::UnitLeaderTraitExperience>(frame)? {
        let key = (row.leader_reference, row.ordinal);
        if !seen.insert(key) {
            return Err(invalid("duplicate trait experience position"));
        }
        if !trait_owner_comparable(manager, unrepresented, row.leader_reference, report) {
            continue;
        }
        let Some(expected) = manager
            .unit_leaders
            .get(&row.leader_reference)
            .and_then(|leader| leader.trait_experience.get(row.ordinal as usize))
        else {
            membership(report, "trait experience", key, true);
            continue;
        };
        count += 1;
        let label = format!("leader trait experience {key:?}");
        // AE590 writes its key through AAC50; its reserved fixed15 token
        // needs formatting rather than a literal dictionary comparison.
        if row.token_id == 0x167 {
            formatted_tokens.push(key);
        } else {
            trait_token_value(
                report,
                &format!("{label}.token"),
                tokens,
                row.token_id,
                &expected.token,
            );
        }
        report.scalar(
            &label,
            "experience",
            Some(row.experience),
            Some(&expected.experience),
        );
    }
    for (&id, leader) in &manager.unit_leaders {
        for ordinal in 0..leader.trait_experience.len() {
            let ordinal = u32::try_from(ordinal)
                .map_err(|_| invalid("saved trait experience ordinal exceeds u32"))?;
            if !seen.contains(&(id, ordinal)) {
                membership(report, "trait experience", (id, ordinal), false);
            }
        }
    }
    report.compared("UnitLeaderTraitExperience", count);
    report.limitation("trait serializer resolves the native lexer ID; captured definition ordinals and separate definition-string bytes are not serialized", "UnitLeaderTraitExperience");
    if formatted_tokens.is_empty() {
        Ok(())
    } else {
        Err(invalid(format!(
            "trait experience keys use fixed15 formatter token 0x167, not a directly serialized lexical spelling: {formatted_tokens:?}"
        )))
    }
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
    for (owner, rows) in &pack.abilities {
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
mod tests {
    use super::*;
    use installed_content::save::{Decimal, GameDate};
    use replay::{
        frame::{Metrics, TableView},
        rows::{
            ActiveAbilityRow, CharacterRow, TableId, TableRows, UnitLeaderRow,
            UnitLeaderTraitExperienceRow, UnitLeaderTraitRow,
        },
    };

    fn fixture() -> (DecodedFrame, saved::Pack, CountryRow) {
        let character = handle(73, u32::MAX);
        let leader = handle(4713, u32::MAX);
        // Exercise all persistent tables on their shared refresh slot. The
        // cadence-specific test below explicitly supplies an off-slot hour.
        // Save hours are one-based; slot 21 is text-save hour 22.
        let date = GameDate::parse(b"1936.1.1.22").unwrap();
        let mut frame = DecodedFrame {
            branch: 1,
            hour: date.engine_hour().unwrap(),
            tables: std::array::from_fn(|index| {
                TableView::Complete(TableRows::empty(TableId::ALL[index]))
            })
            .map(Into::into),
            metrics: Metrics::default(),
        };
        *frame.tables[TableId::Characters.index()] =
            TableView::Complete(TableRows::Characters(vec![CharacterRow {
                id: u32::MAX,
                country: 102,
                name_token_id: 5,
                roster: 1,
                ordinal: 0,
                name: b"unsaved slot".to_vec(),
                token: b"saved literal name".to_vec(),
            }]));
        *frame.tables[TableId::UnitLeaders.index()] =
            TableView::Complete(TableRows::UnitLeaders(vec![UnitLeaderRow {
                reference_type: 4713,
                id: u32::MAX,
                character_type: 73,
                character_id: u32::MAX,
                category: 2,
                kind: 2,
                skill_status: 2,
                skill_valid: 1,
                skill_level: i32::MAX,
                experience: -9_007_199_254_740_993,
                components: [i32::MIN, 3, 2, 1],
                ..Default::default()
            }]));
        *frame.tables[TableId::UnitLeaderTraits.index()] =
            TableView::Complete(TableRows::UnitLeaderTraits(vec![UnitLeaderTraitRow {
                leader_reference: leader,
                ordinal: 0,
                token_id: 6,
                token: b"separate unsaved token bytes".to_vec(),
                ..Default::default()
            }]));
        *frame.tables[TableId::UnitLeaderTraitExperience.index()] =
            TableView::Complete(TableRows::UnitLeaderTraitExperience(vec![
                UnitLeaderTraitExperienceRow {
                    leader_reference: leader,
                    ordinal: 0,
                    token_id: 7,
                    experience: 1,
                    ..Default::default()
                },
            ]));
        let pack = saved::Pack {
            portraits: Default::default(),
            manager: Some(saved::Manager {
                characters: BTreeMap::from([(
                    character,
                    saved::Character {
                        portraits: None,
                        id: character,
                        country: Some(b"GER".to_vec()),
                        roster: 1,
                        ordinal: 0,
                        token: b"script_token".to_vec(),
                        name: Some(b"saved literal name".to_vec()),
                        country_leaders: Vec::new(),
                        advisors: Vec::new(),
                        scientist: None,
                    },
                )]),
                unit_leaders: BTreeMap::from([(
                    leader,
                    saved::UnitLeader {
                        id: leader,
                        character,
                        category: 2,
                        name: None,
                        skill: i32::MAX,
                        experience: Decimal::parse(b"-90071992547.40993").unwrap(),
                        components: [i32::MIN, 3, 2, 1],
                        traits: vec![b"assigned".to_vec()],
                        trait_experience: vec![saved::TraitExperience {
                            token: b"progress".to_vec(),
                            experience: Decimal::parse(b"0.00001").unwrap(),
                        }],
                    },
                )]),
            }),
            abilities: BTreeMap::from([(b"GER".to_vec(), Vec::new())]),
        };
        let country = CountryRow {
            id: 102,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        (frame, pack, country)
    }

    #[test]
    fn character_tables_are_compared_only_when_fresh() {
        let (mut frame, pack, country) = fixture();
        frame.hour = 1;
        let links = BTreeMap::from([(102, &country)]);
        let tokens = BTreeMap::from([
            (5, b"script_token".as_slice()),
            (6, b"assigned"),
            (7, b"progress"),
        ]);
        let TableView::Complete(TableRows::UnitLeaderTraitExperience(rows)) =
            &mut *frame.tables[TableId::UnitLeaderTraitExperience.index()]
        else {
            unreachable!()
        };
        rows[0].experience += 1;
        let mut mixed = Report::default();
        mixed.record_tables(&frame);
        compare(&frame, &pack, &links, &tokens, &mut mixed).unwrap();
        assert!(mixed.differences.is_empty());
        frame.hour = 21;
        let mut current = Report::default();
        current.record_tables(&frame);
        compare(&frame, &pack, &links, &tokens, &mut current).unwrap();
        assert_eq!(current.differences.len(), 1);
    }

    #[test]
    fn leader_oracle_checks_native_precision_full_backlinks_and_independent_traits() {
        let (mut frame, pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let tokens = BTreeMap::from([
            (5, b"script_token".as_slice()),
            (6, b"assigned"),
            (7, b"progress"),
        ]);
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
        else {
            unreachable!()
        };
        let leader = &mut rows[0];
        leader.experience += 1;
        leader.character_type ^= 1;
        let TableView::Complete(TableRows::UnitLeaderTraits(rows)) =
            &mut *frame.tables[TableId::UnitLeaderTraits.index()]
        else {
            unreachable!()
        };
        let trait_row = &mut rows[0];
        trait_row.token_id = 7;
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, &mut report).unwrap();
        assert_eq!(report.differences.len(), 4, "{:?}", report.differences);
        *frame.tables[TableId::UnitLeaderTraitExperience.index()] = TableView::Unavailable {
            reason: "read fault".into(),
        };
        assert!(compare(&frame, &pack, &links, &tokens, &mut Report::default()).is_err());
    }

    #[test]
    fn ability_cost_omission_does_not_certify_zero_and_caster_type_is_exact() {
        let (mut frame, mut pack, country) = fixture();
        let date = GameDate::parse(b"1936.1.1.1").unwrap();
        let leader = handle(4713, u32::MAX);
        *frame.tables[TableId::ActiveAbilities.index()] =
            TableView::Complete(TableRows::ActiveAbilities(vec![ActiveAbilityRow {
                country: 102,
                scope_country: u32::MAX,
                token: b"ability".to_vec(),
                caster: Some(leader),
                strategic_region: Some(u32::MAX),
                division_count: i32::MIN,
                activation_hour: date.engine_hour().unwrap(),
                phase: 128,
                allocate_cost: 1,
                allocated_command_power: -1,
                ..Default::default()
            }]));
        pack.abilities
            .get_mut(b"GER".as_slice())
            .unwrap()
            .push(saved::Ability {
                token: b"ability".to_vec(),
                start_date: date,
                country: None,
                leader: Some(leader),
                num_units: Some(i32::MIN),
                strategic_region: Some(-1),
                cost: None,
            });
        let links = BTreeMap::from([(102, &country)]);
        let mut report = Report::default();
        compare_abilities(&frame, &pack, &links, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let TableView::Complete(TableRows::ActiveAbilities(rows)) =
            &mut *frame.tables[TableId::ActiveAbilities.index()]
        else {
            unreachable!()
        };
        let ability = &mut rows[0];
        ability.caster = Some(leader ^ 1);
        ability.allocated_command_power = 1;
        let mut report = Report::default();
        compare_abilities(&frame, &pack, &links, &mut report).unwrap();
        assert_eq!(report.differences.len(), 2);
    }

    #[test]
    fn native_identity_and_string_omission_do_not_use_display_fallbacks() {
        assert_ne!(handle(73, u32::MAX), handle(4713, u32::MAX));
        let mut report = Report::default();
        optional_text(&mut report, "name", b"exact\0suffix", Some(b"exact"));
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.len(), 1);
        optional_text(&mut report, "name", b"", None);
        optional_text(&mut report, "name", b"", Some(b""));
        optional_text(&mut report, "name", b"visible", None);
        assert_eq!(report.differences.len(), 2);
        let tokens = BTreeMap::from([(u32::MAX, b"negative token".as_slice())]);
        let mut report = Report::default();
        token_value(&mut report, "token", &tokens, -1, b"negative token");
        assert!(report.differences.is_empty());
        token_value(&mut report, "token", &tokens, 73, b"unknown");
        assert!(report.differences.is_empty());
        assert_eq!(report.limitations.len(), 1);
    }

    #[test]
    fn nonempty_dictionary_checks_all_three_spellings_and_preserves_missing_negative_ids() {
        let (mut frame, pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let tokens = BTreeMap::from([
            (5, b"script_token\0unsaved".as_slice()),
            (6, b"unproved assigned\0unsaved"),
            (7, b"unproved progress\0unsaved"),
        ]);
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(
            report.limitations["text serialization stops at the first NUL; captured suffix bytes are not comparable"].len(),
            1
        );
        assert_eq!(
            report.limitations["trait lexer literal contains an interior NUL; the text stream's projection is unproved, so this token spelling is unverified"].len(),
            2
        );
        let wrong = BTreeMap::from([
            (5, b"wrong character".as_slice()),
            (6, b"wrong assigned"),
            (7, b"wrong progress"),
        ]);
        let mut report = Report::default();
        compare(&frame, &pack, &links, &wrong, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        let TableView::Complete(TableRows::Characters(rows)) =
            &mut *frame.tables[TableId::Characters.index()]
        else {
            unreachable!()
        };
        rows[0].name_token_id = -1;
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let missing = report
            .limitations
            .iter()
            .find(|(reason, _)| reason.contains("lexer literal is not recorded"))
            .unwrap()
            .1;
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("native token -1"));
    }

    #[test]
    fn reserved_trait_formatter_fails_both_tables_after_independent_experience_comparison() {
        let (mut frame, pack, country) = fixture();
        let TableView::Complete(TableRows::Characters(rows)) =
            &mut *frame.tables[TableId::Characters.index()]
        else {
            unreachable!()
        };
        // Character token values use ADF40's quoted string path, so this
        // native id still denotes the ordinary dictionary spelling there.
        rows[0].name_token_id = 0x167;
        let TableView::Complete(TableRows::UnitLeaderTraits(rows)) =
            &mut *frame.tables[TableId::UnitLeaderTraits.index()]
        else {
            unreachable!()
        };
        rows[0].token_id = 0x167;
        let TableView::Complete(TableRows::UnitLeaderTraitExperience(rows)) =
            &mut *frame.tables[TableId::UnitLeaderTraitExperience.index()]
        else {
            unreachable!()
        };
        rows[0].token_id = 0x167;
        rows[0].experience += 1;
        let mut report = Report::default();
        assert!(
            compare(
                &frame,
                &pack,
                &BTreeMap::from([(102, &country)]),
                &BTreeMap::from([(0x167, b"script_token".as_slice())]),
                &mut report,
            )
            .is_err()
        );
        assert_eq!(report.failed_comparisons.len(), 2);
        for table in ["UnitLeaderTraits", "UnitLeaderTraitExperience"] {
            assert!(report.failed_comparisons[table].contains("fixed15 formatter token 0x167"));
            assert_eq!(report.compared_tables[table], 1);
        }
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("experience"));
    }

    #[test]
    fn missing_dictionary_keeps_position_census_and_raw_value_comparisons() {
        let (mut frame, pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let mut report = Report::default();
        compare(&frame, &pack, &links, &BTreeMap::new(), &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let missing_names = report
            .limitations
            .iter()
            .find(|(reason, _)| reason.contains("lexer literal is not recorded"))
            .unwrap()
            .1;
        assert_eq!(missing_names.len(), 3);
        let TableView::Complete(TableRows::UnitLeaderTraitExperience(rows)) =
            &mut *frame.tables[TableId::UnitLeaderTraitExperience.index()]
        else {
            unreachable!()
        };
        rows[0].experience += 1;
        let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
        else {
            unreachable!()
        };
        rows[0].skill_level -= 1;
        let mut report = Report::default();
        compare(&frame, &pack, &links, &BTreeMap::new(), &mut report).unwrap();
        assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
        assert!(
            !report
                .differences
                .iter()
                .any(|difference| difference.contains("membership"))
        );
    }

    #[test]
    fn unavailable_advisors_and_traits_preserve_later_table_results_and_errors() {
        let (mut frame, mut pack, country) = fixture();
        *frame.tables[TableId::AdvisorRoles.index()] = TableView::Unavailable {
            reason: "advisor role map key order invalid".into(),
        };
        *frame.tables[TableId::UnitLeaderTraits.index()] = TableView::Unavailable {
            reason: "assigned trait read fault".into(),
        };
        let TableView::Complete(TableRows::UnitLeaders(rows)) =
            &mut *frame.tables[TableId::UnitLeaders.index()]
        else {
            unreachable!()
        };
        rows[0].experience += 1;
        let TableView::Complete(TableRows::UnitLeaderTraitExperience(rows)) =
            &mut *frame.tables[TableId::UnitLeaderTraitExperience.index()]
        else {
            unreachable!()
        };
        rows[0].experience += 1;
        pack.abilities
            .get_mut(b"GER".as_slice())
            .unwrap()
            .push(saved::Ability {
                token: b"missing ability".to_vec(),
                start_date: GameDate::parse(b"1936.1.1.1").unwrap(),
                country: None,
                leader: None,
                num_units: None,
                strategic_region: None,
                cost: None,
            });
        let mut report = Report::default();
        assert!(
            compare(
                &frame,
                &pack,
                &BTreeMap::from([(102, &country)]),
                &BTreeMap::new(),
                &mut report
            )
            .is_err()
        );
        assert!(
            report.failed_comparisons["AdvisorRoles"]
                .contains("advisor role map key order invalid")
        );
        assert!(
            report.failed_comparisons["UnitLeaderTraits"].contains("assigned trait read fault")
        );
        assert_eq!(report.compared_tables["UnitLeaders"], 1);
        assert_eq!(report.compared_tables["UnitLeaderTraitExperience"], 1);
        for table in [
            "UnitLeaders",
            "UnitLeaderTraitExperience",
            "ActiveAbilities",
        ] {
            assert_eq!(report.difference_groups[table], 1);
        }
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    }

    #[test]
    fn unavailable_leader_categories_do_not_invent_extra_trait_owners() {
        let (mut frame, pack, country) = fixture();
        *frame.tables[TableId::UnitLeaders.index()] = TableView::Unavailable {
            reason: "unit leader read fault".into(),
        };
        let TableView::Complete(TableRows::UnitLeaderTraitExperience(rows)) =
            &mut *frame.tables[TableId::UnitLeaderTraitExperience.index()]
        else {
            unreachable!()
        };
        rows[0].experience += 1;
        rows.push(UnitLeaderTraitExperienceRow {
            leader_reference: handle(4713, 1),
            ordinal: 0,
            token_id: 7,
            experience: 123,
            ..Default::default()
        });
        let mut report = Report::default();
        assert!(
            compare(
                &frame,
                &pack,
                &BTreeMap::from([(102, &country)]),
                &BTreeMap::new(),
                &mut report
            )
            .is_err()
        );
        assert_eq!(report.differences.len(), 1, "{:?}", report.differences);
        assert!(report.differences[0].contains("experience"));
        assert_eq!(report.compared_tables["UnitLeaderTraits"], 1);
        assert_eq!(report.compared_tables["UnitLeaderTraitExperience"], 1);
        assert!(report.compared_tables.contains_key("ActiveAbilities"));
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("native category cannot establish"))
        );
    }
}
