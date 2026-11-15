//! A template's equipment need per archetype, as the game sums it: each grouped sub-unit's
//! requirements times its count, by archetype (its lexer token). A global definition's
//! requirements come from the sub-unit needs definition (the recording's facts), their archetype
//! names spelled to tokens through LexicalTokens; a country-local clone's are its recorded
//! CountryLocalSubunits row, already by archetype token, else, when that complete table names
//! none for it, its global definition's.

use super::inputs::{self, LocalSubunit, TemplateFacts};
use crate::forces::Owners;
use crate::framework::Hour;
use installed_content_core::unit_leader::needs::SubUnitNeeds;
use std::collections::{HashMap, HashSet};

/// A clone's requirements, or why they cannot be used.
type Override = Result<Vec<(u32, i64)>, String>;

/// Which clones' requirements an hour moved: some, by (canonical slot, token), or every one (why
/// none can be used changed).
pub(super) enum Moved {
    All,
    Clones(HashSet<(u32, u32)>),
}

impl Moved {
    /// Whether a need that looked up `clones` may read otherwise now.
    pub fn touches(&self, clones: &[(u32, u32)]) -> bool {
        match self {
            Self::All => !clones.is_empty(),
            Self::Clones(moved) => clones.iter().any(|clone| moved.contains(clone)),
        }
    }
}

#[derive(Default)]
pub(super) struct Rules {
    /// Each archetype name the needs definition holds, under the fold, and its lexer token; none
    /// for a name two tokens spell.
    tokens: HashMap<String, Option<u32>>,
    /// Each differing clone by (canonical slot, token), with the raw country whose row it is.
    local: HashMap<(u32, u32), (u32, Override)>,
    /// Why no clone can be used: a row's country no link resolves, or two rows of one slot name
    /// one clone.
    local_error: Option<String>,
    /// CountryLocalSubunits is complete: a clone it does not name is its global definition.
    local_complete: bool,
}

/// The archetype names of `needs`, folded.
fn names(needs: &SubUnitNeeds) -> HashSet<String> {
    needs.needs.values().flatten().map(|(name, _)| name.to_ascii_lowercase()).collect()
}

/// Token `token` spelling a name of `names` (folded into `folded`) in `tokens`: a name two
/// tokens spell is spelled by none.
fn spells(tokens: &mut HashMap<String, Option<u32>>, names: &HashSet<String>, folded: &mut String, token: u32, spelling: &[u8]) {
    let Ok(text) = std::str::from_utf8(spelling) else { return };
    folded.clear();
    folded.push_str(text);
    folded.make_ascii_lowercase();
    if names.contains(folded.as_str()) {
        tokens
            .entry(folded.clone())
            .and_modify(|held| {
                if *held != Some(token) {
                    *held = None;
                }
            })
            .or_insert(Some(token));
    }
}

/// A clone's requirements as a need reads them.
fn clone_of(slot: u32, token: u32, requirements: Vec<(u32, i64)>) -> Override {
    let mut seen = HashSet::new();
    match requirements.iter().find(|(archetype, _)| *archetype == 0 || !seen.insert(*archetype)) {
        Some((0, _)) => Err(format!("local sub-unit {slot}:{token} names the null archetype")),
        Some((archetype, _)) => Err(format!("local sub-unit {slot}:{token} repeats archetype {archetype}")),
        None => Ok(requirements),
    }
}

impl Rules {
    /// The archetype names of `needs` spelled to tokens, from every token the hour holds; whether
    /// any name's token moved.
    pub fn spell(&mut self, hour: &Hour<'_>, needs: Result<&SubUnitNeeds, &str>) -> bool {
        let mut tokens = HashMap::new();
        if let Ok(needs) = needs {
            let names = names(needs);
            if !names.is_empty() {
                let mut folded = String::new();
                for (token, spelling) in inputs::spellings(hour) {
                    spells(&mut tokens, &names, &mut folded, token, spelling);
                }
            }
        }
        let moved = tokens != self.tokens;
        self.tokens = tokens;
        moved
    }

    /// The names `tokens`, tokens the hour added, spell besides; whether any name's token moved.
    pub fn spell_more(&mut self, hour: &Hour<'_>, needs: Result<&SubUnitNeeds, &str>, tokens: &[u32]) -> bool {
        let Ok(needs) = needs else { return false };
        let names = names(needs);
        let before = self.tokens.clone();
        let mut folded = String::new();
        for &token in tokens {
            if let Some(spelling) = inputs::spelling(hour, token) {
                spells(&mut self.tokens, &names, &mut folded, token, spelling);
            }
        }
        before != self.tokens
    }

    /// The differing clones the hour holds, by slot through `owners`, read whole.
    pub fn read_local(&mut self, hour: &Hour<'_>, owners: &Owners, complete: bool) -> std::io::Result<Moved> {
        self.local_complete = complete;
        let (local, error) = Self::whole(inputs::local_subunits(hour)?, owners);
        let moved = if error != self.local_error {
            Moved::All
        } else {
            let mut moved = HashSet::new();
            for (clone, held) in &self.local {
                if local.get(clone) != Some(held) {
                    moved.insert(*clone);
                }
            }
            moved.extend(local.keys().filter(|clone| !self.local.contains_key(*clone)));
            Moved::Clones(moved)
        };
        (self.local, self.local_error) = (local, error);
        Ok(moved)
    }

