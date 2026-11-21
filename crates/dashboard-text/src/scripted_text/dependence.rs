
use replay_data::rows::TableId;

use super::{DATES, EventObject, PRONOUNS, TextQuery, named, recorded_names};
use crate::triggers::inputs::TableRead;

fn links() -> TableRead {
    TableRead::fields(TableId::CountryLinks, &["canonical_slot"])
}

fn tags() -> TableRead {
    TableRead::fields(TableId::Countries, &["canonical_slot", "tag"])
}

fn owned(table: TableId, fields: &[&str], slot: u32, shift: u32) -> TableRead {
    TableRead::fields(table, fields).owned_by(slot, shift)
}

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

fn country_tag(country: u32) -> Vec<TableRead> {
    if country == 0 {
        vec![TableRead::fields(TableId::ReservedCountryNaming, &["tag"])]
    } else {
        vec![tags()]
    }
}

fn any_country_tag() -> Vec<TableRead> {
    [country_tag(0), country_tag(1)].concat()
}

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

fn ruling(slot: u32) -> TableRead {
    owned(TableId::Politics, &["ruling_party"], slot, 0)
}

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

fn state_name(state: Option<u32>) -> TableRead {
    let read = TableRead::fields(TableId::States, &["custom_name", "owner_name"]);
    match state {
        Some(state) => read.row(state.into()),
        None => read,
    }
}

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

fn primary(province: u32) -> Vec<TableRead> {
    [
        vec![TableRead::fields(TableId::ProvinceNames, &["name"]).row(province.into())],
        victory_point(province),
    ]
    .concat()
}

impl TextQuery {
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
            TextQuery::CharacterNamed(..) => vec![
                TableRead::fields(T::Characters, &["country", "name_token_id"]),
                TableRead::fields(T::LexicalTokens, &["literal"]),
                links(),
            ],
            &TextQuery::StateNameFor(id, _) => vec![state_name(Some(id))],
            &TextQuery::ProvinceDisplayNamePrimary(id) => primary(id),
            &TextQuery::ProvinceDisplayName(id) => [primary(id), vec![state_name(None)]].concat(),
            &TextQuery::ProvinceNameOverride(id) => {
                vec![TableRead::fields(T::ProvinceNames, &["name"]).row(id.into())]
            }
            &TextQuery::ProvinceVictoryPointName(id) => victory_point(id),
            &TextQuery::PartySupport(slot) => vec![
                ruling(slot),
                owned(T::PoliticalParties, &["popularity"], slot, 32),
                links(),
            ],
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
    pub(crate) fn dependence(&self) -> Vec<TableRead> {
        names()
    }
}
