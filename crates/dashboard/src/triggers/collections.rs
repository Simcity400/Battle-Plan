//! Collection representative selection uses the one evaluator (1404B8B30).
use super::{Evaluator, Scope, Scopes};
use installed_content::{
    collections::{Collection, Input, Operator},
    script_constants::CollectionConstant,
};
use replay::rows::tables;

impl Evaluator<'_, '_> {
    /// CEveryCollectionElementEffect's representative is the first terminal
    /// scope, in source/operator order (1404C3640/1404BEDA0/1404C1760).
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
                Scope::Country(0) | Scope::State(0) => return Ok(None),
                Scope::Country(_) | Scope::State(_) => source.push(caller.this),
                _ => return Err("game:scope collection object is not ported (1404C2DC0)".into()),
            },
            Input::Countries { possible } => {
                let mut rows = world
                    .frame()
                    .complete::<tables::Countries>()
                    .map_err(|why| why.to_string())?
                    .iter()
                    .collect::<Vec<_>>();
                rows.sort_by_key(|row| row.canonical_slot); // CGameState+0x310.
                if *possible && !rows.iter().any(|row| row.canonical_slot == 0) {
                    source.push(Scope::Country(0)); // The native possible-country vector includes its null slot.
                }
                for row in rows {
                    if *possible || world.exists(row.canonical_slot)? {
                        source.push(Scope::Country(row.canonical_slot));
                    }
                }
            }
            Input::States => {
                let mut rows = world
                    .frame()
                    .complete::<tables::States>()
                    .map_err(|why| why.to_string())?
                    .iter()
                    .collect::<Vec<_>>();
                rows.sort_by_key(|row| row.id); // CGameState+0x2C8, skips index zero.
                source.extend(
                    rows.into_iter()
                        .filter(|row| row.id != 0)
                        .map(|row| Scope::State(row.id)),
                );
            }
            Input::Constant(path) => {
                let text = std::str::from_utf8(path)
                    .map_err(|_| "collection constant path is not UTF-8")?;
                if let Some(value) = world
                    .script_constants()?
                    .collection(&text.split('.').collect::<Vec<_>>())?
                {
                    world
                        .frame()
                        .complete::<tables::Countries>()
                        .map_err(|why| why.to_string())?;
                    let country = |tag: &[u8]| {
                        std::str::from_utf8(installed_content::parser::c_string(tag))
                            .ok()
                            .and_then(|tag| world.recorded().country(tag))
                            .unwrap_or(0)
                    };
                    let state = |id: i32| {
                        world
                            .frame()
                            .complete::<tables::States>()
                            .map_err(|why| why.to_string())?;
                        if id > 0 && world.states.contains_key(&(id as u32)) {
                            Ok(Scope::State(id as u32))
                        } else {
                            Err(format!(
                                "collection constant state {id} resolves to a null pointer read by SetStateFromPointer (1404B8110/14052EBA0)"
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
        // the output slot is written only once (1404B8B30/1404BEDA0).
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
            // Country operators log and return false on a state (1404C1760).
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
                rows.sort_by_key(|row| row.ordinal); // CFaction+0x58, 1404BEDA0.
                for row in rows {
                    targets.push(Scope::Country(slot(row.country)?));
                }
            }
            Operator::OwnedStates | Operator::ControlledStates => {
                let rows = world
                    .frame()
                    .complete::<tables::States>()
                    .map_err(|why| why.to_string())?;
                world
                    .frame()
                    .complete::<tables::Provinces>()
                    .map_err(|why| why.to_string())?;
                let nonempty = world.state_members()?;
                for row in rows {
                    if nonempty
                        .get(&row.id)
                        .is_none_or(|provinces| provinces.is_empty())
                    {
                        continue;
                    } // 1406C52B0/1406C3890.
                    let raw = if matches!(operator, Operator::OwnedStates) {
                        row.owner
                    } else {
                        row.controller
                    };
                    if raw != 0 && slot(raw)? == country {
                        targets.push(Scope::State(row.id));
                    }
                }
                if matches!(operator, Operator::OwnedStates)
                    && world.owned_state_count(country)? != targets.len() as i32
                {
                    return Err("collection owned-state membership differs from owned_state_count (CCountry+0x484)".into());
                }
            }
            Operator::CountryAndSubjects => {
                // 1404B7980 tries the country before even reading its subjects.
                let intermediate = self.collection_scope(caller, target, false)?;
                if let Some(found) = self.collection_path(intermediate, target, tail)? {
                    return Ok(Some(found));
                }
                for &raw in &world.appearance(country)?.subjects {
                    targets.push(Scope::Country(if raw == 0 { 0 } else { slot(raw as u32)? })); // CDiplomacyStatus+0x170.
                }
            }
            Operator::Limit(_) => unreachable!(),
        }
        let mut selected = None;
        let unordered = matches!(operator, Operator::OwnedStates | Operator::ControlledStates);
        for next in targets {
            let intermediate = self.collection_scope(caller, target, false)?;
            if let Some(found) = self.collection_path(intermediate, next, tail)? {
                if selected.is_some() && unordered {
                    return Err(match operator {
                        Operator::OwnedStates => "owned_state_order: CCountry+0x478 order is unrecorded and two or more collection targets pass (1404BEDA0)",
                        _ => "controlled_state_order: CCountry+0x460 order is unrecorded and two or more collection targets pass (1404BEDA0)",
                    }.into());
                }
                selected = Some(found);
                if !unordered {
                    break;
                } // Native stops the operator chain at its first success.
            }
        }
        Ok(selected)
    }
}
