//! Tag aliases are resolved through their
//! candidates, each checked against its triggers in a scope whose
//! THIS is the candidate, PREV the scope that used the alias and ROOT and
//! FROM that scope's; the first to pass, or the best `country_score`, else
//! the fallback. Nothing caches the answer natively, so it is recomputed
//! from the recorded hour on every use.
use crate::triggers::Failure;

use installed_content_core::{tag_aliases::TagAlias, tag_aliases::VariableKey};

use super::{Eval, Scope, Scopes, eval::Evaluator};

type Candidates = Vec<Result<u32, Failure>>;

impl Evaluator<'_, '_> {
    /// State-mode script targets use state array values and state specifiers, with the
    /// same initiator ROOT and target FROM filter as country targets.
    pub fn script_state_target_candidates(
        &self,
        targets: &installed_content_core::country_scorers::ScriptTargets,
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
            // This selector never consults the country selector's state-target marker.
            let unported: Vec<_> = targets
                .unported
                .iter()
                .map(String::as_str)
                .filter(|reason| *reason != "state_target selection mode")
                .collect();
            if !unported.is_empty() {
                return Err(format!("state script targets: {}", unported.join("; ")).into());
            }
            // The state selector does not use the country-only dynamic tail.
            let mut selected = std::collections::BTreeSet::new();
            let mut accept = |state| -> Result<(), Failure> {
                // Accepted states are skipped before their filters can write temps again.
                // A rejected candidate remains eligible for a later visit.
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

    /// Resolve a country tag or tag alias in `scopes`: an alias through its candidates,
    /// with no original-tag substitution of its answer; a tag through the
    /// recorded resolver (crate::scope). `None` is no country.
    pub(super) fn resolve_tag(&self, tag: &str, scopes: Scopes) -> Result<Option<u32>, Failure> {
        let recorded = self.world().recorded();
        match recorded.alias_entry(tag) {
            Some(alias) => self.resolve_alias(tag, alias, scopes),
            None => recorded.resolve_tag(tag).map(Some),
        }
    }

    /// Resolve `alias` for the scope `caller`.
    fn resolve_alias(
        &self,
        name: &str,
        alias: &TagAlias,
        caller: Scopes,
    ) -> Result<Option<u32>, Failure> {
        let within = |why: Failure| why.context(|why| format!("tag alias {name}: {why}"));
        // A body the reading cannot be followed through holds unknown
        // fields, the variable among them.
        if let Some(why) = &alias.refused {
            return Err(within(why.clone().into()));
        }
        // The variable, event target or global event target
        // decides alone, in the caller's
        // scope.
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
                    // Prepend the target prefix and append ".id", then
                    // parse the resulting scoped operand.
                    self.scoped_country(&[prefix.as_bytes(), text, b".id"].concat(), caller)
                        .map_err(within)
                }
            };
        }
        let frame = format!("tag alias {name}").into_bytes();
        if let Some(why) = self.enter_frame(frame) {
            return Err(why.into());
        }
        // The frame holds through the fallback too, so a self or mutual
        // fallback cycle, which the game follows without a guard,
        // is refused by name.
        let resolved = self
            .choose(alias, caller)
            .map_err(within)
            .and_then(|found| {
                if found.is_some() {
                    return Ok(found);
                }
                // Only when nothing won: neither checked nor tested for
                // existence.
                match &alias.fallback {
                    Some(fallback) => self
                        .operand_tag(fallback, caller)
                        .map_err(|why| within(why.context(|why| format!("its fallback: {why}")))),
                    None => Ok(None),
                }
            });
        self.leave_frame();
        resolved
    }

    /// A scoped operand's country text: a recorded tag or an alias,
    /// else a variable evaluated in
    /// `scopes`.
    fn operand_tag(&self, text: &[u8], scopes: Scopes) -> Result<Option<u32>, Failure> {
        let recorded = self.world().recorded();
        match std::str::from_utf8(text) {
            Ok(tag) if recorded.alias_entry(tag).is_some() || recorded.country(tag).is_some() => {
                self.resolve_tag(tag, scopes)
            }
            _ => self.scoped_country(text, scopes),
        }
    }

    /// Native physical-array order for checks_all; otherwise the selected
    /// original's own country, followed by its actual stored bucket. Unknown
    /// later operands are deferred until an earlier unscored candidate fails.
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

    /// Sorted/deduplicated script targets, reached after the original phase
    /// for an unscored alias, or included in scoring for a scored alias.
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
            &installed_content_core::country_scorers::ScriptTargets {
                targets: alias.targets.clone(),
                ..Default::default()
            },
            caller,
        )?;
        Ok(targets.into_iter().map(Ok).collect())
    }

    /// Script target selection, shared by aliases, scorers and scripted map modes.
    pub fn script_target_candidates(
        &self,
        targets: &installed_content_core::country_scorers::ScriptTargets,
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
            // The dynamic tail requires a country slot boundary
            // that this selector does not yet resolve.
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
                    // The target filter runs on the initiator with FROM=target;
                    // the score subsequently reverses them.
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

    /// Native first passing candidate, or highest positive score with the
    /// first occurrence winning ties. Duplicate native visits remain visits.
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
