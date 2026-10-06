//! Tag aliases resolved as AliasResolve (1407141F0) resolves them: the
//! alias's candidates, each checked against its triggers in a scope whose
//! THIS is the candidate, PREV the scope that used the alias and ROOT and
//! FROM that scope's; the first to pass, or the best `country_score`, else
//! the fallback. Nothing caches the answer natively, so it is recomputed
//! from the recorded hour on every use.

use installed_content::tag_aliases::{TagAlias, VariableKey};

use super::{Eval, Scope, Scopes, eval::Evaluator};

/// Candidates whose order among themselves is known: one, or several
/// appended in one recorded interval (crate::scope::OriginalTagLists).
struct Group {
    members: Vec<u32>,
    when: Option<String>,
}

impl Evaluator<'_, '_> {
    /// A country tag or tag alias as ResolveScriptCountryTag (140BA5D10)
    /// resolves it in `scopes`: an alias through AliasResolve (1407141F0),
    /// with no original-tag substitution of its answer; a tag through the
    /// recorded resolver (crate::scope). `None` is no country.
    pub(super) fn resolve_tag(&self, tag: &str, scopes: Scopes) -> Result<Option<u32>, String> {
        let recorded = self.world().recorded();
        match recorded.alias_entry(tag) {
            Some(alias) => self.resolve_alias(tag, alias, scopes),
            None => recorded.resolve_tag(tag).map(Some),
        }
    }

    /// AliasResolve (1407141F0) of `alias` for the scope `caller`.
    fn resolve_alias(
        &self,
        name: &str,
        alias: &TagAlias,
        caller: Scopes,
    ) -> Result<Option<u32>, String> {
        let within = |why: String| format!("tag alias {name}: {why}");
        // A body the reading cannot be followed through holds unknown
        // fields, the variable among them.
        if let Some(why) = &alias.refused {
            return Err(within(why.clone()));
        }
        // The variable, event target or global event target (+0x110)
        // decides alone (EvaluateCountryTag 140536ED0), in the caller's
        // scope.
        if let Some((key, text)) = &alias.variable {
            return match key {
                VariableKey::Variable => self
                    .scoped_country(text, caller)
                    .map_err(|why| within(format!("its variable: {why}"))),
                VariableKey::EventTarget | VariableKey::GlobalEventTarget => {
                    let prefix = if *key == VariableKey::GlobalEventTarget {
                        "global_event_target:"
                    } else {
                        "event_target:"
                    };
                    // 140539800/14053BD20 prepend the prefix and append
                    // ".id" (1427BED90), then parse through 140539B50.
                    self.scoped_country(&[prefix.as_bytes(), text, b".id"].concat(), caller)
                        .map_err(within)
                }
            };
        }
        let frame = format!("tag alias {name}").into_bytes();
        if let Some(why) = self.enter_frame(&frame) {
            return Err(why);
        }
        // The frame holds through the fallback too, so a self or mutual
        // fallback cycle, which the game follows without a guard
        // (1407141F0 -> 140536ED0), is refused by name.
        let resolved = self
            .choose(alias, caller)
            .map_err(within)
            .and_then(|found| {
                if found.is_some() {
                    return Ok(found);
                }
                // Only when nothing won (+0x298): neither checked nor tested for
                // existence.
                match &alias.fallback {
                    Some(fallback) => self
                        .operand_tag(fallback, caller)
                        .map_err(|why| within(format!("its fallback: {why}"))),
                    None => Ok(None),
                }
            });
        self.leave_frame();
        resolved
    }

    /// A country text a CScopedVariable holds (EvaluateCountryTag
    /// 140536ED0): a recorded tag or an alias, else a variable evaluated in
    /// `scopes`.
    fn operand_tag(&self, text: &[u8], scopes: Scopes) -> Result<Option<u32>, String> {
        let recorded = self.world().recorded();
        match std::str::from_utf8(text) {
            Ok(tag) if recorded.alias_entry(tag).is_some() || recorded.country(tag).is_some() => {
                self.resolve_tag(tag, scopes)
            }
            _ => self.scoped_country(text, scopes),
        }
    }

