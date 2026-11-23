use super::style_fold::Binding;
use super::*;
use viewer_scene_base::projection_fold::{RowFold, fields};
use map_display_data::value_index::ValueIndex;
use std::sync::Weak;

type Paint = ValueIndex<u32, Style>;
#[derive(Default)]
pub(super) struct SelectedStates {
    occupation: [Option<Occupation>; 2],
    networks: Option<Networks>,
}
struct Controllers {
    links: RowFold<u32>,
    states: RowFold<u32>,
    by_raw_state: ValueIndex<(u32, u32), ()>,
    dirty: ValueIndex<u32, ()>,
}
impl Controllers {
    fn read(inputs: &Inputs, old: Option<&Self>) -> io::Result<Self> {
        let links = RowFold::read(
            inputs.frame,
            TableId::CountryLinks,
            core::<tables::CountryLinks>(inputs.frame)?,
            fields(TableId::CountryLinks, &["canonical_slot"]),
            old.map(|old| &old.links),
            |row| Ok(row.canonical_slot),
        )?;
        let _ = core::<tables::Countries>(inputs.frame)?;
        let states = RowFold::read(
            inputs.frame,
            TableId::States,
            core::<tables::States>(inputs.frame)?,
            fields(TableId::States, &["controller"]),
            old.map(|old| &old.states),
            |row| Ok(row.controller),
        )?;
        let mut by_raw_state = old.map_or_else(ValueIndex::default, |old| old.by_raw_state.clone());
        let mut dirty = ValueIndex::default();
        for &key in states.edits.iter() {
            if let Some(&raw) = old.and_then(|old| old.states.values.get(&key)) {
                by_raw_state = by_raw_state.removed(&(raw, key as u32))?;
            }
            if let Some(&raw) = states.values.get(&key) {
                by_raw_state = by_raw_state.inserted((raw, key as u32), ())?;
            }
            dirty = dirty.inserted(key as u32, ())?;
        }
        for &raw in links.edits.iter() {
            for (&(_, state), _) in by_raw_state
                .iter_from(&(raw as u32, 0))
                .take_while(|(key, _)| key.0 == raw as u32)
            {
                dirty = dirty.inserted(state, ())?;
            }
        }
        Ok(Self {
            links,
            states,
            by_raw_state,
            dirty,
        })
    }
    fn slot(&self, raw: u32) -> Option<u32> {
        self.links.values.get(&(raw as u128)).copied()
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Resistance {
    country: u32,
    level: i64,
}
struct Occupation {
    binding: Binding,
    selected: u32,
    controllers: Controllers,
    rows: RowFold<Resistance>,
    styles: Paint,
    missing: ValueIndex<u32, ()>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Network {
    owner: u32,
    target: u32,
    state: i32,
    strength: i64,
    quiet: bool,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct Outcome {
    state: Option<u32>,
    style: Option<Style>,
    unknown: bool,
}
struct Networks {
    binding: Binding,
    selected: u32,
    controllers: Controllers,
    rows: RowFold<Network>,
    by_raw_network: ValueIndex<(u32, u128), ()>,
    by_state_network: ValueIndex<(u32, u128), ()>,
    outcomes: ValueIndex<u128, Outcome>,
    by_state_winner: ValueIndex<(u32, u128), Style>,
    styles: Paint,
    unknown: usize,
}
impl SelectedStates {
    pub(super) fn occupation(
        &mut self,
        inputs: &Inputs,
        selected: u32,
        compliance: bool,
        bank: Option<&Weak<MapData>>,
        warnings: &mut Vec<String>,
    ) -> io::Result<Option<Paint>> {
        let mode = if compliance {
            "compliance"
        } else {
            "resistance"
        };
        let _ = core::<tables::CountryLinks>(inputs.frame)?;
        let _ = core::<tables::Countries>(inputs.frame)?;
        let source = match complete::<tables::StateResistance>(inputs.frame, mode) {
            Ok(rows) => rows,
            Err(why) => {
                warnings.push(why);
                return Ok(None);
            }
        };
        let old = self.occupation[usize::from(compliance)]
            .as_ref()
            .filter(|old| old.selected == selected && old.binding.accepts(inputs, bank));
        let controllers = Controllers::read(inputs, old.map(|old| &old.controllers))?;
        let rows = RowFold::read(
            inputs.frame,
            TableId::StateResistance,
            source,
            fields(
                TableId::StateResistance,
                &["country", if compliance { "compliance" } else { "level" }],
            ),
            old.map(|old| &old.rows),
            |row| {
                Ok(Resistance {
                    country: row.country,
                    level: if compliance {
                        row.compliance
                    } else {
                        row.level
                    },
                })
            },
        )?;
        let mut dirty = controllers.dirty.clone();
        for &key in rows.edits.iter() {
            dirty = dirty.inserted(key as u32, ())?;
        }
        let mut styles = old.map_or_else(Paint::default, |old| old.styles.clone());
        let mut missing = old.map_or_else(ValueIndex::default, |old| old.missing.clone());
        for &state in dirty.keys() {
            let owned = controllers
                .states
                .values
                .get(&(state as u128))
                .is_some_and(|raw| controllers.slot(*raw) == Some(selected));
            let row = rows.values.get(&(state as u128)).filter(|_| owned);
            missing = if owned && row.is_none() {
                missing.inserted(state, ())?
            } else {
                missing.removed(&state)?
            };
            styles = match row {
                Some(row) => {
                    let color = if row.country as i32 > 0 {
                        let list = if compliance {
                            &inputs.content.defines.compliance_colors
                        } else {
                            &inputs.content.defines.resistance_colors
                        };
                        list.color(row.level as f32 / RESISTANCE_SCALE)
                    } else {
                        inputs.content.defines.resistance_uninitialised
                    };
                    styles.inserted(state + 1, style(color))?
                }
                None => styles.removed(&(state + 1))?,
            };
        }
        if !missing.is_empty() {
            warnings.push(format!("The {mode} map mode leaves {} controlled states unfilled without a StateResistance row", missing.len()));
        }
        warnings.push(format!(
            "The {mode} map mode shows the recording's sampled StateResistance values"
        ));
        if let Some(bank) = bank {
            self.occupation[usize::from(compliance)] = Some(Occupation {
                binding: Binding::new(inputs, bank.clone()),
                selected,
                controllers,
                rows,
                styles: styles.clone(),
                missing,
            });
        }
        Ok(Some(styles))
    }
    pub(super) fn operatives(
        &mut self,
        inputs: &Inputs,
        selected: u32,
        bank: Option<&Weak<MapData>>,
        warnings: &mut Vec<String>,
    ) -> io::Result<Option<Paint>> {
        let source = match complete::<tables::IntelNetworks>(inputs.frame, "operatives") {
            Ok(rows) => rows,
            Err(why) => {
                warnings.push(why);
                return Ok(None);
            }
        };
        let old = self
            .networks
            .as_ref()
            .filter(|old| old.selected == selected && old.binding.accepts(inputs, bank));
        let controllers = Controllers::read(inputs, old.map(|old| &old.controllers))?;
        let rows = RowFold::read(
            inputs.frame,
            TableId::IntelNetworks,
            source,
            fields(
                TableId::IntelNetworks,
                &["local_strength_raw", "local_quiet"],
            ),
            old.map(|old| &old.rows),
            |row| {
                Ok(Network {
                    owner: row.owner_country,
                    target: row.target_country,
                    state: row.state_id,
                    strength: row.local_strength_raw,
                    quiet: row.local_quiet != 0,
                })
            },
        )?;
        let mut by_raw_network =
            old.map_or_else(ValueIndex::default, |old| old.by_raw_network.clone());
        let mut by_state_network =
            old.map_or_else(ValueIndex::default, |old| old.by_state_network.clone());
        let mut dirty = ValueIndex::default();
        for &key in rows.edits.iter() {
            if let Some(row) = old.and_then(|old| old.rows.values.get(&key)) {
                by_raw_network = by_raw_network
                    .removed(&(row.owner, key))?
                    .removed(&(row.target, key))?;
                if let Ok(state) = u32::try_from(row.state) {
                    by_state_network = by_state_network.removed(&(state, key))?;
                }
            }
            if let Some(row) = rows.values.get(&key) {
                by_raw_network = by_raw_network
                    .inserted((row.owner, key), ())?
                    .inserted((row.target, key), ())?;
                if let Ok(state) = u32::try_from(row.state) {
                    by_state_network = by_state_network.inserted((state, key), ())?;
                }
            }
            dirty = dirty.inserted(key, ())?;
        }
        for &raw in controllers.links.edits.iter() {
            for (&(_, key), _) in by_raw_network
                .iter_from(&(raw as u32, 0))
                .take_while(|(key, _)| key.0 == raw as u32)
            {
                dirty = dirty.inserted(key, ())?;
            }
        }
        for &state in controllers.dirty.keys() {
            for (&(_, key), _) in by_state_network
                .iter_from(&(state, 0))
                .take_while(|(key, _)| key.0 == state)
            {
                dirty = dirty.inserted(key, ())?;
            }
        }
        let mut outcomes = old.map_or_else(ValueIndex::default, |old| old.outcomes.clone());
        let mut by_state_winner =
            old.map_or_else(ValueIndex::default, |old| old.by_state_winner.clone());
        let mut styles = old.map_or_else(Paint::default, |old| old.styles.clone());
        let mut unknown = old.map_or(0, |old| old.unknown);
        let mut dirty_states = ValueIndex::default();
        for &key in dirty.keys() {
            if let Some(before) = outcomes.get(&key) {
                unknown -= usize::from(before.unknown);
                if let Some(state) = before.state.filter(|_| before.style.is_some()) {
                    by_state_winner = by_state_winner.removed(&(state, key))?;
                    dirty_states = dirty_states.inserted(state, ())?;
                }
            }
            let outcome = rows.values.get(&key).map(|row| {
                let mut result = Outcome {
                    state: None,
                    style: None,
                    unknown: false,
                };
                if controllers.slot(row.owner) != Some(selected) || row.strength <= 0 {
                    return result;
                }
                let Some((state, controller)) = u32::try_from(row.state).ok().and_then(|state| {
                    controllers
                        .states
                        .values
                        .get(&(state as u128))
                        .map(|raw| (state, *raw))
                }) else {
                    result.unknown = true;
                    return result;
                };
                if controller != 0
                    && controllers.slot(controller).is_some()
                    && controllers.slot(controller) == controllers.slot(row.target)
                {
                    result.state = Some(state);
                    result.style = Some(style(network_color(
                        &inputs.content.defines,
                        row.strength,
                        row.quiet,
                    )));
                }
                result
            });
            if let Some(result) = outcome {
                unknown += usize::from(result.unknown);
                if let Some((state, paint)) = result.state.zip(result.style) {
                    by_state_winner = by_state_winner.inserted((state, key), paint)?;
                    dirty_states = dirty_states.inserted(state, ())?;
                }
                outcomes = outcomes.inserted(key, result)?;
            } else {
                outcomes = outcomes.removed(&key)?;
            }
        }
        for &state in dirty_states.keys() {
            styles = match by_state_winner
                .last_at_or_before(&(state, u128::MAX))
                .filter(|(key, _)| key.0 == state)
            {
                Some((_, paint)) => styles.inserted(state + 1, *paint)?,
                None => styles.removed(&(state + 1))?,
            };
        }
        if unknown != 0 {
            warnings.push(format!("The operatives map mode leaves {unknown} network states unfilled that are not recorded states"));
        }
        if let Some(bank) = bank {
            self.networks = Some(Networks {
                binding: Binding::new(inputs, bank.clone()),
                selected,
                controllers,
                rows,
                by_raw_network,
                by_state_network,
                outcomes,
                by_state_winner,
                styles: styles.clone(),
                unknown,
            });
        }
        Ok(Some(styles))
    }
}
