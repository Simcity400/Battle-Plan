//! What each world query's answer is a function of
//! ([`super::super::inputs::Key::dependence`]): the tables, fields and rows
//! its implementation reads, nested queries and lazily built indexes
//! included. Every world query also needs the hour's world, which the
//! caller adds. Acquisition freshness is checked again independently of row changes;
//! the other queries declare their table reads.
//!
//! A query that reads one country's rows of a table keyed by raw country,
//! and refuses when a row's country does not resolve, declares those rows
//! ([`TableRead::owned_by`]): another country's rows changing leave it
//! unchanged.
//!
//! A declaration that misses a read is a wrong answer reused: check mode
//! reads every query a declaration let through and fails on any change, and
//! the tests read each declared query on frames that differ only outside
//! its declaration.

use replay_data::rows::TableId;

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

/// Whether a country owns a state (`World::exists`): a positive country's
/// stored count, Country0's from its singleton, as the hour's recorded
/// scopes read it (`OriginalTagLists::reserved_owned_states`).
fn exists(slot: u32) -> Vec<TableRead> {
    if slot == 0 {
        vec![TableRead::fields(
            TableId::ReservedCountryNaming,
            &["owned_state_count"],
        )]
    } else {
        vec![
            TableRead::fields(TableId::CountryCounterInputs, &["owned_state_count"]),
            raw_ids(),
        ]
    }
}

/// [`exists`] of a country the query finds in the hour's rows.
fn any_exists() -> Vec<TableRead> {
    [exists(0), exists(1)].concat()
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

/// A country's enemy vector (`World::enemies`).
fn enemies() -> Vec<TableRead> {
    [vec![TableRead::all(TableId::CountryWars)], wars().to_vec()].concat()
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
        Store::Project { owner, id } => vec![
            TableRead::fields(TableId::Projects, &["runtime"])
                .row((u128::from(owner) << 32) | u128::from(id)),
        ],
        Store::IndustrialOrganisation(reference) => vec![
            TableRead::fields(TableId::IndustrialOrganisationDetails, &["variables"])
                .row(reference.into()),
        ],
    }
}

/// A country's parties and their leaders (`World::party_leaders`): its own
/// rows of both tables, and any row whose country does not resolve.
fn party_leaders(slot: u32) -> Vec<TableRead> {
    vec![
        TableRead::fields(TableId::PoliticalParties, &["is_ruling"]).owned_by(slot, 32),
        TableRead::fields(TableId::PartyLeaders, &["character", "leader_id", "role"])
            .owned_by(slot, 64),
        links(),
    ]
}

/// A dynamic-modifier target's match (`World::modifier_matches`): the
/// target country's raw reference and each entry country's slot.
fn modifier_target() -> [TableRead; 2] {
    [links(), raw_ids()]
}

