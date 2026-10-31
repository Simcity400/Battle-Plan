//! What each scripted text's own read and each country-name read is a
//! function of ([`crate::triggers::inputs::Key::dependence`]): the tables,
//! fields and rows its implementation reads, the recorded scopes, names and
//! texts it reads inside included. None reads the hour's trigger world.
//!
//! A date property is a function of the hour itself, which no table
//! declares, and stays undeclared: it is read again at every hour.
//!
//! The tests read each declared query on frames that differ only outside
//! its declaration, and check mode fails on any declaration that misses a
//! read (`dependence_tests.rs`).

use replay_data::rows::TableId;

use super::{DATES, EventObject, PRONOUNS, TextQuery, named, recorded_names};
use crate::triggers::inputs::TableRead;

/// A raw country reference's canonical slot (`RecordedScopes::slot`): the
/// links of every row whose country a read resolves.
fn links() -> TableRead {
    TableRead::fields(TableId::CountryLinks, &["canonical_slot"])
}

/// A canonical slot's tag (`RecordedScopes::tag`): the last Countries row
/// of the slot.
fn tags() -> TableRead {
    TableRead::fields(TableId::Countries, &["canonical_slot", "tag"])
}

/// The rows of `slot` in a table keyed by raw country above `shift`, and
/// the rows whose country does not resolve, with every link that decides
/// which those are.
fn owned(table: TableId, fields: &[&str], slot: u32, shift: u32) -> TableRead {
    TableRead::fields(table, fields).owned_by(slot, shift)
}

/// Every country's naming operands (`installed_content_country_names::country_names`), the
/// tables of `country_names::TABLES`: a name reads its country's
/// operands and, through its text, its overlord's, of any country.
pub(crate) fn names() -> Vec<TableRead> {
    use TableId as T;
    vec![
        tags(),
        links(),
        TableRead::fields(
            T::CountryAppearance,
            &["cosmetic_tag", "name_tag", "leader_ideology", "overlord"],
        ),
        TableRead::fields(T::Politics, &["ruling_party"]),
        TableRead::fields(T::PoliticalParties, &["token"]),
        TableRead::fields(T::Autonomy, &["state"]),
        TableRead::fields(
            T::ReservedCountryNaming,
            &[
                "tag",
                "name_tag",
                "cosmetic_tag",
                "leader_ideology",
                "group",
                "overlord",
                "autonomy",
                "built",
            ],
        ),
    ]
}

/// A country's tag for text (`ScriptedText::country_tag`): Country0's from
/// its singleton.
fn country_tag(country: u32) -> Vec<TableRead> {
    if country == 0 {
        vec![TableRead::fields(TableId::ReservedCountryNaming, &["tag"])]
    } else {
        vec![tags()]
    }
}

/// [`country_tag`] of a country found in the hour.
fn any_country_tag() -> Vec<TableRead> {
    [country_tag(0), country_tag(1)].concat()
}

/// The original-or-own tag of a country found in the hour
/// (`country_names::original_or_own_tag`).
fn any_original_tag() -> Vec<TableRead> {
    use TableId as T;
    vec![
        tags(),
        TableRead::fields(
            T::CountryAppearance,
            &["original_tag_reference", "original_tag"],
        ),
        TableRead::fields(
            T::ReservedCountryNaming,
            &["original_tag_reference", "original_tag", "tag"],
        ),
    ]
}

/// The country's Politics row (`Party::Ruling`): its ruling party.
fn ruling(slot: u32) -> TableRead {
    owned(TableId::Politics, &["ruling_party"], slot, 0)
}

/// A party of `slot` named for text (`ScriptedText::party_name`): its
/// naming fields, and its tag's country's tag or original tag.
fn party_name(slot: u32) -> Vec<TableRead> {
    [
        vec![
            owned(
                TableId::PoliticalParties,
                &["token", "tag", "name_default", "name", "long_name"],
                slot,
                32,
            ),
            links(),
        ],
        any_country_tag(),
        any_original_tag(),
    ]
    .concat()
}

