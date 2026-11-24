use super::{STAT_COUNT, stat_index};
use content_reader_parser::{fixed5::script_fixed5, parser::Item};

use content_reader_script::equipment_stats::{RELIABILITY_STAT, CARRIER_SIZE_STAT};

pub type Amount = Result<i64, String>;
pub type ResourcePool = Vec<(Vec<u8>, Amount)>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UpgradeCurrency {
    #[default]
    Land,
    Naval,
    Air,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpgradeDesign {
    pub currency: UpgradeCurrency,
    pub linear: Option<[Amount; 2]>,
    pub resource_thresholds: Vec<(u8, ResourcePool)>,
}
impl UpgradeDesign {
    pub(super) fn read(fields: &[Item]) -> Self {
        let mut result = Self::default();
        for field in fields {
            if field.keyword_is("cost") {
                match content_reader_parser::parser::c_string(field.value.scalar().unwrap_or_default()) {
                    value if value.eq_ignore_ascii_case(b"land") => {
                        result.currency = UpgradeCurrency::Land
                    }
                    value if value.eq_ignore_ascii_case(b"naval") => {
                        result.currency = UpgradeCurrency::Naval
                    }
                    value if value.eq_ignore_ascii_case(b"air") => {
                        result.currency = UpgradeCurrency::Air
                    }
                    _ => {}
                }
            } else if field.keyword_is("resource_cost_thresholds") {
                result.resource_thresholds.clear();
                for level in field.value.items().unwrap_or_default() {
                    let key = level.key.as_deref().unwrap_or_default();
                    let number = content_reader_parser::fixed5::decimal_i32(key);
                    if !content_reader_parser::parser::number_token(key, false) || !(1..=255).contains(&number) {
                        continue;
                    }
                    let mut pool = Vec::new();
                    read_resources(&mut pool, level.value.items().unwrap_or_default());
                    match result
                        .resource_thresholds
                        .binary_search_by_key(&(number as u8), |(level, _)| *level)
                    {
                        Ok(index) => result.resource_thresholds[index].1 = pool,
                        Err(index) => result
                            .resource_thresholds
                            .insert(index, (number as u8, pool)),
                    }
                }
            } else if field.keyword_is("linear_cost") {
                let costs = result.linear.get_or_insert([Ok(0), Ok(0)]);
                for entry in field.value.items().unwrap_or_default() {
                    if entry.keyword_is("cost_by_level") {
                        costs[0] = number(entry);
                    }
                    if entry.keyword_is("cost_by_level_for_licensed_equipment") {
                        costs[1] = number(entry);
                    }
                }
            }
        }
        result
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpgradeXpRules {
    pub base: [i32; 3],
    pub ramp: [i32; 3],
    pub licensed_factor: i64,
}
impl UpgradeXpRules {
    pub fn load(defines: &content_reader_defines::defines::Defines) -> std::io::Result<Self> {
        Ok(Self {
            base: [
                defines.int32("NMilitary", "LAND_EQUIPMENT_BASE_COST")?,
                defines.int32("NMilitary", "NAVAL_EQUIPMENT_BASE_COST")?,
                defines.int32("NMilitary", "AIR_EQUIPMENT_BASE_COST")?,
            ],
            ramp: [
                defines.int32("NMilitary", "LAND_EQUIPMENT_RAMP_COST")?,
                defines.int32("NMilitary", "NAVAL_EQUIPMENT_RAMP_COST")?,
                defines.int32("NMilitary", "AIR_EQUIPMENT_RAMP_COST")?,
            ],
            licensed_factor: defines
                .fixed5("NProduction", "LICENSE_EQUIPMENT_UPGRADE_XP_FACTOR")?,
        })
    }
    pub fn change(
        &self,
        definition: &UpgradeDesign,
        source: u8,
        target: u8,
        licensed: bool,
    ) -> Amount {
        if let Some(linear) = &definition.linear {
            return linear[usize::from(licensed)].clone().map(|cost| i64::from(source.abs_diff(target)).wrapping_mul(cost));
        }
        let currency = match definition.currency {
            UpgradeCurrency::Land => 0,
            UpgradeCurrency::Naval => 1,
            UpgradeCurrency::Air => 2,
        };
        let mut level = i32::from(source);
        let target = i32::from(target);
        let step = if level < target { 1 } else { -1 };
        let mut cost = 0_i64;
        while level != target {
            let value = level
                .wrapping_mul(self.ramp[currency])
                .wrapping_add(self.base[currency]);
            cost = cost.wrapping_add(i64::from(value).wrapping_mul(100_000));
            level += step;
        }
        Ok(cost)
    }
    pub fn finish(
        &self,
        raw: i64,
        common: i64,
        family: i64,
        archetype: i64,
        licensed: bool,
    ) -> i64 {
        let mut factor = common
            .wrapping_add(100_000)
            .wrapping_add(family)
            .wrapping_add(archetype);
        if licensed {
            factor = factor.wrapping_mul(self.licensed_factor) / 100_000;
        }
        raw.wrapping_mul(factor.max(0)) / 100_000
    }
}

#[allow(clippy::too_many_arguments)]
pub fn module_save_xp(
    upgrade_xp: i64,
    module_xp: i64,
    licensed: bool,
    licensed_factor: i64,
    common: i64,
    family: i64,
    archetype: i64,
    template_discount: i64,
) -> i64 {
    let slots = if licensed {
        module_xp.wrapping_mul(licensed_factor) / 100_000
    } else {
        module_xp
    }
    .max(0);
    let mut cost = upgrade_xp.wrapping_add(slots);
    let factor = common
        .wrapping_add(100_000)
        .wrapping_add(family)
        .wrapping_add(archetype);
    if factor != 100_000 && cost > 0 {
        cost = (cost.wrapping_mul(factor.max(0)) / 100_000).max(100_000);
    }
    cost.wrapping_mul(100_000 - template_discount.clamp(0, 100_000)) / 100_000
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleDesign {
    pub category: Vec<u8>,
    pub gui_category: Vec<u8>,
    pub gfx: Vec<u8>,
    pub xp_cost: Option<Amount>,
    pub dismantle_cost_ic: Option<Amount>,
    pub build_resources: ResourcePool,
    pub dismantle_resources: ResourcePool,
    pub conversions: Vec<ModuleConversion>,
    pub allow_types: u64,
    pub forbid_types: u64,
    pub forbid_exact_types: u64,
    pub forbid_exact_for_category: Vec<(Vec<u8>, u64)>,
    pub allowed_module_categories: Vec<(Vec<u8>, Vec<u8>)>,
    pub forbidden_categories: Vec<Vec<u8>>,
    pub mission_terms: Vec<(Vec<Vec<u8>>, StatGroup)>,
    pub manpower: Option<Amount>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleConversion {
    pub module: Vec<u8>,
    pub category: Vec<u8>,
    pub cost_ic: Amount,
    pub resources: ResourcePool,
}

fn token(field: &Item) -> Vec<u8> {
    content_reader_parser::parser::c_string(field.value.scalar().unwrap_or_default()).to_ascii_lowercase()
}
pub(super) fn number(field: &Item) -> Amount {
    let scalar = field.value.scalar();
    scalar.and_then(script_fixed5).ok_or_else(|| {
        let key = String::from_utf8_lossy(content_reader_parser::parser::c_string(field.key.as_deref().unwrap_or_default()));
        match scalar {
            Some(text) => format!("{key} = {} is not a number", String::from_utf8_lossy(content_reader_parser::parser::c_string(text))),
            None => format!("{key} is not a number"),
        }
    })
}

pub(super) fn read_resources(pool: &mut ResourcePool, fields: &[Item]) {
    for field in fields {
        let Some(name) = field.key.as_deref() else {
            continue;
        };
        let name = content_reader_parser::parser::c_string(name).to_ascii_lowercase();
        let value = number(field);
        if let Some(entry) = pool.iter_mut().find(|(held, _)| held == &name) {
            entry.1 = value;
        } else {
            pool.push((name, value));
        }
    }
}

impl ModuleDesign {
    pub(super) fn read(fields: &[Item]) -> Self {
        let mut result = Self::default();
        for field in fields {
            if field.keyword_is("allow_equipment_type") {
                result.allow_types = super::category_list(&field.value, field.quoted());
            } else if field.keyword_is("forbid_equipment_type") {
                result.forbid_types = super::category_list(&field.value, field.quoted());
            } else if field.keyword_is("forbid_equipment_type_exact_match") {
                result.forbid_exact_types = super::category_list(&field.value, field.quoted());
            } else if field.keyword_is("forbid_equipment_type_exact_match_for_category") {
                for entry in field.value.items().unwrap_or_default() {
                    let category =
                        content_reader_parser::parser::c_string(entry.key.as_deref().unwrap_or_default())
                            .to_ascii_lowercase();
                    let mask = super::category_list(&entry.value, entry.quoted());
                    if let Some(previous) = result
                        .forbid_exact_for_category
                        .iter_mut()
                        .find(|(key, _)| key == &category)
                    {
                        previous.1 = mask;
                    } else {
                        result.forbid_exact_for_category.push((category, mask));
                    }
                }
            } else if field.keyword_is("allowed_module_categories") {
                for slot in field.value.items().unwrap_or_default() {
                    let name = content_reader_parser::parser::c_string(slot.key.as_deref().unwrap_or_default())
                        .to_ascii_lowercase();
                    for category in slot.value.items().unwrap_or_default() {
                        let pair = (name.clone(), token(category));
                        if !result.allowed_module_categories.contains(&pair) {
                            result.allowed_module_categories.push(pair);
                        }
                    }
                }
            } else if field.keyword_is("forbid_module_categories") {
                result.forbidden_categories = field
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .map(token)
                    .collect();
            } else if field.keyword_is("mission_type_stats") {
                let fields = field.value.items().unwrap_or_default();
                let mut missions = Vec::new();
                for limit in fields.iter().filter(|field| field.keyword_is("limit")) {
                    match limit.value.items() {
                        Some(items) => missions.extend(items.iter().map(token)),
                        None => missions.push(token(limit)),
                    }
                }
                result
                    .mission_terms
                    .push((missions, StatGroup::read(fields, false)));
            } else if field.keyword_is("manpower") {
                result.manpower = Some(number(field));
            } else if field.keyword_is("category") {
                result.category = token(field);
            } else if field.keyword_is("gui_category") {
                result.gui_category = token(field);
            } else if field.keyword_is("gfx") {
                if let Some(value) = field.value.scalar() {
                    result.gfx = content_reader_parser::parser::c_string(value).to_vec();
                }
            } else if field.keyword_is("xp_cost") {
                result.xp_cost = Some(number(field));
            } else if field.keyword_is("dismantle_cost_ic") {
                result.dismantle_cost_ic = Some(number(field));
            } else if field.keyword_is("build_cost_resources") {
                read_resources(
                    &mut result.build_resources,
                    field.value.items().unwrap_or_default(),
                );
            } else if field.keyword_is("dismantle_cost_resources") {
                read_resources(
                    &mut result.dismantle_resources,
                    field.value.items().unwrap_or_default(),
                );
            } else if field.keyword_is("can_convert_from") {
                let mut conversion = ModuleConversion { module: Vec::new(), category: Vec::new(), cost_ic: Ok(0), resources: Vec::new() };
                for value in field.value.items().unwrap_or_default() {
                    if value.keyword_is("module") {
                        conversion.module = token(value);
                    } else if value.keyword_is("module_category") {
                        conversion.category = token(value);
                    } else if value.keyword_is("convert_cost_ic") {
                        conversion.cost_ic = number(value);
                    } else if value.keyword_is("convert_cost_resources") {
                        read_resources(
                            &mut conversion.resources,
                            value.value.items().unwrap_or_default(),
                        );
                    }
                }
                result.conversions.push(conversion);
            }
        }
        result
    }

    pub fn conversion_from(
        &self,
        source_name: &[u8],
        source: &ModuleDesign,
    ) -> Option<&ModuleConversion> {
        let mut category = None;
        for conversion in &self.conversions {
            if !conversion.module.is_empty() && conversion.module.eq_ignore_ascii_case(source_name)
            {
                return Some(conversion);
            }
            if conversion.category == source.category {
                category = Some(conversion);
            }
        }
        category
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleXpRules {
    pub add: i64,
    pub replace: i64,
    pub convert: i64,
    pub remove: i64,
}
impl ModuleXpRules {
    pub fn load(defines: &content_reader_defines::defines::Defines) -> std::io::Result<Self> {
        let value = |key| defines.fixed5("NProduction", key);
        Ok(Self {
            add: value("EQUIPMENT_MODULE_ADD_XP_COST")?,
            replace: value("EQUIPMENT_MODULE_REPLACE_XP_COST")?,
            convert: value("EQUIPMENT_MODULE_CONVERT_XP_COST")?,
            remove: value("EQUIPMENT_MODULE_REMOVE_XP_COST")?,
        })
    }

    pub fn slot(
        &self,
        target: Option<(&[u8], &ModuleDesign)>,
        source: Option<(&[u8], &ModuleDesign)>,
        source_active: bool,
        module_modifier: i64,
    ) -> Amount {
        let (module, default) = match target {
            None => {
                let Some((_, module)) = source else {
                    return Ok(0);
                };
                (module, self.remove)
            }
            Some((target_name, module)) => {
                let default = if let Some((source_name, source)) = source.filter(|_| source_active)
                {
                    if target_name.eq_ignore_ascii_case(source_name) {
                        return Ok(0);
                    }
                    if module.conversion_from(source_name, source).is_some() {
                        self.convert
                    } else {
                        self.replace
                    }
                } else {
                    self.add
                };
                (module, default)
            }
        };
        let cost = module.xp_cost.clone().unwrap_or(Ok(default))?;
        Ok(module_modifier.wrapping_add(100_000).wrapping_mul(cost) / 100_000)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatValue {
    pub index: usize,
    pub value: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatGroup {
    pub add: Vec<StatValue>,
    pub average: Vec<StatValue>,
    pub multiply: Vec<StatValue>,
    pub unread: Vec<(usize, String)>,
}
impl StatGroup {
    pub(super) fn read(fields: &[Item], direct: bool) -> Self {
        let mut result = Self::default();
        for field in fields {
            let target = if field.keyword_is("add_stats") {
                Some(&mut result.add)
            } else if field.keyword_is("add_average_stats") {
                Some(&mut result.average)
            } else if field.keyword_is("multiply_stats") {
                Some(&mut result.multiply)
            } else {
                None
            };
            let unread = if let Some(target) = target {
                Self::read_entries(target, field.value.items().unwrap_or_default())
            } else if direct {
                Self::read_entries(&mut result.multiply, std::slice::from_ref(field))
            } else {
                Vec::new()
            };
            result.unread.extend(unread);
        }
        result
    }
    pub(super) fn read_entries(target: &mut Vec<StatValue>, fields: &[Item]) -> Vec<(usize, String)> {
        let mut unread = Vec::new();
        for field in fields {
            let Some(index) = field.keyword().and_then(|key| stat_index(&key.folded())) else {
                continue;
            };
            match number(field) {
                Ok(value) => match target.iter_mut().find(|held| held.index == index) {
                    Some(held) => held.value = held.value.wrapping_add(value),
                    None => target.push(StatValue { index, value }),
                },
                Err(why) => unread.push((index, why)),
            }
        }
        unread
    }
}

fn stat_term(values: &[StatValue], index: usize) -> i64 {
    values
        .iter()
        .find(|entry| entry.index == index)
        .map_or(0, |entry| entry.value)
}

fn average_component(source: &StatContribution) -> i64 {
    if source.average_count > 0 {
        if source.average_sum <= i64::MIN + 1 || source.average_sum == i64::MAX {
            0xffff_ffff
        } else {
            source.average_sum.wrapping_mul(100_000) / (i64::from(source.average_count) * 100_000)
        }
    } else {
        0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatContribution {
    pub additive: i64,
    pub average_sum: i64,
    pub average_count: i32,
    pub multiplier: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatComponents {
    pub values: [StatContribution; STAT_COUNT],
}
impl Default for StatComponents {
    fn default() -> Self {
        Self {
            values: std::array::from_fn(|_| StatContribution::default()),
        }
    }
}
impl StatComponents {
    pub fn base(values: &[i64; STAT_COUNT]) -> Self {
        Self {
            values: std::array::from_fn(|index| StatContribution {
                additive: values[index],
                ..Default::default()
            }),
        }
    }
    pub fn applied(
        &mut self,
        additive: bool,
        index: usize,
        value: i64,
    ) -> Result<(), &'static str> {
        let target = self
            .values
            .get_mut(index)
            .ok_or("applied equipment stat index outside native metadata")?;
        let target = if additive {
            &mut target.additive
        } else {
            &mut target.multiplier
        };
        *target = target.wrapping_add(value);
        Ok(())
    }
    pub fn apply(&mut self, group: &StatGroup, scale: i64) -> Result<(), &'static str> {
        for value in &group.add {
            let target = self
                .values
                .get_mut(value.index)
                .ok_or("equipment additive stat index outside native metadata")?;
            target.additive = target
                .additive
                .wrapping_add(value.value.wrapping_mul(scale) / 100_000);
        }
        for value in &group.multiply {
            let target = self
                .values
                .get_mut(value.index)
                .ok_or("equipment multiplier stat index outside native metadata")?;
            target.multiplier = target
                .multiplier
                .wrapping_add(value.value.wrapping_mul(scale) / 100_000);
        }
        for value in &group.average {
            let target = self
                .values
                .get_mut(value.index)
                .ok_or("equipment average stat index outside native metadata")?;
            target.average_sum = target
                .average_sum
                .wrapping_add(value.value.wrapping_mul(scale) / 100_000);
            target.average_count = target.average_count.wrapping_add(1);
        }
        Ok(())
    }
    pub fn apply_module(
        &mut self,
        group: &StatGroup,
        equipment: &super::Equipment,
    ) -> Result<(), &'static str> {
        for value in &group.add {
            let target = self
                .values
                .get_mut(value.index)
                .ok_or("equipment additive stat index outside native metadata")?;
            let factor = stat_term(&equipment.module_add_stats_multiplier, value.index)
                .wrapping_add(100_000);
            target.additive = target
                .additive
                .wrapping_add(factor.wrapping_mul(value.value) / 100_000);
        }
        for value in &group.multiply {
            let target = self
                .values
                .get_mut(value.index)
                .ok_or("equipment multiplier stat index outside native metadata")?;
            let factor = stat_term(&equipment.module_multiply_stats_multiplier, value.index)
                .wrapping_add(100_000);
            target.multiplier = target
                .multiplier
                .wrapping_add(factor.wrapping_mul(value.value) / 100_000);
        }
        for value in &group.average {
            let target = self
                .values
                .get_mut(value.index)
                .ok_or("equipment average stat index outside native metadata")?;
            let factor = stat_term(&equipment.module_add_stats_multiplier, value.index)
                .wrapping_add(100_000);
            target.average_sum = target
                .average_sum
                .wrapping_add(factor.wrapping_mul(value.value) / 100_000);
            target.average_count = target.average_count.wrapping_add(1);
        }
        Ok(())
    }
    pub fn effective_reliability(&self) -> i64 {
        let source = &self.values[RELIABILITY_STAT];
        let base = source.additive.wrapping_add(average_component(source));
        (source.multiplier.wrapping_add(100_000).wrapping_mul(base) / 100_000).clamp(0, 100_000)
    }
    pub fn apply_reliability_scaling(
        &mut self,
        equipment: &super::Equipment,
    ) -> Result<(), &'static str> {
        if equipment.scaled_by_reliability.is_empty() {
            return Ok(());
        }
        let reliability = self.effective_reliability();
        for term in &equipment.scaled_by_reliability {
            let target = self
                .values
                .get_mut(term.index)
                .ok_or("reliability-scaled equipment stat index outside native metadata")?;
            target.additive = target
                .additive
                .wrapping_add(term.value.wrapping_mul(reliability) / 100_000);
        }
        Ok(())
    }
    pub fn finish_for(
        &self,
        equipment: &super::Equipment,
    ) -> Result<[i64; STAT_COUNT], &'static str> {
        let mut combined = self.clone();
        combined.apply_reliability_scaling(equipment)?;
        Ok(combined.finish())
    }
    pub fn finish(&self) -> [i64; STAT_COUNT] {
        std::array::from_fn(|index| {
            let source = &self.values[index];
            let average = average_component(source);
            let value = source.additive.wrapping_add(average);
            if index == CARRIER_SIZE_STAT {
                let value = value.wrapping_add(source.multiplier);
                if value % 100_000 == 0 {
                    value
                } else {
                    (value / 100_000 + (value >> 63)).wrapping_mul(100_000)
                }
            } else {
                value.wrapping_mul(source.multiplier.wrapping_add(100_000)) / 100_000
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn industrial_content_retains_pool_replacements_threshold_resets_and_last_duplicate_level() {
        content_reader_script::test_game::enter();
        let mut db = super::super::Database::default();
        db.read_file(br#"equipment_modules = { custom = {
            build_cost_resources = { alpha = 1 beta = 2 alpha = 0 }
            build_cost_resources = { gamma = -0.5 beta = 3 }
            dismantle_cost_resources = { beta = 4 beta = 0 }
            can_convert_from = { module = old convert_cost_resources = { beta = 1 alpha = 2 beta = 0 } }
        } }
        upgrades = { custom = {
            resource_cost_thresholds = { 1 = { alpha = 99 } }
            resource_cost_thresholds = { 7 = { beta = 1 } 3 = { alpha = 2 } 7 = { gamma = 4 gamma = 0 } }
        } }"#, "common/units/equipment/synthetic.txt", &content_reader_script::key_names::KeyNames::default());
        assert!(db.lost.is_none(), "{:?}", db.lost);
        let module = db.module_design(b"custom").unwrap();
        assert_eq!(
            module.build_resources,
            [
                (b"alpha".to_vec(), Ok(0)),
                (b"beta".to_vec(), Ok(300_000)),
                (b"gamma".to_vec(), Ok(-50_000))
            ]
        );
        assert_eq!(module.dismantle_resources, [(b"beta".to_vec(), Ok(0))]);
        assert_eq!(
            module.conversions[0].resources,
            [(b"beta".to_vec(), Ok(0)), (b"alpha".to_vec(), Ok(200_000))]
        );
        assert_eq!(
            db.upgrade_design(b"custom").unwrap().resource_thresholds,
            [
                (3, vec![(b"alpha".to_vec(), Ok(200_000))]),
                (7, vec![(b"gamma".to_vec(), Ok(0))])
            ]
        );
    }
    #[test]
    fn design_upgrade_xp_retains_declared_currencies_linear_zero_and_native_stage_rounding() {
        content_reader_script::test_game::enter();
        let mut db = super::super::Database::default();
        db.read_file(b"upgrades = { mixed = { cost = air } free = { cost = naval linear_cost = { cost_by_level = 0 cost_by_level_for_licensed_equipment = 1.23457 } } }", "common/units/equipment/upgrades/synthetic.txt", &content_reader_script::key_names::KeyNames::default());
        let rules = UpgradeXpRules {
            base: [10, 20, 30],
            ramp: [1, 2, 3],
            licensed_factor: 123_457,
        };
        let mixed = db.upgrade_design(b"MIXED").unwrap();
        assert_eq!(mixed.currency, UpgradeCurrency::Air);
        assert_eq!(rules.change(mixed, 2, 4, false), Ok(7_500_000));
        assert_eq!(rules.change(mixed, 4, 2, false), Ok(8_100_000));
        let free = db.upgrade_design(b"free").unwrap();
        assert_eq!(rules.change(free, 0, 7, false), Ok(0));
        assert_eq!(rules.change(free, 7, 0, true), Ok(864_199));
        assert_eq!(rules.finish(123_459, -12_345, 2_345, 0, true), 137_176);
        assert_eq!(
            module_save_xp(0, 123_459, true, 123_457, -12_345, 2_345, 0, 12_345),
            120_241
        );
        assert_eq!(
            module_save_xp(0, 1, false, 100_000, -100_000, 0, 0, 0),
            100_000
        );
        assert_eq!(
            module_save_xp(0, 1, false, 100_000, -100_000, 0, 0, 100_001),
            0
        );
        let wrapping = UpgradeXpRules {
            base: [i32::MAX; 3],
            ramp: [i32::MAX; 3],
            licensed_factor: 100_000,
        };
        assert_eq!(
            wrapping.change(&UpgradeDesign::default(), 1, 2, false),
            Ok(-200_000)
        );
    }
    #[test]
    fn design_stats_preserve_zero_average_presence_and_signed_integer_rounding() {
        let mut base = [0; STAT_COUNT];
        base[77] = 123_456;
        base[72] = -100_001;
        let mut stats = StatComponents::base(&base);
        stats
            .apply(
                &StatGroup {
                    add: vec![StatValue {
                        index: 77,
                        value: 5_000,
                    }],
                    average: vec![StatValue {
                        index: 77,
                        value: 0,
                    }],
                    multiply: vec![StatValue {
                        index: 77,
                        value: 10_000,
                    }],
                    unread: Vec::new(),
                },
                100_000,
            )
            .unwrap();
        stats
            .apply(
                &StatGroup {
                    average: vec![StatValue {
                        index: 77,
                        value: 10_001,
                    }],
                    ..Default::default()
                },
                100_000,
            )
            .unwrap();
        let values = stats.finish();
        assert_eq!(values[77], 146_801);
        assert_eq!(values[72], -200_000);
        assert_eq!(stats.values[77].average_count, 2);
        let mut stats = StatComponents::default();
        stats.applied(false, 77, i64::MAX).unwrap();
        assert_eq!(stats.values[77].multiplier, i64::MAX);
    }
    #[test]
    fn design_content_keeps_module_and_upgrade_terms_including_duplicate_zeroes() {
        content_reader_script::test_game::enter();
        let mut db = super::super::Database::default();
        for (path, text) in [
            (
                "common/units/equipment/modules/synthetic.txt",
                "equipment_modules = { custom = { add_stats = { air_attack = 1 air_attack = 2 } add_average_stats = { air_agility = 0 } multiply_stats = { air_attack = -0.1 } } }",
            ),
            (
                "common/units/equipment/upgrades/synthetic.txt",
                "upgrades = { custom = { air_attack = 0.25 add_stats = { air_defence = 2 } } }",
            ),
        ] {
            db.read_file(
                text.as_bytes(),
                path,
                &content_reader_script::key_names::KeyNames::default(),
            );
        }
        let module = db.module_stats(b"CUSTOM").unwrap();
        assert_eq!(module.add[0].value, 300_000);
        assert_eq!(module.average[0].value, 0);
        assert_eq!(module.multiply[0].value, -10_000);
        assert_eq!(
            db.upgrade_stats(b"custom").unwrap().multiply[0].value,
            25_000
        );
    }

    #[test]
    fn design_module_costs_keep_source_specific_rules_and_exact_xp_rounding() {
        content_reader_script::test_game::enter();
        let mut db = super::super::Database::default();
        db.read_file(br#"equipment_modules = {
            old = { category = synthetic_category }
            next = { category = synthetic_category dismantle_cost_ic = 1.25
                can_convert_from = { module_category = synthetic_category convert_cost_ic = 10 }
                can_convert_from = { module = old convert_cost_ic = 3.25 convert_cost_resources = { synthetic_resource = 0.5 } }
                can_convert_from = { module_category = synthetic_category convert_cost_ic = 20 } }
            override = { category = synthetic_category xp_cost = 0 }
            other = { category = other_category }
            ungrouped = { }
            implicit = { can_convert_from = { module = old convert_cost_ic = 7 } }
        }"#, "common/units/equipment/modules/synthetic.txt", &content_reader_script::key_names::KeyNames::default());
        let old = db.module_design(b"old").unwrap();
        let next = db.module_design(b"next").unwrap();
        let conversion = next.conversion_from(b"OLD", old).unwrap();
        assert_eq!(conversion.cost_ic, Ok(325_000));
        assert_eq!(
            conversion.resources,
            [(b"synthetic_resource".to_vec(), Ok(50_000))]
        );
        assert_eq!(
            next.conversion_from(b"other_same_category", old)
                .unwrap()
                .cost_ic,
            Ok(2_000_000)
        );
        assert_eq!(next.dismantle_cost_ic, Some(Ok(125_000)));
        let rules = ModuleXpRules {
            add: 300_001,
            replace: 700_001,
            convert: 200_001,
            remove: 100_001,
        };
        assert_eq!(
            rules.slot(Some((b"next", next)), None, false, -12_345),
            Ok(262_965)
        );
        assert_eq!(
            rules.slot(Some((b"next", next)), Some((b"old", old)), true, 0),
            Ok(200_001)
        );
        assert_eq!(
            rules.slot(Some((b"next", next)), Some((b"next", next)), true, i64::MAX),
            Ok(0)
        );
        assert_eq!(
            rules.slot(
                Some((b"next", next)),
                Some((b"other", db.module_design(b"other").unwrap())),
                true,
                0
            ),
            Ok(700_001)
        );
        assert_eq!(
            rules.slot(
                Some((b"override", db.module_design(b"override").unwrap())),
                None,
                false,
                0
            ),
            Ok(0)
        );
        assert_eq!(rules.slot(None, Some((b"old", old)), true, 0), Ok(100_001));
        assert_eq!(
            db.module_design(b"implicit")
                .unwrap()
                .conversion_from(b"ungrouped", db.module_design(b"ungrouped").unwrap())
                .unwrap()
                .cost_ic,
            Ok(700_000)
        );
    }

    #[test]
    fn an_unreadable_amount_is_withheld_by_its_field_never_a_zero() {
        content_reader_script::test_game::enter();
        let mut db = super::super::Database::default();
        db.read_file(br#"equipment_modules = { custom = {
            manpower = unknown_amount
            xp_cost = unknown_amount
            build_cost_resources = { steel = unknown_amount chromium = 2 }
            add_stats = { air_attack = unknown_amount air_defence = 1 }
        } }"#, "common/units/equipment/modules/synthetic.txt", &content_reader_script::key_names::KeyNames::default());
        let module = db.module_design(b"custom").unwrap();
        let unread = |field: &str| Err(format!("{field} = unknown_amount is not a number"));
        assert_eq!(module.manpower, Some(unread("manpower")));
        assert_eq!(module.build_resources, [(b"steel".to_vec(), unread("steel")), (b"chromium".to_vec(), Ok(200_000))]);
        let rules = ModuleXpRules { add: 1, replace: 1, convert: 1, remove: 1 };
        assert_eq!(rules.slot(Some((b"custom", module)), None, false, 0), unread("xp_cost"));
        let stats = db.module_stats(b"custom").unwrap();
        let air_attack = super::super::stat_index(b"air_attack").unwrap();
        assert_eq!(stats.unread, [(air_attack, unread("air_attack").unwrap_err())]);
        assert!(stats.add.iter().all(|term| term.index != air_attack), "an unreadable term adds nothing that could be shown as its value");
    }

    #[test]
    fn a_scalar_mission_limit_is_that_mission() {
        content_reader_script::test_game::enter();
        let mut db = super::super::Database::default();
        db.read_file(b"equipment_modules = { custom = { mission_type_stats = { limit = cas add_stats = { air_attack = 1 } } } }", "common/units/equipment/modules/synthetic.txt", &content_reader_script::key_names::KeyNames::default());
        let module = db.module_design(b"custom").unwrap();
        assert_eq!(module.mission_terms.iter().map(|(missions, _)| missions.clone()).collect::<Vec<_>>(), [vec![b"cas".to_vec()]]);
    }

    #[test]
    fn reliability_scaling_clamps_effective_reliability_and_applies_before_final_multiplier() {
        let mut equipment = super::super::Equipment::new(b"test".to_vec(), 0);
        equipment.scaled_by_reliability = vec![StatValue {
            index: 0,
            value: 1_000_000,
        }];
        let mut over = [0; STAT_COUNT];
        over[RELIABILITY_STAT] = 150_000;
        let mut comp = StatComponents::base(&over);
        comp.applied(false, 0, 50_000).unwrap();
        assert_eq!(comp.effective_reliability(), 100_000);
        assert_eq!(comp.finish_for(&equipment).unwrap()[0], 1_500_000);

        let mut negative = [0; STAT_COUNT];
        negative[RELIABILITY_STAT] = -25_000;
        let comp = StatComponents::base(&negative);
        assert_eq!(comp.effective_reliability(), 0);
        assert_eq!(comp.finish_for(&equipment).unwrap()[0], 0);
    }
}
