use crate::triggers::Failure;

use content_reader_key_names::tag_aliases::{TagAlias, VariableKey};
use std::{cell::RefCell, rc::Rc};

use super::{Eval, Scope, Scopes, eval::Evaluator};

type Candidates = Vec<Result<u32, Failure>>;

pub(super) struct Entry {
    name: String,
    caller: Scopes,
    graph: Option<Rc<RefCell<crate::scripted_text::EventScopes>>>,
    effects: u64,
}

impl Evaluator<'_, '_> {
    pub fn script_state_target_candidates(
        &self,
        targets: &content_reader_politics::country_scorers::ScriptTargets,
        caller: Scopes,
    ) -> Result<Vec<u32>, Failure> {
        self.with_variable_context(caller, || {
            let country = match caller.this {
                Scope::Country(country) | Scope::CountryLeader { country, .. } => Some(country),
                _ => None,
            };
            if !targets.root_limit.is_empty() {
                let root = self.fresh_country_scope(country, None)?;
                if !self.evaluate(&targets.root_limit, root).decided_input()? {
                    return Ok(Vec::new());
                }
            }
            let unported: Vec<_> = targets
                .unported
                .iter()
                .map(String::as_str)
                .filter(|reason| *reason != "state_target selection mode")
                .collect();
            if !unported.is_empty() {
                return Err(format!("state script targets: {}", unported.join("; ")).into());
            }
            let mut selected = std::collections::BTreeSet::new();
            let mut accept = |state| -> Result<(), Failure> {
                if state == 0 || state == super::NULL_STATE || selected.contains(&state) {
                    return Ok(());
                }
                if !targets.target_limit.is_empty() {
                    let target = self.fresh_country_scope(None, None)?;
                    let target = self.repoint_this(target, Scope::State(state))?;
                    let initiator = self.fresh_country_scope(country, Some(target))?;
                    if !self
                        .evaluate(&targets.target_limit, initiator)
                        .decided_input()?
                    {
                        return Ok(());
                    }
                }
                selected.insert(state);
                Ok(())
            };
            for array in &targets.arrays {
                self.visit_scoped_array_scopes(array, caller, |target| {
                    if let Some(Scope::State(state)) = target {
                        accept(state)?;
                    }
                    Ok(())
                })?;
            }
            for target in &targets.targets {
                accept(self.scoped_state(target, caller)?)?;
            }
            Ok(selected.into_iter().collect())
        })
    }

    pub(super) fn resolve_tag(&self, tag: &str, scopes: Scopes) -> Result<Option<u32>, Failure> {
        let recorded = self.world().recorded();
        match recorded.alias_entry(tag) {
            Some(alias) => self.resolve_alias(tag, alias, scopes),
            None => recorded.resolve_tag(tag).map(Some),
        }
    }

    fn resolve_alias(
        &self,
        name: &str,
        alias: &TagAlias,
        caller: Scopes,
    ) -> Result<Option<u32>, Failure> {
        let within = |why: Failure| why.context(|why| format!("tag alias {name}: {why}"));
        if let Some(why) = &alias.refused {
            return Err(within(why.clone().into()));
        }
        let graph = self.event_graph.borrow().clone();
        let frame = format!("tag alias {name}").into_bytes();
        if let Some(why) = self.enter_frame(frame) {
            let why = Failure::from(why);
            let endless = self.aliases.borrow().iter().any(|entry| {
                entry.name == name
                    && entry.caller == caller
                    && entry.effects == self.variable_effects()
                    && match (&entry.graph, &graph) {
                        (Some(entered), Some(graph)) => Rc::ptr_eq(entered, graph),
                        (entered, graph) => entered.is_none() && graph.is_none(),
                    }
            });
            if endless {
                self.diverged.replace(Some(why.clone()));
            }
            return Err(why);
        }
        self.aliases.borrow_mut().push(Entry {
            name: name.to_owned(),
            caller,
            graph,
            effects: self.variable_effects(),
        });
        let resolved = self.resolve_entered(alias, caller, within);
        self.aliases.borrow_mut().pop();
        self.leave_frame();
        resolved
    }

    fn resolve_entered(
        &self,
        alias: &TagAlias,
        caller: Scopes,
        within: impl Fn(Failure) -> Failure,
    ) -> Result<Option<u32>, Failure> {
        if let Some((key, text)) = &alias.variable {
            return match key {
                VariableKey::Variable => self
                    .scoped_country(text, caller)
                    .map_err(|why| within(why.context(|why| format!("its variable: {why}")))),
                VariableKey::EventTarget | VariableKey::GlobalEventTarget => {
                    let prefix = if *key == VariableKey::GlobalEventTarget {
                        "global_event_target:"
                    } else {
                        "event_target:"
                    };
                    self.scoped_country(&[prefix.as_bytes(), text, b".id"].concat(), caller)
                        .map_err(within)
                }
            };
        }
        let found = self.choose(alias, caller).map_err(&within)?;
        if found.is_some() {
            return Ok(found);
        }
        match &alias.fallback {
            Some(fallback) => self
                .operand_tag(fallback, caller)
                .map_err(|why| within(why.context(|why| format!("its fallback: {why}")))),
            None => Ok(None),
        }
    }

    fn operand_tag(&self, text: &[u8], scopes: Scopes) -> Result<Option<u32>, Failure> {
        let recorded = self.world().recorded();
        match std::str::from_utf8(text) {
            Ok(tag) if recorded.alias_entry(tag).is_some() || recorded.country(tag).is_some() => {
                self.resolve_tag(tag, scopes)
            }
            _ => self.scoped_country(text, scopes),
        }
    }

