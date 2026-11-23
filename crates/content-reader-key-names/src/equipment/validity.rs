use super::*;
use content_reader_parser::parser::Operator;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CountKey {
    Category(Vec<u8>),
    Module(Vec<u8>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleCountLimit {
    pub key: CountKey,
    pub operator: Operator,
    pub count: i32,
}
impl ModuleCountLimit {
    pub(super) fn read(fields: &[Item]) -> Option<Self> {
        let mut category = None;
        let mut module = None;
        let mut count = None;
        let mut operator = Operator::Equals;
        for field in fields {
            let text = c_string(field.value.scalar().unwrap_or_default()).to_ascii_lowercase();
            if field.keyword_is("category") {
                category = Some(text);
            } else if field.keyword_is("module") {
                module = Some(text);
            } else if field.keyword_is("count") {
                count = Some(content_reader_parser::fixed5::decimal_i32(&text));
                operator = field.operator;
            }
        }
        let module = module.unwrap_or_default();
        let key = if module.is_empty() {
            CountKey::Category(category?)
        } else if module == b"any" || module == b"empty" {
            CountKey::Category(module)
        } else {
            CountKey::Module(module)
        };
        Some(Self {
            key,
            operator,
            count: count?,
        })
    }

    fn admits(&self, actual: i32) -> bool {
        match self.operator {
            Operator::Equals => actual > self.count,
            Operator::Less => actual < self.count,
            Operator::Greater => actual == self.count,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleValidity {
    pub failures: Vec<String>,
}

pub struct Assignment<'a> {
    pub slot: &'a [u8],
    pub module: &'a [u8],
    pub active: bool,
}
type Node = (Token, Token);
type Edge = (Node, Token);

impl Database {
    fn validation_module(&self, name: &[u8]) -> Option<&ModuleDesign> {
        static EMPTY: std::sync::LazyLock<ModuleDesign> =
            std::sync::LazyLock::new(ModuleDesign::default);
        if Token::c_str(name).is_rejected() {
            Some(&EMPTY)
        } else {
            self.module_design(name)
        }
    }

    pub fn permitted_role_mask(
        &self,
        equipment: &Equipment,
        assignments: &[Assignment<'_>],
    ) -> Result<u64, String> {
        if let Some(reason) = &self.lost {
            return Err(reason.clone());
        }
        if equipment.module_slots.is_empty() {
            return Ok(self.category(equipment));
        }
        let modules = assignments
            .iter()
            .map(|row| {
                self.validation_module(row.module).ok_or_else(|| {
                    format!(
                        "module {} role content is absent (count: 1)",
                        String::from_utf8_lossy(row.module)
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut mask = self.category(&self.equipment[equipment.family]);
        mask |= modules
            .iter()
            .fold(0, |mask, module| mask | module.allow_types);
        let allowed = equipment
            .archetype
            .map_or(0, |index| self.equipment[index].allowed_types);
        let mut blocked = if allowed == 0 { 0 } else { !allowed };
        let override_slot = equipment
            .module_slots
            .iter()
            .find(|slot| Token::c_str(slot) == Token::c_str(&equipment.type_override));
        if let Some(slot) = override_slot {
            if let Some((row, module)) = assignments
                .iter()
                .zip(&modules)
                .find(|(row, _)| Token::c_str(row.slot) == Token::c_str(slot))
            {
                let added = self.module_category(row.module).unwrap_or(0);
                mask |= added;
                blocked |= !added;
                blocked |= module.forbid_types;
            }
        } else {
            for (row, module) in assignments.iter().zip(&modules) {
                mask |= self.module_category(row.module).unwrap_or(0);
                blocked |= module.forbid_types;
            }
        }
        Ok(normalize_category(mask & !blocked))
    }

    pub fn role_permitted(
        &self,
        equipment: &Equipment,
        assignments: &[Assignment<'_>],
        role: u64,
    ) -> Result<bool, String> {
        let mask = self.permitted_role_mask(equipment, assignments)?;
        if role & mask != role {
            return Ok(false);
        }
        if role == 0 {
            return Ok(true);
        }
        let modules = assignments
            .iter()
            .map(|row| {
                self.validation_module(row.module).ok_or_else(|| {
                    format!(
                        "module {} role content is absent (count: 1)",
                        String::from_utf8_lossy(row.module)
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        for module in &modules {
            if module.forbid_exact_types == role {
                return Ok(false);
            }
            for source in &modules {
                if source
                    .forbid_exact_for_category
                    .iter()
                    .any(|(category, mask)| {
                        Token::c_str(category) == Token::c_str(&module.category) && *mask == role
                    })
                {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }
    fn index_of(&self, equipment: &Equipment) -> Result<usize, String> {
        self.equipment
            .iter()
            .position(|held| std::ptr::eq(held, equipment))
            .ok_or_else(|| {
                format!(
                    "equipment {} is not a type of this equipment database",
                    String::from_utf8_lossy(&equipment.name)
                )
            })
    }

    fn chain(&self, at: usize) -> Result<Vec<usize>, String> {
        let mut chain = vec![at];
        while let Some(next) = self.next(chain[chain.len() - 1]) {
            if chain.contains(&next) {
                return Err(format!(
                    "equipment {} links back through its parents and archetypes",
                    String::from_utf8_lossy(&self.equipment[at].name)
                ));
            }
            chain.push(next);
        }
        Ok(chain)
    }

    fn default_module(&self, chain: &[usize], slot: &[u8]) -> Option<&[u8]> {
        chain
            .iter()
            .find_map(|&at| {
                self.equipment[at]
                    .default_modules
                    .iter()
                    .find(|(key, _)| Token::c_str(key) == Token::c_str(slot))
            })
            .map(|(_, module)| module.as_slice())
            .filter(|module| !module.eq_ignore_ascii_case(b"empty"))
    }

    pub fn default_modules<'a>(&'a self, equipment: &'a Equipment) -> Result<Vec<(&'a [u8], &'a [u8])>, String> {
        let chain = self.chain(self.index_of(equipment)?)?;
        Ok(equipment.module_slots.iter().filter_map(|slot| Some((slot.as_slice(), self.default_module(&chain, slot)?))).collect())
    }

    pub fn lineage(&self, equipment: &Equipment) -> Result<Vec<&Equipment>, String> {
        Ok(self.chain(self.index_of(equipment)?)?.into_iter().map(|at| &self.equipment[at]).collect())
    }
    fn dependency_graph(&self, equipment: &Equipment) -> BTreeMap<Node, Vec<Edge>> {
        let mut categories: Vec<BTreeSet<Token>> = equipment
            .module_slot_rules
            .iter()
            .map(|slot| {
                slot.categories
                    .iter()
                    .map(|name| Token::c_str(name))
                    .collect()
            })
            .collect();
        loop {
            let mut changed = false;
            for (source, _) in equipment.module_slots.iter().enumerate() {
                for module in self.module_designs.values() {
                    if !categories[source].contains(&Token::c_str(&module.category)) {
                        continue;
                    }
                    for (target, category) in &module.allowed_module_categories {
                        if let Some(index) = equipment
                            .module_slots
                            .iter()
                            .position(|slot| Token::c_str(slot) == Token::c_str(target))
                            && index != source
                        {
                            changed |= categories[index].insert(Token::c_str(category));
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut graph = BTreeMap::<Node, Vec<Edge>>::new();
        for (source, slot) in equipment.module_slots.iter().enumerate() {
            for (name, module) in &self.module_designs {
                if !categories[source].contains(&Token::c_str(&module.category)) {
                    continue;
                }
                for (target, category) in &module.allowed_module_categories {
                    let Some(index) = equipment
                        .module_slots
                        .iter()
                        .position(|slot| Token::c_str(slot) == Token::c_str(target))
                    else {
                        continue;
                    };
                    if index == source
                        || equipment.module_slot_rules[index]
                            .categories
                            .iter()
                            .any(|held| Token::c_str(held) == Token::c_str(category))
                    {
                        continue;
                    }
                    let provider = (Token::c_str(slot), Token::c_str(&module.category));
                    graph.entry(provider.clone()).or_default();
                    let edge = (provider, Token::c_str(name));
                    let edges = graph
                        .entry((Token::c_str(target), Token::c_str(category)))
                        .or_default();
                    if !edges.contains(&edge) {
                        edges.push(edge);
                    }
                }
            }
        }
        graph
    }

    pub fn validate_modules(
        &self,
        equipment: &Equipment,
        assignments: &[Assignment<'_>],
    ) -> Result<ModuleValidity, String> {
        if let Some(reason) = &self.lost {
            return Err(reason.clone());
        }
        let chain = self.chain(self.index_of(equipment)?)?;
        let graph = self.dependency_graph(equipment);
        let explicit = |slot: &[u8]| {
            assignments
                .iter()
                .find(|row| Token::c_str(row.slot) == Token::c_str(slot))
        };
        let mut result = ModuleValidity::default();
        for (slot, rule) in equipment
            .module_slots
            .iter()
            .zip(&equipment.module_slot_rules)
        {
            let row = explicit(slot);
            let name = row
                .map(|row| row.module)
                .filter(|name| !name.is_empty())
                .or_else(|| self.default_module(&chain, slot));
            let active = row.map_or(name.is_some(), |row| row.active);
            if !active || name.is_none() {
                if rule.required {
                    result.failures.push(format!(
                        "required slot {} is empty",
                        String::from_utf8_lossy(slot)
                    ));
                }
                continue;
            }
            let name = name.unwrap();
            let module = self.validation_module(name).ok_or_else(|| {
                format!(
                    "module {} content is absent (count: 1)",
                    String::from_utf8_lossy(name)
                )
            })?;
            if assignments
                .iter()
                .filter_map(|row| self.validation_module(row.module))
                .any(|held| {
                    held.forbidden_categories
                        .iter()
                        .any(|category| Token::c_str(category) == Token::c_str(&module.category))
                })
            {
                result.failures.push(format!(
                    "slot {} category {} is forbidden by an assigned module",
                    String::from_utf8_lossy(slot),
                    String::from_utf8_lossy(&module.category)
                ));
                continue;
            }
            let node = (Token::c_str(slot), Token::c_str(&module.category));
            let admitted = graph.get(&node).map_or_else(
                || {
                    rule.categories
                        .iter()
                        .any(|category| Token::c_str(category) == node.1)
                },
                |edges| {
                    edges.is_empty()
                        || edges.iter().any(|((provider, _), module)| {
                            assignments.iter().any(|row| {
                                Token::c_str(row.slot) == *provider
                                    && Token::c_str(row.module) == *module
                            })
                        })
                },
            );
            if !admitted {
                result.failures.push(format!(
                    "slot {} category {} has no installed dependency",
                    String::from_utf8_lossy(slot),
                    String::from_utf8_lossy(&module.category)
                ));
            }
        }
        let mut limits: Vec<&ModuleCountLimit> = equipment.module_count_limits.iter().collect();
        for &index in &chain[1..] {
            for limit in &self.equipment[index].module_count_limits {
                if !limits.iter().any(|held| held.key == limit.key) {
                    limits.push(limit);
                }
            }
        }
        if limits.is_empty() {
            return Ok(result);
        }
        let mut categories = BTreeMap::<Token, i32>::new();
        let mut modules = BTreeMap::<Token, i32>::new();
        for row in assignments {
            let module = self.validation_module(row.module).ok_or_else(|| {
                format!(
                    "module {} count content is absent (count: 1)",
                    String::from_utf8_lossy(row.module)
                )
            })?;
            let category = categories
                .entry(Token::c_str(&module.category))
                .or_default();
            *category = category.wrapping_add(1);
            let count = modules.entry(Token::c_str(row.module)).or_default();
            *count = count.wrapping_add(1);
            let any = modules.entry(Token::c_str(b"any")).or_default();
            *any = any.wrapping_add(1);
        }
        for limit in limits {
            let (name, actual) = match &limit.key {
                CountKey::Category(name) if name == b"any" || name == b"empty" => {
                    (name, modules.get(&Token::c_str(name)))
                }
                CountKey::Category(name) => (name, categories.get(&Token::c_str(name))),
                CountKey::Module(name) => (name, modules.get(&Token::c_str(name))),
            };
            if let Some(actual) = actual
                && !limit.admits(*actual)
            {
                result.failures.push(format!(
                    "module count {} {} {} failed (actual {})",
                    String::from_utf8_lossy(name),
                    String::from_utf8_lossy(limit.operator.text()),
                    limit.count,
                    actual
                ));
            }
        }
        Ok(result)
    }
}
