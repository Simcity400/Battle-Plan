use super::*;
use viewer_scene_base::projection_fold::{RowFold, fields};
use map_display_data::{landscape::ProvinceIds, retained::Retained, value_index::ValueIndex};
use replay_data::rows::{Recorded, TableId};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Province {
    state: u32,
    raw: u32,
}
#[derive(Clone, PartialEq, Eq)]
struct State {
    raw: u32,
    single: Recorded<Option<u32>>,
}
#[derive(Clone, PartialEq, Eq)]
enum Routing {
    Single(u32),
    Split,
    Refused(Retained<String>),
}
#[derive(Clone)]
pub(super) struct Groups {
    provinces: RowFold<Province>,
    links: RowFold<u32>,
    states: Option<RowFold<State>>,
    by_raw_province: ValueIndex<(u32, u32), ()>,
    by_raw_state: ValueIndex<(u32, u32), ()>,
    by_state: ValueIndex<(u32, u32), ()>,
    by_state_target: ValueIndex<(u32, u32), ()>,
    routing: ValueIndex<u32, Routing>,
    province_target: ValueIndex<u32, TargetId>,
    province_error: ValueIndex<u32, (u32, Retained<String>)>,
    by_state_error: ValueIndex<(u32, u32), Retained<String>>,
    deferred: ValueIndex<u32, ()>,
    deferred_states: ValueIndex<u32, ()>,
    pub targets: ValueIndex<TargetId, ProvinceIds>,
    pub errors: ValueIndex<u32, Retained<String>>,
}
impl Groups {
    pub(super) fn read(
        kind: LayerType,
        inputs: &Inputs,
        old: Option<&Self>,
        need_states: bool,
    ) -> std::io::Result<Self> {
        let frame = inputs.frame;
        let provinces = RowFold::read(
            frame,
            TableId::Provinces,
            core::<tables::Provinces>(frame)?,
            fields(TableId::Provinces, &["state", "controller"]),
            old.map(|old| &old.provinces),
            |row| {
                Ok(Province {
                    state: row.state,
                    raw: row.controller,
                })
            },
        )?;
        let links = RowFold::read(
            frame,
            TableId::CountryLinks,
            core::<tables::CountryLinks>(frame)?,
            fields(TableId::CountryLinks, &["canonical_slot"]),
            old.map(|old| &old.links),
            |row| Ok(row.canonical_slot),
        )?;
        let states = if kind == LayerType::StateController && need_states {
            Some(RowFold::read(
                frame,
                TableId::States,
                core::<tables::States>(frame)?,
                fields(TableId::States, &["controller", "single_controller"]),
                old.and_then(|old| old.states.as_ref()),
                |row| {
                    Ok(State {
                        raw: row.controller,
                        single: row.single_controller.clone(),
                    })
                },
            )?)
        } else {
            old.and_then(|old| old.states.clone())
        };
        let mut next = Self {
            provinces,
            links,
            states,
            by_raw_province: old
                .map_or_else(ValueIndex::default, |old| old.by_raw_province.clone()),
            by_raw_state: old.map_or_else(ValueIndex::default, |old| old.by_raw_state.clone()),
            by_state: old.map_or_else(ValueIndex::default, |old| old.by_state.clone()),
            by_state_target: old
                .map_or_else(ValueIndex::default, |old| old.by_state_target.clone()),
            routing: old.map_or_else(ValueIndex::default, |old| old.routing.clone()),
            province_target: old
                .map_or_else(ValueIndex::default, |old| old.province_target.clone()),
            province_error: old.map_or_else(ValueIndex::default, |old| old.province_error.clone()),
            by_state_error: old.map_or_else(ValueIndex::default, |old| old.by_state_error.clone()),
            deferred: ValueIndex::default(),
            deferred_states: ValueIndex::default(),
            targets: old.map_or_else(ValueIndex::default, |old| old.targets.clone()),
            errors: old.map_or_else(ValueIndex::default, |old| old.errors.clone()),
        };
        let mut dirty = old.map_or_else(ValueIndex::default, |old| old.deferred.clone());
        let mut dirty_states =
            old.map_or_else(ValueIndex::default, |old| old.deferred_states.clone());
        let mut touched_states = ValueIndex::default();
        for &id in next.provinces.edits.iter() {
            dirty = dirty.inserted(id as u32, ())?;
        }
        for &raw in next.links.edits.iter() {
            for (&(_, id), _) in next
                .by_raw_province
                .iter_from(&(raw as u32, 0))
                .take_while(|(key, _)| key.0 == raw as u32)
            {
                dirty = dirty.inserted(id, ())?;
            }
            for (&(_, id), _) in next
                .by_raw_state
                .iter_from(&(raw as u32, 0))
                .take_while(|(key, _)| key.0 == raw as u32)
            {
                dirty_states = dirty_states.inserted(id, ())?;
            }
        }
        for &id in next.provinces.edits.iter() {
            let before = old.and_then(|old| old.provinces.values.get(&id)).copied();
            let after = next.provinces.values.get(&id).copied();
            if let Some(row) = before {
                next.by_raw_province = next.by_raw_province.removed(&(row.raw, id as u32))?;
                if row.state != 0 {
                    next.by_state = next.by_state.removed(&(row.state, id as u32))?;
                }
            }
            if let Some(row) = after {
                next.by_raw_province = next.by_raw_province.inserted((row.raw, id as u32), ())?;
                if row.state != 0 {
                    next.by_state = next.by_state.inserted((row.state, id as u32), ())?;
                }
            }
        }
        if need_states && let Some(states) = &next.states {
            for &id in states.edits.iter() {
                if let Some(row) = old
                    .and_then(|old| old.states.as_ref())
                    .and_then(|rows| rows.values.get(&id))
                {
                    next.by_raw_state = next.by_raw_state.removed(&(row.raw, id as u32))?;
                }
                if let Some(row) = states.values.get(&id) {
                    next.by_raw_state = next.by_raw_state.inserted((row.raw, id as u32), ())?;
                }
                dirty_states = dirty_states.inserted(id as u32, ())?;
            }
        }
        for &state in dirty_states.keys() {
            if need_states {
                next.routing = next.routing.inserted(state, next.route(state)?)?;
            }
            for index in old
                .map(|old| &old.by_state)
                .into_iter()
                .chain(std::iter::once(&next.by_state))
            {
                for (&(_, id), _) in index
                    .iter_from(&(state, 0))
                    .take_while(|(key, _)| key.0 == state)
                {
                    dirty = dirty.inserted(id, ())?;
                }
            }
        }
        for &id in dirty.keys() {
            let row = next.provinces.values.get(&(id as u128)).copied();
            if kind == LayerType::StateController
                && !need_states
                && row.is_some_and(|row| row.state != 0)
            {
                next.deferred = next.deferred.inserted(id, ())?;
                continue;
            }
            if let Some(&target) = next.province_target.get(&id) {
                next.member(target, id, false)?;
                next.province_target = next.province_target.removed(&id)?;
            }
            if let Some((state, _)) = next.province_error.get(&id) {
                let state = *state;
                next.by_state_error = next.by_state_error.removed(&(state, id))?;
                next.province_error = next.province_error.removed(&id)?;
                touched_states = touched_states.inserted(state, ())?;
            }
            let Some(row) = row else {
                continue;
            };
            let target = match kind {
                LayerType::Country => (row.raw > 0)
                    .then(|| next.links.values.get(&(row.raw as u128)))
                    .flatten()
                    .copied()
                    .map(TargetId::Country),
                LayerType::State => (row.state != 0).then_some(TargetId::State(row.state)),
                LayerType::StateController if row.state != 0 => {
                    if !next.routing.contains_key(&row.state) {
                        next.routing = next.routing.inserted(row.state, next.route(row.state)?)?;
                    }
                    let (target, error) = match next
                        .routing
                        .get(&row.state)
                        .expect("state controller route")
                    {
                        Routing::Single(controller) => (
                            Some(TargetId::Portion {
                                state: row.state,
                                controller: *controller,
                            }),
                            None,
                        ),
                        Routing::Split if row.raw as i32 > 0 => {
                            match next.links.values.get(&(row.raw as u128)) {
                                Some(&controller) => (
                                    Some(TargetId::Portion {
                                        state: row.state,
                                        controller,
                                    }),
                                    None,
                                ),
                                None => (
                                    None,
                                    Some(Retained::new(format!(
                                        "province {id} controller is unresolved"
                                    ))),
                                ),
                            }
                        }
                        Routing::Split => (None, None),
                        Routing::Refused(why) => (None, Some(why.clone())),
                    };
                    if let Some(why) = error {
                        next.province_error =
                            next.province_error.inserted(id, (row.state, why.clone()))?;
                        next.by_state_error = next.by_state_error.inserted((row.state, id), why)?;
                        touched_states = touched_states.inserted(row.state, ())?;
                    }
                    target
                }
                _ => None,
            };
            if let Some(target) = target {
                next.member(target, id, true)?;
                next.province_target = next.province_target.inserted(id, target)?;
            }
        }
        for &state in touched_states.keys() {
            next.errors = match next
                .by_state_error
                .iter_from(&(state, 0))
                .next()
                .filter(|(key, _)| key.0 == state)
            {
                Some((_, why)) => next.errors.inserted(state, why.clone())?,
                None => next.errors.removed(&state)?,
            };
        }
        if kind == LayerType::StateController && !need_states {
            next.deferred_states = dirty_states;
        }
        Ok(next)
    }
    fn route(&self, state: u32) -> std::io::Result<Routing> {
        let route = (|| -> Result<Routing, String> {
            let row = self
                .states
                .as_ref()
                .and_then(|states| states.values.get(&(state as u128)))
                .ok_or_else(|| format!("States lacks state {state}"))?;
            let single = row
                .single
                .require("States.single_controller")?
                .ok_or_else(|| format!("state {state} cached controller grouping unavailable"))?;
            if single == 0 {
                return Ok(Routing::Split);
            }
            let slot = if row.raw == 0 {
                0
            } else {
                self.links
                    .values
                    .get(&(row.raw as u128))
                    .copied()
                    .ok_or_else(|| format!("state {state} controller is unresolved"))?
            };
            Ok(Routing::Single(slot))
        })();
        match route {
            Ok(route) => Ok(route),
            Err(why) => Ok(Routing::Refused(Retained::new(why))),
        }
    }
    fn member(&mut self, target: TargetId, id: u32, present: bool) -> std::io::Result<()> {
        let old = self.targets.get(&target).cloned().unwrap_or_default();
        let members = ProvinceIds(if present {
            old.0.inserted(id, ())?
        } else {
            old.0.removed(&id)?
        });
        let empty = members.is_empty();
        self.targets = if empty {
            self.targets.removed(&target)?
        } else {
            self.targets.inserted(target, members)?
        };
        if let TargetId::Portion { state, controller } = target {
            self.by_state_target = if empty {
                self.by_state_target.removed(&(state, controller))?
            } else {
                self.by_state_target.inserted((state, controller), ())?
            };
        }
        Ok(())
    }
    pub(super) fn select(
        &self,
        kind: LayerType,
        chosen: u32,
        mut visit: impl FnMut(TargetId) -> std::io::Result<()>,
    ) -> std::io::Result<()> {
        match kind {
            LayerType::Country => visit(TargetId::Country(chosen))?,
            LayerType::State => visit(TargetId::State(chosen))?,
            LayerType::StateController => {
                for (&(_, controller), _) in self
                    .by_state_target
                    .iter_from(&(chosen, 0))
                    .take_while(|(key, _)| key.0 == chosen)
                {
                    visit(TargetId::Portion {
                        state: chosen,
                        controller,
                    })?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub(super) fn states_exist(&self, chosen: Option<&[u32]>) -> bool {
        chosen.map_or_else(
            || !self.by_state.is_empty(),
            |chosen| {
                chosen.iter().any(|&state| {
                    self.by_state
                        .iter_from(&(state, 0))
                        .next()
                        .is_some_and(|(key, _)| key.0 == state)
                })
            },
        )
    }
}