    fn original_candidates(&self, alias: &TagAlias, caller: Scopes) -> Result<Candidates, Failure> {
        let recorded = self.world().recorded();
        let candidate = |raw| super::refusal::deferred(|| recorded.positive_country(raw));
        if alias.checks_all() {
            return Ok(recorded
                .original_tags()
                .country_array()?
                .iter()
                .filter(|&&raw| (raw as i32) > 0)
                .map(|&raw| candidate(raw))
                .filter_map(|result| result.transpose())
                .collect());
        }
        if alias.original_tag_assignments > 1 {
            return Err(crate::triggers::refusal::refused(format!(
                "it assigns original_tag {} times, and the country specifier's state across assignments is not replayed",
                alias.original_tag_assignments
            )));
        }
        let Some(original) = &alias.original_tag else {
            return Ok(Vec::new());
        };
        let Some(slot) = self
            .operand_tag(original, caller)
            .map_err(|why| why.context(|why| format!("its original tag: {why}")))?
        else {
            return Ok(Vec::new());
        };
        let selected = self.world().original_reference(slot)?;
        if selected <= 0 {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        match candidate(selected as u32) {
            Ok(Some(slot)) => out.push(Ok(slot)),
            Ok(None) => {}
            Err(why) => {
                out.push(Err(why));
                return Ok(out);
            }
        }
        match super::refusal::deferred(|| recorded.original_bucket(selected)) {
            Ok(members) => out.extend(
                members
                    .iter()
                    .map(|&raw| candidate(raw))
                    .filter_map(|result| result.transpose()),
            ),
            Err(why) => out.push(Err(why)),
        }
        Ok(out)
    }

    fn target_candidates(&self, alias: &TagAlias, caller: Scopes) -> Result<Candidates, Failure> {
        if alias.checks_all() {
            return Ok(Vec::new());
        }
        if !alias.unported.is_empty() {
            return Err(format!(
                "its targets read {}, which the selector does not follow",
                alias.unported.join(", ")
            )
            .into());
        }
        let targets = self.script_target_candidates(
            &content_reader_politics::country_scorers::ScriptTargets {
                targets: alias.targets.clone(),
                ..Default::default()
            },
            caller,
        )?;
        Ok(targets.into_iter().map(Ok).collect())
    }

    pub fn script_target_candidates(
        &self,
        targets: &content_reader_politics::country_scorers::ScriptTargets,
        caller: Scopes,
    ) -> Result<Vec<u32>, Failure> {
        self.with_variable_context(caller, || {
            let country = match caller.this {
                Scope::Country(country) | Scope::CountryLeader { country, .. } => Some(country),
                Scope::Combat => Some(self.combat_country()?),
                _ => None,
            };
            if !targets.root_limit.is_empty() {
                let root = self.fresh_country_scope(country, None)?;
                if !self.evaluate(&targets.root_limit, root).decided_input()? {
                    return Ok(Vec::new());
                }
            }
            if !targets.unported.is_empty() {
                return Err(format!("script targets: {}", targets.unported.join("; ")).into());
            }
            if targets.dynamic {
                return Err(
                    "script targets_dynamic: dynamic country slot boundary is not supported".into(),
                );
            }
            let mut selected = Vec::new();
            let mut accept = |slot| -> Result<(), Failure> {
                if !targets.non_existing && self.world().recorded().owned_states(slot)? < 1 {
                    return Ok(());
                }
                if !targets.target_limit.is_empty() {
                    let target = self.fresh_country_scope(Some(slot), None)?;
                    let initiator = self.fresh_country_scope(country, Some(target))?;
                    if !self
                        .evaluate(&targets.target_limit, initiator)
                        .decided_input()?
                    {
                        return Ok(());
                    }
                }
                selected.push(slot);
                Ok(())
            };
            for array in &targets.arrays {
                self.visit_scoped_array_scopes(array, caller, |target| {
                    if let Some(Scope::Country(slot)) = target {
                        accept(slot)?;
                    }
                    Ok(())
                })?;
            }
            for target in &targets.targets {
                if let Some(slot) = self.operand_tag(target, caller)? {
                    accept(slot)?;
                }
            }
            selected.sort_unstable();
            selected.dedup();
            Ok(selected)
        })
    }

    fn choose(&self, alias: &TagAlias, caller: Scopes) -> Result<Option<u32>, Failure> {
        let phases: [&dyn Fn() -> Result<Candidates, Failure>; 2] =
            [&|| self.original_candidates(alias, caller), &|| {
                self.target_candidates(alias, caller)
            }];
        let scopes = |slot| self.with_this(caller, Scope::Country(slot));
        let passes = |slot| match self.evaluate(&alias.limit, scopes(slot)?) {
            Eval::Unknown(reasons) => Err(Failure::merge(reasons)),
            value => Ok(value == Eval::True),
        };
        let mut best = None;
        let mut best_score = 0_i64;
        for phase in phases {
            for candidate in phase()? {
                let slot = candidate.inspect_err(Failure::reached)?;
                if !passes(slot)? {
                    continue;
                }
                let Some(score) = alias.scored() else {
                    return Ok(Some(slot));
                };
                let score = self.mean_time_to_happen("country_score", score, scopes(slot)?)?;
                if score > best_score {
                    best = Some(slot);
                    best_score = score;
                }
            }
        }
        Ok(best)
    }
}
