//! Targeted-decision descriptions, 14038BF60 / 140720D40.

use super::*;
use crate::{
    scripted_text::{EventObject, EventScope, Link, random},
    triggers::{NULL_STATE, Scope},
};
use installed_content::decisions::Decision;
use replay::rows::{DecisionRow, tables};

#[cfg(test)]
mod tests;

/// 14071C730 compares both native target slots and ignores lifecycle/ignore.
fn matching<'a>(
    rows: &'a [DecisionRow],
    definition: &[u8],
    owner: u32,
    mission: bool,
    target_country: Option<u32>,
    target_state: u32,
    slot: impl Fn(u32) -> Option<u32>,
) -> Result<Option<&'a DecisionRow>, String> {
    let mut candidates = rows
        .iter()
        .filter(|row| {
            row.container == if mission { 4 } else { 3 }
                && row.token.eq_ignore_ascii_case(definition)
        })
        .collect::<Vec<_>>();
    candidates.sort_by_key(|row| row.ordinal);
    for row in candidates {
        let row_owner = slot(row.country)
            .ok_or_else(|| format!("CountryLinks lacks Decisions owner {}", row.country))?;
        if row_owner != owner || row.numeric_target != target_state {
            continue;
        }
        let country = if row.country_target == 0 {
            None
        } else {
            Some(slot(row.country_target).ok_or_else(|| {
                format!("CountryLinks lacks Decisions target {}", row.country_target)
            })?)
        };
        if country == target_country {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

/// 1407290D0 skips the execution-count lookup only for fixed_random_seed.
fn decision_seed(
    game_seed: i32,
    definition: &Decision,
    additive: Option<Option<i32>>,
    target: i32,
) -> Result<u32, String> {
    let additive = if definition.fixed_random_seed {
        0
    } else {
        additive
            .ok_or("Decisions description_seed is unread (1407290D0)")?
            .unwrap_or(0)
    };
    Ok(game_seed
        .wrapping_add(definition.seed_component)
        .wrapping_add(additive)
        .wrapping_add(target) as u32)
}

/// 140721730 tests cosmetic/tag/base keys, each ideology rung before its base.
fn name_key(
    base: &str,
    cosmetic: &str,
    tag: &str,
    ideology: &str,
    exists: impl Fn(&str) -> bool,
) -> String {
    let suffix = format!("_{ideology}");
    let mut candidates = Vec::new();
    if !cosmetic.is_empty() {
        candidates.extend([
            format!("{cosmetic}_{base}{suffix}"),
            format!("{cosmetic}_{base}"),
        ]);
    }
    candidates.extend([
        format!("{tag}_{base}{suffix}"),
        format!("{tag}_{base}"),
        format!("{base}{suffix}"),
        base.to_owned(),
    ]);
    candidates
        .into_iter()
        .find(|key| exists(key))
        .unwrap_or_else(|| base.to_owned())
}

impl Renderer<'_, '_, '_> {
    pub(super) fn removed_decision(
        &mut self,
        item: &Item,
        scopes: Scopes,
    ) -> Result<String, String> {
        let fields = children(item);
        let id = field(fields, "decision")
            .and_then(|value| value.value.read_string())
            .unwrap_or_default();
        let database = self
            .context
            .content
            .decisions
            .ok_or("decision content is unread (14038BF60)")?;
        let Some(definition) = database.decision(id)? else {
            return Ok(String::new());
        };
        let owner = match scopes.this {
            Scope::Country(country) => country,
            _ => return Err("remove_targeted_decision scope has no country (14038BF60)".into()),
        };
        let (country, state) = self.decision_target(field(fields, "target"), scopes)?;
        if country.is_none() && state == 0 {
            return Ok(String::new());
        }
        let world = self.context.world()?;
        let rows = world
            .frame()
            .complete::<tables::Decisions>()
            .map_err(|why| why.to_string())?;
        let Some(row) = matching(
            rows,
            &definition.id,
            owner,
            definition.mission,
            country,
            state,
            |raw| world.recorded().slot(raw),
        )?
        else {
            return Ok(String::new());
        };
        if let Some(why) = &definition.refused {
            return Err(why.clone());
        }
        let title_scope = self.decision_title_scope(definition, row, owner)?;
        let label_key = self.decision_name_key(definition, owner)?;
        let name = self.localised(label_key.as_bytes(), &[], title_scope, true)?;
        let mut text = self.localised(
            b"EFFECT_REMOVE_TARGETED_DECISION",
            &[("NAME", Argument::Text(&name))],
            scopes,
            true,
        )?;
        text.push('\n');
        Ok(text)
    }

    fn decision_target(
        &self,
        target: Option<&Item>,
        scopes: Scopes,
    ) -> Result<(Option<u32>, u32), String> {
        let evaluator = self.evaluator()?;
        if let Some(item) = target {
            let value = item.value.scalar().unwrap_or_default();
            let text = std::str::from_utf8(installed_content::parser::c_string(value))
                .map_err(|_| "decision target is not UTF-8")?;
            // ParseScopeSelector's native event-target fallback includes tooltip
            // targets (1405302E0 / 14052FA10), unlike a trigger country operand.
            let parsed =
                installed_content::scope::parse(text, item.quoted(), self.context.world()?.recorded());
            if matches!(
                parsed,
                installed_content::scope::Scope::Keyword(_) | installed_content::scope::Scope::EventTarget { .. }
            ) {
                return Ok(
                    match evaluator
                        .enter_scope(&parsed, scopes)?
                        .map(|selected| selected.this)
                    {
                        Some(Scope::Country(country)) => (Some(country), 0),
                        Some(Scope::State(state)) if state != NULL_STATE => (None, state),
                        _ => (None, 0),
                    },
                );
            }
            if let Some(country) = evaluator.scoped_country(value, scopes)? {
                return Ok((Some(country), 0));
            }
            let state = evaluator.scoped_state_id(value, scopes)? as u32;
            if state != 0 {
                return Ok((None, state));
            }
        }
        // 14036C6E0: invalid scoped value falls back to the selected state,
        // then selected country. A recognised missing selector stays invalid.
        Ok(match scopes.this {
            Scope::Country(country) => (Some(country), 0),
            Scope::State(state) if state != NULL_STATE => (None, state),
            _ => (None, 0),
        })
    }

    fn decision_name_key(&self, definition: &Decision, country: u32) -> Result<String, String> {
        let world = self.context.world()?;
        let appearance = world
            .frame()
            .complete::<tables::CountryAppearance>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| world.recorded().slot(row.country) == Some(country))
            .ok_or("CountryAppearance lacks decision-name cosmetic tag (140721730)")?;
        let politics = world
            .frame()
            .complete::<tables::Politics>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| world.recorded().slot(row.country) == Some(country))
            .ok_or("Politics lacks decision-name ruling party (140721730)")?;
        let ruling = politics
            .ruling_party
            .ok_or("decision-name ruling party is absent (140721730)")?;
        let party = world
            .frame()
            .complete::<tables::PoliticalParties>()
            .map_err(|why| why.to_string())?
            .iter()
            .find(|row| world.recorded().slot(row.country) == Some(country) && row.id == ruling)
            .ok_or("PoliticalParties lacks decision-name ruling ideology (140721730)")?;
        let utf8 = |bytes: &[u8]| {
            std::str::from_utf8(installed_content::parser::c_string(bytes))
                .map(str::to_owned)
                .map_err(|_| "decision-name key is not UTF-8".to_owned())
        };
        Ok(name_key(
            &utf8(&definition.name)?,
            &utf8(&appearance.cosmetic_tag)?,
            self.tag(country)?,
            &utf8(&party.token)?,
            |key| self.context.localisation.get(key).is_some(),
        ))
    }

    fn decision_title_scope(
        &self,
        definition: &Decision,
        row: &DecisionRow,
        owner: u32,
    ) -> Result<Scopes, String> {
        let game_seed = self
            .context
            .world()?
            .frame()
            .complete::<tables::Clock>()
            .map_err(|why| why.to_string())?
            .first()
            .ok_or("Clock lacks decision-description seed")?
            .seed;
        let country_target = if row.country_target == 0 {
            None
        } else {
            Some(
                self.context
                    .world()?
                    .recorded()
                    .slot(row.country_target)
                    .ok_or("CountryLinks lacks decision title target")?,
            )
        };
        let target = country_target.unwrap_or(row.numeric_target) as i32;
        let seed = decision_seed(game_seed, definition, row.description_seed, target)?;
        let (at, from) = {
            let mut graph = self.graph.borrow_mut();
            let at = graph.scopes.len();
            let from = at + 1;
            let mut own = EventScope::of(EventObject::Country(owner));
            own.from = Link::Scope(from);
            own.random_seed = random::scope_seed(seed);
            own.random_counter = random::scope_counter(seed);
            let mut target = EventScope::of(match country_target {
                Some(country) => EventObject::Country(country),
                None => EventObject::State(row.numeric_target),
            });
            // Native 1407290D0 seeds FROM from the already-mixed +0x0C word.
            target.random_seed = random::scope_seed(own.random_seed);
            target.random_counter = random::scope_counter(own.random_seed);
            graph.scopes.extend([own, target]);
            (at, from)
        };
        let scopes = self.evaluator()?.event_scopes(at)?;
        let highlighted = definition.has_state_targets
            || self
                .evaluator()?
                .has_children(&definition.highlight_conditions)?;
        if highlighted && let Some(state) = self.highlight_state(definition, scopes)? {
            let mut graph = self.graph.borrow_mut();
            let index = graph.scopes.len();
            graph.scopes.push(EventScope::of(EventObject::State(state)));
            graph.scopes[from].from = Link::Scope(index);
        }
        self.evaluator()?.event_scopes(at)
    }

    fn fresh_country_scope(&self, owner: u32, target: Option<u32>) -> Result<Scopes, String> {
        let at = {
            let mut graph = self.graph.borrow_mut();
            let at = graph.scopes.len();
            let mut scope = EventScope::of(EventObject::Country(owner));
            scope.random_seed = random::scope_seed(0);
            scope.random_counter = random::scope_counter(0);
            if let Some(state) = target {
                scope.from = Link::Scope(at + 1);
                graph.scopes.push(scope);
                graph.scopes.push(EventScope::of(EventObject::State(state)));
            } else {
                graph.scopes.push(scope);
            }
            at
        };
        self.evaluator()?.event_scopes(at)
    }

    fn highlight_state(
        &self,
        definition: &Decision,
        scopes: Scopes,
    ) -> Result<Option<u32>, String> {
        // 14071C840 takes THIS/FROM states before any scripted candidate list.
        if let Scope::State(state) = scopes.this
            && state != NULL_STATE
            && state != 0
        {
            return Ok(Some(state));
        }
        if let Some(Scope::State(state)) = scopes.from
            && state != NULL_STATE
            && state != 0
        {
            return Ok(Some(state));
        }
        let evaluator = self.evaluator()?;
        self.context
            .world()?
            .frame()
            .complete::<tables::States>()
            .map_err(|why| why.to_string())?;
        let owner = match scopes.this {
            Scope::Country(owner) => owner,
            _ => return Ok(None),
        };
        let mut candidates = std::collections::BTreeSet::new();
        let root = self.fresh_country_scope(owner, None)?;
        if evaluator
            .evaluate(&definition.target_root_trigger, root)
            .decided()?
        {
            // 140C1BD20 maintains ascending unique state IDs, independently of
            // authored array order, then checks is_valid_target in fresh scopes.
            for array in &definition.target_arrays {
                for object in evaluator.scoped_array_scopes(array, scopes)? {
                    if let Some(Scope::State(state)) = object
                        && state > 0
                        && state != NULL_STATE
                        && self.context.world()?.states.contains_key(&state)
                        && evaluator
                            .evaluate(
                                &definition.target_trigger,
                                self.fresh_country_scope(owner, Some(state))?,
                            )
                            .decided()?
                    {
                        candidates.insert(state);
                    }
                }
            }
            for operand in &definition.target_states {
                let state = evaluator.scoped_state_id(operand, scopes)? as u32;
                if state > 0
                    && state != NULL_STATE
                    && self.context.world()?.states.contains_key(&state)
                    && evaluator
                        .evaluate(
                            &definition.target_trigger,
                            self.fresh_country_scope(owner, Some(state))?,
                        )
                        .decided()?
                {
                    candidates.insert(state);
                }
            }
        }
        if candidates.is_empty() && evaluator.has_children(&definition.highlight_conditions)? {
            for &state in self
                .context
                .world()?
                .states
                .keys()
                .filter(|state| **state > 0)
            {
                let selected = evaluator.with_this(scopes, Scope::State(state))?;
                if evaluator
                    .evaluate(&definition.highlight_conditions, selected)
                    .decided()?
                {
                    candidates.insert(state);
                }
            }
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        let graph = self.graph.borrow();
        let scope = &graph.scopes[scopes
            .cursor
            .ok_or("decision highlight lacks scope cursor")?];
        // 14071C840 draws on a copy, incrementing no caller seed word.
        let index = random::scoped_draw(scope.random_seed, scope.random_counter) as usize
            % candidates.len();
        Ok(candidates.into_iter().nth(index))
    }
}
