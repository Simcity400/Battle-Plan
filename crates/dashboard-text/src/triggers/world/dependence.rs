//! What each world query's answer is a function of
//! ([`super::super::inputs::Key::dependence`]): the tables, fields and rows
//! its implementation reads, nested queries and lazily built indexes
//! included. Every world query also needs the hour's world, which the
//! caller adds. A query not listed is undeclared and is always read again.
//!
//! A declaration that misses a read is a wrong answer reused: check mode
//! reads every query a declaration let through and fails on any change, and
//! the tests read each declared query on frames that differ only outside
//! its declaration.

use replay::rows::TableId;

use super::{super::inputs::TableRead, Query, Store};

/// A raw country reference's canonical slot (`World::slot`): the links of
/// every row whose country a query resolves.
fn links() -> TableRead {
    TableRead::fields(TableId::CountryLinks, &["canonical_slot"])
}

/// A canonical slot's own raw reference (`World::raw_country`).
fn raw_ids() -> TableRead {
    TableRead::fields(TableId::Countries, &["canonical_slot"])
}

/// The lexer's folds over every literal (`World::token_of`).
fn folds() -> TableRead {
    TableRead::fields(TableId::LexicalTokens, &["literal"])
}

/// The rows keyed by `prefix` in the bits above `shift`.
fn prefixed(read: TableRead, prefix: u128, shift: u32) -> TableRead {
    let first = prefix << shift;
    read.rows(first, first | ((1_u128 << shift) - 1))
}

/// The ideology index every party query builds (`World::political_parties`).
fn parties() -> [TableRead; 2] {
    [
        TableRead::fields(TableId::PoliticalParties, &["is_ruling", "token"]),
        links(),
    ]
}

/// A country's faction (`World::faction`).
fn faction(slot: u32) -> Vec<TableRead> {
    if slot == 0 {
        vec![TableRead::fields(
            TableId::ReservedCountryNaming,
            &["faction"],
        )]
    } else {
        vec![
            TableRead::fields(TableId::FactionMembers, &["faction"]),
            links(),
        ]
    }
}

/// Whether a positive country owns a state (`World::exists`); Country0's
/// count is a recorded scope's, which is undeclared.
fn exists(slot: u32) -> Option<Vec<TableRead>> {
    (slot != 0).then(|| {
        vec![
            TableRead::fields(TableId::CountryCounterInputs, &["owned_state_count"]),
            raw_ids(),
        ]
    })
}

/// The ongoing war relations, between slots.
fn wars() -> [TableRead; 2] {
    [
        TableRead::fields(TableId::WarRelations, &["start_hour", "end_hour"]).ending(),
        links(),
    ]
}

/// The ongoing subject relations, between slots.
fn subjects() -> [TableRead; 2] {
    [
        TableRead::fields(TableId::Subjects, &["start_hour", "end_hour"]).ending(),
        links(),
    ]
}

/// A variable store's row, by its scope (`World::variable_store`).
fn store(store: Store, field: &str) -> Vec<TableRead> {
    let read = TableRead::fields(TableId::ScriptVariables, &[field]);
    let row = |kind: u128, id: u32| read.row(kind << 32 | u128::from(id));
    match store {
        Store::Global => vec![row(0, 0)],
        // The country's raw id keys its row.
        Store::Country(_) => vec![read, raw_ids()],
        Store::State(id) => vec![row(2, id)],
        Store::Character(id) => vec![row(3, id)],
    }
}

