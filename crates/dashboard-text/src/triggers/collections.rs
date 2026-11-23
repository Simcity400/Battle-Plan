use super::{Evaluator, Scope, Scopes};
use crate::triggers::Failure;
use content_reader_politics::{collections::{Collection, Input, Operator}, script_constants::CollectionConstant};
use replay_data::rows::{TableSchema, tables};

impl Evaluator<'_, '_> {
    pub fn collection_representative(
        &self,
        collection: &Collection,
        caller: Scopes,
    ) -> Result<Option<Scopes>, Failure> {
        let world = self.world();
        let mut source = Vec::new();
        match &collection.input {
            Input::Empty => return Ok(None),
            Input::Scope => match caller.this {
                Scope::CountryLeader { country: 0, .. } => return Ok(None),
                Scope::CountryLeader { country, .. } => source.push(Scope::Country(country)),
                Scope::Country(0) | Scope::State(0) => return Ok(None),
                Scope::Country(_) | Scope::State(_) => source.push(caller.this),
                _ => return Err("game:scope collection object is not supported".into()),
            },
            Input::Countries { possible } => {
                let slots = world.country_slots()?;
                if *possible && !slots.contains(&0) {
                    source.push(Scope::Country(0));
                }
                for slot in slots {
                    if *possible || world.exists(slot)? {
                        source.push(Scope::Country(slot));
                    }
                }
            }
            Input::States => {
                source.extend(
                    world
                        .state_ids()?
                        .into_iter()
                        .filter(|id| *id != 0)
                        .map(Scope::State),
                );
            }
            Input::Constant(path) => {
                let text = std::str::from_utf8(path)
                    .map_err(|_| "collection constant path is not UTF-8")?;
                if let Some(value) = world
                    .script_constants()?
                    .collection(&text.split('.').collect::<Vec<_>>())?
                {
                    world.table_status(replay_data::rows::TableId::Countries)?;
                    let country = |tag: &[u8]| {
                        std::str::from_utf8(content_reader_parser::parser::c_string(tag))
                            .ok()
                            .and_then(|tag| world.recorded().country(tag))
                            .unwrap_or(0)
                    };
                    let state = |id: i32| {
                        let present = world.state_exists(id as u32)?;
                        if id > 0 && present {
                            Ok::<_, Failure>(Scope::State(id as u32))
                        } else {
                            Err(format!(
                                "collection constant state {id} resolves to a null state pointer"
                            )
                            .into())
                        }
                    };
                    match value {
                        CollectionConstant::Country(tag) => {
                            let slot = country(tag);
                            if slot != 0 {
                                source.push(Scope::Country(slot));
                            }
                        }
                        CollectionConstant::State(id) => source.push(state(*id)?),
                        CollectionConstant::Countries(tags) => {
                            source.extend(tags.iter().map(|tag| Scope::Country(country(tag))))
                        }
                        CollectionConstant::States(ids) => {
                            for id in ids {
                                source.push(state(*id)?);
                            }
                        }
                    }
                }
            }
        }
        let mut representative = None;
        for target in source {
            let selected = self.collection_path(caller, target, &collection.operators)?;
            if representative.is_none() {
                representative = selected;
            }
        }
        Ok(representative)
    }

    fn collection_path(
        &self,
        caller: Scopes,
        target: Scope,
        operators: &[Operator],
    ) -> Result<Option<Scopes>, Failure> {
        let Some((operator, tail)) = operators.split_first() else {
            return self.collection_scope(caller, target, true).map(Some);
        };
        if let Operator::Limit(items) = operator {
            let selected = self.collection_scope(caller, target, false)?;
            return if self.evaluate(items, selected).decided_input()? {
                self.collection_path(caller, target, tail)
            } else {
                Ok(None)
            };
        }
        let Scope::Country(country) = target else {
            return Ok(None);
        };
        let world = self.world();
        let slot = |raw| {
            if raw == 0 {
                Ok(0)
            } else {
                world
                    .slot(raw)
                    .ok_or_else(|| -> Failure { world.unlinked("collection country", raw) })
            }
        };
        let mut targets = Vec::new();
        match operator {
            Operator::FactionMembers => {
                let Some(faction) = world.faction(country)? else {
                    return Ok(None);
                };
                const MEMBER: u64 =
                    replay_data::rows::mask::<replay_data::rows::FactionMemberRow>(&[
                        "faction", "ordinal",
                    ]);
                let mut rows = Vec::new();
                for row in super::world::table::<tables::FactionMembers>(world.tables(), MEMBER)?
                    .iter()
                {
                    let row = row.map_err(|why| {
                        super::refusal::input_error(tables::FactionMembers::ID, why)
                    })?;
                    if row.faction == faction {
                        rows.push((row.ordinal, row.country));
                    }
                }
                rows.sort_by_key(|row| row.0);
                for (_, country) in rows {
                    targets.push(Scope::Country(slot(country)?));
                }
            }
            Operator::OwnedStates | Operator::ControlledStates => {
                for &state in world
                    .ordered_country_states(
                        country,
                        matches!(operator, Operator::ControlledStates),
                    )?
                    .iter()
                {
                    targets.push(Scope::State(state));
                }
            }
            Operator::CountryAndSubjects => {
                let intermediate = self.collection_scope(caller, target, false)?;
                if let Some(found) = self.collection_path(intermediate, target, tail)? {
                    return Ok(Some(found));
                }
                for &raw in world
                    .appearance(country)?
                    .subjects
                    .require("subjects of country appearance")
                    .map_err(|why| {
                        Failure::input(
                            replay_data::rows::TableId::CountryAppearance,
                            "subjects",
                            Some(super::Store::Country(country)),
                            why,
                        )
                    })?
                {
                    targets.push(Scope::Country(if raw == 0 { 0 } else { slot(raw as u32)? }));
                }
            }
            Operator::Limit(_) => unreachable!(),
        }
        for next in targets {
            let intermediate = self.collection_scope(caller, target, false)?;
            if let Some(found) = self.collection_path(intermediate, next, tail)? {
                return Ok(Some(found));
            }
        }
        Ok(None)
    }
}
