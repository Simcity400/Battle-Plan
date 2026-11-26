use super::{Eval, Evaluator, Scope, Scopes, Store};
use crate::triggers::Failure;
use content_reader_parser::parser::Item;
use replay_data::rows::{TableSchema, mask, tables};
use replay_frame::frame::HourFrame;
use std::{borrow::Cow, collections::BTreeSet};

impl Evaluator<'_, '_> {
    pub(super) fn map_builtin_value(
        &self,
        store: Store,
        operand: &content_reader_script::scoped_variable::ScopedVariable,
        scopes: Scopes,
    ) -> Result<Option<i64>, Failure> {
        if let (Store::Country(slot), b"modifier") = (store, operand.name.as_slice()) {
            let callback = self.builtin_scope(store, operand, scopes)?;
            let token = match operand.target.as_deref() {
                Some(target) => {
                    super::fixed_round(self.raw(self.evaluate_operand(target, callback, None)?)?)
                        as u32
                }
                None => 0,
            };
            let world = self.world();
            const TOKEN: u64 = mask::<replay_data::rows::ModifierDefinitionRow>(&["token"]);
            let definitions = recorded::<tables::ModifierDefinitions>(world.tables(), TOKEN)?;
            if !definitions.iter().any(|row| row.token == token) {
                return Ok(Some(0));
            }
            let name = world
                .token(token)?
                .ok_or("modifier token has no recorded spelling")?;
            if let Some(index) = replay_data::rows::CountryStabilityInputRow::KEY_NAMES[..8]
                .iter()
                .position(|key| *key == name)
            {
                let row = self.country_stability_inputs(
                    slot, tables::CountryStabilityInputs::ID.full_mask(),
                )?;
                return self.stability_modifier_input(&row, slot, index).map(Some);
            }
            return Err(format!(
                "modifier@{}: the country accumulator entry is not recorded",
                String::from_utf8_lossy(name)
            )
            .into());
        }
        let Store::State(state) = store else {
            return Ok(None);
        };
        let name = operand.name.as_slice();
        let world = self.world();
        if matches!(
            name,
            b"owner" | b"controller" | b"state_population" | b"state_population_k"
        ) {
            const BUILTIN: u64 =
                mask::<replay_data::rows::StateRow>(&["owner", "controller", "population"]);
            let row = recorded_row::<tables::States>(world.tables(), BUILTIN, u128::from(state))?
                .ok_or("state builtin lacks its States row")?;
            return match name {
                b"state_population" => Ok(Some(
                    i64::from(*row.population.require("population of states").map_err(|why| {
                        Failure::input(
                            replay_data::rows::TableId::States,
                            "population",
                            Some(Store::State(state)),
                            why,
                        )
                    })?) * 100_000,
                )),
                b"state_population_k" => Ok(Some(
                    i64::from(*row.population.require("population of states").map_err(|why| {
                        Failure::input(
                            replay_data::rows::TableId::States,
                            "population",
                            Some(Store::State(state)),
                            why,
                        )
                    })?)
                    .wrapping_mul(10_000_000_000)
                        / 100_000_000,
                )),
                _ => {
                    let raw = if name == b"owner" {
                        row.owner
                    } else {
                        row.controller
                    };
                    let slot = if raw == 0 {
                        0
                    } else {
                        world
                            .slot(raw)
                            .ok_or_else(|| world.unlinked("state builtin country", raw))?
                    };
                    self.encode(Scope::Country(slot)).map(Some)
                }
            };
        }
        if matches!(
            name,
            b"resistance" | b"compliance" | b"resistance_speed" | b"compliance_speed"
        ) {
            world.table_fresh(replay_data::rows::TableId::StateResistance)?;
            const RESISTANCE: u64 = mask::<replay_data::rows::StateResistanceRow>(&[
                "level",
                "speed",
                "compliance",
                "compliance_speed",
            ]);
            let row = recorded_row::<tables::StateResistance>(
                world.tables(),
                RESISTANCE,
                u128::from(state),
            )?
            .ok_or_else(|| {
                    super::refusal::missing_reference(
                        replay_data::rows::TableId::StateResistance,
                        Some(Store::State(state)),
                    )
                })?;
            return Ok(Some(match name {
                b"resistance" => row.level,
                b"compliance" => row.compliance,
                b"resistance_speed" => row.speed,
                _ => row.compliance_speed,
            }));
        }
        if name == b"building_level" {
            let callback = self.builtin_scope(store, operand, scopes)?;
            let token = if let Some(target) = operand.target.as_deref() {
                super::fixed_round(self.raw(self.evaluate_operand(target, callback, None)?)?) as u32
            } else {
                0
            };
            return Ok(Some(
                i64::from(world.building_level(false, state, token)?.unwrap_or(0)) * 100_000,
            ));
        }
        Ok(None)
    }
    pub(super) fn idea_has_trait(&self, slot: u32, name: &str) -> Result<bool, Failure> {
        let world = self.world();
        let (_, traits) = world.character_traits.map_err(Failure::from)?;
        let Some(ordinal) = traits.first_ordinal(name).map_err(|why| why.to_string())? else {
            return Ok(false);
        };
        const HIRED: u64 = mask::<replay_data::rows::HiredAdvisorRow>(&["character", "idea", "slot"]);
        const ROLE: u64 = mask::<replay_data::rows::AdvisorRoleRow>(&["idea", "slot"]);
        const HELD: u64 = mask::<replay_data::rows::CharacterRoleTraitRow>(&["definitions"]);
        let hired = recorded::<tables::HiredAdvisors>(world.tables(), HIRED)?;
        let roles = recorded::<tables::AdvisorRoles>(world.tables(), ROLE)?;
        let held = recorded::<tables::CharacterRoleTraits>(world.tables(), HELD)?;
        for advisor in hired {
            let owner = world
                .slot(advisor.country)
                .ok_or("hired advisors country is unresolved")?;
            if owner != slot {
                continue;
            }
            let mut matches = roles.iter().filter(|role| {
                role.character == advisor.character
                    && role.idea == advisor.idea
                    && role.slot == advisor.slot
            });
            let role = matches
                .next()
                .ok_or("appointed advisor has no recorded role")?;
            if matches.next().is_some() {
                return Err("appointed advisor matches several recorded roles".into());
            }
            if held.iter().any(|row| {
                row.character == role.character
                    && row.kind == replay_data::rows::CharacterRoleTraitRow::ADVISOR
                    && row.role == role.ordinal
                    && row.definitions.contains(&{ ordinal })
            }) {
                return Ok(true);
            }
        }
        let active = recorded::<tables::ActiveIdeas>(
            world.tables(),
            mask::<replay_data::rows::ActiveIdeaRow>(&["token"]),
        )?;
        let ideas = world.ideas.map_err(Failure::from)?;
        for row in active {
            let owner = world
                .slot(row.country)
                .ok_or("active ideas country is unresolved")?;
            if owner != slot {
                continue;
            }
            let token =
                std::str::from_utf8(&row.token).map_err(|_| "active idea token is not UTF-8")?;
            let idea = ideas
                .idea(token)
                .map_err(|why| why.to_string())?
                .ok_or_else(|| -> Failure {
                    format!("active idea {token} has no recorded-content definition").into()
                })?;
            for trait_name in &idea.traits {
                if traits
                    .first_ordinal(trait_name)
                    .map_err(|why| why.to_string())?
                    == Some(ordinal)
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub(super) fn state_in_home_area(&self, state: u32) -> Result<bool, Failure> {
        let world = self.world();
        let row = recorded_row::<tables::States>(
            world.tables(),
            mask::<replay_data::rows::StateRow>(&["owner"]),
            u128::from(state),
        )?
        .ok_or("home area state is absent")?;
        let owner = if row.owner == 0 {
            0
        } else {
            world
                .slot(row.owner)
                .ok_or("home area owner is unresolved")?
        };
        let politics = recorded::<tables::Politics>(
            world.tables(),
            mask::<replay_data::rows::PoliticsRow>(&["capital"]),
        )?;
        let capital = politics
            .iter()
            .find(|row| world.slot(row.country) == Some(owner))
            .ok_or("home area owner has no Politics row")?
            .capital;
        if capital <= 0 {
            return Ok(false);
        }
        let counts = world.controller_area_sizes()?;
        let by_id = world.provinces();
        let capital_members = world.state_order(capital as u32)?;
        let mut largest = None;
        let mut size = 0;
        for id in capital_members {
            let province = by_id
                .get(id)
                .ok_or("home area capital province is absent")?;
            let controller = if province.controller == 0 {
                0
            } else {
                world
                    .slot(province.controller)
                    .ok_or("home area province controller is unresolved")?
            };
            if controller != owner {
                continue;
            }
            let area = province
                .controller_area
                .require("controller area of provinces")
                .map_err(|why| {
                    Failure::input(
                        replay_data::rows::TableId::Provinces,
                        "controller_area",
                        None,
                        why,
                    )
                })?
                .unwrap();
            let count = counts.get(&area).copied().unwrap_or(0);
            if count > size {
                size = count;
                largest = Some(area);
            }
        }
        let first = world
            .state_order(state)?
            .first()
            .ok_or("home area state has no first province")?;
        let area = by_id
            .get(first)
            .ok_or("home area first province is absent")?
            .controller_area
            .require("controller area of provinces")
            .map_err(|why| {
                Failure::input(
                    replay_data::rows::TableId::Provinces,
                    "controller_area",
                    None,
                    why,
                )
            })?
            .unwrap();
        Ok(area != 0 && largest == Some(area))
    }
    pub(super) fn country_is_ai(&self, slot: u32) -> Result<bool, Failure> {
        let features = recorded::<tables::SimulationFeatures>(
            self.world().tables(),
            tables::SimulationFeatures::ID.full_mask(),
        )?;
        let [features] = features.as_slice() else {
            return Err(Failure::input(
                replay_data::rows::TableId::SimulationFeatures,
                "",
                None,
                "simulation features lacks its singleton",
            ));
        };
        if features.human_ai != 0 {
            return Ok(true);
        }
        let flag = recorded_row::<tables::RaidCountryInputs>(
            self.world().tables(),
            mask::<replay_data::rows::RaidCountryInputRow>(&["autonomous"]),
            u128::from(slot),
        )?
        .ok_or("raid country inputs lacks AI-control country")?
        .autonomous;
        if let Some(flag) = flag {
            let rows = recorded::<tables::HumanControllers>(
                self.world().tables(),
                mask::<replay_data::rows::HumanControllerRow>(&["count"]),
            )?;
            let row = rows
                .iter()
                .find(|row| self.world().slot(row.country) == Some(slot))
                .ok_or("human controllers lacks the AI-control country")?;
            return Ok(flag != 0 && row.count == 0);
        }
        for player in recorded::<tables::Players>(
            self.world().tables(),
            mask::<replay_data::rows::PlayerRow>(&["country"]),
        )? {
            let country = if player.country == 0 {
                0
            } else {
                self.world()
                    .slot(player.country)
                    .ok_or("Players country is unresolved")?
            };
            if country == slot {
                return Ok(false);
            }
        }
        Ok(true)
    }
    pub(super) fn in_tech_sharing_group(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        self.tech_sharing_roster(slot, name)
    }

    fn tech_sharing_roster(&self, slot: u32, name: &[u8]) -> Result<bool, Failure> {
        let world = self.world();
        let rows = recorded::<tables::ResearchModifiers>(
            world.tables(),
            tables::ResearchModifiers::ID.full_mask(),
        )?;
        let token = if name.is_empty() {
            Some(0)
        } else {
            world.token_of(name)?
        };
        let (mut witness, mut seen, mut matched) = (None, 0_usize, false);
        for row in rows.iter().filter(|row| {
            row.kind == replay_data::rows::ResearchModifierRow::SHARING_ROSTER
                && row.index == u32::MAX
        }) {
            let owner = world
                .slot(row.country)
                .ok_or_else(|| world.unlinked("research modifiers sharing-roster", row.country))?;
            if owner != slot {
                continue;
            }
            if row.raw == -1 {
                if witness.replace(row.count).is_some() {
                    return Err(
                        "research modifiers has duplicate sharing-roster acquisition witnesses"
                            .into(),
                    );
                }
            } else {
                let id = u32::try_from(row.raw)
                    .map_err(|_| "research modifiers has an invalid sharing-roster group ID")?;
                seen += 1;
                matched |= Some(id) == token;
            }
        }
        match witness {
            Some(count) if count as usize == seen => Ok(matched),
            Some(_) => Err(Failure::input(
                replay_data::rows::TableId::ResearchModifiers,
                "sharing_roster",
                Some(Store::Country(slot)),
                "research modifiers sharing-roster membership is incomplete",
            )),
            None => Err(Failure::input(
                replay_data::rows::TableId::ResearchModifiers,
                "sharing_roster",
                Some(Store::Country(slot)),
                "research modifiers sharing-roster witness unavailable: the fact was not demanded or its capture failed",
            )),
        }
    }

    pub(super) fn array_scope_iterator(
        &self,
        name: &str,
        block: &[Item],
        scopes: Scopes,
    ) -> Result<Eval, Failure> {
        let mut operand = super::variables::temps::zero();
        let mut body = Vec::new();
        for item in block {
            if item.keyword_is("array") {
                super::variables::temps::assign(
                    self,
                    &mut operand,
                    item.value.scalar().unwrap_or_default(),
                )?;
            } else if !item.keyword_is("tooltip") {
                body.push(item.clone());
            }
        }
        let mut length = operand.clone();
        length.index = Some(Box::new(content_reader_script::scoped_variable::ScopedVariable {
            parsed: true,
            num: true,
            ..Default::default()
        }));
        let count =
            super::fixed_round(self.raw(self.evaluate_operand(&length, scopes, None)?)?) as i32;
        let all = name.starts_with("all_");
        let mut result = Eval::from_bool(all);
        let mut child = None;
        self.visit_indexed_values(&operand, scopes, i64::from(count), |value| {
            let Some(target) = self.decoded(value, scopes)? else {
                return Ok(false);
            };
            let entered = match child {
                Some(previous) => self.repoint_this(previous, target)?,
                None if scopes.cursor.is_some() => self.collection_scope(scopes, target, false)?,
                None => Scopes {
                    bare: true,
                    ..scopes.with_this(target)
                },
            };
            child = Some(entered);
            let value = self.evaluate(&body, entered);
            result = if all {
                result.clone().and(|| value)
            } else {
                result.clone().or(|| value)
            };
            Ok(result == Eval::from_bool(!all))
        })?;
        Ok(result)
    }
    pub(super) fn map_builtin_array(
        &self,
        store: Store,
        name: &[u8],
        at: i32,
    ) -> Result<Option<i64>, Failure> {
        let world = self.world();
        if let (Store::Global, b"states") = (store, name) {
            let features = recorded::<tables::SimulationFeatures>(
                world.tables(),
                tables::SimulationFeatures::ID.full_mask(),
            )?;
            let [features] = features.as_slice() else {
                return Err(Failure::input(
                    replay_data::rows::TableId::SimulationFeatures,
                    "",
                    None,
                    "simulation features lacks its singleton",
                ));
            };
            let count = features
                .state_array_len
                .require("state array len of simulation features")
                .map_err(|why| {
                    Failure::input(
                        replay_data::rows::TableId::SimulationFeatures,
                        "state_array_len",
                        None,
                        why,
                    )
                })?
                .ok_or_else(|| -> Failure {
                    Failure::input(
                        replay_data::rows::TableId::SimulationFeatures,
                        "state_array_len",
                        None,
                        "native state-array extent is unavailable",
                    )
                })? as i32;
            let length = count.wrapping_sub(1);
            return if at == -1 {
                Ok(Some(i64::from(length) * 100_000))
            } else if at < 0 || at >= length {
                Ok(Some(0))
            } else {
                self.encode(Scope::State(at as u32 + 1)).map(Some)
            };
        }
        let (countries, values): (bool, std::borrow::Cow<'_, [u32]>) = match (store, name) {
            (Store::State(state), b"core_countries") => {
                let row = recorded_row::<tables::StateCores>(
                    world.tables(),
                    mask::<replay_data::rows::StateCoreRow>(&["cores"]),
                    u128::from(state),
                )?
                .ok_or("core_countries lacks its state cores row")?;
                (true, std::borrow::Cow::Owned(row.into_owned().cores))
            }
            (Store::Global, b"countries") => (
                true,
                std::borrow::Cow::Borrowed(
                    world
                        .recorded()
                        .original_tags()
                        .country_array()?
                        .get(1..)
                        .ok_or("native country array lacks its reserved first slot")?,
                ),
            ),
            (Store::Country(slot), b"owned_controlled_states") => {
                let owned = self.country_state_array(slot, b"owned_states")?;
                let mut values = Vec::new();
                for &id in owned.iter() {
                    let row = recorded_row::<tables::States>(
                        world.tables(),
                        mask::<replay_data::rows::StateRow>(&["controller"]),
                        u128::from(id),
                    )?
                    .ok_or("owned state is not recorded")?;
                    if world.slot(row.controller) == Some(slot) {
                        values.push(id);
                    }
                }
                (false, std::borrow::Cow::Owned(values))
            }
            (Store::Country(slot), b"owned_states" | b"controlled_states" | b"core_states") => (
                false,
                self.country_state_array(slot, name)?,
            ),
            _ => return Ok(None),
        };
        if at == -1 {
            return Ok(Some((values.len() as i64).wrapping_mul(100_000)));
        }
        let Some(&raw) = values.get(at as usize) else {
            return Ok(Some(0));
        };
        let scope = if countries {
            Scope::Country(if raw == 0 {
                0
            } else {
                world.slot(raw).ok_or_else(|| -> Failure {
                    world.unlinked("country array of reserved country naming", raw)
                })?
            })
        } else {
            Scope::State(raw)
        };
        self.encode(scope).map(Some)
    }

    fn country_state_array(&self, slot: u32, name: &[u8]) -> Result<Cow<'_, [u32]>, Failure> {
        if name == b"owned_states" || name == b"controlled_states" {
            return self
                .world()
                .ordered_country_states(slot, name == b"controlled_states");
        }
        let (field_name, what) = match name {
            b"owned_states" => ("owned_states", "owned states of country counter inputs"),
            b"controlled_states" => (
                "controlled_states",
                "controlled states of country counter inputs",
            ),
            _ => ("core_states", "core states of country counter inputs"),
        };
        (|| {
            let inputs = self.world().country_counter_inputs(slot)?;
            fn field<'r>(
                inputs: &'r replay_data::rows::CountryCounterInputRow,
                name: &[u8],
            ) -> &'r replay_data::rows::Recorded<Option<Vec<u32>>> {
                match name {
                    b"owned_states" => &inputs.owned_states,
                    b"controlled_states" => &inputs.controlled_states,
                    _ => &inputs.core_states,
                }
            }
            fn states<'r>(
                inputs: &'r replay_data::rows::CountryCounterInputRow,
                name: &[u8],
            ) -> &'r [u32] {
                match field(inputs, name) {
                    replay_data::rows::Recorded::Value(Some(states)) => states.as_slice(),
                    _ => &[],
                }
            }
            field(&inputs, name)
                .require(what)
                .map_err(|why| {
                    Failure::input(
                        replay_data::rows::TableId::CountryCounterInputs,
                        field_name,
                        Some(Store::Country(slot)),
                        why,
                    )
                })?
                .as_deref()
                .ok_or_else(|| -> Failure {
                    Failure::input(
                        replay_data::rows::TableId::CountryCounterInputs,
                        field_name,
                        Some(Store::Country(slot)),
                        format!("{what} capture unavailable"),
                    )
                })?;
            Ok(match inputs {
                Cow::Borrowed(inputs) => Cow::Borrowed(states(inputs, name)),
                Cow::Owned(inputs) => Cow::Owned(states(&inputs, name).to_vec()),
            })
        })()
    }

    pub(super) fn surrender_progress(&self, slot: u32) -> Result<i64, Failure> {
        use replay_data::rows::{CapitulationRow, CountryGameScoreInputRow};
        let world = self.world();
        let raw = world
            .raw_country(slot)
            .ok_or("Countries lacks the surrender scope country")?;
        const INPUTS: u64 = mask::<CapitulationRow>(&[
            "cached_held_core_weight",
            "cached_total_core_weight",
            "cached_held_territory_weight",
            "cached_total_territory_weight",
            "surrender_limit_modifier",
            "forced_surrender_limit_modifier",
            "max_surrender_limit_offset",
            "selected_collaboration",
        ]);
        const CONTROLLED: u64 = mask::<CountryGameScoreInputRow>(&["land_count", "land_error"]);
        let inputs = recorded_row::<tables::Capitulation>(world.tables(), INPUTS, raw.into())?
            .ok_or_else(|| {
                Failure::input(
                    tables::Capitulation::ID,
                    "",
                    Some(Store::Country(slot)),
                    "surrender inputs not recorded for the country",
                )
            })?;
        let territory =
            recorded_row::<tables::CountryGameScoreInputs>(world.tables(), CONTROLLED, raw.into())?
                .ok_or_else(|| {
                    Failure::input(
                        tables::CountryGameScoreInputs::ID,
                        "land_count",
                        Some(Store::Country(slot)),
                        "controlled province count not recorded for the country",
                    )
                })?;
        Ok(dashboard_query::surrender::progress(&inputs, &territory, world.recorded_defines()?)?.0)
    }

    pub(super) fn country_stability_inputs(
        &self,
        slot: u32,
        fields: u64,
    ) -> Result<Cow<'_, replay_data::rows::CountryStabilityInputRow>, Failure> {
        let world = self.world();
        let raw = match world.raw_country(slot) {
            Some(raw) => raw,
            None => {
                world.table_status(replay_data::rows::TableId::Countries)?;
                return Err(Failure::input(
                    replay_data::rows::TableId::Countries,
                    "",
                    Some(Store::Country(slot)),
                    "stability country is not recorded",
                ));
            }
        };
        recorded_row::<tables::CountryStabilityInputs>(world.tables(), fields, u128::from(raw))?
            .ok_or_else(|| {
                Failure::input(
                    replay_data::rows::TableId::CountryStabilityInputs,
                    "",
                    Some(Store::Country(slot)),
                    "country stability inputs lacks the country",
                )
            })
    }

    pub(super) fn stability_modifier_input(
        &self,
        row: &replay_data::rows::CountryStabilityInputRow,
        slot: u32,
        index: usize,
    ) -> Result<i64, Failure> {
        row.totals()[index].ok_or_else(|| {
            let field =
                std::str::from_utf8(replay_data::rows::CountryStabilityInputRow::KEY_NAMES[index])
                    .expect("field name");
            Failure::input(
                replay_data::rows::TableId::CountryStabilityInputs,
                field,
                Some(Store::Country(slot)),
                format!("{} capture unavailable", replay_data::rows::TableId::CountryStabilityInputs.field_words(field)),
            )
        })
    }

    pub(super) fn country_stability_value(&self, slot: u32, support: bool) -> Result<i64, Failure> {
        let row =
            self.country_stability_inputs(slot, tables::CountryStabilityInputs::ID.full_mask())?;
        let mut missing = Vec::new();
        for index in 0..6 {
            let needed = if support {
                index == 5
            } else {
                index < 2
                    || (row.at_war != 0
                        && match index {
                            2 => row.offensive_war != 0 || row.defensive_war != 0,
                            3 => row.offensive_war != 0,
                            4 => row.defensive_war != 0,
                            _ => false,
                        })
            };
            if needed && let Err(failure) = self.stability_modifier_input(&row, slot, index) {
                missing.push(failure);
            }
        }
        if !missing.is_empty() {
            return Err(Failure::merge(missing));
        }
        let terms = if support {
            crate::stability::war_support(&row)
        } else {
            crate::stability::stability(&row)
        };
        Ok(terms
            .value
            .expect("required effective totals were acquired"))
    }

    pub(super) fn country_has_war(&self, slot: u32) -> Result<bool, Failure> {
        if !self.world().enemies(slot)?.is_empty() {
            return Ok(true);
        }
        let world = self.world();
        for row in recorded::<tables::WarRelations>(
            world.tables(),
            mask::<replay_data::rows::WarRelationRow>(&["start_hour", "end_hour"]),
        )? {
            if !super::ongoing(row.start_hour, row.end_hour, world.clock()) {
                continue;
            }
            let first = world
                .slot(row.first_country)
                .ok_or("war relations first country is unresolved")?;
            let second = world
                .slot(row.second_country)
                .ok_or("war relations second country is unresolved")?;
            if first == slot || second == slot {
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(super) fn country_neighbors(&self, slot: u32) -> Result<Vec<u32>, Failure> {
        (|| {
            let world = self.world();
            let inputs = world.country_counter_inputs(slot)?;
            inputs
                .neighbors
                .require("neighbors of country counter inputs")
                .map_err(|why| {
                    Failure::input(
                        replay_data::rows::TableId::CountryCounterInputs,
                        "neighbors",
                        Some(Store::Country(slot)),
                        why,
                    )
                })?
                .as_deref()
                .ok_or_else(|| -> Failure {
                    Failure::input(
                        replay_data::rows::TableId::CountryCounterInputs,
                        "neighbors",
                        Some(Store::Country(slot)),
                        "neighbors of country counter inputs capture unavailable",
                    )
                })?
                .iter()
                .map(|&raw| {
                    if raw == 0 {
                        Ok(0)
                    } else {
                        world.slot(raw).ok_or_else(|| -> Failure {
                            world.unlinked("neighbors of country counter inputs", raw)
                        })
                    }
                })
                .collect()
        })()
    }

    pub(super) fn map_country_iterator(
        &self,
        name: &str,
        block: &[Item],
        scopes: Scopes,
    ) -> Result<Eval, Failure> {
        let world = self.world();
        let candidates = if name == "any_country_with_core" {
            let Scope::State(state) = scopes.this else {
                return Ok(Eval::False);
            };
            let row = recorded_row::<tables::StateCores>(
                world.tables(),
                mask::<replay_data::rows::StateCoreRow>(&["cores"]),
                u128::from(state),
            )?
            .ok_or("state cores lacks iterator state")?;
            row.cores
                .iter()
                .map(|&raw| {
                    if raw == 0 {
                        Ok(0)
                    } else {
                        world
                            .slot(raw)
                            .ok_or_else(|| -> Failure { world.unlinked("state cores", raw) })
                    }
                })
                .collect::<Result<Vec<_>, Failure>>()?
        } else {
            let Scope::Country(slot) = scopes.this else {
                return Ok(Eval::False);
            };
            if slot == 0 {
                Vec::new()
            } else {
                self.country_neighbors(slot)?
            }
        };
        let all = name.starts_with("all_");
        let mut count = 1_i32;
        let mut body = Vec::new();
        for item in block {
            if item.keyword_is("count") {
                count = (self.scoped_value(item.value.scalar().unwrap_or_default(), scopes)?
                    / 100_000) as i32;
            } else if !item.keyword_is("tooltip") {
                body.push(item.clone());
            }
        }
        let mut reasons = BTreeSet::new();
        let mut undecided = 0_i64;
        for country in candidates {
            let entered = self.with_this(scopes, Scope::Country(country))?;
            match self.evaluate(&body, entered) {
                Eval::False if all => return Ok(Eval::False),
                Eval::True if !all => count = count.wrapping_sub(1),
                Eval::Unknown(why) => {
                    undecided += 1;
                    reasons.extend(why);
                    if !all && count == i32::MIN {
                        return Ok(Eval::Unknown(reasons));
                    }
                }
                _ => {}
            }
            if !all && count < 1 {
                return Ok(Eval::True);
            }
        }
        Ok(if undecided > 0 && (all || i64::from(count) <= undecided) {
            Eval::Unknown(reasons)
        } else {
            Eval::from_bool(all)
        })
    }
}

fn recorded<'a, T: replay_data::rows::TableSchema>(
    frame: HourFrame<'a>,
    mask: u64,
) -> Result<Vec<Cow<'a, T::Row>>, Failure> {
    super::inputs::table(frame, T::ID);
    super::world::table::<T>(frame, mask)?
        .iter()
        .map(|row| row.map_err(|why| super::refusal::input_error(T::ID, why)))
        .collect()
}

fn recorded_row<'a, T: replay_data::rows::TableSchema>(
    frame: HourFrame<'a>,
    mask: u64,
    key: u128,
) -> Result<Option<Cow<'a, T::Row>>, Failure> {
    super::inputs::table(frame, T::ID);
    super::world::table::<T>(frame, mask)?
        .get(key)
        .map_err(|why| super::refusal::input_error(T::ID, why))
}
