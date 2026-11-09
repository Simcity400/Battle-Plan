//! Shared equipment database from the recorded content stack.
//!
//! [`files`] lists direct files in `upgrades`, then `modules`, then root,
//! each in filename byte order. [`native`] reads `equipments`, `upgrades`,
//! `equipment_modules`, `search_filters`, and `duplicate_archetypes` with
//! their own field readers, retaining token consumption and file variables.
//! Other roots report errors and skip opening blocks.
//!
//! [`Database`] retains names, archetype/parent/family links, categories,
//! allowed types, slots, default modules, model inputs, and indexed stats.
//! Duplicate archetypes are generated as their file blocks are read.
//! Finalization after the last file derives module categories, model
//! priority, and parent depth.
//!
//! [`Groups`] reads `common/equipment_groups` and recursively expands member
//! lists; [`Database::group_admits`] shares its membership rules.

mod design;
mod validity;
pub use validity::{Assignment, CountKey, ModuleCountLimit, ModuleValidity};
mod groups;
pub use design::{
    ModuleConversion, ModuleDesign, ModuleXpRules, StatComponents, StatContribution, StatGroup,
    StatValue, UpgradeCurrency, UpgradeDesign, UpgradeXpRules, module_save_xp,
};

pub use groups::{GROUPS_DIRECTORY, Group, Groups};

use std::{
    collections::{BTreeMap, BTreeSet},
    io,
};

use crate::{
    ResolvedLayers,
    crt::{Scanned, scan_i32},
    fixed5::script_fixed5,
    key_names::KeyNames,
    parser::{
        Item, Value, c_string,
        native::{self, Field, Kind, Shape},
    },
    tokens::{self, Token},
};

/// The equipment database's root directory.
pub const DIRECTORY: &str = "common/units/equipment/";

/// Equipment-type fields the game 1.19.4 applies in a stat calculation that is not reproduced.
pub const UNREPRODUCED_STAT_FIELDS: [&str; 3] = [
    "module_add_stats_multiplier",
    "module_multiply_stats_multiplier",
    "scaled_by_reliability",
];

/// Direct `.txt` files in `upgrades`, then `modules`, then root, each in
/// filename byte order ([`crate::script_files::paths`]).
pub fn files(layers: &ResolvedLayers) -> io::Result<Vec<crate::script_files::ScriptPath>> {
    let mut files = Vec::new();
    for directory in [
        format!("{DIRECTORY}upgrades/"),
        format!("{DIRECTORY}modules/"),
        DIRECTORY.to_owned(),
    ] {
        files.extend(crate::script_files::paths(layers, &directory, Some(".txt"))?);
    }
    Ok(files)
}

/// Finds an executable category keyword with ASCII case folding.
pub fn category_mask(name: impl AsRef<[u8]>) -> Option<u64> {
    let name = name.as_ref();
    engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.category_names))
        .iter()
        .find(|(key, _)| name.eq_ignore_ascii_case(key.as_bytes()))
        .map(|(_, mask)| *mask)
}

/// Category keywords, or the bits without a keyword.
pub fn category_names(mask: u64) -> Result<Vec<&'static str>, u64> {
    let mut unknown = mask;
    let mut names = Vec::new();
    for &(name, bit) in engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.category_names)) {
        if bit != 0 && mask & bit != 0 {
            names.push(name);
            unknown &= !bit;
        }
    }
    if unknown == 0 {
        Ok(names)
    } else {
        Err(unknown)
    }
}

/// A value token's category as the token-id readers find it: a bare token
/// through the lexer's folded lookup; a quoted one is
/// no category.
fn token_category(text: &[u8], quoted: bool) -> u64 {
    if quoted {
        return 0;
    }
    category_mask(text).unwrap_or(0)
}

/// Unsigned decimal conversion accepts initial whitespace and one sign,
/// then requires a digit. Negative values wrap, overflow saturates, and
/// invalid text yields zero. Reads stop at the first NUL.
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

/// Clear the destination, then OR each category token from an opening
/// list or scalar value. Tokens with no category use unsigned decimal conversion.
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

/// Number of entries in the equipment stat block.
pub const STAT_COUNT: usize = 78;

/// Script keyword, localisation label and positive polarity by stat index.
pub fn stat_metadata() -> &'static [(&'static str, &'static str, bool)] {
    use engine_executable::executable_tables::{auxiliary_part, required};
    let stat_metadata = required(auxiliary_part(|tables| &tables.stat_metadata));
    stat_metadata
}

/// A stat keyword's index in the equipment stat block.
pub fn stat_index(keyword: &[u8]) -> Option<usize> {
    stat_metadata()
        .iter()
        .position(|(name, _, _)| name.as_bytes() == keyword)
}

