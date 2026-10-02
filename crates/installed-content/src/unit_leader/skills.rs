use std::collections::BTreeMap;

use super::{Conditional, leader_category};
use crate::{
    crt::{self, Scanned},
    parser::{Item, Value},
};

/// Matching content's overall commander levels, not the separately authored
/// attack, defence, logistics or other component skill tables.
#[derive(Clone, Debug, Default)]
pub struct Skills {
    costs: BTreeMap<(i32, i32), i32>,
    unavailable: bool,
    /// Block values under modifier entries read before or inside a
    /// `leader_skills` block of the same file.
    pub(super) conditions: Vec<Conditional>,
    warnings: Vec<String>,
}

impl Skills {
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// What must be settled against the modifier registry before
    /// [`Self::next_required_experience`] holds: a registered one makes the
    /// levels read after it unknown.
    pub fn conditions(&self) -> &[Conditional] {
        &self.conditions
    }

    /// Outer `None`: unsupported content or category. `Some(None)`: native
    /// null definition (at cap or a sparse level). `Some(Some(cost))`: the
    /// signed whole-XP cost, including an authored zero or negative value.
    pub fn next_required_experience(&self, level: i32, category: i32) -> Option<Option<i32>> {
        if self.unavailable || !(0..4).contains(&category) {
            return None;
        }
        let next = level.wrapping_add(1);
        Some(self.costs.get(&(category, next)).copied())
    }

    pub(super) fn depend_on(&mut self, pending: &[Conditional]) {
        for condition in pending {
            if !self.conditions.contains(condition) {
                self.conditions.push(condition.clone());
            }
        }
    }

    /// One `leader_skills` block as 140ADE200 read it.
    pub(super) fn read(&mut self, levels: &[Item], path: &str) {
        for definition in levels {
            let name = String::from_utf8_lossy(definition.key.as_deref().unwrap_or_default());
            // A level the reader read (a positive ParseTokenInt32 key).
            let key = definition.key.as_deref().unwrap_or_default();
            // The level key reads through ParseTokenInt32 (1424B0080); the
            // constructor reports and skips a level that is not positive
            // (1424AD100), which defines nothing.
            let level = crate::fixed5::decimal_i32(key);
            if level <= 0 {
                self.note(format!(
                    "{path}: leader_skills key {name} reads level {level}, not a positive level; the game reports it and skips its block"
                ));
                continue;
            }
            let Value::Block(fields) = &definition.value else {
                continue;
            };
            let place = format!("leader_skills level {name}");
            // CUnitLeaderSkill's constructor: cost 0, and category 4, which
            // 140ADE200 registers in no category shard.
            let mut cost = 0;
            let mut category = 4;
            for field in fields {
                let value = field.value.scalar().unwrap_or_default();
                match field.keyword().map(|keyword| keyword.folded()).as_deref() {
                    // ReadScriptSignedInt32 (1424AB970 -> 1424B02C0).
                    Some(b"cost") => match crt::scan_i32(value) {
                        Scanned::Stored(read) => cost = read,
                        // sscanf's EOF counts as success: nothing is
                        // stored and nothing reported.
                        Scanned::End => {}
                        Scanned::NoMatch => self.note(format!(
                            "{path}: {place} cost {} does not scan as an integer; the game reports it (1424ACEE0) and keeps {cost}",
                            String::from_utf8_lossy(value)
                        )),
                    },
                    // 140ADC960 dispatches on the value token's id: a quoted
                    // token, or a variable defined by one (1424AC830 keeps
                    // its token), is no keyword; like any other value that
                    // names no category, a block included, it is reported
                    // (1424ACF90, whose skip the reading followed) and the
                    // category stays.
                    Some(b"type") => {
                        let named = leader_category(field.value_keyword().as_deref().map(str::as_bytes));
                        match named {
                            Some(value) => category = value,
                            None => self.note(format!(
                                "{path}: {place} type {}{} is not a leader-category keyword; the game reports it (1424ACF90) and the level keeps its category",
                                String::from_utf8_lossy(value),
                                if field.quoted() { " (quoted)" } else { "" }
                            )),
                        }
                    }
                    // Modifiers affect other displays, not this lookup.
                    _ => {}
                }
            }
            if category != 4 {
                self.costs.insert((category, level), cost);
            }
        }
    }

    /// Something the game reports and goes on from, which the lookup
    /// follows.
    fn note(&mut self, what: String) {
        let warning = format!("Commander skill levels: {what}.");
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    pub(super) fn refuse(&mut self, reason: String) {
        self.unavailable = true;
        let warning = format!("Commander skill progress unavailable: {reason}");
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }
}
