use std::collections::BTreeMap;

use super::{Conditional, leader_category};
use crate::{
    crt::{self, Scanned},
    modifiers::Modifiers,
    parser::{Item, Value},
};

/// Component tables dispatched by CUnitLeaderDatabase::build_trait_shards
/// (140AD6030), each read by CUnitLeaderSkill::construct (140ADE200).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Component {
    Attack,
    Defense,
    Planning,
    Logistics,
    Maneuvering,
    Coordination,
}

impl Component {
    pub(super) fn from_key(key: &[u8]) -> Option<Self> {
        match key {
            b"leader_attack_skills" => Some(Self::Attack),
            b"leader_defense_skills" => Some(Self::Defense),
            b"leader_planning_skills" => Some(Self::Planning),
            b"leader_logistics_skills" => Some(Self::Logistics),
            b"leader_maneuvering_skills" => Some(Self::Maneuvering),
            b"leader_coordination_skills" => Some(Self::Coordination),
            _ => None,
        }
    }

    fn key(self) -> &'static str {
        match self {
            Self::Attack => "leader_attack_skills",
            Self::Defense => "leader_defense_skills",
            Self::Planning => "leader_planning_skills",
            Self::Logistics => "leader_logistics_skills",
            Self::Maneuvering => "leader_maneuvering_skills",
            Self::Coordination => "leader_coordination_skills",
        }
    }
}

/// Matching content's overall commander costs and component modifiers.
#[derive(Clone, Debug, Default)]
pub struct Skills {
    costs: BTreeMap<(i32, i32), i32>,
    components: BTreeMap<(Component, i32, i32), Modifiers>,
    unavailable: bool,
    /// Block values under modifier entries read before or inside a
    /// `leader_skills` block of the same file.
    pub(super) conditions: Vec<Conditional>,
    pub(super) component_conditions: Vec<Conditional>,
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

    pub fn component_conditions(&self) -> &[Conditional] {
        &self.component_conditions
    }

    /// The selected definition's modifier (140ADC960, token 0x2965).
    /// Missing and negative levels use the null definition (140AD3D90).
    pub fn component_modifiers(
        &self,
        component: Component,
        level: i32,
        category: i32,
    ) -> Result<Option<&Modifiers>, &'static str> {
        // 140ADE200 registers definitions only in category shards 0, 1, 2 and 3.
        if self.unavailable || !(0..4).contains(&category) {
            return Err("the component skill table or category is unavailable");
        }
        Ok(self.components.get(&(component, category, level)))
    }

    /// Outer `None`: unsupported content or category. `Some(None)`: native
    /// null definition (at cap or a sparse level). `Some(Some(cost))`: the
    /// signed whole-XP cost, including an authored zero or negative value.
    pub fn next_required_experience(&self, level: i32, category: i32) -> Option<Option<i32>> {
        // The same four category shards constructed by 140ADE200.
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

    /// One skill table as 140ADE200 read it.
    pub(super) fn read(&mut self, levels: &[Item], path: &str, component: Option<Component>) {
        let table = component.map_or("leader_skills", Component::key);
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
                    "{path}: {table} key {name} reads level {level}, not a positive level; the game reports it and skips its block"
                ));
                continue;
            }
            let Value::Block(fields) = &definition.value else {
                continue;
            };
            let place = format!("{table} level {name}");
            // CUnitLeaderSkill's constructor: cost 0, and category 4, which
            // 140ADE200 registers in no category shard.
            let mut cost = 0;
            let mut category = 4;
            let mut modifiers = Modifiers::default();
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
                        let named =
                            leader_category(field.value_keyword().as_deref().map(str::as_bytes));
                        match named {
                            Some(value) => category = value,
                            None => self.note(format!(
                                "{path}: {place} type {}{} is not a leader-category keyword; the game reports it (1424ACF90) and the level keeps its category",
                                String::from_utf8_lossy(value),
                                if field.quoted() { " (quoted)" } else { "" }
                            )),
                        }
                    }
                    Some(b"modifier") if component.is_some() => {
                        modifiers.read_all(field.value.items().unwrap_or_default());
                    }
                    // building_module_modifier is a separate CModifier
                    // (+0xD0); 140AD9010 only describes +0x20.
                    _ => {}
                }
            }
            if category != 4 {
                if let Some(component) = component {
                    self.components.insert((component, category, level), modifiers);
                } else {
                    self.costs.insert((category, level), cost);
                }
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