/// Equipment mission names use their own native bit domain, independently of
/// the AI order enum. Unrecognized authored names contribute no bit.
pub fn mission_mask(name: &[u8]) -> u32 {
    let name = c_string(name);
    engine_executable::executable_tables::required(engine_executable::executable_tables::auxiliary_part(|tables| &tables.equipment_missions))
        .iter()
        .find(|(known, _)| known.as_bytes().eq_ignore_ascii_case(name))
        .map_or(0, |(_, mask)| *mask)
}

/// Equipment fields whose reader consumes one value token.
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

/// The readers of an equipment file.
#[derive(Clone, Copy, Debug)]
enum Node {
    Top,
    /// Each `equipments` key declares a type.
    Table,
    /// Equipment-type fields.
    Equipment,
    /// One-value entry loops for resources, terrain attrition, and default modules.
    Entries,
    /// The `module_slots` loop reads one slot at each key.
    Slots,
    /// Module-slot fields.
    Slot,
    /// Module-count limit fields.
    Limit,
    /// Modifier-stat fields.
    ModifierStat,
    /// Upgrade-window override fields.
    Override,
    /// Each `upgrades` key declares an upgrade.
    Upgrades,
    /// Upgrade fields.
    Upgrade,
    /// Number-token levels from 1 to 255 read triggers.
    LevelTriggers,
    /// Number-token levels from 1 to 255 read resource pools.
    LevelPools,
    /// Upgrade-cost fields.
    Cost,
    /// Indexed equipment stats.
    Stats,
    /// Each `equipment_modules` key declares a module.
    Modules,
    /// Module fields.
    Module,
    /// Module-conversion fields.
    Conversion,
    /// Each key declares a module-category mask.
    CategoryMasks,
    /// Each key names a slot and its token list.
    SlotCategories,
    /// Mission-stat helper fields.
    MissionStats,
    /// Equipment-filter fields.
    Filter,
    /// Each `duplicate_archetypes` key declares a duplicate definition.
    Duplicates,
    /// Duplicate-archetype fields.
    Duplicate,
    /// Its `variant_name` loop: every key one value.
    Pairs,
    /// Its `for_each` loop.
    ForEach,
    /// One `for_each` statement read as [`Shape::Pair`].
    Statement(Statement),
}

/// A `for_each` entry: a stat operation or `variant_name`
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Statement {
    Stat,
    Variant,
}

/// What a file's dispatch keeps while it reads.
#[derive(Default)]
struct Reading {
    slots: Slots,
    /// Every module slot field read so far in the file, counted as
    /// `Database::slot_trace` counts them.
    slot_fields: usize,
    /// A slot field's answer to whether its definition already has the
    /// slot where the parent's slots decide it, learnt by loading the file
    /// ([`Database::read_file`]).
    decided: BTreeMap<usize, bool>,
    /// The slot fields read as new slots before their answer was learnt.
    assumed: Vec<usize>,
}

/// Slots read by one equipment definition.
#[derive(Default)]
struct Slots {
    /// Slots by token, registered from first-NUL key text. Rejected names
    /// share token 0 ([`Token`]).
    names: Vec<Token>,
    /// Slots whose making depends on the parent's slots (a per-slot
    /// `inherit`, or an alias of such a slot), which this reading does not
    /// hold.
    maybe: Vec<Token>,
    /// `module_slots = inherit` copied the parent's slots.
    inherited: bool,
    /// The first slot whose reading depends on those names.
    undecided: Option<String>,
}

