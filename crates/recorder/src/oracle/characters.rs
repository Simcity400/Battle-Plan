//! Character and persistent-ability comparisons against native text serialization.

use super::{Report, TokenNames, complete, invalid, required_country_tag};
use installed_content::{leader_traits::LeaderTraits, parser::c_string, save::characters as saved};
use replay::{
    frame::DecodedFrame,
    rows::{CountryRow, tables},
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
    country_leader_traits: Result<&LeaderTraits, &str>,
    report: &mut Report,
) -> io::Result<()> {
    let mut complete = true;
    // Every table of the family is hourly (replay::cadence), so each is
    // read at the oracle hour.
    if let Some(manager) = &pack.manager {
        complete &= report.attempt("Characters", |report| {
            compare_characters(frame, manager, links, tokens, report)
        });
        complete &= report.attempt("CountryLeaderRoles", |report| {
            compare_country_leader_roles(frame, manager, links, report)
        });
        complete &= report.attempt("AdvisorRoles", |report| {
            compare_advisors(frame, manager, links, report)
        });
        let mut unrepresented = None;
        complete &= report.attempt("UnitLeaders", |report| {
            unrepresented = Some(compare_leaders(frame, manager, links, report)?);
            Ok(())
        });
        complete &= report.attempt("UnitLeaderTraits", |report| {
            compare_traits(frame, manager, tokens, unrepresented.as_ref(), report)
        });
        complete &= report.attempt("UnitLeaderTraitExperience", |report| {
            compare_trait_experience(frame, manager, tokens, unrepresented.as_ref(), report)
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
        // 1413DB0B0: enum 1 -> male, 2 -> female, every other value ->
        // undefined. Compare the writer's projection without changing storage.
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
        // F913A0 writes `template` exactly when CCharacter +20 is set, as the
        // lexer literal of the template's tag (CCharacterTemplate +8).
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
    report.limitation("CCharacter +28 (captured name field) is not serialized; saved name comes from captured +48 token field", "Characters.name");
    report.limitation("the character writer saves only its template's tag; the template's source file and line (CCharacterTemplate +18, +38) and generated byte (+240) are runtime-only", "Characters.template_file, template_line, template_generated");
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
    report.limitation("country leader serializer does not write numeric role-map keys; the oracle compares the per-character census and joins trait and party-leader rows by signed map order", "CountryLeaderRoles.ideology, PartyLeaders.role");
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

/// Signed map order is the serializer's role order (140F913A0, comparator 140F8A750).
pub(super) fn leader_role_keys(frame: &DecodedFrame) -> io::Result<BTreeMap<u64, Vec<i32>>> {
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

/// Each recorded role trait array against the role's saved `traits`, in
/// order, the membership built from the save and the role tables alone:
/// `CCharacter::Serialize@140F913A0` writes the country-leader roles
/// (`CCountryLeader::vf2@140FB73E0`, traits by token) and the advisor roles
/// (`CAdvisor::vf2@1412902F0`, traits by name) in their maps' order, so the
/// saved role at a position is the recorded CountryLeaderRoles key
/// (ascending, signed: the map's comparator, `140F8A750`) or AdvisorRoles
/// ordinal at that position; the scientist (`CScientist::vf2@1413F7B60`, by
/// token) is joined directly. The saved `ideology` is `CCountryLeader+0x140`,
/// which `set_country_leader_ideology` (`1403611C0`) rewrites without moving
/// the role in the map, so it is not the join. Every writer omits `traits`
/// for an empty array, which has no row.
///
/// A token's literal is not always its trait's name. `CCountryLeaderTrait`'s
/// constructor (`140710C80`) copies the definition's key to `+0x18`, which
/// the advisor writer saves, and sets its token (`+0x8`) to
/// `AddOrFindDynamicToken@1424A6500` of that key. The lexer finds an existing
/// token case-insensitively and keeps the literal it was first given
/// (`add_dynamic_token_impl@1424A65A0`), so a key spelled differently
/// elsewhere earlier saves differently by name than by token. An advisor
/// trait is therefore named by the content's definition at its recorded
/// ordinal (`+0x1A0`), whose key must be its token's under the lexer's rule.
fn compare_role_traits(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    tokens: &TokenNames<'_>,
    country_leader_traits: Result<&LeaderTraits, &str>,
    report: &mut Report,
) -> io::Result<()> {
    use replay::rows::CharacterRoleTraitRow as Row;
    let rows = complete::<tables::CharacterRoleTraits>(frame)?;
    let mut recorded = BTreeMap::<(u64, u8, u32), &Row>::new();
    for row in rows {
        recorded.insert((handle(73, row.character), row.kind, row.role), row);
    }
    let mut leader_keys = leader_role_keys(frame)?;
    let mut unnamed = Vec::new();
    let mut compared = 0;
    for (&id, character) in &manager.characters {
        // (kind, role) -> the saved trait names, for every nonempty saved array.
        let mut expected = BTreeMap::<(u8, u32), &[Vec<u8>]>::new();
        let keys = leader_keys.remove(&id).unwrap_or_default();
        if keys.len() == character.country_leaders.len() {
            for (key, role) in keys.iter().zip(&character.country_leaders) {
                if !role.traits.is_empty() {
                    expected.insert((Row::COUNTRY_LEADER, *key as u32), &role.traits);
                }
            }
        } else {
            // Without the positions the saved arrays are joined to none; the
            // recorded ones then show as extra and the saved ones as missing.
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
            // Compared as bytes; the text is for the report only.
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
        "the country-leader role writer saves a trait by token, not its database ordinal (CCountryLeaderTrait+0x1A0), so a country-leader row's definitions are not compared; an advisor row's select the keys its names are compared as",
        "CharacterRoleTraits.definitions",
    );
    Ok(())
}

/// Why a role's traits cannot be named for the comparison.
enum Unnamed {
    /// The recording or the content cannot say: a limitation.
    Limitation(String),
    /// The content's key at the recorded ordinal does not make the recorded
    /// token: the recording and the content disagree.
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

/// An advisor role's traits as `CAdvisor::vf2` saves them: each definition's
/// key (`CCountryLeaderTrait+0x18`), found in the content at the recorded
/// ordinal (`+0x1A0`). The key must give the recorded token: token 0 for an
/// empty key or one beginning with a digit or `-`, which the lexer refuses
/// (`1424A65A0`), otherwise the token whose literal equals it ignoring ASCII
/// case.
fn advisor_trait_names<'a>(
    row: &replay::rows::CharacterRoleTraitRow,
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

/// Each recorded unit-leader reference against the character's saved
/// `operative` (`CCharacter::Serialize@140F913A0` writes it when either word
/// is nonzero, so a row exists exactly for a saved reference). The resolved
/// leader's operands are the unit leader's own, which the save keeps with the
/// intelligence agency's operatives, not with the character.
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

fn compare_leaders(
    frame: &DecodedFrame,
    manager: &saved::Manager,
    links: &BTreeMap<u32, &CountryRow>,
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
        // C18CA0 writes `captured=yes` only for a set army leader byte.
        if row.kind == engine::cunit_leader::KIND_ARMY {
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
        // +0x120 is not saved; the role's character writes its own country,
        // which Capture leaves unchanged, as the owner is.
        match manager
            .characters
            .get(&character)
            .and_then(|saved| saved.country.as_deref())
        {
            Some(tag) => equal(
                report,
                format!("{label}.owner"),
                country(row.owner, links)?,
                Some(tag),
            ),
            None => report.limitation(
                "the saved character has no country to compare the leader owner with",
                format!("{label}.owner"),
            ),
        }
    }
    for id in manager.unit_leaders.keys().filter(|id| !seen.contains(id)) {
        membership(report, "unit leader", id, false);
    }
    report.compared("UnitLeaders", count);
    report.limitation("unit leader secondary inherited text, native class discriminator, skill validity byte, skill definition category and required experience are not serialized by the compared writers", "UnitLeaders.inherited_text_b/kind/skill_valid/skill_category/skill_required_experience");
    report.limitation(
        "CUnitLeader::IsVisible inputs (+0x18 choice presence, +0xF50) are not serialized",
        "UnitLeaders.visibility_choice/template_visibility",
    );
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
        if row.token_id == engine::cpdx_lexer::FIXED15_TOKEN as i32 {
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
        if row.token_id == engine::cpdx_lexer::FIXED15_TOKEN as i32 {
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

    const NO_TRAITS: Result<&LeaderTraits, &str> = Err("outside this fixture");

    /// The country-leader traits of one `common/country_leader` file.
    fn leader_traits(text: &str) -> LeaderTraits {
        let root = std::env::temp_dir().join(format!(
            "oracle_leader_traits_{}_{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let file = root.join("common/country_leader/traits.txt");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        let layers = installed_content::ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap();
        let (traits, _) = installed_content::leader_traits::read(&layers).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        traits
    }

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
                template_present: 1,
                template_tag: 8,
                template_file: b"common/characters/GER.txt".to_vec(),
                template_line: 3,
                template_generated: 0,
                gender: 0,
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
                owner: 102,
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
                        gender: Some(b"undefined".to_vec()),
                        id: character,
                        country: Some(b"GER".to_vec()),
                        roster: 1,
                        ordinal: 0,
                        token: b"script_token".to_vec(),
                        template: Some(b"GER_template".to_vec()),
                        flags: None,
                        variables: None,
                        name: Some(b"saved literal name".to_vec()),
                        country_leaders: Vec::new(),
                        advisors: Vec::new(),
                        scientist: None,
                        operative: None,
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
                        timed_traits: Vec::new(),
                        captured: false,
                    },
                )]),
            }),
            abilities: BTreeMap::from([(b"GER".to_vec(), Vec::new())]),
            appointed_advisors: BTreeMap::from([(b"GER".to_vec(), Vec::new())]),
            character_status: BTreeMap::from([(b"GER".to_vec(), Vec::new())]),
            retired_character_status: BTreeMap::from([(b"GER".to_vec(), Vec::new())]),
        };
        let country = CountryRow {
            id: 102,
            tag: b"GER".to_vec(),
            ..Default::default()
        };
        (frame, pack, country)
    }

    #[test]
    fn character_tables_are_compared_at_every_hour() {
        // compare() needs no freshness gate while the family is hourly.
        for table in [
            TableId::Characters,
            TableId::CountryLeaderRoles,
            TableId::AdvisorRoles,
            TableId::UnitLeaders,
            TableId::UnitLeaderTraits,
            TableId::UnitLeaderTraitExperience,
        ] {
            assert_eq!(
                replay::cadence::cadence(table),
                replay::cadence::Cadence::Hourly,
                "{table:?}"
            );
        }
        let (mut frame, pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let tokens = BTreeMap::from([
            (5, b"script_token".as_slice()),
            (6, b"assigned"),
            (7, b"progress"),
            (8, b"GER_template"),
        ]);
        let TableView::Complete(TableRows::UnitLeaderTraitExperience(rows)) =
            &mut *frame.tables[TableId::UnitLeaderTraitExperience.index()]
        else {
            unreachable!()
        };
        rows[0].experience += 1;
        for hour in [1, 21] {
            frame.hour = hour;
            let mut report = Report::default();
            report.record_tables(&frame);
            compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
            assert_eq!(report.differences.len(), 1, "hour {hour}");
        }
    }

    #[test]
    fn gender_oracle_rejects_mismatch_and_absent_writer_field() {
        let (mut frame, mut pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let tokens = BTreeMap::from([
            (5, b"script_token".as_slice()),
            (6, b"assigned"),
            (7, b"progress"),
            (8, b"GER_template"),
        ]);
        if let TableView::Complete(TableRows::Characters(rows)) =
            &mut *frame.tables[TableId::Characters.index()]
        {
            rows[0].gender = 2;
        }
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
        pack.manager
            .as_mut()
            .unwrap()
            .characters
            .values_mut()
            .next()
            .unwrap()
            .gender = None;
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
        assert_eq!(report.differences.len(), 1);
    }

    #[test]
    fn leader_oracle_checks_native_precision_full_backlinks_and_independent_traits() {
        let (mut frame, pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let tokens = BTreeMap::from([
            (5, b"script_token".as_slice()),
            (6, b"assigned"),
            (7, b"progress"),
            (8, b"GER_template"),
        ]);
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
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
        compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
        assert_eq!(report.differences.len(), 4, "{:?}", report.differences);
        *frame.tables[TableId::UnitLeaderTraitExperience.index()] = TableView::Unavailable {
            reason: "read fault".into(),
        };
        assert!(
            compare(
                &frame,
                &pack,
                &links,
                &tokens,
                NO_TRAITS,
                &mut Report::default()
            )
            .is_err()
        );
    }

    /// Each role's trait tokens compare, by name, with the role's saved
    /// `traits` (a country-leader role joined by its ideology literal), and
    /// the operative reference with the saved `operative`.
    #[test]
    fn role_traits_and_operative_references_compare_with_the_save() {
        use replay::rows::{CharacterOperativeRow, CharacterRoleTraitRow as Row};
        let (mut frame, mut pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let tokens = BTreeMap::from([
            (5, b"script_token".as_slice()),
            (6, b"assigned"),
            (7, b"progress"),
            (8, b"GER_template"),
            (20, b"despotism"),
            (21, b"rich_sultan"),
            (22, b"captain"),
            (23, b"inventor"),
        ]);
        let character = handle(73, u32::MAX);
        let saved = pack
            .manager
            .as_mut()
            .unwrap()
            .characters
            .get_mut(&character)
            .unwrap();
        saved.country_leaders = vec![saved::CountryLeader {
            ideology: b"despotism".to_vec(),
            id: None,
            traits: vec![b"rich_sultan".to_vec()],
        }];
        saved.advisors = vec![saved::Advisor {
            slot: None,
            idea: None,
            traits: vec![b"captain".to_vec()],
        }];
        saved.scientist = Some(saved::Scientist {
            traits: Some(vec![b"inventor".to_vec()]),
            skills: Vec::new(),
        });
        saved.operative = Some((77 << 32) | 4713);
        let row = |kind, role, token| Row {
            character: u32::MAX,
            kind,
            role,
            tokens: vec![token],
            definitions: if kind == Row::SCIENTIST {
                Vec::new()
            } else {
                vec![1]
            },
        };
        *frame.tables[TableId::CharacterRoleTraits.index()] =
            TableView::Complete(TableRows::CharacterRoleTraits(vec![
                row(Row::COUNTRY_LEADER, 20, 21),
                row(Row::ADVISOR, 0, 22),
                row(Row::SCIENTIST, 0, 23),
            ]));
        *frame.tables[TableId::CountryLeaderRoles.index()] =
            TableView::Complete(TableRows::CountryLeaderRoles(vec![
                replay::rows::CountryLeaderRoleRow {
                    character: u32::MAX,
                    country: 102,
                    ideology: 20,
                },
            ]));
        *frame.tables[TableId::AdvisorRoles.index()] =
            TableView::Complete(TableRows::AdvisorRoles(vec![
                replay::rows::AdvisorRoleRow {
                    character: u32::MAX,
                    country: 102,
                    ..Default::default()
                },
            ]));
        *frame.tables[TableId::CharacterOperatives.index()] =
            TableView::Complete(TableRows::CharacterOperatives(vec![
                CharacterOperativeRow {
                    character: u32::MAX,
                    reference: (77 << 32) | 4713,
                    resolved: true,
                    ..Default::default()
                },
            ]));
        let traits = leader_traits("leader_traits = { captain = { } }");
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, Ok(&traits), &mut report).unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CharacterRoleTraits"], 3);
        // A different trait, a missing role and a different reference.
        let TableView::Complete(TableRows::CharacterRoleTraits(rows)) =
            &mut *frame.tables[TableId::CharacterRoleTraits.index()]
        else {
            unreachable!()
        };
        rows[0].tokens = vec![22];
        rows.pop();
        let TableView::Complete(TableRows::CharacterOperatives(rows)) =
            &mut *frame.tables[TableId::CharacterOperatives.index()]
        else {
            unreachable!()
        };
        rows[0].reference ^= 1 << 32;
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, Ok(&traits), &mut report).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
    }

    /// An advisor trait saves its definition's key, not its token's literal:
    /// the token can have been made earlier from another spelling of the key
    /// (in vanilla, `SWI_FASCIST` in an AI strategy before the `SWI_fascist`
    /// trait). The key at the recorded ordinal is compared; one that cannot
    /// have made the token is a difference, content that cannot say a
    /// limitation, and a key the lexer refuses makes token 0.
    #[test]
    fn advisor_traits_compare_by_the_content_key_at_their_ordinal() {
        use replay::rows::CharacterRoleTraitRow as Row;
        let (mut frame, mut pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let saved = pack
            .manager
            .as_mut()
            .unwrap()
            .characters
            .get_mut(&handle(73, u32::MAX))
            .unwrap();
        saved.advisors = vec![saved::Advisor {
            slot: None,
            idea: None,
            traits: vec![b"SWI_fascist".to_vec(), b"1st".to_vec()],
        }];
        *frame.tables[TableId::AdvisorRoles.index()] =
            TableView::Complete(TableRows::AdvisorRoles(vec![
                replay::rows::AdvisorRoleRow {
                    character: u32::MAX,
                    country: 102,
                    ..Default::default()
                },
            ]));
        let set = |frame: &mut DecodedFrame, tokens: Vec<u32>, definitions: Vec<u32>| {
            *frame.tables[TableId::CharacterRoleTraits.index()] =
                TableView::Complete(TableRows::CharacterRoleTraits(vec![Row {
                    character: u32::MAX,
                    kind: Row::ADVISOR,
                    role: 0,
                    tokens,
                    definitions,
                }]));
        };
        let tokens = BTreeMap::from([
            (5, b"script_token".as_slice()),
            (8, b"GER_template"),
            (30, b"SWI_FASCIST"),
            (31, b"other"),
        ]);
        let traits = leader_traits("leader_traits = { other = { } SWI_fascist = { } 1st = { } }");
        let run = |frame: &DecodedFrame, content| {
            let mut report = Report::default();
            compare(frame, &pack, &links, &tokens, content, &mut report).unwrap();
            report
        };
        set(&mut frame, vec![30, 0], vec![2, 3]);
        let report = run(&frame, Ok(&traits));
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CharacterRoleTraits"], 1);
        // Without the content the traits are not named.
        let report = run(&frame, NO_TRAITS);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CharacterRoleTraits"], 0);
        // A key that cannot have made the recorded token, and a refused key
        // with a nonzero token.
        for (tokens, definitions) in [(vec![31, 0], vec![2, 3]), (vec![30, 31], vec![2, 3])] {
            set(&mut frame, tokens, definitions);
            let differences = run(&frame, Ok(&traits)).differences;
            assert!(
                differences.len() == 1 && differences[0].contains("definition"),
                "{differences:?}"
            );
        }
        // A wrong ordinal whose key does make the token still differs from
        // the save, and one the content lacks is not named.
        set(&mut frame, vec![31, 0], vec![1, 3]);
        assert_eq!(run(&frame, Ok(&traits)).differences.len(), 1);
        set(&mut frame, vec![30, 0], vec![2, 9]);
        let report = run(&frame, Ok(&traits));
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert_eq!(report.compared_tables["CharacterRoleTraits"], 0);
    }

    /// The saved country-leader role is joined by its map position, not by
    /// the recorded rows: a deleted leader row is a missing array, an extra
    /// one an extra array, and trait names compare as bytes.
    #[test]
    fn missing_extra_and_byte_different_role_traits_are_differences() {
        use replay::rows::CharacterRoleTraitRow as Row;
        let (mut frame, mut pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let character = handle(73, u32::MAX);
        let saved = pack
            .manager
            .as_mut()
            .unwrap()
            .characters
            .get_mut(&character)
            .unwrap();
        // The saved ideology is not the join: a rewritten one still matches.
        saved.country_leaders = vec![saved::CountryLeader {
            ideology: b"rewritten".to_vec(),
            id: None,
            traits: vec![b"\xff".to_vec()],
        }];
        *frame.tables[TableId::CountryLeaderRoles.index()] =
            TableView::Complete(TableRows::CountryLeaderRoles(vec![
                replay::rows::CountryLeaderRoleRow {
                    character: u32::MAX,
                    country: 102,
                    ideology: 20,
                },
            ]));
        let leader = Row {
            character: u32::MAX,
            kind: Row::COUNTRY_LEADER,
            role: 20,
            tokens: vec![21],
            definitions: vec![1],
        };
        let run = |frame: &DecodedFrame, name: &'static [u8]| {
            let tokens = BTreeMap::from([
                (5, b"script_token".as_slice()),
                (6, b"assigned"),
                (7, b"progress"),
                (8, b"GER_template"),
                (21, name),
            ]);
            let mut report = Report::default();
            compare(frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
            report.differences
        };
        let set = |frame: &mut DecodedFrame, rows: Vec<Row>| {
            *frame.tables[TableId::CharacterRoleTraits.index()] =
                TableView::Complete(TableRows::CharacterRoleTraits(rows));
        };
        set(&mut frame, vec![leader.clone()]);
        assert!(run(&frame, b"\xff").is_empty());
        // Distinct invalid bytes that read as the same replacement text.
        assert_eq!(run(&frame, b"\xfe").len(), 1);
        set(&mut frame, Vec::new());
        let missing = run(&frame, b"\xff");
        assert!(
            missing.len() == 1 && missing[0].contains("membership"),
            "{missing:?}"
        );
        set(
            &mut frame,
            vec![
                leader.clone(),
                Row {
                    kind: Row::ADVISOR,
                    role: 0,
                    ..leader
                },
            ],
        );
        let extra = run(&frame, b"\xff");
        assert!(
            extra.len() == 1 && extra[0].contains("membership"),
            "{extra:?}"
        );
    }

    /// The template's tag compares with the saved `template`; a template on
    /// one side only is a difference.
    #[test]
    fn character_template_tag_compares_with_the_saved_template() {
        let (frame, mut pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let tokens = BTreeMap::from([
            (5, b"script_token".as_slice()),
            (6, b"assigned"),
            (7, b"progress"),
            (8, b"GER_template"),
            (9, b"other"),
        ]);
        let run = |frame: &DecodedFrame, pack: &saved::Pack| {
            let mut report = Report::default();
            compare(frame, pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
            report
        };
        let edit = |edit: &dyn Fn(&mut CharacterRow)| {
            let mut frame = frame.clone();
            let TableView::Complete(TableRows::Characters(rows)) =
                &mut *frame.tables[TableId::Characters.index()]
            else {
                unreachable!()
            };
            edit(&mut rows[0]);
            frame
        };
        let report = run(&frame, &pack);
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        assert!(
            report
                .limitations
                .keys()
                .any(|reason| reason.contains("runtime-only"))
        );
        let other = run(&edit(&|row| row.template_tag = 9), &pack).differences;
        assert!(
            other.len() == 1 && other[0].contains("template"),
            "{other:?}"
        );
        let absent = edit(&|row| {
            row.template_present = 0;
            row.template_tag = 0;
        });
        let presence = run(&absent, &pack).differences;
        assert!(
            presence.len() == 1 && presence[0].contains("template presence"),
            "{presence:?}"
        );
        pack.manager
            .as_mut()
            .unwrap()
            .characters
            .values_mut()
            .next()
            .unwrap()
            .template = None;
        assert!(run(&absent, &pack).differences.is_empty());
    }

    /// The owner compares with the saved character's country; an army
    /// leader's captured byte with `captured=yes`.
    #[test]
    fn leader_owner_and_captured_byte_compare_with_the_save() {
        let (frame, mut pack, country) = fixture();
        let links = BTreeMap::from([(102, &country)]);
        let tokens = BTreeMap::from([
            (5, b"script_token".as_slice()),
            (6, b"assigned"),
            (7, b"progress"),
            (8, b"GER_template"),
        ]);
        let run = |frame: &DecodedFrame, pack: &saved::Pack| {
            let mut report = Report::default();
            compare(frame, pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
            report.differences
        };
        let edit = |edit: &dyn Fn(&mut UnitLeaderRow)| {
            let mut frame = frame.clone();
            let TableView::Complete(TableRows::UnitLeaders(rows)) =
                &mut *frame.tables[TableId::UnitLeaders.index()]
            else {
                unreachable!()
            };
            edit(&mut rows[0]);
            frame
        };
        assert!(run(&frame, &pack).is_empty());
        let other = run(&edit(&|row| row.owner = 0), &pack);
        assert!(other.len() == 1 && other[0].contains("owner"), "{other:?}");
        // A navy leader has no captured byte.
        let navy = run(&edit(&|row| row.captured = 1), &pack);
        assert!(navy.len() == 1 && navy[0].contains("captured"), "{navy:?}");
        let captured = edit(&|row| {
            row.kind = engine::cunit_leader::KIND_ARMY;
            row.captured = 1;
        });
        let free = run(&captured, &pack);
        assert!(free.len() == 1 && free[0].contains("captured"), "{free:?}");
        let leader = pack
            .manager
            .as_mut()
            .unwrap()
            .unit_leaders
            .values_mut()
            .next()
            .unwrap();
        leader.captured = true;
        assert!(run(&captured, &pack).is_empty());
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
            (8, b"GER_template"),
        ]);
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
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
        compare(&frame, &pack, &links, &wrong, NO_TRAITS, &mut report).unwrap();
        assert_eq!(report.differences.len(), 3, "{:?}", report.differences);
        let TableView::Complete(TableRows::Characters(rows)) =
            &mut *frame.tables[TableId::Characters.index()]
        else {
            unreachable!()
        };
        rows[0].name_token_id = -1;
        let mut report = Report::default();
        compare(&frame, &pack, &links, &tokens, NO_TRAITS, &mut report).unwrap();
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
                NO_TRAITS,
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
        compare(
            &frame,
            &pack,
            &links,
            &BTreeMap::new(),
            NO_TRAITS,
            &mut report,
        )
        .unwrap();
        assert!(report.differences.is_empty(), "{:?}", report.differences);
        let missing_names = report
            .limitations
            .iter()
            .find(|(reason, _)| reason.contains("lexer literal is not recorded"))
            .unwrap()
            .1;
        // Character token and template tag, trait and trait experience.
        assert_eq!(missing_names.len(), 4);
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
        compare(
            &frame,
            &pack,
            &links,
            &BTreeMap::new(),
            NO_TRAITS,
            &mut report,
        )
        .unwrap();
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
                NO_TRAITS,
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
                NO_TRAITS,
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