/// A state's name (`ScriptedText::state_name_for`), of `state` or of any.
fn state_name(state: Option<u32>) -> TableRead {
    let read = TableRead::fields(TableId::States, &["custom_name", "owner_name"]);
    match state {
        Some(state) => read.row(state.into()),
        None => read,
    }
}

/// A province's victory-point name: its state's owner's original tag.
fn victory_point(province: u32) -> Vec<TableRead> {
    [
        vec![
            TableRead::fields(TableId::Provinces, &["state"]).row(province.into()),
            TableRead::fields(TableId::States, &["owner"]),
            links(),
        ],
        any_original_tag(),
    ]
    .concat()
}

/// A province's name override, else its victory-point name.
fn primary(province: u32) -> Vec<TableRead> {
    [
        vec![TableRead::fields(TableId::ProvinceNames, &["name"]).row(province.into())],
        victory_point(province),
    ]
    .concat()
}

impl TextQuery {
    /// What this read's value is a function of besides the content; None
    /// when undeclared.
    pub(crate) fn dependence(&self) -> Option<Vec<TableRead>> {
        use TableId as T;
        Some(match self {
            TextQuery::CountryName(..) => names(),
            &TextQuery::CountryTag(country) => country_tag(country),
            &TextQuery::CharacterProperty(id, ref name) => {
                let is = |entry: &str| named(name, entry);
                if is("GetName") {
                    vec![TableRead::fields(T::Characters, &["name"]).row(id.into())]
                } else if PRONOUNS.iter().any(|entry| is(entry)) {
                    vec![TableRead::fields(T::Characters, &["gender"]).row(id.into())]
                } else {
                    Vec::new()
                }
            }
            &TextQuery::CharacterName(id) => {
                vec![TableRead::fields(T::Characters, &["name", "token"]).row(id.into())]
            }
            // The first Politics row of the country.
            &TextQuery::Capital(slot, _) => vec![
                owned(T::Politics, &["capital", "original_capital"], slot, 0),
                links(),
            ],
            &TextQuery::Overlord(slot) => {
                vec![owned(T::CountryAppearance, &["overlord"], slot, 0), links()]
            }
            &TextQuery::StateRecord(id) => vec![TableRead::keys(T::States).row(id.into())],
            &TextQuery::StateCountry(id, owner) => vec![
                TableRead::fields(T::States, &[if owner { "owner" } else { "controller" }])
                    .row(id.into()),
                links(),
            ],
            &TextQuery::CharacterCountry(id) => vec![
                TableRead::fields(T::Characters, &["country"]).row(id.into()),
                links(),
            ],
            // A unit leader's character, by its reference (its key).
            TextQuery::EventObject(object, _) => match *object {
                EventObject::UnitLeader(reference)
                | EventObject::CountryUnitLeader {
                    leader: reference, ..
                } => {
                    vec![TableRead::fields(T::UnitLeaders, &["character_id"]).row(reference.into())]
                }
                _ => Vec::new(),
            },
            TextQuery::LocalPlayer => vec![
                TableRead::fields(T::LocalPlayer, &["country", "observer_country"]),
                links(),
            ],
            &TextQuery::RulingParty(slot, ..) => [vec![ruling(slot)], party_name(slot)].concat(),
            // The country's faction, then that faction's name.
            &TextQuery::FactionName(slot) => [
                if slot == 0 {
                    vec![TableRead::fields(T::ReservedCountryNaming, &["faction"])]
                } else {
                    vec![owned(T::FactionMembers, &["faction"], slot, 0), links()]
                },
                faction_name(None),
            ]
            .concat(),
            &TextQuery::FactionNameById(id) => faction_name(Some(id)),
            // Each character of the country and its token's spelling.
            TextQuery::CharacterNamed(..) => vec![
                TableRead::fields(T::Characters, &["country", "name_token_id"]),
                TableRead::fields(T::LexicalTokens, &["literal"]),
                links(),
            ],
            &TextQuery::StateNameFor(id, _) => vec![state_name(Some(id))],
            &TextQuery::ProvinceDisplayNamePrimary(id) => primary(id),
            // Else its state's name.
            &TextQuery::ProvinceDisplayName(id) => [primary(id), vec![state_name(None)]].concat(),
            &TextQuery::ProvinceNameOverride(id) => {
                vec![TableRead::fields(T::ProvinceNames, &["name"]).row(id.into())]
            }
            &TextQuery::ProvinceVictoryPointName(id) => victory_point(id),
            // The ruling party's popularity over every party of the
            // country; a party whose country does not resolve refuses.
            &TextQuery::PartySupport(slot) => vec![
                ruling(slot),
                owned(T::PoliticalParties, &["popularity"], slot, 32),
                links(),
            ],
            // The party's first leader's character's name.
            &TextQuery::PartyLeader(slot, _) => vec![
                ruling(slot),
                owned(T::PoliticalParties, &["token"], slot, 32),
                owned(T::PartyLeaders, &["character"], slot, 64),
                TableRead::fields(T::Characters, &["name", "token"]),
                links(),
            ],
            &TextQuery::NamedPartyName(slot, ..) => party_name(slot),
            &TextQuery::LastElection(slot) => {
                vec![owned(T::Politics, &["last_election"], slot, 0), links()]
            }
            // The country's residual plus each controlled state's pool, of
            // states some province names.
            TextQuery::Manpower(_) => vec![
                TableRead::fields(T::Countries, &["canonical_slot", "manpower_residual"]),
                TableRead::fields(T::States, &["controller", "manpower_available"]),
                TableRead::fields(T::Provinces, &["state"]),
                links(),
            ],
            &TextQuery::Agency(slot) => vec![
                owned(T::IntelligenceAgencies, &["created", "name"], slot, 0),
                links(),
            ],
            // The retained old text, else the current name.
            &TextQuery::OldName(slot, _, ref suffix) => {
                let field = match suffix.as_str() {
                    "_DEF" => "old_definite_name",
                    "_ADJ" => "old_adjective",
                    _ => "old_name",
                };
                [
                    vec![owned(T::CountryAppearance, &[field], slot, 0), links()],
                    names(),
                ]
                .concat()
            }
            &TextQuery::CharacterGender(id) => {
                vec![TableRead::fields(T::Characters, &["gender"]).row(id.into())]
            }
            // The ruling party's first leader's character's gender.
            &TextQuery::CountryPronoun(slot, _) => vec![
                ruling(slot),
                TableRead::keys(T::PoliticalParties).owned_by(slot, 32),
                owned(T::PartyLeaders, &["character"], slot, 64),
                TableRead::fields(T::Characters, &["gender"]),
                links(),
            ],
            TextQuery::DateProperty(name) => {
                if DATES.iter().any(|date| named(name, date)) {
                    return None;
                }
                Vec::new()
            }
            TextQuery::CountriesGap => vec![TableRead::keys(T::Countries)],
        })
    }
}

/// A faction's name (`ScriptedText::faction_name_by_id`), of faction `id`
/// or of any: its name or template token, the template's spelling and the
/// lexer's folds.
fn faction_name(id: Option<u64>) -> Vec<TableRead> {
    let factions = TableRead::fields(TableId::Factions, &["name", "template_token"]);
    vec![
        match id {
            Some(id) => factions.row(id.into()),
            None => factions,
        },
        TableRead::fields(TableId::LexicalTokens, &["literal"]),
    ]
}

impl recorded_names::Query {
    /// What this name read's value is a function of besides the content:
    /// every country's naming operands ([`names`]).
    pub(crate) fn dependence(&self) -> Vec<TableRead> {
        names()
    }
}
