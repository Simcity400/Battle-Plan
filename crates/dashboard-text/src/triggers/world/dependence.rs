
use replay_data::rows::TableId;

use super::{super::inputs::TableRead, Query, Store};

fn links() -> TableRead {
    TableRead::fields(TableId::CountryLinks, &["canonical_slot"])
}

fn raw_ids() -> TableRead {
    TableRead::fields(TableId::Countries, &["canonical_slot"])
}

fn character_fields(table: TableId) -> TableRead {
    let (_, names) = crate::character_scope::FIELDS
        .iter()
        .find(|(held, _)| *held == table)
        .expect("a character table");
    TableRead::fields(table, names)
}

fn character_leader(character: u32) -> Vec<TableRead> {
    vec![
        character_fields(TableId::UnitLeaders),
        character_fields(TableId::CharacterOperatives).row(character.into()),
    ]
}

fn character_retired() -> Vec<TableRead> {
    vec![
        character_fields(TableId::Characters),
        character_fields(TableId::CountryRetiredCharacters),
        links(),
    ]
}

fn scientist(character: u32, fields: &[&'static str]) -> Vec<TableRead> {
    vec![
        TableRead::fields(TableId::Scientists, fields).row(character.into()),
        TableRead::fields(TableId::CountryCharacters, &["character"]),
    ]
}

fn folds() -> TableRead {
    TableRead::fields(TableId::LexicalTokens, &["literal"])
}

fn prefixed(read: TableRead, prefix: u128, shift: u32) -> TableRead {
    let first = prefix << shift;
    read.rows(first, first | ((1_u128 << shift) - 1))
}

fn parties() -> [TableRead; 2] {
    [
        TableRead::fields(TableId::PoliticalParties, &["is_ruling", "token"]),
        links(),
    ]
}

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

fn any_exists() -> Vec<TableRead> {
    [exists(0), exists(1)].concat()
}

fn wars() -> [TableRead; 2] {
    [
        TableRead::fields(TableId::WarRelations, &["start_hour", "end_hour"]).ending(),
        links(),
    ]
}

fn subjects() -> [TableRead; 2] {
    [
        TableRead::fields(TableId::Subjects, &["start_hour", "end_hour"]).ending(),
        links(),
    ]
}

fn enemies() -> Vec<TableRead> {
    [vec![TableRead::all(TableId::CountryWars)], wars().to_vec()].concat()
}

fn store(store: Store, field: &str) -> Vec<TableRead> {
    let read = TableRead::fields(TableId::ScriptVariables, &[field]);
    let row = |kind: u128, id: u32| read.row(kind << 32 | u128::from(id));
    match store {
        Store::Global => vec![row(0, 0)],
        Store::Country(slot) => vec![prefixed(read, 1, 32).of_country(slot, 0), raw_ids()],
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

fn variable_store(store: Store) -> Vec<TableRead> {
    let mut reads = self::store(store, "entries");
    if matches!(
        store,
        Store::Project { .. } | Store::IndustrialOrganisation(_)
    ) {
        return reads;
    }
    reads.push(TableRead::fields(TableId::ScriptVariableNames, &["name"]));
    let values = TableRead::fields(TableId::ScriptVariableValues, &["value_raw"]);
    let scoped = match store {
        Store::Global => prefixed(values, 0, 32),
        Store::Country(slot) => prefixed(values, 1, 64).of_country(slot, 32),
        Store::State(id) => prefixed(values, (2_u128 << 32) | u128::from(id), 32),
        Store::Character(id) => prefixed(values, (3_u128 << 32) | u128::from(id), 32),
        Store::Project { .. } | Store::IndustrialOrganisation(_) => unreachable!(),
    };
    reads.push(scoped);
    reads
}

fn party_leaders(slot: u32) -> Vec<TableRead> {
    vec![
        TableRead::fields(TableId::PoliticalParties, &["is_ruling"]).owned_by(slot, 32),
        TableRead::fields(TableId::PartyLeaders, &["character", "leader_id", "role"])
            .owned_by(slot, 64),
        links(),
    ]
}

fn modifier_target() -> [TableRead; 2] {
    [links(), raw_ids()]
}

impl Query {
    pub(crate) fn dependence(&self) -> Option<Vec<TableRead>> {
        use TableId as T;
        Some(match self {
            Query::TableFresh(_) => return None,
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
            &Query::UnitStatus(reference) => vec![
                TableRead::keys(T::Divisions).row(reference.into()),
                TableRead::fields(T::DivisionStatus, &["name", "naming", "country_ordinal"])
                    .row(reference.into()),
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
            Query::SameTag(..) => vec![links()],
            &Query::HasFocus(slot, _) => vec![
                TableRead::fields(T::CompletedFocuses, &["token"]).owned_by(slot, 32),
                links(),
            ],
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
            Query::RulingParty(_) => {
                vec![TableRead::fields(T::Politics, &["ruling_party"]), links()]
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
                prefixed(
                    TableRead::fields(T::ScriptFlags, &["value", "set_hour"]),
                    u128::from(kind) << 32 | u128::from(id),
                    32,
                ),
            ],
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
            Query::Variable(scope, _) => variable_store(*scope),
            Query::VariableSeed(scope) => store(*scope, "seed"),
            Query::StateOfValue(_) => vec![TableRead::keys(T::States)],
            &Query::Character(id) => vec![TableRead::all(T::Characters).row(id.into())],
            &Query::CharacterExists(id) => vec![TableRead::keys(T::Characters).row(id.into())],
            &Query::CharacterTag(character, _) => [
                character_leader(character),
                vec![
                    TableRead::fields(T::Characters, &["country"]).row(character.into()),
                    links(),
                ],
            ]
            .concat(),
            &Query::CharacterSkill(character) => character_leader(character),
            &Query::CharacterRetired(_) => character_retired(),
            Query::CharacterHasTrait(character, _) => [
                character_leader(*character),
                character_retired(),
                vec![
                    character_fields(T::UnitLeaderTraits),
                    prefixed(character_fields(T::CharacterRoleTraits), u128::from(*character), 40),
                    folds(),
                ],
            ]
            .concat(),
            &Query::UnitLeader(reference) => {
                vec![
                    TableRead::fields(T::UnitLeaders, replay_frame::frame::Leader::FIELDS)
                        .row(reference.into()),
                ]
            }
            &Query::PartyLeaders(slot) => party_leaders(slot),
            &Query::CountryCharacters(slot) => vec![
                TableRead::fields(T::CountryCharacters, &["character"]).owned_by(slot, 32),
                links(),
            ],
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
            Query::HasTemplate(..) => vec![
                TableRead::fields(T::DivisionTemplates, &["owner", "name", "name_key"]),
                TableRead::fields(T::DivisionTemplateIcons, &["country"]),
                links(),
                TableRead::fields(T::Countries, &["canonical_slot", "tag"]),
            ],
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
            &Query::Nukes(slot) => vec![
                TableRead::fields(T::Nukes, &["amount_raw"]).owned_by(slot, 32),
                links(),
            ],
            Query::CountryScientists(_) => {
                vec![TableRead::fields(T::Scientists, &["country", "ordinal"]), links()]
            }
            &Query::ScientistActive(character) => scientist(character, &["active"]),
            &Query::ScientistLevel(character, _) => [
                scientist(character, &["skill_count"]),
                vec![prefixed(
                    TableRead::fields(T::ScientistSkills, &["token", "level"]),
                    character.into(),
                    32,
                )],
            ]
            .concat(),
        })
    }
}

fn resource_amount(
    state: u32,
    resource: u32,
    controller: Option<u32>,
    any_controller: bool,
) -> Vec<TableRead> {
    use TableId as T;
    let resource_row = u128::from(state) << 32 | u128::from(resource);
    let overrides = TableRead::fields(
        T::CountryBuildingResources,
        &["building", "count", "resource", "amount"],
    );
    let overrides = match controller {
        Some(slot) if !any_controller => overrides.owned_by(slot, 32),
        _ => overrides,
    };
    vec![
        TableRead::fields(T::Clock, &["hour"]),
        TableRead::fields(T::SliceReads, &["sample_hour"])
            .row(crate::state_resources::supply_read_key(state)),
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