impl Slots {
    /// Existing slots report an error and skip opening blocks. New opening
    /// values read slot objects. `inherit` copies a source slot of the same
    /// name; aliases copy an existing named slot, otherwise report an error.
    /// When source slots determine whether a block already exists, `decided`
    /// supplies the answer; unknown slots are read as new and recorded in `assumed`.
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
        // The lexer never holds a rejected name, so its lookup finds no slot.
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

/// Whether a key is a number token whose integer conversion
/// is a level from 1 to 255.
fn level(field: &Field<'_>) -> bool {
    field.key.kind == Kind::Bare
        && crate::parser::number_token(&field.key.text, false)
        && (1..=255).contains(&crate::fixed5::decimal_i32(&field.key.text))
}

/// A key a reader reads as a stat, [`stat_metadata`].
fn is_stat(keyword: Option<&[u8]>) -> bool {
    keyword.is_some_and(|keyword| stat_index(keyword).is_some())
}

/// Each reader's shape per field.
fn shape(node: Node, field: &Field<'_>, reading: &mut Reading) -> Shape<Node> {
    let keyword = field.keyword();
    let keyword = keyword.as_deref();
    match node {
        // Dispatch the five root keywords; other roots report and skip.
        Node::Top => match keyword {
            Some(b"equipments") => Shape::Object(Node::Table),
            Some(b"upgrades") => Shape::Object(Node::Upgrades),
            Some(b"equipment_modules") => Shape::Object(Node::Modules),
            // Search filters use persistent object reads.
            Some(b"search_filters") => Shape::Object(Node::Filter),
            // Its loop only when the value slot is `{`, else error recovery.
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
        // Any other key returns at once, its value token read.
        Node::Override => Shape::One,
        Node::Upgrades => Shape::Object(Node::Upgrade),
        Node::Upgrade => match keyword {
            Some(b"level_requirements") if field.opens() => Shape::Object(Node::LevelTriggers),
            Some(b"resource_cost_thresholds") if field.opens() => Shape::Object(Node::LevelPools),
            Some(b"level_requirements" | b"resource_cost_thresholds") => Shape::Skip,
            Some(b"linear_cost") => Shape::Object(Node::Cost),
            // The value slot's id against `land`, `naval` and `air`, else
            // error recovery.
            Some(b"cost") => match field.value_keyword().as_deref() {
                Some(b"land" | b"naval" | b"air") => Shape::One,
                _ => Shape::Skip,
            },
            Some(b"abbreviation" | b"max_level") => Shape::One,
            // The three stat-accumulation blocks.
            Some(b"add_stats" | b"add_average_stats" | b"multiply_stats") => {
                Shape::Object(Node::Stats)
            }
            // Read and accumulate indexed stats.
            _ if is_stat(keyword) => Shape::One,
            // Equipment bonus fields.
            Some(b"attrition") => Shape::Object(Node::Entries),
            _ => Shape::Skip,
        },
        // Read an AND trigger body.
        Node::LevelTriggers if level(field) => Shape::Trigger,
        Node::LevelPools if level(field) => Shape::Object(Node::Entries),
        Node::LevelTriggers | Node::LevelPools => Shape::Skip,
        Node::Cost => match keyword {
            Some(b"cost_by_level" | b"cost_by_level_for_licensed_equipment") => Shape::One,
            _ => Shape::Skip,
        },
        Node::Stats if is_stat(keyword) => Shape::One,
        Node::Stats => Shape::Skip,
        // `limit` reads an AND trigger; other keys read module objects.
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
        // `name` retains the value slot; `values` reads tokens; other fields return.
        Node::Filter if matches!(keyword, Some(b"values")) => Shape::List,
        Node::Filter => Shape::One,
        // Each key reads an object.
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
            // Skip an opening block; `none` sets the flag, and other values report errors.
            Some(b"module_slots") if field.opens() => Shape::SkipIfOpen,
            Some(b"module_slots") if field.value.is_keyword("none") => Shape::One,
            Some(b"variant_name") if field.opens() => Shape::Object(Node::Pairs),
            Some(b"for_each") if field.opens() => Shape::Object(Node::ForEach),
            _ => Shape::Skip,
        },
        Node::Pairs => Shape::One,
        // Stat and `variant_name` entries each require one key/operator/value
        // statement in an opening block. Extra statements report errors and skip
        // to the close; scalar entries report errors.
        Node::ForEach => match keyword {
            Some(b"variant_name") => Shape::Pair(Node::Statement(Statement::Variant)),
            _ if is_stat(keyword) => Shape::Pair(Node::Statement(Statement::Stat)),
            _ => Shape::Skip,
        },
        // Stat operations `set`, `multiply`, and `add` use fixed5 scans.
        // Other stat fields are reported. `find_and_replace` reads string pairs
        // ([`Shape::Strings`]).
        Node::Statement(statement) => match (statement, keyword) {
            (Statement::Stat, Some(b"set" | b"multiply" | b"add")) => Shape::One,
            (Statement::Variant, Some(b"find_and_replace")) => Shape::Strings,
            _ => Shape::Skip,
        },
    }
}

/// Equipment field consumption:
/// - [`single_value_field`] fields consume one token;
/// - conversion and upgrade lists read through `}` regardless of opening value;
/// - category, mission, allowed-type, and alias lists open only on `{`;
/// - attrition, resource, and default-module loops read one-value entries;
/// - slot blocks use [`Slots::shape`], while `inherit` replaces source slots;
/// - modifier-stat, upgrade-window, and count-limit fields read objects;
/// - production and lend-lease conditions read AND triggers;
/// - other keys read indexed fixed5 stats or report and skip their blocks.
fn equipment_field(field: &Field<'_>, keyword: Option<&[u8]>, slots: &mut Slots) -> Shape<Node> {
    match keyword {
        Some(b"can_convert_from" | b"upgrades") => Shape::List,
        Some(
            b"type" | b"allow_mission_type" | b"forbid_mission_type" | b"allowed_types" | b"alias",
        ) => Shape::ListIfOpen,
        Some(b"attrition" | b"resources" | b"default_modules") => Shape::Object(Node::Entries),
        Some(b"module_slots") => {
            // Bare `inherit` folds, while quoted text is not the keyword.
            // Source slots are unavailable during this preliminary read.
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
        Some(b"can_be_produced" | b"can_be_lend_leased") => Shape::Trigger,
        Some(name) if single_value_field(name) => Shape::One,
        _ if is_stat(keyword) => Shape::One,
        _ => Shape::Skip,
    }
}

/// Module fields dispatch to stat-accumulation blocks or report and skip
/// unknown fields.
fn module_field(field: &Field<'_>, keyword: Option<&[u8]>) -> Shape<Node> {
    match keyword {
        Some(b"can_convert_from") => Shape::Object(Node::Conversion),
        // Stat loops require opening blocks; scalar values report errors.
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
        // Read the condition as an AND trigger.
        Some(b"allowed") => Shape::Trigger,
        Some(b"critical_parts") => Shape::List,
        Some(b"build_cost_resources" | b"dismantle_cost_resources") => Shape::Object(Node::Entries),
        Some(b"add_stats" | b"add_average_stats" | b"multiply_stats") => Shape::Object(Node::Stats),
        _ => Shape::Skip,
    }
}

/// Slot admission operands retained from each native slot read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModuleSlot {
    pub required: bool,
    /// Repeated category declarations append, including duplicate tokens.
    pub categories: Vec<Vec<u8>>,
}
impl ModuleSlot {
    fn read(fields: &[Item]) -> Self {
        let mut result = Self::default();
        for field in fields {
            if field.keyword_is("required") {
                if let Some(value) = field.value.scalar().and_then(crate::parser::read_bool) {
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

/// One equipment type as the file loop leaves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Equipment {
    /// First declared name bytes through NUL. Names compare and join byte by
    /// byte and are never decoded here.
    pub name: Vec<u8>,
    /// Folded aliases register in the name index after the body is read and
    /// are erased before redefinition. Unknown token-0 text refuses the database.
    aliases: Vec<Vec<u8>>,
    /// `is_archetype`.
    pub is_archetype: bool,
    /// `supply_truck`: logistics adds the supply system trucks.
    pub supply_truck: bool,
    /// ARCHETYPE; `None` the null object.
    pub archetype: Option<usize>,
    /// PARENT; itself by default.
    pub parent: usize,
    /// Family defaults to this type, follows authored `family`, and points to
    /// the source type for generated copies. Parent reads change family only
    /// from the null object, which is not used here.
    pub family: usize,
    /// Authored category flags.
    pub category: u64,
    /// Category flags derived from default modules during finalization.
    pub module_category: u64,
    /// `allowed_types`.
    pub allowed_types: u64,
    pub type_override: Vec<u8>,
    /// `default_modules`: each slot and its module, as the texts
    /// the reader registers. They are tokens to the
    /// game: a slot of the same [`Token`] is overwritten, keeping the first
    /// slot's text, and the module is found by its token.
    pub default_modules: Vec<(Vec<u8>, Vec<u8>)>,
    /// Module slots in declaration order, one per [`Token`].
    pub module_slots: Vec<Vec<u8>>,
    /// Required flags and accepted categories in the same slot order.
    pub module_slot_rules: Vec<ModuleSlot>,
    /// Local count limits; ancestor limits join at query time.
    pub module_count_limits: Vec<ModuleCountLimit>,
    /// An empty sprite inherits from its archetype when that field is read.
    pub sprite: Vec<u8>,
    /// The model string is retained across redefinition.
    pub model: Vec<u8>,
    /// `variant_name`: a localisation key naming the type's unnamed variants,
    /// retained across redefinition.
    pub variant_name: Vec<u8>,
    /// `derived_variant_name`: the key that takes precedence over
    /// `variant_name`, retained across redefinition.
    pub derived_variant_name: Vec<u8>,
    /// The visual level is retained across redefinition.
    pub visual_level: i32,
    /// Authored signed `air_map_icon_frame`, reset to zero on redefinition.
    pub air_map_icon_frame: i32,
    /// Authored `carrier_capable`; reset to false on redefinition.
    pub carrier_capable: bool,
    /// Tech-level addition defaults to one and reads a signed integer.
    pub visual_tech_level_addition: i32,
    /// Model priority sums tech-level additions for the type and all parents,
    /// with a minimum of one.
    pub model_priority: i32,
    /// The number of parents above the type.
    pub depth: i32,
    /// The fixed5 stat block, indexed by [`stat_metadata`].
    pub stats: [i64; STAT_COUNT],
    /// [`UNREPRODUCED_STAT_FIELDS`] this type reads, kept across redefinition.
    pub unreproduced_stat_fields: Vec<&'static str>,
    /// Generated by a duplicated archetype.
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
            unreproduced_stat_fields: Vec::new(),
            duplicate: false,
        }
    }

    /// The name as the port's text, or why it is not.
    pub fn name_str(&self) -> Result<&str, crate::parser::NotText> {
        crate::parser::text(&self.name)
    }
}

/// One duplicate archetype definition as its block reads it.
#[derive(Clone, Debug)]
struct Duplicate {
    /// The first-NUL definition key is registered as a token; repeated tokens
    /// read into the same definition.
    token: Token,
    /// The name prefix for each clone uses the first registered spelling of
    /// its token. Earlier folded-name registrations can change that spelling
    /// without changing the clones' folded identities.
    name: Vec<u8>,
    /// The archetype value token; `None` projects the default token 0.
    archetype: Option<Token>,
    /// `only_duplicate_archetype`.
    only_archetype: bool,
    /// `module_slots = none`.
    no_slots: bool,
    /// `type` category mask read.
    category: u64,
    /// A nonempty sprite replaces every clone's sprite.
    sprite: Vec<u8>,
    /// `for_each` stat operations: stat index, operation, value;
    /// one per stat, the last read replacing.
    operations: Vec<(usize, Operation, i64)>,
}

/// A stat operation applied to each generated clone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
    /// `set`: replace with the value.
    Set,
    /// `multiply`: stat times value over 100000, wrapping, the
    /// division truncating.
    Multiply,
    /// `add`: stat plus value.
    Add,
}

/// The equipment database in registration order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Database {
    /// Every equipment type in registration order, generated
    /// ones included.
    pub equipment: Vec<Equipment>,
    /// Definitions and aliases by ASCII-folded first-NUL name bytes.
    by_name: BTreeMap<Vec<u8>, usize>,
    /// Registered upgrade tokens, independent of equipment aliases.
    upgrade_names: BTreeSet<Vec<u8>>,
    upgrade_stats: BTreeMap<Vec<u8>, StatGroup>,
    upgrade_designs: BTreeMap<Vec<u8>, UpgradeDesign>,
    module_stats: BTreeMap<Vec<u8>, StatGroup>,
    module_designs: BTreeMap<Vec<u8>, ModuleDesign>,
    /// Modules in registration order after the null object, retaining token,
    /// first declared name, and added equipment categories.
    modules: Vec<(Token, Vec<u8>, u64)>,
    /// Module indices by folded name. Repeated modules reset and read again.
    module_names: BTreeMap<Vec<u8>, usize>,
    module_conditions: BTreeMap<Vec<u8>, ModuleConditions>,
    /// Each place a file's reading leaves the text, and what the loader
    /// reports, by file and line.
    pub misreads: Vec<String>,
    /// Where the port cannot follow the loading, and why: the database is
    /// unknown from there on (the rest of that file and the files after
    /// it).
    pub lost: Option<String>,
    /// Whether each module slot field of the file being loaded, in reading
    /// order, found its definition already holding the slot.
    slot_trace: Vec<bool>,
}

/// Module admission gates evaluated in a fresh country scope: block limit,
/// then allowed.
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
    /// Authored module stat groups under the resolved first-registered token.
    pub fn module_stats(&self, name: impl AsRef<[u8]>) -> Option<&StatGroup> {
        self.module_stats
            .get(&self.module(name)?.to_ascii_lowercase())
    }
    /// Authored upgrade stat groups; names obey the upgrade token namespace.
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
    /// Reads every equipment file in the loader's order ([`files`]), then
    /// finalizes.
    pub fn load(layers: &ResolvedLayers) -> io::Result<Self> {
        layers.key_names()?.equipment_database().map(|database| database.as_ref().clone())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "key names have no equipment database"))
    }

    /// The same loader with names already resolved by the caller, for
    /// KeyNames' equipment roster without recursively loading KeyNames.
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

    /// [`Self::load`] for a consumer that needs the whole database: where
    /// the loading cannot be followed ([`Self::lost`]), the types after that
    /// point are unknown, so the database is refused with that reason.
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

    /// ASCII-folded equipment lookup through the first NUL; `None` is the null object.
    pub fn get(&self, name: impl AsRef<[u8]>) -> Option<usize> {
        self.by_name
            .get(&c_string(name.as_ref()).to_ascii_lowercase())
            .copied()
    }

    /// Registered type-name keys including aliases. Token lookup converts
    /// to text before using the same name lookup.
    pub(crate) fn registered_names(&self) -> impl Iterator<Item = &[u8]> {
        self.by_name.keys().map(Vec::as_slice)
    }

    /// Registered upgrade tokens.
    pub(crate) fn registered_upgrades(&self) -> impl Iterator<Item = &[u8]> {
        self.upgrade_names.iter().map(Vec::as_slice)
    }

    /// The type a name finds ([`Self::get`]).
    pub fn named(&self, name: impl AsRef<[u8]>) -> Option<&Equipment> {
        self.get(name).map(|at| &self.equipment[at])
    }

    /// Technology construction admits registered types, then rejects category
    /// bit 9 unless the corresponding feature is enabled. Category normalization
    /// matches `category`. Callers provide DLC-mask bit 7 at construction or load,
    /// rather than a later country's trigger state.
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

    /// Read one file with triggers resolved against `names`. Source slots from
    /// `module_slots = inherit` determine whether later slot blocks skip or read.
    /// Check assumed new slots against the loaded source and repeat the read
    /// until consumption agrees.
    fn read_file(&mut self, bytes: &[u8], path: &str, names: &KeyNames) {
        let mut decided = BTreeMap::new();
        loop {
            let mut reading = Reading {
                decided: decided.clone(),
                ..Reading::default()
            };
            // Each file uses lexer mode 0.
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

    /// Loads one reading of a file; the slot fields it read as new slots
    /// without an answer.
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
                // Apply the last block limit to every module declared in that block
                // after reading its definitions.
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

    /// Read module-added category masks. Repeated modules reset first. Names
    /// compare folded first-NUL text, and new modules register their tokens.
    fn load_module(&mut self, module: &Item) {
        let key = crate::parser::c_string(module.key.as_deref().unwrap_or_default());
        let mut mask = 0;
        self.module_stats.insert(
            key.to_ascii_lowercase(),
            StatGroup::read(module.value.items().unwrap_or_default(), false),
        );
        self.module_designs.insert(
            key.to_ascii_lowercase(),
            ModuleDesign::read(module.value.items().unwrap_or_default()),
        );
        let mut conditions = ModuleConditions::default(); // Reset clears both gates.
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

    /// Register new names before reading their bodies. Redefinition resets
    /// links, family, archetype flag, categories, allowed types, defaults, slots,
    /// sprite, and tech-level addition; it retains stats, model, and visual level.
    /// Then apply fields in order.
    fn load_equipment(&mut self, path: &str, entry: &Item) -> Result<(), String> {
        // Hash the key's first-NUL name bytes.
        let key = c_string(entry.key.as_deref().unwrap_or_default());
        let at = match self.get(key) {
            Some(at) => {
                let equipment = &mut self.equipment[at];
                // Erase all old aliases even when another type has overwritten
                // their name-index entries.
                for alias in equipment.aliases.drain(..) {
                    self.by_name.remove(&alias);
                }
                equipment.is_archetype = false;
                equipment.supply_truck = false;
                // Redefinition clears the generated flag, making the type clonable again.
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
                equipment.sprite.clear();
                equipment.air_map_icon_frame = 0;
                equipment.carrier_capable = false;
                equipment.visual_tech_level_addition = 1;
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
            // Parent and archetype names use first-NUL string reads and the shared
            // line-feed fallback, then folded type lookup. Unregistered names
            // resolve to the null object.
            let string = field.value.read_string().unwrap_or(b"{");
            let shown = String::from_utf8_lossy(string).into_owned();
            let named = self.get(string);
            let mut notes = Vec::new();
            match keyword.as_slice() {
                // Exactly `yes` or `no`, else
                // the value stays; set, the archetype is itself.
                b"supply_truck" => match text {
                    b"yes" => self.equipment[at].supply_truck = true,
                    b"no" => self.equipment[at].supply_truck = false,
                    _ => {}
                },
                b"carrier_capable" => {
                    if let Some(value) = crate::parser::read_bool(text) {
                        self.equipment[at].carrier_capable = value;
                    }
                }
                b"is_archetype" => {
                    if let Some(value) = crate::parser::read_bool(text) {
                        self.equipment[at].is_archetype = value;
                    }
                    if self.equipment[at].is_archetype {
                        self.equipment[at].archetype = Some(at);
                    }
                }
                // An archetype valid at this read supplies its current stats and,
                // when the target sprite is empty, its sprite.
                b"archetype" => match named.filter(|&found| self.equipment[found].is_archetype) {
                    Some(found) => {
                        let archetype = &self.equipment[found];
                        let (stats, sprite) = (archetype.stats, archetype.sprite.clone());
                        let equipment = &mut self.equipment[at];
                        equipment.archetype = Some(found);
                        equipment.stats = stats;
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
                // Any registered definition, and while the archetype is the
                // null object, the parent's archetype; no stats.
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
                // Any registered definition is accepted; other names are reported.
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
                // Alias tokens accumulate; the name index changes only after this body.
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
                // A failed signed-integer scan keeps the stored value.
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
                // The block is consumed as any skipped field; only its presence is kept.
                b"module_add_stats_multiplier"
                | b"module_multiply_stats_multiplier"
                | b"scaled_by_reliability" => {
                    let fields = &mut self.equipment[at].unreproduced_stat_fields;
                    if let Some(stat_field) = UNREPRODUCED_STAT_FIELDS
                        .into_iter()
                        .find(|known| known.as_bytes() == keyword.as_slice())
                        && !fields.contains(&stat_field)
                    {
                        fields.push(stat_field);
                    }
                }
                b"module_slots" => notes.extend(self.module_slots(at, field)),
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
                // Category tokens accumulate their masks. Quoted tokens and unknown
                // names contribute zero and report an error.
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
                // Register slot and module values by first-NUL text. Equal slot tokens
                // replace the module while retaining the first slot name. Rejected names
                // share token 0, so `1 = a 2 = b` leaves one slot holding `b`.
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
                // Successful fixed5 scans replace the stat; failed scans keep it.
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
        // Native operator[]: the last registration wins, including an
        // alias that shadows another type's original name.
        for alias in &self.equipment[at].aliases {
            self.by_name.insert(alias.clone(), at);
        }
        Ok(())
    }

    /// Read `module_slots` for `at`. `inherit` replaces slots from the parent
    /// unless it is the type itself, otherwise from a distinct archetype.
    /// The null object is a source with no slots. Missing sources report an
    /// error. Opening blocks read individual slots; other values are ignored.
    fn module_slots(&mut self, at: usize, field: &Item) -> Vec<String> {
        let mut notes = Vec::new();
        let equipment = &self.equipment[at];
        // The source both readers take: `Some(None)` the null object.
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
                // The slot of the same token in the source.
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
                // Folded value lookup cannot find a rejected lexer name.
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

    /// A new equipment type registered under `name`.
    fn register(&mut self, name: Vec<u8>) -> usize {
        let at = self.equipment.len();
        self.by_name.insert(name.to_ascii_lowercase(), at);
        self.equipment.push(Equipment::new(name, at));
        at
    }

    /// Read duplicate definitions by token, reusing repeated entries, then
    /// generate copies in order. Generation requiring token-0 text stays
    /// refused because content does not supply that text.
    fn duplicate(&mut self, path: &str, entries: &[Item]) -> Result<(), String> {
        let mut definitions: Vec<Duplicate> = Vec::new();
        for entry in entries {
            let key = crate::parser::c_string(entry.key.as_deref().unwrap_or_default());
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
            // Archetypes resolve through token text; the constructor leaves token 0.
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

    /// Order source-group types by hierarchy, remove duplicates, and clone
    /// each under the definition name plus the source suffix after its common
    /// prefix with the archetype. Archetype-only mode selects just that type.
    fn generate(&mut self, path: &str, definition: &Duplicate, archetype: usize) {
        // The source group starts with its archetype, then remaining matching
        // types in registration order.
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
            // Compare common prefixes exactly as bytes and keep the remaining
            // name bytes, even when the cut falls inside UTF-8.
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
            // Copy slots, defaults, sprite, model, visual fields, and stats.
            let mut copy = Equipment {
                name,
                ..source.clone()
            };
            // Aliases are cleared rather than copied.
            copy.aliases.clear();
            // `module_slots = none` clears the slots, the default modules
            // and the allowed types.
            if definition.no_slots {
                copy.module_slots.clear();
                copy.module_slot_rules.clear();
                copy.default_modules.clear();
                copy.allowed_types = 0;
            }
            // The links remapped through the clones made so far: the
            // archetype's own clone (the first) is its own archetype.
            copy.archetype = Some(
                source
                    .archetype
                    .and_then(|link| clones.get(&link).copied())
                    .unwrap_or(clone),
            );
            copy.is_archetype = copy.archetype == Some(clone);
            copy.parent = clones.get(&source.parent).copied().unwrap_or(clone);
            // Family points to the source type; categories use the definition's
            // `type`; the definition's own non-empty sprite replaces the
            // copied one.
            copy.family = original;
            copy.category = definition.category;
            if !definition.sprite.is_empty() {
                copy.sprite.clone_from(&definition.sprite);
            }
            copy.duplicate = true;
            // The `for_each` operations on the copied stats.
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

    /// Stable hierarchy ordering uses insertion sort with [`Self::before`].
    /// Larger groups use the same stable comparison.
    fn hierarchy_sort(&self, types: &mut [usize], archetype: usize) {
        for at in 1..types.len() {
            let mut to = at;
            while to > 0 && self.before(types[to], types[to - 1], archetype) {
                types.swap(to, to - 1);
                to -= 1;
            }
        }
    }

    /// Compare `a` with preceding `b`: duplicates never move ahead, while
    /// ordinary types move ahead of duplicates. Otherwise follow parent, then
    /// archetype links for at most 64 steps. `a` moves ahead when it reaches
    /// null or the archetype first, using the loader's test order.
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

    /// Finalize types in registration order. Parent/archetype cycles remain
    /// refused because the game would not finish loading. Own default modules
    /// add categories using the first module with each token; parent and
    /// archetype defaults are not inherited. Sum type and parent tech-level
    /// additions with wrapping arithmetic, clamp model priority to at least one,
    /// and count parents.
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

    /// Follow a distinct parent, otherwise a nonnull, distinct archetype.
    fn next(&self, at: usize) -> Option<usize> {
        let equipment = &self.equipment[at];
        if equipment.parent != at {
            return Some(equipment.parent);
        }
        equipment.archetype.filter(|&archetype| archetype != at)
    }

    /// Detect and name a loop in the inheritance walk.
    fn cycle(&self) -> Option<String> {
        // 0 unvisited, 1 on the current walk, 2 ends.
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

    /// Primary categories ([`Self::primary_category`]) combined with module
    /// categories and passed through [`normalize_category`].
    pub fn category(&self, equipment: &Equipment) -> u64 {
        normalize_category(self.primary_category(equipment) | equipment.module_category)
    }

    /// The first [`UNREPRODUCED_STAT_FIELDS`] entry read by the type, its parents or its
    /// archetypes, and the type that reads it.
    pub fn unreproduced_stat_field(&self, at: usize) -> Option<(&'static str, &[u8])> {
        let mut pending = vec![at];
        let mut seen = BTreeSet::new();
        while let Some(at) = pending.pop() {
            if !seen.insert(at) {
                continue;
            }
            let equipment = &self.equipment[at];
            if let Some(&field) = equipment.unreproduced_stat_fields.first() {
                return Some((field, equipment.name.as_slice()));
            }
            pending.extend(equipment.archetype);
            pending.push(equipment.parent);
        }
        None
    }

    /// Designer family predicates include every archetype in the chain. Each
    /// step uses that type's categories and its own default modules, rather
    /// than installed modules; parent-first cycle checks can miss loops in archetype-only chains.
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

    /// Use own categories, otherwise one archetype step. Parent categories
    /// are not inherited; the null object has none.
    pub fn primary_category(&self, equipment: &Equipment) -> u64 {
        match equipment.category {
            0 => equipment
                .archetype
                .map_or(0, |archetype| self.equipment[archetype].category),
            own => own,
        }
    }

    /// Whether any module's `add_equipment_type` has a bit of
    /// `bits`.
    pub fn modules_add(&self, bits: u64) -> bool {
        self.modules.iter().any(|(_, _, mask)| mask & bits != 0)
    }

    /// Resolve first-NUL token identity ([`Token::c_str`]) to the first
    /// registered module name; `None` projects the null object.
    pub fn module(&self, name: impl AsRef<[u8]>) -> Option<&[u8]> {
        tokens::first(
            self.modules
                .iter()
                .map(|(token, name, _)| (token, name.as_slice())),
            &Token::c_str(name.as_ref()),
        )
    }

    /// The resolved module's added equipment categories.
    pub fn module_category(&self, name: impl AsRef<[u8]>) -> Option<u64> {
        tokens::first(
            self.modules.iter().map(|(token, _, mask)| (token, *mask)),
            &Token::c_str(name.as_ref()),
        )
    }

    /// Expanded group members ([`Groups::expand`]) admit `at` by folded type,
    /// archetype, or archetype-family name. Otherwise a category keyword
    /// ([`category_mask`], without decimal conversion) must intersect its
    /// effective categories ([`Self::category`]). The null object has an empty name.
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

/// Capital ships clear convoy, submarine, and screen bits; otherwise
/// submarines clear convoy and screen bits, and screens clear convoy.
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

/// Apply duplicate-definition fields in order: archetype token, archetype-only
/// flag, slot-removal flag, category mask, sprite string, and stat operations.
/// `set`, `multiply`, and `add` use successful fixed5 scans, retaining only
/// the last operation for each stat.
fn read_duplicate(definition: &mut Duplicate, fields: &[Item]) {
    for field in fields {
        let Some(keyword) = field.keyword().map(|keyword| keyword.folded()) else {
            continue;
        };
        match keyword.as_slice() {
            // Register the value's first-NUL text as a token.
            b"archetype" => {
                definition.archetype = field.value.scalar().map(Token::c_str);
            }
            b"only_duplicate_archetype" => {
                if let Some(value) = field.value.scalar().and_then(crate::parser::read_bool) {
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

/// Fixtures for other crates' tests, through the real load path.
#[cfg(feature = "test-support")]
pub mod fixture {
    use super::{Database, Groups};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Writes `files` (content paths and their text) into a fresh folder
    /// under the temporary directory, loads the database and the groups as
    /// the game does through [`crate::ResolvedLayers::from_folders`], and
    /// removes the folder. Runtime metadata is initialized from `BATTLE_PLAN_GAME_DIR` first.
    pub fn load(files: &[(&str, &str)]) -> (Database, Groups) {
        engine_executable::executable_tables::initialize_for_tests()
            .expect("BATTLE_PLAN_GAME_DIR must select the supported installation for runtime table tests");
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "hoi4_installed_content_equipment_fixture_{}_{}",
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
        let layers = crate::ResolvedLayers::from_folders(&[(&root, &[][..])])
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
