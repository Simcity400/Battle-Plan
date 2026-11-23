use std::collections::BTreeMap;

use super::{Conditional, leader_category};
use content_reader_parser::{crt::{self, Scanned}, parser::{Item, Value}};
use content_reader_script::modifiers::Modifiers;

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

#[derive(Clone, Debug, Default)]
pub struct Skills {
    costs: BTreeMap<(i32, i32), i32>,
    components: BTreeMap<(Component, i32, i32), Modifiers>,
    unavailable: bool,
    pub(super) conditions: Vec<Conditional>,
    pub(super) component_conditions: Vec<Conditional>,
    warnings: Vec<String>,
}

impl Skills {
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn conditions(&self) -> &[Conditional] {
        &self.conditions
    }

    pub fn component_conditions(&self) -> &[Conditional] {
        &self.component_conditions
    }

    pub fn component_modifiers(
        &self,
        component: Component,
        level: i32,
        category: i32,
    ) -> Result<Option<&Modifiers>, &'static str> {
        if self.unavailable || !(0..4).contains(&category) {
            return Err("the component skill table or category is unavailable");
        }
        Ok(self.components.get(&(component, category, level)))
    }

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

    pub(super) fn read(&mut self, levels: &[Item], path: &str, component: Option<Component>) {
        let table = component.map_or("leader_skills", Component::key);
        for definition in levels {
            let name = String::from_utf8_lossy(definition.key.as_deref().unwrap_or_default());
            let key = definition.key.as_deref().unwrap_or_default();
            let level = content_reader_parser::fixed5::decimal_i32(key);
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
            let mut cost = 0;
            let mut category = 4;
            let mut modifiers = Modifiers::default();
            for field in fields {
                let value = field.value.scalar().unwrap_or_default();
                match field.keyword().map(|keyword| keyword.folded()).as_deref() {
                    Some(b"cost") => match crt::scan_i32(value) {
                        Scanned::Stored(read) => cost = read,
                        Scanned::End => {}
                        Scanned::NoMatch => self.note(format!(
                            "{path}: {place} cost {} does not scan as an integer; the game reports it and keeps {cost}",
                            String::from_utf8_lossy(value)
                        )),
                    },
                    Some(b"type") => {
                        let named =
                            leader_category(field.value_keyword().as_deref().map(str::as_bytes));
                        match named {
                            Some(value) => category = value,
                            None => self.note(format!(
                                "{path}: {place} type {}{} is not a leader-category keyword; the game reports it and the level keeps its category",
                                String::from_utf8_lossy(value),
                                if field.quoted() { " (quoted)" } else { "" }
                            )),
                        }
                    }
                    Some(b"modifier") if component.is_some() => {
                        modifiers.read_all(field.value.items().unwrap_or_default());
                    }
                    _ => {}
                }
            }
            if category != 4 {
                if let Some(component) = component {
                    self.components
                        .insert((component, category, level), modifiers);
                } else {
                    self.costs.insert((category, level), cost);
                }
            }
        }
    }

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
