use crate::key_name_products::KeyNamesExt as _;
use content_reader_script::equipment_stats::{category_mask, STAT_COUNT, stat_index, stat_metadata, FORBIDDEN_RELIABILITY_SCALED_STATS};

mod design;
mod validity;
pub use validity::{Assignment, CountKey, ModuleCountLimit, ModuleValidity};
mod groups;
pub use design::{
    ModuleConversion,
    ModuleDesign, ModuleXpRules, StatComponents,
    StatContribution, StatGroup, StatValue, UpgradeCurrency, UpgradeDesign, UpgradeXpRules,
    module_save_xp,
};

pub use groups::{GROUPS_DIRECTORY, Group, Groups};

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use content_reader_layers::ResolvedLayers;
use content_reader_parser::{crt::{Scanned, scan_i32}, fixed5::script_fixed5, parser::{Item, Value, c_string}, tokens::{self, Token}};
use content_reader_script::{key_names::KeyNames, native::{self, Field, Kind, Shape}};

pub const DIRECTORY: &str = "common/units/equipment/";

pub fn files(layers: &ResolvedLayers) -> io::Result<Vec<content_reader_layers::script_files::ScriptPath>> {
    let mut files = Vec::new();
    for directory in [
        format!("{DIRECTORY}upgrades/"),
        format!("{DIRECTORY}modules/"),
        DIRECTORY.to_owned(),
    ] {
        files.extend(content_reader_layers::script_files::paths(layers, &directory, Some(".txt"))?);
    }
    Ok(files)
}

fn token_category(text: &[u8], quoted: bool) -> u64 {
    if quoted {
        return 0;
    }
    category_mask(text).unwrap_or(0)
}

fn unsigned_decimal(text: &[u8]) -> u64 {
    let start = text
        .iter()
        .position(|byte| !matches!(byte, b' ' | b'\t'..=b'\r'))
        .unwrap_or(text.len());
    let text = &text[start..];
    let (negative, digits) = match text.first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let count = digits
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if count == 0 {
        return 0;
    }
    let value = digits[..count].iter().try_fold(0_u64, |value, digit| {
        value
            .checked_mul(10)
            .and_then(|value| value.checked_add(u64::from(digit - b'0')))
    });
    match value {
        None => u64::MAX,
        Some(value) if negative => value.wrapping_neg(),
        Some(value) => value,
    }
}

fn category_list(value: &Value, quoted: bool) -> u64 {
    let one = |text: &[u8], quoted: bool| match token_category(text, quoted) {
        0 => unsigned_decimal(text),
        mask => mask,
    };
    match value.scalar() {
        Some(text) => one(text, quoted),
        None => value
            .items()
            .unwrap_or_default()
            .iter()
            .fold(0, |mask, item| {
                mask | item
                    .value
                    .scalar()
                    .map_or(0, |text| one(text, item.quoted()))
            }),
    }
}

pub fn equipment_pool_roster(database: &Database) -> BTreeSet<Vec<u8>> {
    database
        .registered_names()
        .filter_map(|name| match Token::c_str(name) {
            Token::Name(name) => Some(name),
            Token::Rejected => None,
        })
        .collect()
}

pub fn mission_mask(name: &[u8]) -> u32 {
    let name = c_string(name);
    engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.equipment_missions))
        .iter()
        .find(|(known, _)| known.as_bytes().eq_ignore_ascii_case(name))
        .map_or(0, |(_, mask)| *mask)
}

fn single_value_field(keyword: &[u8]) -> bool {
    matches!(
        keyword,
        b"abbreviation"
            | b"active"
            | b"ai_type"
            | b"air_map_icon_frame"
            | b"archetype"
            | b"can_license"
            | b"carrier_capable"
            | b"default_carrier_composition_weight"
            | b"derived_variant_name"
            | b"family"
            | b"group_by"
            | b"interface_category"
            | b"interface_overview_category_index"
            | b"is_archetype"
            | b"is_buildable"
            | b"is_convertable"
            | b"is_frame"
            | b"lend_lease_cost"
            | b"manpower"
            | b"max_dockyard_factories"
            | b"max_military_factories"
            | b"model"
            | b"naval_dominance_factor"
            | b"offensive_weapons"
            | b"one_use_only"
            | b"parent"
            | b"picture"
            | b"priority"
            | b"sprite"
            | b"substitute"
            | b"supply_truck"
            | b"type_override"
            | b"variant_name"
            | b"visual_level"
            | b"visual_tech_level_addition"
            | b"year"
    )
}