    fn whole(rows: Vec<LocalSubunit>, owners: &Owners) -> (HashMap<(u32, u32), (u32, Override)>, Option<String>) {
        let mut local = HashMap::new();
        for (country, token, requirements) in rows {
            let Some(slot) = owners.slot(country) else {
                return (HashMap::new(), Some(format!("local sub-unit country {country} has no link")));
            };
            if local.insert((slot, token), (country, clone_of(slot, token, requirements))).is_some() {
                return (HashMap::new(), Some(format!("duplicate local sub-unit {slot}:{token}")));
            }
        }
        (local, None)
    }

    /// The clones of the rows the hour deleted (`gone`, raw country and token) and wrote or
    /// inserted (`rows`, slots) moved; read whole when a row's country no link resolves, two rows
    /// of a slot name one clone, or none can be used already.
    pub fn update_local(&mut self, hour: &Hour<'_>, owners: &Owners, gone: &[(u32, u32)], rows: &[u32]) -> std::io::Result<Moved> {
        let whole = |rules: &mut Self| rules.read_local(hour, owners, rules.local_complete).map(|_| Moved::All);
        if self.local_error.is_some() {
            return whole(self);
        }
        let mut moved = HashSet::new();
        for &(country, token) in gone {
            let Some(slot) = owners.slot(country) else { return whole(self) };
            if self.local.get(&(slot, token)).is_some_and(|(held, _)| *held == country) {
                self.local.remove(&(slot, token));
                moved.insert((slot, token));
            }
        }
        for &at in rows {
            let (country, token, requirements) = inputs::local_subunit(hour, at)?;
            let Some(slot) = owners.slot(country) else { return whole(self) };
            if self.local.get(&(slot, token)).is_some_and(|(held, _)| *held != country) {
                return whole(self);
            }
            let clone = (country, clone_of(slot, token, requirements));
            if self.local.get(&(slot, token)) != Some(&clone) {
                self.local.insert((slot, token), clone);
                moved.insert((slot, token));
            }
        }
        Ok(Moved::Clones(moved))
    }

    /// Template `handle`'s need by archetype token, ascending; why it is not known. Each clone it
    /// looks up, (slot, token), is added to `clones`.
    pub fn need(&self, hour: &Hour<'_>, needs: Result<&SubUnitNeeds, &str>, owners: &Owners, handle: u64, facts: Option<&TemplateFacts>, clones: &mut Vec<(u32, u32)>) -> Result<Vec<(u32, i64)>, String> {
        let facts = facts.ok_or_else(|| format!("template {handle} equipment composition was not recorded"))?;
        let units = facts.units.as_ref().ok_or_else(|| format!("template {handle} grouped sub-units were not readable"))?;
        let needs = needs.map_err(|why| format!("sub-unit equipment needs are not held: {why}"))?;
        let mut out: Vec<(u32, i64)> = Vec::new();
        for unit in units.iter().filter(|unit| unit.token != 0) {
            let spelling = inputs::spelling(hour, unit.token)
                .and_then(|bytes| std::str::from_utf8(bytes).ok())
                .ok_or_else(|| format!("template {handle} sub-unit token {} is not spelled", unit.token))?;
            let requirements = if unit.local == 0 {
                self.global(needs, spelling)?
            } else {
                let slot = u32::try_from(facts.country)
                    .ok()
                    .and_then(|raw| owners.slot(raw))
                    .ok_or_else(|| format!("template {handle} country {} names no country for its local sub-units", facts.country))?;
                clones.push((slot, unit.token));
                if let Some(why) = &self.local_error {
                    return Err(why.clone());
                }
                match self.local.get(&(slot, unit.token)) {
                    Some((_, clone)) => clone.clone()?,
                    None if self.local_complete => self.global(needs, spelling)?,
                    None => return Err(format!("local sub-unit {slot}:{} requirements are unavailable in an incomplete census", unit.token)),
                }
            };
            for (archetype, amount) in requirements {
                let overflow = || format!("template {handle} archetype {archetype} equipment need overflows fixed5");
                let amount = amount.checked_mul(i64::from(unit.count)).ok_or_else(overflow)?;
                match out.iter_mut().find(|(held, _)| *held == archetype) {
                    Some((_, total)) => *total = total.checked_add(amount).ok_or_else(overflow)?,
                    None => out.push((archetype, amount)),
                }
            }
        }
        out.sort_unstable_by_key(|&(archetype, _)| archetype);
        Ok(out)
    }

    /// Global sub-unit `spelling`'s requirements by archetype token.
    fn global(&self, needs: &SubUnitNeeds, spelling: &str) -> Result<Vec<(u32, i64)>, String> {
        let requirements = needs.get(spelling).ok_or_else(|| format!("missing authored sub-unit {spelling}"))?;
        requirements
            .iter()
            .map(|(name, amount)| match self.tokens.get(&name.to_ascii_lowercase()) {
                Some(Some(token)) => Ok((*token, *amount)),
                Some(None) => Err(format!("archetype {name} is spelled by two lexer tokens")),
                None => Err(format!("archetype {name} has no lexer token in the recording")),
            })
            .collect()
    }
}