impl Query {
    /// What this query's answer is a function of, besides the content and
    /// the hour's world; None when undeclared.
    pub(crate) fn dependence(&self) -> Option<Vec<TableRead>> {
        use TableId as T;
        Some(match self {
            // Freshness reads acquisition metadata and the enclosing hour, not row values.
            Query::TableFresh(_) => return None,
            // The hour itself changes every hour: read it again.
            Query::Clock => return None,
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
            // The Divisions row's presence, then its whole status row.
            &Query::UnitStatus(reference) => vec![
                TableRead::keys(T::Divisions).row(reference.into()),
                TableRead::all(T::DivisionStatus).row(reference.into()),
            ],
            Query::RaidWingName(_) => vec![
                TableRead::all(T::Raids),
                TableRead::all(T::RaidTooltipOperands),
            ],
            &Query::IndustrialOrganisation(reference) => {
                vec![TableRead::all(T::IndustrialOrganisations).row(reference.into())]
            }
            &Query::IndustrialOrganisationSize(reference) => vec![
                TableRead::fields(T::IndustrialOrganisationDetails, &["state.size"])
                    .row(reference.into()),
            ],
            &Query::IndustrialOrganisationUnlocked(reference) => vec![
                TableRead::fields(T::IndustrialOrganisationDetails, &["unlocked"])
                    .row(reference.into()),
            ],
            &Query::IndustrialOrganisationPolicies(reference) => vec![
                TableRead::fields(
                    T::IndustrialOrganisationDetails,
                    &["allowed_policies", "state.policy"],
                )
                .row(reference.into()),
            ],
            &Query::IndustrialOrganisationFlags(reference) => vec![
                TableRead::fields(T::IndustrialOrganisationDetails, &["flags"])
                    .row(reference.into()),
            ],
            &Query::IndustrialOrganisationTasks(reference) => vec![
                TableRead::fields(T::IndustrialOrganisationDetails, &["tasks"])
                    .row(reference.into()),
            ],
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
            // The country's completed focuses, and whether a row's country
            // does not resolve, which refuses an answer its rows lack.
            &Query::HasFocus(slot, _) => vec![
                TableRead::fields(T::CompletedFocuses, &["token"]).owned_by(slot, 32),
                links(),
            ],
            // Every definition's name; the country's completed rows, and any
            // completed row whose country does not resolve.
            &Query::HasTech(slot, _) => vec![
                TableRead::fields(T::TechnologyDefinitions, &["id"]),
                TableRead::fields(T::ResearchTechnologies, &["classifier"]).owned_by(slot, 32),
                links(),
            ],
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
            // The country's entries, and any entry whose owner does not
            // resolve, which may be the country's.
            &Query::HasCountryModifier(slot, ..) => [
                vec![
                    TableRead::fields(
                        T::DynamicModifiers,
                        &["token", "scope_country", "scope_state"],
                    )
                    .owned_by(slot, 32),
                ],
                modifier_target().to_vec(),
            ]
            .concat(),
            // Keyed by state and ordinal.
            &Query::HasStateModifier(state, ..) => [
                vec![prefixed(
                    TableRead::fields(
                        T::StateDynamicModifiers,
                        &["token", "scope_country", "scope_state"],
                    ),
                    state.into(),
                    32,
                )],
                modifier_target().to_vec(),
            ]
            .concat(),
            // The state's controller, then its amount.
            &Query::StateResource(state, resource) => [
                vec![
                    TableRead::fields(T::States, &["controller"]).row(state.into()),
                    links(),
                ],
                resource_amount(state, resource, None, true),
            ]
            .concat(),
            &Query::StateResourceAmount(state, resource, controller) => {
                resource_amount(state, resource, controller, false)
            }
            // Building rows are keyed by location and type.
            &Query::BuildingLevel(province, location, building_type) => {
                let table = if province {
                    T::ProvinceBuildings
                } else {
                    T::StateBuildings
                };
                vec![
                    TableRead::fields(table, &["level"])
                        .row(u128::from(location) << 32 | u128::from(building_type)),
                ]
            }
            // Every province's state.
            Query::StateMembers | Query::StateOrder(_) => {
                vec![TableRead::fields(T::Provinces, &["state"])]
            }
            &Query::Exists(slot) | &Query::OwnedStateCount(slot) => exists(slot),
            Query::CountryCounterInputs(_) => {
                vec![TableRead::all(T::CountryCounterInputs), raw_ids()]
            }
            &Query::CountryStateRoster(_, controlled) => vec![
                TableRead::fields(
                    T::CountryCounterInputs,
                    &[if controlled {
                        "controlled_states"
                    } else {
                        "owned_states"
                    }],
                ),
                raw_ids(),
            ],
            &Query::InvasionOrders(slot) => [
                if slot == 0 {
                    vec![TableRead::fields(
                        T::ReservedCountryNaming,
                        &["land_theatre_count"],
                    )]
                } else {
                    Vec::new()
                },
                vec![
                    TableRead::fields(T::LandTheatres, &["owner", "ordinal"]),
                    TableRead::fields(
                        T::OrdersGroups,
                        &["is_army_group", "theatre", "theatre_ordinal"],
                    ),
                    TableRead::fields(T::OrdersGroupPlans, &["primary_orders"]),
                    TableRead::fields(T::ArmyOrders, &["order_type", "active", "path"]),
                    TableRead::fields(T::ArmyOrderPlans, &["successors"]),
                    links(),
                ],
            ]
            .concat(),
            Query::ControlledStateCount(_) => vec![
                TableRead::fields(T::CountryCounterInputs, &["controlled_state_count"]),
                raw_ids(),
            ],
            &Query::IsAllyWith(slot, other) => match other {
                None => Vec::new(),
                Some(other) => [
                    exists(slot),
                    exists(other),
                    wars().to_vec(),
                    faction(slot),
                    faction(other),
                    subjects().to_vec(),
                ]
                .concat(),
            },
            Query::Variable(scope, _) => store(*scope, "entries"),
            Query::VariableSeed(scope) => store(*scope, "seed"),
            // The States rows' keys, and whether the table is complete.
            Query::StateOfValue(_) => vec![TableRead::keys(T::States)],
            &Query::Character(id) => vec![TableRead::all(T::Characters).row(id.into())],
            &Query::CharacterExists(id) => vec![TableRead::keys(T::Characters).row(id.into())],
            &Query::UnitLeader(reference) => {
                vec![TableRead::all(T::UnitLeaders).row(reference.into())]
            }
            &Query::PartyLeaders(slot) => party_leaders(slot),
            &Query::CountryCharacters(slot) => vec![
                TableRead::fields(T::CountryCharacters, &["character"]).owned_by(slot, 32),
                links(),
            ],
            // The ruling party's first leader; its role's presence, then its
            // country-leader traits, of a character found in the hour.
            &Query::LeaderHasTrait(slot, _) => [
                party_leaders(slot),
                vec![
                    TableRead::keys(T::CountryLeaderRoles),
                    TableRead::fields(T::CharacterRoleTraits, &["tokens", "definitions"]),
                    folds(),
                ],
            ]
            .concat(),
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
            // Its own and each existing enemy's civil-war lists, of
            // countries found in the hour, Country0 among them.
            Query::HasCivilWar(_) => [
                vec![
                    raw_ids(),
                    TableRead::fields(T::CountryCivilWars, &["target"]),
                    links(),
                ],
                any_exists(),
                enemies(),
            ]
            .concat(),
            Query::Enemies(_) => enemies(),
            // An advisor's hire is read from the country's rows, an active
            // idea from its rows and any row whose country does not resolve.
            &Query::HasIdea(slot, _) => [
                vec![
                    TableRead::fields(T::AdvisorTokens, &["token"]),
                    TableRead::fields(T::HiredAdvisors, &["idea"]).owned_by(slot, 32),
                    TableRead::fields(T::FactionAdvisors, &["token"]),
                    TableRead::fields(T::ActiveIdeas, &["token"]).owned_by(slot, 32),
                    links(),
                ],
                faction(slot),
            ]
            .concat(),
            &Query::Opinion(_, target) => match target.filter(|&slot| slot != 0) {
                None => Vec::new(),
                Some(_) => vec![
                    raw_ids(),
                    links(),
                    TableRead::fields(T::RelationOpinions, &["opinion"]),
                ],
            },
            Query::BuiltinVariable(store, name) => match (store, name.as_slice()) {
                (&Store::Country(slot), b"num_owned_states") => exists(slot),
                (Store::Country(_), b"num_owned_controlled_states") => vec![
                    raw_ids(),
                    links(),
                    TableRead::fields(T::Provinces, &["state"]),
                    TableRead::fields(T::States, &["owner", "controller"]),
                ],
                (Store::Country(_), b"num_of_controlled_factories") => vec![
                    raw_ids(),
                    TableRead::fields(
                        T::CountryFactories,
                        &["civilian_levels", "naval_levels", "military_levels"],
                    ),
                ],
                _ => Vec::new(),
            },
            // Each template's owner and name, its icon's country, and that
            // country's tag.
            Query::HasTemplate(..) => vec![
                TableRead::fields(T::DivisionTemplates, &["owner", "name", "name_key"]),
                TableRead::fields(T::DivisionTemplateIcons, &["country"]),
                links(),
                TableRead::fields(T::Countries, &["canonical_slot", "tag"]),
            ],
            // The country's whole row, and any row whose country does not
            // resolve.
            &Query::Appearance(slot) => vec![
                TableRead::all(T::CountryAppearance).owned_by(slot, 0),
                links(),
            ],
            &Query::IsCoreOf(state, _) => vec![
                TableRead::fields(T::StateCores, &["cores"]).row(state.into()),
                links(),
            ],
            &Query::IsClaimedBy(state, _) => vec![
                TableRead::fields(T::StateCores, &["claims"]).row(state.into()),
                links(),
            ],
            // Keyed by country and nuke type; a row whose country does not
            // resolve is skipped, so only the country's rows count.
            &Query::Nukes(slot) => vec![
                TableRead::fields(T::Nukes, &["amount_raw"]).owned_by(slot, 32),
                links(),
            ],
        })
    }
}