#[derive(Clone, Copy, Debug)]
enum Node {
    Top,
    Table,
    Equipment,
    Entries,
    Slots,
    Slot,
    Limit,
    ModifierStat,
    Override,
    Upgrades,
    Upgrade,
    LevelTriggers,
    LevelPools,
    Cost,
    Stats,
    Modules,
    Module,
    Conversion,
    CategoryMasks,
    SlotCategories,
    MissionStats,
    Filter,
    Duplicates,
    Duplicate,
    Pairs,
    ForEach,
    Statement(Statement),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Statement {
    Stat,
    Variant,
}

#[derive(Default)]
struct Reading {
    slots: Slots,
    slot_fields: usize,
    decided: BTreeMap<usize, bool>,
    assumed: Vec<usize>,
}

#[derive(Default)]
struct Slots {
    names: Vec<Token>,
    maybe: Vec<Token>,
    inherited: bool,
    undecided: Option<String>,
}

impl Slots {
    fn shape(
        &mut self,
        field: &Field<'_>,
        index: usize,
        decided: &BTreeMap<usize, bool>,
        assumed: &mut Vec<usize>,
    ) -> Shape<Node> {
        let name = Token::c_str(&field.key.text);
        let mut known = self.names.contains(&name);
        let mut unknown = !known && (self.inherited || self.maybe.contains(&name));
        if unknown && field.fresh && field.opens() {
            match decided.get(&index) {
                Some(&answer) => known = answer,
                None => assumed.push(index),
            }
            unknown = false;
        }
        if unknown && field.skips_block() && self.undecided.is_none() {
            self.undecided = Some(format!(
                "line {}: module slot {} has a block value where the parent's slots decide whether it is already defined; the slot reader skips an existing slot and reads it otherwise, and the parent's slots are not read here",
                field.key.line,
                String::from_utf8_lossy(&field.key.text)
            ));
        }
        if known {
            return Shape::Skip;
        }
        if field.opens() {
            self.names.push(name);
            return Shape::Object(Node::Slot);
        }
        if field.value.is_keyword("inherit") {
            self.maybe.push(name);
            return Shape::One;
        }
        let source = Token::register(c_string(&field.value.text));
        if source.is_rejected() {
            return Shape::One;
        }
        if self.names.contains(&source) {
            self.names.push(name);
        } else if self.inherited || self.maybe.contains(&source) {
            self.maybe.push(name);
        }
        Shape::One
    }
}

fn level(field: &Field<'_>) -> bool {
    field.key.kind == Kind::Bare
        && content_reader_parser::parser::number_token(&field.key.text, false)
        && (1..=255).contains(&content_reader_parser::fixed5::decimal_i32(&field.key.text))
}

fn is_stat(keyword: Option<&[u8]>) -> bool {
    keyword.is_some_and(|keyword| stat_index(keyword).is_some())
}

fn shape(node: Node, field: &Field<'_>, reading: &mut Reading) -> Shape<Node> {
    let keyword = field.keyword();
    let keyword = keyword.as_deref();
    match node {
        Node::Top => match keyword {
            Some(b"equipments") => Shape::Object(Node::Table),
            Some(b"upgrades") => Shape::Object(Node::Upgrades),
            Some(b"equipment_modules") => Shape::Object(Node::Modules),
            Some(b"search_filters") => Shape::Object(Node::Filter),
            Some(b"duplicate_archetypes") if field.opens() => Shape::Object(Node::Duplicates),
            _ => Shape::Skip,
        },
        Node::Table => {
            let undecided = reading.slots.undecided.take();
            reading.slots = Slots {
                undecided,
                ..Slots::default()
            };
            Shape::Object(Node::Equipment)
        }
        Node::Equipment => equipment_field(field, keyword, &mut reading.slots),
        Node::Entries => Shape::One,
        Node::Slots => {
            let index = reading.slot_fields;
            reading.slot_fields += 1;
            reading
                .slots
                .shape(field, index, &reading.decided, &mut reading.assumed)
        }
        Node::Slot => match keyword {
            Some(b"gfx" | b"required") => Shape::One,
            Some(b"allowed_module_categories") => Shape::ListIfOpen,
            _ => Shape::Skip,
        },
        Node::Limit => match keyword {
            Some(b"category" | b"module" | b"title" | b"count") => Shape::One,
            _ => Shape::Skip,
        },
        Node::ModifierStat => match keyword {
            Some(b"type" | b"value" | b"modifier") => Shape::One,
            _ => Shape::Skip,
        },
        Node::Override => Shape::One,
        Node::Upgrades => Shape::Object(Node::Upgrade),
        Node::Upgrade => match keyword {
            Some(b"level_requirements") if field.opens() => Shape::Object(Node::LevelTriggers),
            Some(b"resource_cost_thresholds") if field.opens() => Shape::Object(Node::LevelPools),
            Some(b"level_requirements" | b"resource_cost_thresholds") => Shape::Skip,
            Some(b"linear_cost") => Shape::Object(Node::Cost),
            Some(b"cost") => match field.value_keyword().as_deref() {
                Some(b"land" | b"naval" | b"air") => Shape::One,
                _ => Shape::Skip,
            },
            Some(b"abbreviation" | b"max_level") => Shape::One,
            Some(b"add_stats" | b"add_average_stats" | b"multiply_stats") => {
                Shape::Object(Node::Stats)
            }
            _ if is_stat(keyword) => Shape::One,
            Some(b"attrition") => Shape::Object(Node::Entries),
            _ => Shape::Skip,
        },
        Node::LevelTriggers if level(field) => Shape::Trigger,
        Node::LevelPools if level(field) => Shape::Object(Node::Entries),
        Node::LevelTriggers | Node::LevelPools => Shape::Skip,
        Node::Cost => match keyword {
            Some(b"cost_by_level" | b"cost_by_level_for_licensed_equipment") => Shape::One,
            _ => Shape::Skip,
        },
        Node::Stats if is_stat(keyword) => Shape::One,
        Node::Stats => Shape::Skip,
        Node::Modules if matches!(keyword, Some(b"limit")) => Shape::Trigger,
        Node::Modules => Shape::Object(Node::Module),
        Node::Module => module_field(field, keyword),
        Node::Conversion => match keyword {
            Some(b"convert_cost_ic" | b"module_category" | b"module") => Shape::One,
            Some(b"convert_cost_resources") => Shape::Object(Node::Entries),
            _ => Shape::Skip,
        },
        Node::CategoryMasks => Shape::ListIfOpen,
        Node::SlotCategories if field.opens() => Shape::List,
        Node::SlotCategories => Shape::Skip,
        Node::MissionStats => match keyword {
            Some(b"limit") => Shape::ListIfOpen,
            Some(b"add_stats" | b"add_average_stats" | b"multiply_stats") => {
                Shape::Object(Node::Stats)
            }
            _ => Shape::Skip,
        },
        Node::Filter if matches!(keyword, Some(b"values")) => Shape::List,
        Node::Filter => Shape::One,
        Node::Duplicates => Shape::Object(Node::Duplicate),
        Node::Duplicate => match keyword {
            Some(
                b"ai_type"
                | b"archetype"
                | b"substitute"
                | b"sprite"
                | b"picture"
                | b"only_duplicate_archetype"
                | b"default_carrier_composition_weight"
                | b"interface_overview_category_index"
                | b"air_map_icon_frame",
            ) => Shape::One,
            Some(b"type" | b"forbid_mission_type") => Shape::ListIfOpen,
            Some(b"module_slots") if field.opens() => Shape::SkipIfOpen,
            Some(b"module_slots") if field.value.is_keyword("none") => Shape::One,
            Some(b"variant_name") if field.opens() => Shape::Object(Node::Pairs),
            Some(b"for_each") if field.opens() => Shape::Object(Node::ForEach),
            _ => Shape::Skip,
        },
        Node::Pairs => Shape::One,
        Node::ForEach => match keyword {
            Some(b"variant_name") => Shape::Pair(Node::Statement(Statement::Variant)),
            _ if is_stat(keyword) => Shape::Pair(Node::Statement(Statement::Stat)),
            _ => Shape::Skip,
        },
        Node::Statement(statement) => match (statement, keyword) {
            (Statement::Stat, Some(b"set" | b"multiply" | b"add")) => Shape::One,
            (Statement::Variant, Some(b"find_and_replace")) => Shape::Strings,
            _ => Shape::Skip,
        },
    }
}

fn equipment_field(field: &Field<'_>, keyword: Option<&[u8]>, slots: &mut Slots) -> Shape<Node> {
    match keyword {
        Some(b"can_convert_from" | b"upgrades") => Shape::List,
        Some(
            b"type" | b"allow_mission_type" | b"forbid_mission_type" | b"allowed_types" | b"alias",
        ) => Shape::ListIfOpen,
        Some(b"attrition" | b"resources" | b"default_modules") => Shape::Object(Node::Entries),
        Some(b"module_slots") => {
            if field.value.is_keyword("inherit") {
                slots.names.clear();
                slots.maybe.clear();
                slots.inherited = true;
            }
            Shape::ObjectIfOpen(Node::Slots)
        }
        Some(b"modifier_stat") => Shape::Object(Node::ModifierStat),
        Some(b"upgrade_window_override") => Shape::Object(Node::Override),
        Some(b"module_count_limit") => Shape::Object(Node::Limit),
        Some(
            b"module_add_stats_multiplier"
            | b"module_multiply_stats_multiplier"
            | b"scaled_by_reliability",
        ) => Shape::Object(Node::Stats),
        Some(b"can_be_produced" | b"can_be_lend_leased") => Shape::Trigger,
        Some(name) if single_value_field(name) => Shape::One,
        _ if is_stat(keyword) => Shape::One,
        _ => Shape::Skip,
    }
}

fn module_field(field: &Field<'_>, keyword: Option<&[u8]>) -> Shape<Node> {
    match keyword {
        Some(b"can_convert_from") => Shape::Object(Node::Conversion),
        Some(b"forbid_equipment_type_exact_match_for_category") if field.opens() => {
            Shape::Object(Node::CategoryMasks)
        }
        Some(b"allowed_module_categories") if field.opens() => Shape::Object(Node::SlotCategories),
        Some(b"forbid_equipment_type_exact_match_for_category" | b"allowed_module_categories") => {
            Shape::Skip
        }
        Some(
            b"manpower" | b"parent" | b"category" | b"mega_carrier" | b"abbreviation" | b"gfx"
            | b"sfx" | b"dismantle_cost_ic" | b"xp_cost",
        ) => Shape::One,
        Some(
            b"forbid_equipment_type_exact_match"
            | b"allow_mission_type"
            | b"forbid_module_categories"
            | b"gui_category"
            | b"add_equipment_type"
            | b"allow_equipment_type"
            | b"forbid_equipment_type",
        ) => Shape::ListIfOpen,
        Some(b"mission_type_stats") => Shape::Object(Node::MissionStats),
        Some(b"allowed") => Shape::Trigger,
        Some(b"critical_parts") => Shape::List,
        Some(b"build_cost_resources" | b"dismantle_cost_resources") => Shape::Object(Node::Entries),
        Some(b"add_stats" | b"add_average_stats" | b"multiply_stats") => Shape::Object(Node::Stats),
        _ => Shape::Skip,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleSlot {
    pub required: bool,
    pub categories: Vec<Vec<u8>>,
}
impl ModuleSlot {
    fn read(fields: &[Item]) -> Self {
        let mut result = Self::default();
        for field in fields {
            if field.keyword_is("required") {
                if let Some(value) = field.value.scalar().and_then(content_reader_parser::parser::read_bool) {
                    result.required = value;
                }
            } else if field.keyword_is("allowed_module_categories") {
                for category in field
                    .value
                    .items()
                    .unwrap_or_else(|| std::slice::from_ref(field))
                {
                    result.categories.push(
                        c_string(category.value.scalar().unwrap_or_default()).to_ascii_lowercase(),
                    );
                }
            }
        }
        result
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Equipment {
    pub name: Vec<u8>,
    aliases: Vec<Vec<u8>>,
    pub is_archetype: bool,
    pub supply_truck: bool,
    pub archetype: Option<usize>,
    pub parent: usize,
    pub family: usize,
    pub category: u64,
    pub module_category: u64,
    pub allowed_types: u64,
    pub type_override: Vec<u8>,
    pub default_modules: Vec<(Vec<u8>, Vec<u8>)>,
    pub module_slots: Vec<Vec<u8>>,
    pub module_slot_rules: Vec<ModuleSlot>,
    pub module_count_limits: Vec<ModuleCountLimit>,
    pub upgrades: Vec<Vec<u8>>,
    pub sprite: Vec<u8>,
    pub model: Vec<u8>,
    pub variant_name: Vec<u8>,
    pub derived_variant_name: Vec<u8>,
    pub visual_level: i32,
    pub air_map_icon_frame: i32,
    pub carrier_capable: bool,
    pub visual_tech_level_addition: i32,
    pub model_priority: i32,
    pub depth: i32,
    pub stats: [i64; STAT_COUNT],
    pub module_add_stats_multiplier: Vec<StatValue>,
    pub module_multiply_stats_multiplier: Vec<StatValue>,
    pub scaled_by_reliability: Vec<StatValue>,
    pub duplicate: bool,
}

impl Equipment {
    fn new(name: Vec<u8>, at: usize) -> Self {
        Self {
            name,
            aliases: Vec::new(),
            is_archetype: false,
            supply_truck: false,
            archetype: None,
            parent: at,
            family: at,
            category: 0,
            module_category: 0,
            allowed_types: 0,
            type_override: Vec::new(),
            default_modules: Vec::new(),
            module_slots: Vec::new(),
            module_slot_rules: Vec::new(),
            module_count_limits: Vec::new(),
            upgrades: Vec::new(),
            sprite: Vec::new(),
            model: Vec::new(),
            variant_name: Vec::new(),
            derived_variant_name: Vec::new(),
            visual_level: 0,
            air_map_icon_frame: 0,
            carrier_capable: false,
            visual_tech_level_addition: 1,
            model_priority: 1,
            depth: 0,
            stats: [0; STAT_COUNT],
            module_add_stats_multiplier: Vec::new(),
            module_multiply_stats_multiplier: Vec::new(),
            scaled_by_reliability: Vec::new(),
            duplicate: false,
        }
    }

    pub fn name_str(&self) -> Result<&str, content_reader_parser::parser::NotText> {
        content_reader_parser::parser::text(&self.name)
    }
}

#[derive(Clone, Debug)]
struct Duplicate {
    token: Token,
    name: Vec<u8>,
    archetype: Option<Token>,
    only_archetype: bool,
    no_slots: bool,
    category: u64,
    sprite: Vec<u8>,
    operations: Vec<(usize, Operation, i64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    Set,
    Multiply,
    Add,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    pub equipment: Vec<Equipment>,
    by_name: BTreeMap<Vec<u8>, usize>,
    upgrade_names: BTreeSet<Vec<u8>>,
    upgrade_stats: BTreeMap<Vec<u8>, StatGroup>,
    upgrade_designs: BTreeMap<Vec<u8>, UpgradeDesign>,
    module_stats: BTreeMap<Vec<u8>, StatGroup>,
    module_designs: BTreeMap<Vec<u8>, ModuleDesign>,
    modules: Vec<(Token, Vec<u8>, u64)>,
    module_names: BTreeMap<Vec<u8>, usize>,
    module_conditions: BTreeMap<Vec<u8>, ModuleConditions>,
    pub misreads: Vec<String>,
    pub lost: Option<String>,
    slot_trace: Vec<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleConditions {
    pub limit: Option<Vec<Item>>,
    pub allowed: Vec<Item>,
}

impl Database {
    pub fn upgrade_design(&self, name: impl AsRef<[u8]>) -> Option<&UpgradeDesign> {
        let Token::Name(key) = Token::c_str(name.as_ref()) else {
            return None;
        };
        self.upgrade_designs.get(&key)
    }
    pub fn module_design(&self, name: impl AsRef<[u8]>) -> Option<&ModuleDesign> {
        self.module_designs
            .get(&self.module(name)?.to_ascii_lowercase())
    }
    pub fn module_stats(&self, name: impl AsRef<[u8]>) -> Option<&StatGroup> {
        self.module_stats
            .get(&self.module(name)?.to_ascii_lowercase())
    }
    pub fn all_module_designs(
        &self,
    ) -> impl Iterator<Item = (&[u8], &ModuleDesign, Option<&StatGroup>)> {
        self.module_designs
            .iter()
            .map(|(name, design)| (name.as_slice(), design, self.module_stats.get(name)))
    }
    pub fn upgrade_stats(&self, name: impl AsRef<[u8]>) -> Option<&StatGroup> {
        let Token::Name(key) = Token::c_str(name.as_ref()) else {
            return None;
        };
        self.upgrade_stats.get(&key)
    }
    pub fn module_conditions(&self, name: impl AsRef<[u8]>) -> Option<&ModuleConditions> {
        let spelling = self.module(name)?;
        self.module_conditions.get(&spelling.to_ascii_lowercase())
    }
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        layers.key_name_products()?.equipment_database().map(|database| database.as_ref().clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "key names have no equipment database"))
    }

    pub(crate) fn load_with_names(layers: &ResolvedLayers, names: &KeyNames) -> io::Result<Self> {
        let mut database = Self::default();
        for path in files(layers)? {
            database.read_file(&path.read(layers)?, &path.text(), names);
            if database.lost.is_some() {
                return Ok(database);
            }
        }
        database.finalize();
        Ok(database)
    }

    pub fn load_complete(layers: &ResolvedLayers) -> io::Result<Self> {
        let database = Self::load(layers)?;
        match &database.lost {
            Some(lost) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the equipment database cannot be followed past {lost}"),
            )),
            None => Ok(database),
        }
    }

    pub fn get(&self, name: impl AsRef<[u8]>) -> Option<usize> {
        self.by_name
            .get(&c_string(name.as_ref()).to_ascii_lowercase())
            .copied()
    }

    pub(crate) fn registered_names(&self) -> impl Iterator<Item = &[u8]> {
        self.by_name.keys().map(Vec::as_slice)
    }

    pub(crate) fn registered_upgrades(&self) -> impl Iterator<Item = &[u8]> {
        self.upgrade_names.iter().map(Vec::as_slice)
    }

    pub fn named(&self, name: impl AsRef<[u8]>) -> Option<&Equipment> {
        self.get(name).map(|at| &self.equipment[at])
    }

    pub fn technology_child(
        &self,
        name: &str,
        feature_21: Result<bool, String>,
    ) -> Result<Option<&Equipment>, String> {
        if let Some(lost) = &self.lost {
            return Err(format!(
                "equipment {name}: database unavailable past {lost}"
            ));
        }
        let Some(equipment) = self.named(name) else {
            return Ok(None);
        };
        if self.category(equipment) & (1 << 9) != 0 && !feature_21? {
            return Ok(None);
        }
        Ok(Some(equipment))
    }

    fn read_file(&mut self, bytes: &[u8], path: &str, names: &KeyNames) {
        let mut decided = BTreeMap::new();
        loop {
            let mut reading = Reading {
                decided: decided.clone(),
                ..Reading::default()
            };
            let read = native::read(
                bytes,
                native::Mode::KeepMark,
                native::Top::File(Node::Top),
                Some(names),
                &mut |node, field| shape(node, field, &mut reading),
            );
            if reading.assumed.is_empty() {
                self.load_file(path, read, reading);
                return;
            }
            let mut loaded = self.clone();
            let assumed = loaded.load_file(path, read, reading);
            match assumed
                .into_iter()
                .find(|index| loaded.slot_trace.get(*index) == Some(&true))
            {
                Some(index) => {
                    decided.insert(index, true);
                }
                None => {
                    *self = loaded;
                    return;
                }
            }
        }
    }

    fn load_file(&mut self, path: &str, read: native::Read, mut reading: Reading) -> Vec<usize> {
        self.slot_trace.clear();
        self.misreads.extend(
            read.misreads
                .iter()
                .map(|misread| format!("{path} {misread}")),
        );
        let lost = read
            .unsettled
            .as_ref()
            .map(ToString::to_string)
            .or(reading.slots.undecided.take());
        if let Some(lost) = lost {
            self.lost = Some(format!("{path} {lost}"));
            return reading.assumed;
        }
        for item in &read.items {
            if item.keyword_is("equipments") {
                for entry in item.value.items().unwrap_or_default() {
                    if let Err(lost) = self.load_equipment(path, entry) {
                        self.lost = Some(format!("{path}: {lost}"));
                        return reading.assumed;
                    }
                }
            } else if item.keyword_is("upgrades") {
                for upgrade in item.value.items().unwrap_or_default() {
                    if let Token::Name(name) =
                        Token::c_str(upgrade.key.as_deref().unwrap_or_default())
                    {
                        self.upgrade_stats.insert(
                            name.clone(),
                            StatGroup::read(upgrade.value.items().unwrap_or_default(), true),
                        );
                        self.upgrade_designs.insert(
                            name.clone(),
                            UpgradeDesign::read(upgrade.value.items().unwrap_or_default()),
                        );
                        self.upgrade_names.insert(name);
                    }
                }
            } else if item.keyword_is("equipment_modules") {
                let limit = item
                    .value
                    .items()
                    .unwrap_or_default()
                    .iter()
                    .rfind(|field| field.keyword_is("limit"))
                    .map(|field| field.value.items().unwrap_or_default().to_vec());
                for module in item.value.items().unwrap_or_default() {
                    if !module.keyword_is("limit") {
                        self.load_module(module);
                    }
                }
                if let Some(limit) = limit {
                    for module in item
                        .value
                        .items()
                        .unwrap_or_default()
                        .iter()
                        .filter(|field| !field.keyword_is("limit"))
                    {
                        let key = c_string(module.key.as_deref().unwrap_or_default())
                            .to_ascii_lowercase();
                        if let Some(conditions) = self.module_conditions.get_mut(&key) {
                            conditions.limit = Some(limit.clone());
                        }
                    }
                }
            } else if item.keyword_is("duplicate_archetypes")
                && let Err(lost) = self.duplicate(path, item.value.items().unwrap_or_default())
            {
                self.lost = Some(format!("{path}: {lost}"));
                return reading.assumed;
            }
        }
        reading.assumed
    }

    fn load_module(&mut self, module: &Item) {
        let key = content_reader_parser::parser::c_string(module.key.as_deref().unwrap_or_default());
        let mut mask = 0;
        self.module_stats.insert(
            key.to_ascii_lowercase(),
            StatGroup::read(module.value.items().unwrap_or_default(), false),
        );
        self.module_designs.insert(
            key.to_ascii_lowercase(),
            ModuleDesign::read(module.value.items().unwrap_or_default()),
        );
        let mut conditions = ModuleConditions::default();
        for field in module.value.items().unwrap_or_default() {
            if field.keyword_is("add_equipment_type") {
                mask = category_list(&field.value, field.quoted());
            }
            if field.keyword_is("allowed") {
                conditions
                    .allowed
                    .extend(field.value.items().unwrap_or_default().iter().cloned());
            }
        }
        self.module_conditions
            .insert(key.to_ascii_lowercase(), conditions);
        match self.module_names.get(&key.to_ascii_lowercase()) {
            Some(&at) => self.modules[at].2 = mask,
            None => {
                self.module_names
                    .insert(key.to_ascii_lowercase(), self.modules.len());
                self.modules
                    .push((Token::register(key), key.to_vec(), mask));
            }
        }
    }

    fn load_equipment(&mut self, path: &str, entry: &Item) -> Result<(), String> {
        let key = c_string(entry.key.as_deref().unwrap_or_default());
        let at = match self.get(key) {
            Some(at) => {
                let equipment = &mut self.equipment[at];
                for alias in equipment.aliases.drain(..) {
                    self.by_name.remove(&alias);
                }
                equipment.is_archetype = false;
                equipment.supply_truck = false;
                equipment.duplicate = false;
                equipment.archetype = None;
                equipment.parent = at;
                equipment.family = at;
                equipment.category = 0;
                equipment.module_category = 0;
                equipment.allowed_types = 0;
                equipment.type_override.clear();
                equipment.default_modules.clear();
                equipment.module_slots.clear();
                equipment.module_slot_rules.clear();
                equipment.module_count_limits.clear();
                equipment.upgrades.clear();
                equipment.sprite.clear();
                equipment.air_map_icon_frame = 0;
                equipment.carrier_capable = false;
                equipment.visual_tech_level_addition = 1;
                equipment.module_add_stats_multiplier.clear();
                equipment.module_multiply_stats_multiplier.clear();
                equipment.scaled_by_reliability.clear();
                at
            }
            None => self.register(key.to_vec()),
        };
        let name = String::from_utf8_lossy(&self.equipment[at].name).into_owned();
        for field in entry.value.items().unwrap_or_default() {
            let Some(keyword) = field.keyword().map(|keyword| keyword.folded()) else {
                continue;
            };
            let text = field.value.scalar().unwrap_or(b"{");
            let string = field.value.read_string().unwrap_or(b"{");
            let shown = String::from_utf8_lossy(string).into_owned();
            let named = self.get(string);
            let mut notes = Vec::new();
            match keyword.as_slice() {
                b"supply_truck" => match text {
                    b"yes" => self.equipment[at].supply_truck = true,
                    b"no" => self.equipment[at].supply_truck = false,
                    _ => {}
                },
                b"carrier_capable" => {
                    if let Some(value) = content_reader_parser::parser::read_bool(text) {
                        self.equipment[at].carrier_capable = value;
                    }
                }
                b"is_archetype" => {
                    if let Some(value) = content_reader_parser::parser::read_bool(text) {
                        self.equipment[at].is_archetype = value;
                    }
                    if self.equipment[at].is_archetype {
                        self.equipment[at].archetype = Some(at);
                    }
                }
                b"archetype" => match named.filter(|&found| self.equipment[found].is_archetype) {
                    Some(found) => {
                        let archetype = &self.equipment[found];
                        let (
                            stats,
                            module_add_stats_multiplier,
                            module_multiply_stats_multiplier,
                            scaled_by_reliability,
                            sprite,
                        ) = (
                            archetype.stats,
                            archetype.module_add_stats_multiplier.clone(),
                            archetype.module_multiply_stats_multiplier.clone(),
                            archetype.scaled_by_reliability.clone(),
                            archetype.sprite.clone(),
                        );
                        let equipment = &mut self.equipment[at];
                        equipment.archetype = Some(found);
                        equipment.stats = stats;
                        equipment.module_add_stats_multiplier = module_add_stats_multiplier;
                        equipment.module_multiply_stats_multiplier =
                            module_multiply_stats_multiplier;
                        equipment.scaled_by_reliability = scaled_by_reliability;
                        if equipment.sprite.is_empty() {
                            equipment.sprite = sprite;
                        }
                    }
                    None => notes.push(format!(
                        "archetype {shown} is {} when it is read, which the equipment reader reports, keeping its archetype",
                        if named.is_some() {
                            "not an archetype"
                        } else {
                            "no equipment defined before it"
                        }
                    )),
                },
                b"parent" => match named {
                    Some(found) => {
                        self.equipment[at].parent = found;
                        if self.equipment[at].archetype.is_none() {
                            self.equipment[at].archetype = self.equipment[found].archetype;
                        }
                    }
                    None => notes.push(format!(
                        "parent {shown} is no equipment defined before it, which the equipment reader reports, keeping its parent"
                    )),
                },
                b"family" => match named {
                    Some(found) => self.equipment[at].family = found,
                    None => notes.push(format!(
                        "family {shown} is no equipment defined before it, which the equipment reader reports, keeping its family"
                    )),
                },
                b"sprite" => self.equipment[at].sprite = string.to_vec(),
                b"model" => self.equipment[at].model = string.to_vec(),
                b"variant_name" => self.equipment[at].variant_name = string.to_vec(),
                b"derived_variant_name" => {
                    self.equipment[at].derived_variant_name = string.to_vec()
                }
                b"alias" => {
                    let aliases = match &field.value {
                        Value::Scalar(_) => std::slice::from_ref(field),
                        other => other.items().unwrap_or_default(),
                    };
                    for alias in aliases {
                        let text = alias.value.scalar().unwrap_or_default();
                        match Token::c_str(text) {
                            Token::Name(alias) => self.equipment[at].aliases.push(alias),
                            Token::Rejected => {
                                return Err(format!(
                                    "equipment {name} alias {:?} registers token 0; type registration indexes the text of that token, which the content does not give",
                                    String::from_utf8_lossy(text)
                                ));
                            }
                        }
                    }
                }
                b"visual_level" | b"visual_tech_level_addition" | b"air_map_icon_frame" => {
                    if let Scanned::Stored(value) = scan_i32(text) {
                        let equipment = &mut self.equipment[at];
                        match keyword.as_slice() {
                            b"visual_level" => equipment.visual_level = value,
                            b"air_map_icon_frame" => equipment.air_map_icon_frame = value,
                            _ => equipment.visual_tech_level_addition = value,
                        }
                    }
                }
                b"allowed_types" => {
                    self.equipment[at].allowed_types = category_list(&field.value, field.quoted());
                }
                b"type_override" => self.equipment[at].type_override = string.to_ascii_lowercase(),
                b"module_add_stats_multiplier" => {
                    StatGroup::read_entries(
                        &mut self.equipment[at].module_add_stats_multiplier,
                        field.value.items().unwrap_or_default(),
                    );
                }
                b"module_multiply_stats_multiplier" => {
                    StatGroup::read_entries(
                        &mut self.equipment[at].module_multiply_stats_multiplier,
                        field.value.items().unwrap_or_default(),
                    );
                }
                b"scaled_by_reliability" => {
                    StatGroup::read_entries(
                        &mut self.equipment[at].scaled_by_reliability,
                        field.value.items().unwrap_or_default(),
                    );
                }
                b"module_slots" => notes.extend(self.module_slots(at, field)),
                b"upgrades" => {
                    for upgrade in field.value.items().unwrap_or_default() {
                        let Some(value) = upgrade.value.scalar() else {
                            continue;
                        };
                        let shown = String::from_utf8_lossy(c_string(value));
                        match Token::c_str(value) {
                            Token::Name(name) if self.upgrade_names.contains(&name) => {
                                if self.equipment[at].upgrades.contains(&name) {
                                    notes.push(format!(
                                        "upgrade {shown} is listed again, which the equipment reader reports, skipping it"
                                    ));
                                } else {
                                    self.equipment[at].upgrades.push(name);
                                }
                            }
                            _ => notes.push(format!(
                                "upgrade {shown} is no known upgrade, which the equipment reader reports, skipping it"
                            )),
                        }
                    }
                }
                b"module_count_limit" => {
                    if let Some(limit) = ModuleCountLimit::read(field.value.items().unwrap_or_default()) {
                        let limits = &mut self.equipment[at].module_count_limits;
                        if let Some(previous) = limits.iter_mut().find(|entry| entry.key == limit.key) {
                            *previous = limit;
                        } else {
                            limits.push(limit);
                        }
                    }
                }
                b"type" => {
                    let tokens = match &field.value {
                        Value::Scalar(_) => std::slice::from_ref(field),
                        other => other.items().unwrap_or_default(),
                    };
                    for token in tokens {
                        let value = token.value.scalar().unwrap_or_default();
                        let mask = token_category(value, token.quoted());
                        if mask == 0 {
                            notes.push(format!(
                                "type {} is no equipment category; the category reader reports it",
                                String::from_utf8_lossy(value)
                            ));
                        }
                        self.equipment[at].category |= mask;
                    }
                }
                b"default_modules" => {
                    for entry in field.value.items().unwrap_or_default() {
                        let slot = c_string(entry.key.as_deref().unwrap_or_default());
                        let module = c_string(entry.value.scalar().unwrap_or_default()).to_vec();
                        let modules = &mut self.equipment[at].default_modules;
                        match modules
                            .iter_mut()
                            .find(|(known, _)| Token::register(known) == Token::register(slot))
                        {
                            Some((_, known)) => *known = module,
                            None => modules.push((slot.to_vec(), module)),
                        }
                    }
                }
                stat => {
                    if let Some(slot) = stat_index(stat) {
                        match script_fixed5(text) {
                            Some(value) => self.equipment[at].stats[slot] = value,
                            None => notes.push(format!(
                                "{} {} does not scan, so the stat keeps {}",
                                String::from_utf8_lossy(stat),
                                String::from_utf8_lossy(text),
                                self.equipment[at].stats[slot]
                            )),
                        }
                    }
                }
            }
            self.misreads.extend(
                notes
                    .into_iter()
                    .map(|what| format!("{path}: equipment {name}: {what}")),
            );
        }
        let metadata = stat_metadata();
        for forbidden in FORBIDDEN_RELIABILITY_SCALED_STATS {
            if let Some(entry) = self.equipment[at]
                .scaled_by_reliability
                .iter_mut()
                .find(|entry| entry.index == forbidden)
            {
                let stat_name = metadata
                    .get(forbidden)
                    .map_or("unknown", |(keyword, _, _)| *keyword);
                self.misreads.push(format!(
                    "{path}: equipment {name}: '{stat_name}' is not supported in 'scaled_by_reliability'"
                ));
                entry.value = 0;
            }
        }
        for alias in &self.equipment[at].aliases {
            self.by_name.insert(alias.clone(), at);
        }
        Ok(())
    }

    fn module_slots(&mut self, at: usize, field: &Item) -> Vec<String> {
        let mut notes = Vec::new();
        let equipment = &self.equipment[at];
        let source = if equipment.parent != at {
            Some(Some(equipment.parent))
        } else {
            match equipment.archetype {
                Some(archetype) if archetype == at => None,
                archetype => Some(archetype),
            }
        };
        let Some(entries) = field.value.items() else {
            if field.value_keyword().as_deref() == Some("inherit") {
                match source {
                    Some(source) => {
                        self.equipment[at].module_slots = source
                            .map(|source| self.equipment[source].module_slots.clone())
                            .unwrap_or_default();
                        self.equipment[at].module_slot_rules = source
                            .map(|source| self.equipment[source].module_slot_rules.clone())
                            .unwrap_or_default();
                    }
                    None => notes.push(
                        "module_slots = inherit with no parent yet, which the equipment reader reports, keeping its slots".to_owned(),
                    ),
                }
            }
            return notes;
        };
        for entry in entries {
            let slot = c_string(entry.key.as_deref().unwrap_or_default());
            let token = Token::register(slot);
            let shown = String::from_utf8_lossy(slot);
            let has = |slots: &[Vec<u8>], token: &Token| {
                slots.iter().any(|known| Token::register(known) == *token)
            };
            let held = has(&self.equipment[at].module_slots, &token);
            self.slot_trace.push(held);
            if held {
                notes.push(format!(
                    "module slot {shown} is defined again, which the slot reader reports, skipping it"
                ));
                continue;
            }
            let made = if let Some(fields) = entry.value.items() {
                Ok(ModuleSlot::read(fields))
            } else if entry.value_keyword().as_deref() == Some("inherit") {
                match source {
                    Some(Some(source)) if has(&self.equipment[source].module_slots, &token) => {
                        let index = self.equipment[source]
                            .module_slots
                            .iter()
                            .position(|name| Token::register(name) == token)
                            .unwrap();
                        Ok(self.equipment[source].module_slot_rules[index].clone())
                    }
                    Some(source) => Err(format!(
                        "inherits from {}, which has no module slot of that name",
                        source.map_or_else(
                            || "the null object".into(),
                            |source| String::from_utf8_lossy(&self.equipment[source].name)
                        )
                    )),
                    None => Err("inherits before a parent is specified".to_owned()),
                }
            } else {
                let alias = c_string(entry.value.scalar().unwrap_or_default());
                let alias_token = Token::register(alias);
                if !alias_token.is_rejected() && has(&self.equipment[at].module_slots, &alias_token)
                {
                    let index = self.equipment[at]
                        .module_slots
                        .iter()
                        .position(|name| Token::register(name) == alias_token)
                        .unwrap();
                    Ok(self.equipment[at].module_slot_rules[index].clone())
                } else {
                    Err(format!(
                        "copies undefined module slot {}",
                        String::from_utf8_lossy(alias)
                    ))
                }
            };
            match made {
                Ok(rule) => {
                    self.equipment[at].module_slots.push(slot.to_vec());
                    self.equipment[at].module_slot_rules.push(rule);
                }
                Err(why) => notes.push(format!(
                    "module slot {shown} {why}, which the slot reader reports, making no slot"
                )),
            }
        }
        notes
    }

    fn register(&mut self, name: Vec<u8>) -> usize {
        let at = self.equipment.len();
        self.by_name.insert(name.to_ascii_lowercase(), at);
        self.equipment.push(Equipment::new(name, at));
        at
    }

    fn duplicate(&mut self, path: &str, entries: &[Item]) -> Result<(), String> {
        let mut definitions: Vec<Duplicate> = Vec::new();
        for entry in entries {
            let key = content_reader_parser::parser::c_string(entry.key.as_deref().unwrap_or_default());
            let token = Token::register(key);
            let at = match definitions.iter().position(|known| known.token == token) {
                Some(at) => at,
                None => {
                    definitions.push(Duplicate {
                        token,
                        name: key.to_vec(),
                        archetype: None,
                        only_archetype: false,
                        no_slots: false,
                        category: 0,
                        sprite: Vec::new(),
                        operations: Vec::new(),
                    });
                    definitions.len() - 1
                }
            };
            read_duplicate(
                &mut definitions[at],
                entry.value.items().unwrap_or_default(),
            );
        }
        for definition in definitions {
            let shown = String::from_utf8_lossy(&definition.name).into_owned();
            let (asked, archetype) = match &definition.archetype {
                None | Some(Token::Rejected) => {
                    return Err(format!(
                        "duplicate archetype {shown} names no archetype, or one whose text is empty or starts with a digit or `-`, so its archetype token is 0; duplicate lookup uses the text of token 0, which the content does not give"
                    ));
                }
                Some(Token::Name(name)) => (
                    String::from_utf8_lossy(name).into_owned(),
                    self.by_name.get(name).copied(),
                ),
            };
            match archetype {
                None => self.misreads.push(format!(
                    "{path}: duplicate archetype {shown}: asked to duplicate {asked}, which is not a known archetype and generates nothing"
                )),
                Some(archetype) if !self.equipment[archetype].is_archetype => {
                    self.misreads.push(format!(
                        "{path}: duplicate archetype {shown}: asked to duplicate {asked}, which is not an archetype and generates nothing"
                    ))
                }
                Some(_) if definition.token.is_rejected() => {
                    return Err(format!(
                        "duplicate archetype {shown} has a name that is empty or starts with a digit or `-`, token 0; clone generation prefixes the text of that token to each clone's name, which the content does not give"
                    ));
                }
                Some(archetype) => self.generate(path, &definition, archetype),
            }
        }
        Ok(())
    }

    fn generate(&mut self, path: &str, definition: &Duplicate, archetype: usize) {
        let mut originals: Vec<usize> = if definition.only_archetype {
            vec![archetype]
        } else {
            std::iter::once(archetype)
                .chain((0..self.equipment.len()).filter(|&at| {
                    at != archetype && self.equipment[at].archetype == Some(archetype)
                }))
                .collect()
        };
        self.hierarchy_sort(&mut originals, archetype);
        originals.retain(|&at| !self.equipment[at].duplicate);
        let prefix = self.equipment[archetype].name.clone();
        let mut clones: BTreeMap<usize, usize> = BTreeMap::new();
        for original in originals {
            let name = self.equipment[original].name.clone();
            let common = name.iter().zip(&prefix).take_while(|(a, b)| a == b).count();
            let generated = [definition.name.as_slice(), &name[common..]].concat();
            if self.by_name.contains_key(&generated.to_ascii_lowercase()) {
                self.misreads.push(format!(
                    "{path}: duplicate archetype {}: asked to duplicate {} into {}, which is already defined and is skipped",
                    String::from_utf8_lossy(&definition.name),
                    String::from_utf8_lossy(&name),
                    String::from_utf8_lossy(&generated)
                ));
                continue;
            }
            let clone = self.register(generated);
            let name = std::mem::take(&mut self.equipment[clone].name);
            let source = &self.equipment[original];
            let mut copy = Equipment {
                name,
                ..source.clone()
            };
            copy.aliases.clear();
            if definition.no_slots {
                copy.module_slots.clear();
                copy.module_slot_rules.clear();
                copy.default_modules.clear();
                copy.allowed_types = 0;
            }
            copy.archetype = Some(
                source
                    .archetype
                    .and_then(|link| clones.get(&link).copied())
                    .unwrap_or(clone),
            );
            copy.is_archetype = copy.archetype == Some(clone);
            copy.parent = clones.get(&source.parent).copied().unwrap_or(clone);
            copy.family = original;
            copy.category = definition.category;
            if !definition.sprite.is_empty() {
                copy.sprite.clone_from(&definition.sprite);
            }
            copy.duplicate = true;
            for &(stat, operation, value) in &definition.operations {
                let current = copy.stats[stat];
                copy.stats[stat] = match operation {
                    Operation::Set => value,
                    Operation::Add => current.wrapping_add(value),
                    Operation::Multiply => current.wrapping_mul(value) / 100_000,
                };
            }
            self.equipment[clone] = copy;
            clones.insert(original, clone);
        }
    }

    fn hierarchy_sort(&self, types: &mut [usize], archetype: usize) {
        for at in 1..types.len() {
            let mut to = at;
            while to > 0 && self.before(types[to], types[to - 1], archetype) {
                types.swap(to, to - 1);
                to -= 1;
            }
        }
    }

    fn before(&self, a: usize, b: usize, archetype: usize) -> bool {
        if self.equipment[a].duplicate {
            return false;
        }
        if self.equipment[b].duplicate {
            return true;
        }
        let (mut a, mut b) = (Some(a), Some(b));
        for _ in 0..64 {
            if a == b {
                return false;
            }
            let (Some(at_a), Some(at_b)) = (a, b) else {
                return a.is_none();
            };
            if at_a == archetype {
                return true;
            }
            if at_b == archetype {
                return false;
            }
            a = self.next(at_a);
            b = self.next(at_b);
        }
        false
    }

    fn finalize(&mut self) {
        if let Some(cycle) = self.cycle() {
            self.lost = Some(cycle);
            return;
        }
        for at in 0..self.equipment.len() {
            let mut priority = self.equipment[at].visual_tech_level_addition;
            let mut depth = 0_i32;
            let mut parent = at;
            while self.equipment[parent].parent != parent {
                parent = self.equipment[parent].parent;
                priority = priority.wrapping_add(self.equipment[parent].visual_tech_level_addition);
                depth = depth.wrapping_add(1);
            }
            let mask = self.equipment[at]
                .default_modules
                .iter()
                .filter_map(|(_, module)| self.module_category(module))
                .fold(0, |mask, module| mask | module);
            let equipment = &mut self.equipment[at];
            equipment.model_priority = priority.max(1);
            equipment.depth = depth;
            equipment.module_category = mask;
        }
    }

    fn next(&self, at: usize) -> Option<usize> {
        let equipment = &self.equipment[at];
        if equipment.parent != at {
            return Some(equipment.parent);
        }
        equipment.archetype.filter(|&archetype| archetype != at)
    }

    fn cycle(&self) -> Option<String> {
        let mut state = vec![0_u8; self.equipment.len()];
        for start in 0..self.equipment.len() {
            let mut walk = Vec::new();
            let mut at = Some(start);
            while let Some(current) = at.filter(|&current| state[current] != 2) {
                if state[current] == 1 {
                    let from = walk.iter().position(|&seen| seen == current).unwrap_or(0);
                    let names: Vec<_> = walk[from..]
                        .iter()
                        .chain(std::iter::once(&current))
                        .map(|&seen| String::from_utf8_lossy(&self.equipment[seen].name))
                        .collect();
                    return Some(format!(
                        "equipment {} link back through their parents and archetypes; equipment finalization follows these links with no cycle guard, so the game would never finish loading this content",
                        names.join(" -> ")
                    ));
                }
                state[current] = 1;
                walk.push(current);
                at = self.next(current);
            }
            for seen in walk {
                state[seen] = 2;
            }
        }
        None
    }

    pub fn category(&self, equipment: &Equipment) -> u64 {
        normalize_category(self.primary_category(equipment) | equipment.module_category)
    }

    pub fn designer_families(&self, equipment: &Equipment) -> Result<[bool; 3], String> {
        let mut families = [false; 3];
        let mut seen: Vec<usize> = Vec::new();
        let mut current = equipment;
        loop {
            let mask = self.category(current);
            for (family, bits) in
                families
                    .iter_mut()
                    .zip([0x0040_8000_003c_u64, 0x1f00_37fc00, 0x8000_4003c1])
            {
                *family |= mask & bits != 0;
            }
            let Some(at) = current.archetype else {
                break;
            };
            let next = &self.equipment[at];
            if std::ptr::eq(current, next) {
                break;
            }
            if seen.contains(&at) {
                return Err(format!(
                    "equipment {} reaches archetype {} again through its archetypes, so its designer families never finish",
                    String::from_utf8_lossy(&equipment.name),
                    String::from_utf8_lossy(&next.name)
                ));
            }
            seen.push(at);
            current = next;
        }
        Ok(families)
    }

    pub fn primary_category(&self, equipment: &Equipment) -> u64 {
        match equipment.category {
            0 => equipment
                .archetype
                .map_or(0, |archetype| self.equipment[archetype].category),
            own => own,
        }
    }

    pub fn modules_add(&self, bits: u64) -> bool {
        self.modules.iter().any(|(_, _, mask)| mask & bits != 0)
    }

    pub fn module(&self, name: impl AsRef<[u8]>) -> Option<&[u8]> {
        tokens::first(
            self.modules
                .iter()
                .map(|(token, name, _)| (token, name.as_slice())),
            &Token::c_str(name.as_ref()),
        )
    }

    pub fn module_category(&self, name: impl AsRef<[u8]>) -> Option<u64> {
        tokens::first(
            self.modules.iter().map(|(token, _, mask)| (token, *mask)),
            &Token::c_str(name.as_ref()),
        )
    }

    pub fn group_admits(&self, members: &[Vec<u8>], at: usize) -> bool {
        let equipment = &self.equipment[at];
        let archetype = equipment
            .archetype
            .map(|archetype| &self.equipment[archetype]);
        let names: [&[u8]; 3] = [
            &equipment.name,
            archetype.map_or(&[], |archetype| archetype.name.as_slice()),
            archetype.map_or(&[], |archetype| {
                self.equipment[archetype.family].name.as_slice()
            }),
        ];
        if members
            .iter()
            .any(|member| names.iter().any(|name| member.eq_ignore_ascii_case(name)))
        {
            return true;
        }
        let mask = self.category(equipment);
        members
            .iter()
            .any(|member| category_mask(member).unwrap_or(0) & mask != 0)
    }
}

pub fn normalize_category(mut mask: u64) -> u64 {
    if mask & 0x80_0040_03C1 != 0 {
        if mask & 0x40 != 0 {
            mask &= !0x181;
        } else if mask & 0x80 != 0 {
            mask &= !0x101;
        } else if mask & 0x100 != 0 {
            mask &= !0x1;
        }
    }
    mask
}

fn read_duplicate(definition: &mut Duplicate, fields: &[Item]) {
    for field in fields {
        let Some(keyword) = field.keyword().map(|keyword| keyword.folded()) else {
            continue;
        };
        match keyword.as_slice() {
            b"archetype" => {
                definition.archetype = field.value.scalar().map(Token::c_str);
            }
            b"only_duplicate_archetype" => {
                if let Some(value) = field.value.scalar().and_then(content_reader_parser::parser::read_bool) {
                    definition.only_archetype = value;
                }
            }
            b"module_slots"
                if !field.quoted()
                    && field
                        .value
                        .scalar()
                        .is_some_and(|value| c_string(value).eq_ignore_ascii_case(b"none")) =>
            {
                definition.no_slots = true;
            }
            b"type" => definition.category = category_list(&field.value, field.quoted()),
            b"sprite" => {
                definition.sprite = field.value.read_string().unwrap_or(b"{").to_vec();
            }
            b"for_each" => {
                for entry in field.value.items().unwrap_or_default() {
                    let Some(stat) = entry
                        .keyword()
                        .and_then(|keyword| stat_index(&keyword.folded()))
                    else {
                        continue;
                    };
                    let Some(statement) = entry.value.items().and_then(<[Item]>::first) else {
                        continue;
                    };
                    let operation = match statement
                        .keyword()
                        .map(|keyword| keyword.folded())
                        .as_deref()
                    {
                        Some(b"set") => Operation::Set,
                        Some(b"multiply") => Operation::Multiply,
                        Some(b"add") => Operation::Add,
                        _ => continue,
                    };
                    let Some(value) = statement.value.scalar().and_then(script_fixed5) else {
                        continue;
                    };
                    match definition
                        .operations
                        .iter_mut()
                        .find(|(known, _, _)| *known == stat)
                    {
                        Some(known) => *known = (stat, operation, value),
                        None => definition.operations.push((stat, operation, value)),
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(feature = "test-support")]
pub mod fixture {
    use super::{Database, Groups};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub fn load(files: &[(&str, &str)]) -> (Database, Groups) {
        if !engine_executable::executable_tables::is_initialized() {
            content_reader_script::test_game::enter();
        }
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "hoi4_content_reader_equipment_fixture_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for (path, text) in files {
            let path = root.join(path);
            std::fs::create_dir_all(path.parent().expect("a content path has a folder"))
                .expect("the fixture folder can be made");
            std::fs::write(path, text).expect("the fixture file can be written");
        }
        let layers = content_reader_layers::ResolvedLayers::from_folders(&[(&root, &[][..])])
            .expect("the fixture folder is content");
        let loaded = (
            Database::load(&layers).expect("the equipment files read"),
            Groups::load(&layers).expect("the group files read"),
        );
        std::fs::remove_dir_all(&root).expect("the fixture folder can be removed");
        loaded
    }
}

#[cfg(test)]
mod tests;