impl Query {
    /// What this query's answer is a function of, besides the content and
    /// the hour's world; None when undeclared.
    pub(crate) fn dependence(&self) -> Option<Vec<TableRead>> {
        use TableId as T;
        Some(match self {
            Query::TableStatus(_)
            | Query::CoreGap
            | Query::TableGap(_)
            | Query::StateIds
            | Query::StateExists(_)
            | Query::Unlinked(..) => Vec::new(),
            Query::CountrySlots => vec![raw_ids()],
            &Query::StateContinentToken(state) => {
                vec![TableRead::fields(T::States, &["continent"]).row(state.into())]
            }
            &Query::FocusTree(raw) => vec![TableRead::fields(T::Focus, &["tree"]).row(raw.into())],
            &Query::Slot(raw) => vec![links().row(raw.into())],
            Query::Tag(_) => vec![TableRead::fields(T::Countries, &["canonical_slot", "tag"])],
            Query::RawCountry(_) => vec![raw_ids()],
            &Query::StateRow(state) => {
                vec![TableRead::fields(T::States, &["owner", "controller"]).row(state.into())]
            }
            &Query::ProvinceRow(province) => {
                vec![TableRead::fields(T::Provinces, &["owner", "controller"]).row(province.into())]
            }
            &Query::StateController(state) => vec![
                TableRead::fields(T::States, &["controller"]).row(state.into()),
                links(),
            ],
            &Query::ProvinceController(province) => vec![
                TableRead::fields(T::Provinces, &["controller"]).row(province.into()),
                links(),
            ],
            &Query::Token(token) => vec![folds().row(token.into())],
            Query::TokenOf(_) => vec![folds()],
            &Query::Faction(slot) => faction(slot),
            &Query::SameFaction(a, b) => [faction(a), faction(b)].concat(),
            Query::AtWar(..) => wars().to_vec(),
            Query::Overlord(_) => subjects().to_vec(),
            // Country0's from its singleton; any other's from its appearance,
            // by its own raw reference.
            &Query::OriginalReference(slot) => {
                if slot == 0 {
                    vec![TableRead::fields(
                        T::ReservedCountryNaming,
                        &["original_tag_reference"],
                    )]
                } else {
                    vec![
                        raw_ids(),
                        TableRead::fields(T::CountryAppearance, &["original_tag_reference"]),
                    ]
                }
            }
            // Both references resolve through the recorded scopes' links.
            Query::SameTag(..) => vec![links()],
            Query::HasFocus(..) => {
                vec![TableRead::fields(T::CompletedFocuses, &["token"]), links()]
            }
            Query::CompletedProject(..) => vec![
                folds(),
                TableRead::fields(T::Projects, &["definition_id", "completed"]),
                links(),
            ],
            Query::Government(_) | Query::NamedIdeology(_) | Query::IdeologyToken(_) => {
                parties().to_vec()
            }
            Query::ScopedRulingIdeology(_) => [
                &[TableRead::fields(T::Politics, &["ruling_party"])][..],
                &parties(),
            ]
            .concat(),
            Query::HasElections(_) => {
                vec![TableRead::fields(T::Politics, &["has_elections"]), links()]
            }
            &Query::Flag(kind, id, _) => vec![
                folds(),
                // Keyed by scope kind, scope id and token.
                prefixed(
                    TableRead::fields(T::ScriptFlags, &["value"]),
                    u128::from(kind) << 32 | u128::from(id),
                    32,
                ),
            ],
            &Query::Exists(slot) | &Query::OwnedStateCount(slot) => exists(slot)?,
            Query::CountryCounterInputs(_) => {
                vec![TableRead::all(T::CountryCounterInputs), raw_ids()]
            }
            Query::ControlledStateCount(_) => vec![
                TableRead::fields(T::CountryCounterInputs, &["controlled_state_count"]),
                raw_ids(),
            ],
            &Query::IsAllyWith(slot, other) => match other {
                None => Vec::new(),
                Some(other) => [
                    exists(slot)?,
                    exists(other)?,
                    wars().to_vec(),
                    faction(slot),
                    faction(other),
                    subjects().to_vec(),
                ]
                .concat(),
            },
            Query::Variable(scope, _) => store(*scope, "entries"),
            Query::VariableSeed(scope) => store(*scope, "seed"),
            &Query::Character(id) => vec![TableRead::all(T::Characters).row(id.into())],
            &Query::CharacterExists(id) => vec![TableRead::keys(T::Characters).row(id.into())],
            &Query::UnitLeader(reference) => {
                vec![TableRead::all(T::UnitLeaders).row(reference.into())]
            }
            Query::PartyLeaders(_) => vec![
                TableRead::fields(T::PoliticalParties, &["is_ruling"]),
                TableRead::fields(T::PartyLeaders, &["character", "leader_id", "role"]),
                links(),
            ],
            Query::CountryCharacters(_) => vec![
                TableRead::fields(T::CountryCharacters, &["character"]),
                links(),
            ],
            &Query::CivilWarCountries(raw) => vec![prefixed(
                TableRead::fields(T::CountryCivilWars, &["target"]),
                raw.into(),
                32,
            )],
            Query::AdvisorToken(_) => vec![TableRead::fields(T::AdvisorTokens, &["token"])],
            &Query::FactionAdvisors(faction) => vec![prefixed(
                TableRead::fields(T::FactionAdvisors, &["token"]),
                faction.into(),
                32,
            )],
            &Query::HasIdea(slot, _) => [
                vec![
                    TableRead::fields(T::AdvisorTokens, &["token"]),
                    TableRead::fields(T::HiredAdvisors, &["idea"]),
                    TableRead::fields(T::FactionAdvisors, &["token"]),
                    TableRead::fields(T::ActiveIdeas, &["token"]),
                    links(),
                ],
                faction(slot),
            ]
            .concat(),
            Query::Enemies(_) => [vec![TableRead::all(T::CountryWars)], wars().to_vec()].concat(),
            Query::UnitStatus(_)
            | Query::RaidWingName(_)
            | Query::IndustrialOrganisation(_)
            | Query::HasTech(..)
            | Query::HasCountryModifier(..)
            | Query::HasStateModifier(..)
            | Query::StateResource(..)
            | Query::StateResourceAmount(..)
            | Query::BuildingLevel(..)
            | Query::StateMembers
            | Query::StateOrder(_)
            | Query::CountryStateRoster(..)
            | Query::InvasionOrders(_)
            | Query::StateOfValue(_)
            | Query::LeaderHasTrait(..)
            | Query::HasCivilWar(_)
            | Query::Opinion(..)
            | Query::BuiltinVariable(..)
            | Query::HasTemplate(..)
            | Query::Appearance(_)
            | Query::IsCoreOf(..)
            | Query::IsClaimedBy(..)
            | Query::Nukes(_) => return None,
        })
    }
}