/// A state's resource amount (`StateResources::amount`) under `controller`'s
/// building overrides; `any_controller` when the controller is the state's,
/// found in the hour.
fn resource_amount(
    state: u32,
    resource: u32,
    controller: Option<u32>,
    any_controller: bool,
) -> Vec<TableRead> {
    use TableId as T;
    let resource_row = u128::from(state) << 32 | u128::from(resource);
    // Every row's slot builds the overrides index, which a missing table
    // refuses; the controller's rows are read.
    let overrides = TableRead::fields(
        T::CountryBuildingResources,
        &["building", "count", "resource", "amount"],
    );
    let overrides = match controller {
        Some(slot) if !any_controller => overrides.owned_by(slot, 32),
        _ => overrides,
    };
    vec![
        TableRead::fields(
            T::StateResourceInputs,
            &[
                "gate",
                "global_factor",
                "origin",
                "infrastructure_level",
                "infrastructure_modifier",
                "supply_route",
            ],
        )
        .row(state.into()),
        TableRead::fields(T::StateResources, &["pool", "additive", "factor", "cost"])
            .row(resource_row),
        TableRead::fields(T::StateBaseResources, &["amount"]).row(resource_row),
        prefixed(
            TableRead::fields(T::StateBuildings, &["healthy_level"]),
            state.into(),
            32,
        ),
        overrides,
        TableRead::fields(T::ResourceDefinitions, &["token", "active"]),
        folds(),
        links(),
    ]
}
