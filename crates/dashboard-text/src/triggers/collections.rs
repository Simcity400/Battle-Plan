//! Collection representative selection uses the shared evaluator.
use super::{Evaluator, Scope, Scopes};
use installed_content_core::{
    collections::Collection, collections::Input, collections::Operator,
    script_constants::CollectionConstant,
};
use replay_data::rows::tables;

impl Evaluator<'_, '_> {
    /// A collection effect's representative is the first terminal
    /// scope, in source/operator order.
    pub(crate) fn collection_representative(
        &self,
        collection: &Collection,
        caller: Scopes,
    ) -> Result<Option<Scopes>, String> {
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
                let slots = world.country_slots()?; // Physical country-array order.
                if *possible && !slots.contains(&0) {
                    source.push(Scope::Country(0)); // The native possible-country vector includes its null slot.
                }
                for slot in slots {
                    if *possible || world.exists(slot)? {
                        source.push(Scope::Country(slot));
                    }
                }
            }
            Input::States => {
                // State-array order, skips index zero.
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
                        std::str::from_utf8(installed_content_core::parser::c_string(tag))
                            .ok()
                            .and_then(|tag| world.recorded().country(tag))
                            .unwrap_or(0)
                    };
                    let state = |id: i32| {
                        // Completeness is checked whatever the id, as before.
                        let present = world.state_exists(id as u32)?;
                        if id > 0 && present {
                            Ok(Scope::State(id as u32))
                        } else {
                            Err(format!(
                                "collection constant state {id} resolves to a null state pointer"
                            ))
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
        // The native top-level source loop continues after the first result;
        // the output slot is written only once.
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
    ) -> Result<Option<Scopes>, String> {
        let Some((operator, tail)) = operators.split_first() else {
            return self.collection_scope(caller, target, true).map(Some);
        };
        if let Operator::Limit(items) = operator {
            let selected = self.collection_scope(caller, target, false)?;
            return if self.evaluate(items, selected).decided()? {
                self.collection_path(caller, target, tail)
            } else {
                Ok(None)
            };
        }
        let Scope::Country(country) = target else {
            // Country operators log and return false on a state.
            return Ok(None);
        };
        let world = self.world();
        let slot = |raw| {
            if raw == 0 {
                Ok(0)
            } else {
                world
                    .slot(raw)
                    .ok_or_else(|| format!("collection country reference {raw} is unresolved"))
            }
        };
        let mut targets = Vec::new();
        match operator {
            Operator::FactionMembers => {
                let Some(faction) = world.faction(country)? else {
                    return Ok(None);
                };
                let mut rows = world
                    .frame()
                    .complete::<tables::FactionMembers>()
                    .map_err(|why| why.to_string())?
                    .iter()
                    .filter(|row| row.faction == faction)
                    .collect::<Vec<_>>();
                rows.sort_by_key(|row| row.ordinal); // Stored faction-member order.
                for row in rows {
                    targets.push(Scope::Country(slot(row.country)?));
                }
            }
            Operator::OwnedStates | Operator::ControlledStates => {
                let inputs = world.country_counter_inputs(country)?;
                let (recorded, name) = if matches!(operator, Operator::OwnedStates) {
                    (&inputs.owned_states, "CountryCounterInputs.owned_states")
                } else {
                    (
                        &inputs.controlled_states,
                        "CountryCounterInputs.controlled_states",
                    )
                };
                for &state in recorded
                    .require(name)?
                    .as_deref()
                    .ok_or_else(|| format!("{name} capture unavailable"))?
                {
                    targets.push(Scope::State(state));
                }
            }
            Operator::CountryAndSubjects => {
                // Try the country before even reading its subjects.
                let intermediate = self.collection_scope(caller, target, false)?;
                if let Some(found) = self.collection_path(intermediate, target, tail)? {
                    return Ok(Some(found));
                }
                for &raw in world
                    .appearance(country)?
                    .subjects
                    .require("CountryAppearance.subjects")?
                {
                    targets.push(Scope::Country(if raw == 0 { 0 } else { slot(raw as u32)? })); // Stored subject references.
                }
            }
            Operator::Limit(_) => unreachable!(),
        }
        for next in targets {
            let intermediate = self.collection_scope(caller, target, false)?;
            if let Some(found) = self.collection_path(intermediate, next, tail)? {
                return Ok(Some(found)); // Stop at the first successful path.
            }
        }
        Ok(None)
    }
}