    /// The first phase of candidates 1407141F0 visits: every country slot
    /// (+0x1E0, CGameState+0x310 in slot order), which needs the whole
    /// roster; else the original-tag country, then `_CountriesForOriginalTags`
    /// of it front to back (1401DB900, CGameState+0x988).
    fn original_candidates(&self, alias: &TagAlias, caller: Scopes) -> Result<Vec<Group>, String> {
        let recorded = self.world().recorded();
        let ordered = |slot| Group {
            members: vec![slot],
            when: None,
        };
        if alias.checks_all() {
            return Ok(recorded
                .roster()
                .map_err(|why| format!("it checks every country: {why}"))?
                .map(ordered)
                .collect());
        }
        // Not in the corpus (vanilla and the installed mods, 2026-09-30).
        if alias.original_tag_assignments > 1 {
            return Err(super::refused(format!(
                "it assigns original_tag {} times, and the country specifier's state across assignments (AssignCountrySpecifierFromString 140BA53F0, read by 140BA5D10) is not replayed",
                alias.original_tag_assignments
            )));
        }
        let mut groups = Vec::new();
        if let Some(original) = &alias.original_tag {
            // ResolveScriptCountryTag (param 0) then GetOriginalTagOrTag
            // (1406DF830): a dynamic country's original tag, else its own.
            let original = match self.operand_tag(original, caller) {
                Ok(Some(slot)) => {
                    let lists = recorded.original_tags();
                    if let Some(since) = lists.pending() {
                        return Err(format!(
                            "the original tags are unknown: CountryAppearance is incomplete since hour {since}"
                        ));
                    }
                    match lists.name_tag(slot) {
                        Some(tag) => String::from_utf8_lossy(tag).into_owned(),
                        None => recorded.tag(slot).unwrap_or_default().to_owned(),
                    }
                }
                Ok(None) => String::new(),
                Err(why) => return Err(format!("its original tag: {why}")),
            };
            if let Some(own) = recorded.country(&original) {
                groups.push(ordered(own));
            }
            for group in recorded.original_tags().list(&original) {
                groups.push(Group {
                    members: group.members.clone(),
                    when: (group.members.len() > 1).then(|| group.when.clone()),
                });
            }
        }
        Ok(groups)
    }

    /// by canonical slot and deduplicated), which 1407141F0 reaches only
    /// when no earlier candidate won an unscored alias; none for an alias
    /// checking every country.
    fn target_candidates(&self, alias: &TagAlias, caller: Scopes) -> Result<Vec<Group>, String> {
        let ordered = |slot| Group {
            members: vec![slot],
            when: None,
        };
        if alias.checks_all() {
            return Ok(Vec::new());
        }
        if !alias.unported.is_empty() {
            return Err(format!(
                "its targets read {}, which the port does not follow (ReadScriptTargetsMember 140C1D730, resolved by 140C1C820)",
                alias.unported.join(", ")
            ));
        }
        let targets = self.script_target_candidates(
            &installed_content::country_scorers::ScriptTargets {
                targets: alias.targets.clone(),
                ..Default::default()
            }, caller)?;
        Ok(targets.into_iter().map(ordered).collect())
    }

    /// CScriptTargets::GetTargets (140C1C820), shared by aliases and scorers.
    pub(crate) fn script_target_candidates(
        &self,
        targets: &installed_content::country_scorers::ScriptTargets,
        caller: Scopes,
    ) -> Result<Vec<u32>, String> {
        let country = match caller.this {
            Scope::Country(country) => Some(country),
            Scope::Combat => Some(self.combat_country()?),
            _ => None,
        };
        if !targets.root_limit.is_empty() {
            let root = self.fresh_country_scope(country, None)?;
            if !self.evaluate(&targets.root_limit, root).decided()? { return Ok(Vec::new()); }
        }
        if !targets.unported.is_empty() {
            return Err(format!("script targets: {}", targets.unported.join("; ")));
        }
        // No reached temporary scorer requests the dynamic tail. Its start is
        // CCountryDatabase+0x88, which this selector does not yet resolve.
        if targets.dynamic {
            return Err("script targets_dynamic: dynamic country slot boundary is unported (140C1C820)".into());
        }
        let mut selected = Vec::new();
        let mut accept = |slot| -> Result<(), String> {
            if !targets.non_existing && self.world().recorded().owned_states(slot)? < 1 {
                return Ok(());
            }
            if !targets.target_limit.is_empty() {
                // The target filter runs on the initiator with FROM=target;
                // the score subsequently reverses them (140C1D5B0/14047BB20).
                let target = self.fresh_country_scope(Some(slot), None)?;
                let initiator = self.fresh_country_scope(country, Some(target))?;
                if !self.evaluate(&targets.target_limit, initiator).decided()? { return Ok(()); }
            }
            selected.push(slot);
            Ok(())
        };
        for array in &targets.arrays {
            self.visit_scoped_array_scopes(array, caller, |target| {
                if let Some(Scope::Country(slot)) = target { accept(slot)?; }
                Ok(())
            })?;
        }
        for target in &targets.targets {
            if let Some(slot) = self.operand_tag(target, caller)? { accept(slot)?; }
        }
        selected.sort_unstable();
        selected.dedup();
        Ok(selected)
    }

    /// The first candidate whose triggers pass (+0x1F8), each phase read
    /// only when 1407141F0 reaches it, or with a `country_score` (+0x250)
    /// the highest score above 0 over every phase, a tie keeping the
    /// earlier. Candidates of one group whose order is unknown are refused
    /// by name where their order would decide.
    fn choose(&self, alias: &TagAlias, caller: Scopes) -> Result<Option<u32>, String> {
        let phases: [&dyn Fn() -> Result<Vec<Group>, String>; 2] =
            [&|| self.original_candidates(alias, caller), &|| {
                self.target_candidates(alias, caller)
            }];
        // A fresh scope (140528800) with the caller's ROOT and FROM, PREV
        // the caller, and the candidate set with clearing (set_country
        // 14052DD00 -> 14052F3F0).
        let scopes = |slot: u32| self.with_this(caller, Scope::Country(slot));
        let passes = |slot: u32| match self.evaluate(&alias.limit, scopes(slot)?) {
            Eval::Unknown(reasons) => Err(reasons.into_iter().collect::<Vec<_>>().join("; ")),
            value => Ok(value == Eval::True),
        };
        let ambiguous = |group: &Group, several: &[u32]| {
            format!(
                "{} candidates with the same original tag were added to _CountriesForOriginalTags (CGameState+0x988) in one recorded interval, {}, so which one AliasResolve (1407141F0) meets first is not known: {}",
                several.len(),
                group.when.as_deref().unwrap_or_default(),
                several
                    .iter()
                    .map(|slot| self.describe(Scope::Country(*slot)))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        let Some(score) = alias.scored() else {
            for phase in phases {
                for group in &phase()? {
                    let mut passing = Vec::new();
                    for &slot in &group.members {
                        if passes(slot)? {
                            passing.push(slot);
                        }
                    }
                    match passing.as_slice() {
                        [] => {}
                        [only] => return Ok(Some(*only)),
                        several => return Err(ambiguous(group, several)),
                    }
                }
            }
            return Ok(None);
        };
        let mut groups = Vec::new();
        for phase in phases {
            groups.extend(phase()?);
        }
        let (mut best, mut best_score, mut tied) = (None, 0_i64, None);
        for group in &groups {
            let mut scored = Vec::new();
            for &slot in &group.members {
                if passes(slot)? {
                    scored.push((
                        slot,
                        self.mean_time_to_happen("country_score", score, scopes(slot)?)?,
                    ));
                }
            }
            let Some(top) = scored.iter().map(|(_, score)| *score).max() else {
                continue;
            };
            if top <= best_score {
                continue;
            }
            let winners: Vec<u32> = scored
                .iter()
                .filter(|(_, score)| *score == top)
                .map(|(slot, _)| *slot)
                .collect();
            best = winners.first().copied();
            best_score = top;
            tied = (winners.len() > 1).then(|| ambiguous(group, &winners));
        }
        match tied {
            Some(why) => Err(why),
            None => Ok(best),
        }
    }
}
