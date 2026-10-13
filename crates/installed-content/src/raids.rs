//! `common/raids`: raid categories (`categories/*.txt`) and raid types
//! (`*.txt`), loaded categories first. Types keep database order, which
//! decides their index and the order of every per-target type list.
//!
//! The map icon uses the type's nonempty `custom_map_icon`, otherwise
//! `GFX_raid_type_icon_<name>`, with `_<building template>` appended when
//! that sprite exists.
//!
//! The retained fields supply available-target icons and `allowed`, evaluated
//! in country scope when the player cache rebuilds its allowed list. An
//! empty `allowed` block allows. Each declaration updates the type's
//! persistent target selector, equipment pools, starting point, arrow and
//! unit model. Triggers accumulate, scalars overwrite, and failed conversions
//! keep their values. Reported and skipped fields change nothing and remain
//! in [`Catalog::misreads`]. Quoted trigger keys remain unsupported triggers.
//! A type with an indeterminate value is refused by name in
//! [`Catalog::unsupported`]; the rest of the catalogue still loads.

use std::{collections::BTreeMap, io};

pub mod names;
mod reader;
pub mod target;
pub mod tooltips;

use crate::{
    ResolvedLayers,
    fields::{block, invalid, key_str},
    parser::{self, Item, Value},
    triggers::{self, Triggers},
};

pub const RAIDS_DIR: &str = "common/raids";

/// Whether a folded bare key names an equipment type; `None` when the
/// equipment database was lost before it could hold the key.
pub type Equipment<'a> = &'a dyn Fn(&[u8]) -> Option<bool>;
pub const CATEGORIES_DIR: &str = "common/raids/categories";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidCategory {
    pub name: String,
    pub visible: Triggers,
    pub available: Triggers,
    /// The category filter always admits it.
    pub free_targeting: bool,
}

/// One arm of the type's target selector, selected by the target's scope.
/// Every `target_type` field reads into that same selector; the projection
/// keeps an arm only when it can match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetType {
    Provinces(Vec<u32>),
    Building(BuildingSelector),
    /// Explicit state ids, else the trigger block in a state-only scope.
    State {
        ids: Vec<u32>,
        triggers: Triggers,
    },
    CapturedArmyLeader,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildingSelector {
    pub types: Vec<String>,
    pub tags: Vec<String>,
    pub minimum_level: Option<i32>,
    pub maximum_level: Option<i32>,
    pub is_coastal: bool,
}

/// `starting_point.types` in source-kind order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    AirBase = 0,
    RocketSite = 1,
    NavalBase = 2,
    Carrier = 3,
    Submarine = 4,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StartingPoint {
    pub types: Vec<SourceKind>,
    pub building_types: Vec<String>,
    pub allow_faction_buildings: bool,
}

/// One `unit_requirements` block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnitRequirement {
    pub battalions: Vec<BattalionRequirement>,
    pub equipment: Vec<EquipmentRequirement>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BattalionRequirement {
    pub name: String,
    pub min: Option<i32>,
    pub max: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EquipmentRequirement {
    pub types: Vec<String>,
    /// Each list needs one of its modules on the variant.
    pub modules: Vec<Vec<String>>,
    pub min: Option<i32>,
    pub max: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidType {
    pub name: String,
    /// Text and chance inputs used only on hover. A refused text form does
    /// not remove the ordinary map icon.
    pub tooltip: tooltips::Definition,
    pub category: Option<String>,
    pub custom_map_icon: Option<String>,
    /// Whether the type is allowed for a country at all.
    pub allowed: Triggers,
    pub visible: Triggers,
    pub show_target: Triggers,
    pub available: Triggers,
    /// `launchable` and `launchable_from` are the additional execution
    /// triggers; `available` controls target availability. Each authored
    /// field updates its own persistent trigger list.
    pub launchable: Triggers,
    pub launchable_from: Triggers,
    pub fire_only_once: bool,
    /// `target_type = { province = any }`: such a type never becomes an
    /// offer; it is targeted through the raid interface.
    pub any_province: bool,
    pub target_types: Vec<TargetType>,
    /// `command_power`, fixed5.
    pub command_power_raw: i64,
    /// `essential_equipment` archetypes and amounts, in authored order.
    pub essential_equipment: Vec<(String, i32)>,
    /// `essential_equipment.nukes`.
    pub nukes: i32,
    /// `additional_equipment` archetypes, amounts and `nukes`, read like
    /// `essential_equipment`. Execution checks the raid's own pool against
    /// both lists.
    pub additional_equipment: Vec<(String, i32)>,
    pub additional_nukes: i32,
    /// `nuke_type = thermonuclear_bomb` selects the thermonuclear kind.
    pub thermonuclear: bool,
    pub starting_point: StartingPoint,
    pub unit_requirements: Vec<UnitRequirement>,
    /// `max_distance`, fixed5.
    pub max_distance_raw: Option<i64>,
    /// `range_factor`, fixed5.
    pub range_factor_raw: i64,
    pub unlimited_ai_range: bool,
    /// `arrow = { type = ... }` selects the drawn raid arrow; a rejected
    /// value keeps the previous kind.
    pub arrow: RaidArrowKind,
    /// `unit_model`, the raid's in-flight model.
    pub unit_model: RaidUnitModel,
    /// `speed_multiplier`, fixed5; its constructor value is 100000.
    pub speed_multiplier_raw: i64,
    /// `unit_animations`: `stage` blocks in authored order, appended across
    /// every declaration of the field.
    pub unit_animations: Vec<RaidAnimationStage>,
}

/// One `unit_animations` stage: `state`, `intro` and fixed5 `at_progress`
/// (0 unless given). Other keys are reader errors the game skips.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RaidAnimationStage {
    pub state: String,
    pub intro: Option<String>,
    pub at_progress_raw: i64,
}

/// Raid arrow kinds. The constructor selects line, integer value 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RaidArrowKind {
    /// 0: one straight segment.
    #[default]
    Line,
    /// 1: straight, cut into segments and lifted by the ballistic callback.
    Ballistic,
    /// 2: straight, cut into segments and lifted by the air callback.
    Air,
    /// 3: the naval raid path.
    Naval,
    /// 4: the land raid path.
    Land,
}

impl RaidArrowKind {
    /// The folded bare value keyword for each kind; a quoted value has
    /// no keyword identity and selects none.
    pub fn parse(keyword: &str) -> Option<Self> {
        Some(match keyword {
            "line" => Self::Line,
            "ballistic" => Self::Ballistic,
            "air" => Self::Air,
            "naval" => Self::Naval,
            "land" => Self::Land,
            _ => return None,
        })
    }

    /// The `maparrows.txt` arrow type each raid arrow draws with.
    pub fn arrow_type(self) -> &'static str {
        match self {
            Self::Line => "active_raid_line",
            Self::Ballistic => "active_raid_ballistic",
            Self::Air => "active_raid_air",
            Self::Naval => "active_raid_naval",
            Self::Land => "active_raid_land",
        }
    }

    pub const ALL: [Self; 5] = [
        Self::Line,
        Self::Ballistic,
        Self::Air,
        Self::Naval,
        Self::Land,
    ];
}

/// `unit_model` constructor values: no category, empty names, start
/// offset 0 and fixed5 scale 100000.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RaidUnitModel {
    /// `equipment`: the value token's category mask. Unknown tokens store
    /// 0, and any mask outside the air categories is cleared.
    pub equipment: u64,
    /// `entity`, tried with the country tag and culture prefixes first.
    pub entity: Option<String>,
    /// `default_entity`.
    pub default_entity: Option<String>,
    /// `start_offset`, fixed5 world units along the arrow.
    pub start_offset_raw: i64,
    /// `scale`, fixed5.
    pub scale_raw: i64,
}

impl Default for RaidUnitModel {
    fn default() -> Self {
        Self {
            equipment: 0,
            entity: None,
            default_entity: None,
            start_offset_raw: 0,
            scale_raw: 100_000,
        }
    }
}

impl RaidType {
    /// The sprite key before the building-template suffix. An authored
    /// override is used only when its string is nonempty.
    pub fn base_icon(&self) -> String {
        match &self.custom_map_icon {
            Some(icon) if !icon.is_empty() => icon.clone(),
            _ => format!("GFX_raid_type_icon_{}", self.name),
        }
    }

    /// The sprite key for a target building template; `known` says which
    /// sprites exist.
    pub fn icon(&self, template: Option<&str>, known: impl Fn(&str) -> bool) -> Option<String> {
        let base = self.base_icon();
        if let Some(template) = template.filter(|template| !template.is_empty()) {
            let specialised = format!("{base}_{template}");
            if known(&specialised) {
                return Some(specialised);
            }
        }
        known(&base).then_some(base)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Catalog {
    /// Database order.
    pub types: Vec<RaidType>,
    pub categories: BTreeMap<String, RaidCategory>,
    /// Type names declared more than once, each listed once. The later
    /// declaration resets and rereads the same type, but the database's
    /// ordered list then holds it twice. Its index is therefore not
    /// established and no declaration is used.
    pub refused: Vec<String>,
    /// Types refused for a field the port cannot read, with the reason, in
    /// database order.
    pub unsupported: Vec<(String, String)>,
    /// Each place the game reports and skips a field these readers take
    /// or its reading leaves the text, by file and line.
    pub misreads: Vec<crate::Diagnostic>,
    /// Every refused or unsupported type, which the database still holds and
    /// may offer: consumers must treat its offers as undecided, by name,
    /// rather than drop them.
    pub uncertain: Vec<UncertainType>,
}

/// A raid type the port cannot use, in first-declaration order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UncertainType {
    pub name: String,
    pub kind: crate::DiagnosticKind,
    pub reason: String,
    /// Each declaration, when every one parsed: the stored type is one of
    /// these. Empty when one could not be read, so nothing about it is known.
    pub declarations: Vec<RaidType>,
}

impl Catalog {
    pub fn get(&self, name: &str) -> Option<&RaidType> {
        self.types.iter().find(|kind| kind.name == name)
    }

    /// Each archetype some raid type lists as essential equipment, by name,
    /// with the largest amount any raid type requires of it: the stock
    /// StockpileArchetypes records, capped there. A type holds a repeated
    /// key once, with its last amount, as the native map does. A refused or
    /// unsupported type may list essential equipment the port does not see,
    /// so it leaves the caps unknown.
    pub fn essential_caps(
        &self,
        equipment: &crate::equipment::Database,
    ) -> Result<BTreeMap<Vec<u8>, i32>, String> {
        if let Some(name) = self.refused.first() {
            return Err(format!("raid type {name} is declared twice"));
        }
        if let Some((name, reason)) = self.unsupported.first() {
            return Err(format!("raid type {name} is unsupported: {reason}"));
        }
        let mut caps: BTreeMap<Vec<u8>, i32> = BTreeMap::new();
        for kind in &self.types {
            for (name, amount) in &kind.essential_equipment {
                let listed = equipment
                    .named(name)
                    .ok_or_else(|| format!("essential equipment {name} is not in the content"))?;
                let archetype = listed
                    .archetype
                    .map(|archetype| &equipment.equipment[archetype])
                    .ok_or_else(|| {
                        format!("essential equipment {name} has no archetype in the content")
                    })?;
                caps.entry(archetype.name.clone())
                    .and_modify(|cap| *cap = (*cap).max(*amount))
                    .or_insert(*amount);
            }
        }
        Ok(caps)
    }
}

/// A field's folded keyword; a quoted key has none.
fn keyword(item: &Item) -> Option<Vec<u8>> {
    item.keyword().map(parser::Keyword::folded)
}

/// The value token's keyword; a quoted value has none.
fn value_keyword(item: &Item) -> Option<String> {
    item.value_keyword()
}

fn fields(value: &Value) -> &[Item] {
    value.items().unwrap_or_default()
}

fn text<'a>(value: &'a Value, what: &str) -> io::Result<&'a str> {
    value
        .read_str()
        .map_err(|e| invalid(format!("{what}: {e}")))
}

/// Scan an integer into a slot that a failed scan leaves unchanged.
fn integer(value: &Value, slot: &mut i32) {
    if let crate::crt::Scanned::Stored(read) =
        crate::crt::scan_i32(value.scalar().unwrap_or_default())
    {
        *slot = read;
    }
}

/// Scan an integer into an uninitialized bounds value: a failed scan
/// leaves an indeterminate word, which the reader refuses by name.
fn indeterminate(value: &Value, what: &str) -> io::Result<i32> {
    match crate::crt::scan_i32(value.scalar().unwrap_or_default()) {
        crate::crt::Scanned::Stored(read) => Ok(read),
        _ => Err(invalid(format!(
            "{what}: unsupported indeterminate integer"
        ))),
    }
}

/// Exactly `yes` or `no`; any other value keeps the flag unchanged.
fn boolean(value: &Value, slot: &mut bool) {
    if let Some(read) = value.scalar().and_then(parser::read_bool) {
        *slot = read;
    }
}

/// Read fixed5 into a slot that a failed conversion leaves unchanged.
fn fixed(value: &Value, slot: &mut i64) {
    if let Some(read) = crate::fixed5::script_fixed5(value.scalar().unwrap_or_default()) {
        *slot = read;
    }
}

/// A list of names registered by their text, quoted or bare: the entries
/// of a `{` list, or the one value.
fn names(value: &Value, what: &str) -> io::Result<Vec<String>> {
    match value {
        Value::Block(items) => items
            .iter()
            .map(|item| text(&item.value, what).map(str::to_owned))
            .collect(),
        value => Ok(vec![text(value, what)?.to_owned()]),
    }
}

/// Each trigger field appends its children to one conjunction, retaining
/// token kinds for the shared evaluator (`dashboard::triggers`). A value
/// without a block is reported and adds no children.
fn append_trigger(out: &mut Triggers, value: &Value) -> io::Result<()> {
    out.extend(triggers::body(value).unwrap_or_default());
    Ok(())
}

/// Read one category declaration: triggers accumulate and
/// `free_targeting` accepts exactly `yes` or `no`.
fn category(name: &str, items: &[Item]) -> io::Result<RaidCategory> {
    let mut out = RaidCategory {
        name: name.to_owned(),
        ..RaidCategory::default()
    };
    for item in items {
        match keyword(item).as_deref() {
            Some(b"visible") => append_trigger(&mut out.visible, &item.value)?,
            Some(b"available") => append_trigger(&mut out.available, &item.value)?,
            Some(b"free_targeting") => boolean(&item.value, &mut out.free_targeting),
            _ => {}
        }
    }
    Ok(out)
}

/// An id the selector compares with a province or state id. A negative
/// stored id wraps to one no province or state has, as it matches none.
fn id(value: i32) -> u32 {
    value as u32
}

/// Scan an integer into a zero-initialized slot; a failed scan leaves 0.
fn scanned_i32(bytes: &[u8]) -> i32 {
    let mut out = 0;
    integer(&Value::Scalar(bytes.to_vec()), &mut out);
    out
}

/// A bounds pair: each `min`/`max` field sets its value and flag; the
/// object persists across the fields that read into it.
fn bounds(
    value: &Value,
    min: &mut Option<i32>,
    max: &mut Option<i32>,
    what: &str,
) -> io::Result<()> {
    for item in fields(value) {
        match keyword(item).as_deref() {
            Some(b"min") => *min = Some(indeterminate(&item.value, what)?),
            Some(b"max") => *max = Some(indeterminate(&item.value, what)?),
            _ => {}
        }
    }
    Ok(())
}

/// The type's persistent target selector after each `target_type` field:
/// - `province`: bare `any` sets the unrestricted flag; a block appends
///   integer-scanned ids; any other value appends one integer-scanned id;
/// - `state`: a block appends trigger children; bare `any` sets a flag that
///   state matching does not read; any other value appends a decimal id;
/// - `building`: `type` and `tags` append name lists, `level` updates its
///   bounds, and `is_coastal` accepts exactly `yes` or `no`;
/// - `captured_army_leader`: accepts exactly `yes` or `no`.
///
/// Other keys are reported and skipped, as noted by the reader.
fn target_types(items: &[Item], name: &str) -> io::Result<(Vec<TargetType>, bool)> {
    let what = format!("raid {name} target_type");
    let mut any_province = false;
    let mut provinces = Vec::new();
    let mut building = BuildingSelector::default();
    let (mut state_ids, mut state) = (Vec::new(), Triggers::default());
    let mut captured = false;
    for selector in items.iter().filter(|item| item.keyword_is("target_type")) {
        for item in fields(&selector.value) {
            let any = value_keyword(item).as_deref() == Some("any");
            match keyword(item).as_deref() {
                Some(b"province") if any => any_province = true,
                Some(b"province") => match &item.value {
                    Value::Block(entries) => provinces.extend(
                        entries
                            .iter()
                            .map(|entry| id(scanned_i32(entry.value.scalar().unwrap_or_default()))),
                    ),
                    value => provinces.push(id(scanned_i32(value.scalar().unwrap_or_default()))),
                },
                Some(b"state") if any => {}
                Some(b"state") => match &item.value {
                    Value::Block(_) => append_trigger(&mut state, &item.value)?,
                    value => state_ids.push(id(crate::fixed5::decimal_i32(
                        value.scalar().unwrap_or_default(),
                    ))),
                },
                Some(b"building") => {
                    for field in fields(&item.value) {
                        match keyword(field).as_deref() {
                            Some(b"type") => building.types.extend(names(&field.value, &what)?),
                            Some(b"tags") => building.tags.extend(names(&field.value, &what)?),
                            Some(b"level") => bounds(
                                &field.value,
                                &mut building.minimum_level,
                                &mut building.maximum_level,
                                &what,
                            )?,
                            Some(b"is_coastal") => boolean(&field.value, &mut building.is_coastal),
                            _ => {}
                        }
                    }
                }
                Some(b"captured_army_leader") => boolean(&item.value, &mut captured),
                _ => {}
            }
        }
    }
    // The building arm needs a type or tag; the province arm needs an id
    // or `any`; the state arm needs an id or a trigger child.
    let mut out = Vec::new();
    if !building.types.is_empty() || !building.tags.is_empty() {
        out.push(TargetType::Building(building));
    }
    if !provinces.is_empty() {
        out.push(TargetType::Provinces(provinces));
    }
    if !state_ids.is_empty() || !state.is_empty() {
        out.push(TargetType::State {
            ids: state_ids,
            triggers: state,
        });
    }
    if captured {
        out.push(TargetType::CapturedArmyLeader);
    }
    Ok((out, any_province))
}

/// Each `unit_requirements` field creates a new requirement. Its
/// `battalion_types` fields share one table: repeated tokens are reported
/// and their blocks skipped. Quoted keys cannot identify a sub-unit and
/// are refused by name. Each `equipment` field creates an entry: `type`
/// replaces its list, each `modules` adds a list, and `amount` sets bounds.
/// Other keys are reported and skipped.
fn unit_requirement(value: &Value, what: &str) -> io::Result<UnitRequirement> {
    let mut out = UnitRequirement::default();
    for item in fields(value) {
        match keyword(item).as_deref() {
            Some(b"battalion_types") => {
                for battalion in fields(&item.value) {
                    let Some(key) = keyword(battalion) else {
                        return Err(invalid(format!(
                            "{what}: unsupported quoted battalion type {:?}",
                            battalion.key_str().unwrap_or_default()
                        )));
                    };
                    if out
                        .battalions
                        .iter()
                        .any(|held| held.name.as_bytes().eq_ignore_ascii_case(&key))
                    {
                        continue;
                    }
                    let mut read = BattalionRequirement {
                        name: key_str(battalion, what)?.to_owned(),
                        ..BattalionRequirement::default()
                    };
                    bounds(&battalion.value, &mut read.min, &mut read.max, what)?;
                    out.battalions.push(read);
                }
            }
            Some(b"equipment") => {
                let mut read = EquipmentRequirement::default();
                for field in fields(&item.value) {
                    match keyword(field).as_deref() {
                        Some(b"type") => read.types = names(&field.value, what)?,
                        Some(b"modules") => read.modules.push(names(&field.value, what)?),
                        Some(b"amount") => {
                            bounds(&field.value, &mut read.min, &mut read.max, what)?
                        }
                        _ => {}
                    }
                }
                out.equipment.push(read);
            }
            _ => {}
        }
    }
    Ok(out)
}

/// The type's persistent starting point: `types` appends each list
/// token's source kind, reporting unknown kinds. `building_types` keeps
/// its tokens for [`resolve_launch_buildings`], and
/// `allow_faction_buildings` accepts exactly `yes` or `no`. Kind 5,
/// `building`, is an unmodelled source and is refused by name.
fn starting_point(items: &[Item], what: &str) -> io::Result<StartingPoint> {
    let mut out = StartingPoint::default();
    for point in items
        .iter()
        .filter(|item| item.keyword_is("starting_point"))
    {
        for field in fields(&point.value) {
            match keyword(field).as_deref() {
                Some(b"types") => {
                    for entry in fields(&field.value) {
                        out.types.push(match value_keyword(entry).as_deref() {
                            Some("air_base") => SourceKind::AirBase,
                            Some("rocket_site") => SourceKind::RocketSite,
                            Some("naval_base") => SourceKind::NavalBase,
                            Some("carrier") => SourceKind::Carrier,
                            Some("submarine") => SourceKind::Submarine,
                            Some("building") => {
                                return Err(invalid(format!(
                                    "{what}: unsupported starting point kind building"
                                )));
                            }
                            _ => continue,
                        });
                    }
                }
                Some(b"building_types") => out
                    .building_types
                    .extend(fields(&field.value).iter().filter_map(value_keyword)),
                Some(b"allow_faction_buildings") => {
                    boolean(&field.value, &mut out.allow_faction_buildings)
                }
                _ => {}
            }
        }
    }
    Ok(out)
}

/// The type's arrow reads `type` by its value token. A rejected token
/// is reported and keeps the previous kind; other keys are reported.
fn arrow(items: &[Item]) -> RaidArrowKind {
    let mut out = RaidArrowKind::default();
    for block in items.iter().filter(|item| item.keyword_is("arrow")) {
        for field in fields(&block.value) {
            if field.keyword_is("type")
                && let Some(kind) = value_keyword(field)
                    .as_deref()
                    .and_then(RaidArrowKind::parse)
            {
                out = kind;
            }
        }
    }
    out
}

/// The type's unit model reads names as strings, numeric fields as
/// fixed5 and `equipment` as the value token's category mask.
/// Other keys are reported.
fn unit_model(items: &[Item], what: &str) -> io::Result<RaidUnitModel> {
    let mut out = RaidUnitModel::default();
    for block in items.iter().filter(|item| item.keyword_is("unit_model")) {
        for field in fields(&block.value) {
            match keyword(field).as_deref() {
                Some(b"equipment") => {
                    let mask = value_keyword(field)
                        .and_then(crate::equipment::category_mask)
                        .unwrap_or(0);
                    out.equipment = if mask & 0x1F_0037_FC00 == mask {
                        mask
                    } else {
                        0
                    };
                }
                Some(b"entity") => out.entity = Some(text(&field.value, what)?.to_owned()),
                Some(b"default_entity") => {
                    out.default_entity = Some(text(&field.value, what)?.to_owned())
                }
                Some(b"start_offset") => fixed(&field.value, &mut out.start_offset_raw),
                Some(b"scale") => fixed(&field.value, &mut out.scale_raw),
                _ => {}
            }
        }
    }
    Ok(out)
}

/// Every `unit_animations` field appends its `stage` blocks; a stage
/// reads `state` and `intro` as strings and `at_progress` as fixed5.
fn unit_animations(items: &[Item], what: &str) -> io::Result<Vec<RaidAnimationStage>> {
    let mut out = Vec::new();
    for animations in items
        .iter()
        .filter(|item| item.keyword_is("unit_animations"))
    {
        for stage in fields(&animations.value)
            .iter()
            .filter(|item| item.keyword_is("stage"))
        {
            let mut read = RaidAnimationStage::default();
            for item in fields(&stage.value) {
                match keyword(item).as_deref() {
                    Some(b"state") => read.state = text(&item.value, what)?.to_owned(),
                    Some(b"intro") => read.intro = Some(text(&item.value, what)?.to_owned()),
                    Some(b"at_progress") => fixed(&item.value, &mut read.at_progress_raw),
                    _ => {}
                }
            }
            out.push(read);
        }
    }
    Ok(out)
}

/// Read one type declaration. Triggers accumulate, scalars overwrite
/// (failed conversions keep their values), and persistent sub-objects
/// take every field of their key: selector, equipment pools, starting
/// point, arrow and unit model.
fn raid_type(name: &str, items: &[Item], registry: Equipment<'_>) -> io::Result<RaidType> {
    let what = format!("raid {name}");
    let (target_types, any_province) = target_types(items, name)?;
    let mut kind = RaidType {
        name: name.to_owned(),
        tooltip: tooltips::read(items),
        any_province,
        target_types,
        starting_point: starting_point(items, &what)?,
        arrow: arrow(items),
        unit_model: unit_model(items, &what)?,
        unit_animations: unit_animations(items, &what)?,
        speed_multiplier_raw: 100_000,
        ..RaidType::default()
    };
    for item in items {
        match keyword(item).as_deref() {
            // Find the category by the value's folded text.
            Some(b"category") => kind.category = Some(text(&item.value, &what)?.to_owned()),
            Some(b"custom_map_icon") => {
                kind.custom_map_icon = Some(text(&item.value, &what)?.to_owned())
            }
            Some(b"allowed") => append_trigger(&mut kind.allowed, &item.value)?,
            Some(b"visible") => append_trigger(&mut kind.visible, &item.value)?,
            Some(b"show_target") => append_trigger(&mut kind.show_target, &item.value)?,
            Some(b"available") => append_trigger(&mut kind.available, &item.value)?,
            Some(b"launchable") => append_trigger(&mut kind.launchable, &item.value)?,
            Some(b"launchable_from") => append_trigger(&mut kind.launchable_from, &item.value)?,
            Some(b"fire_only_once") => boolean(&item.value, &mut kind.fire_only_once),
            Some(b"unlimited_ai_range") => boolean(&item.value, &mut kind.unlimited_ai_range),
            Some(b"command_power") => fixed(&item.value, &mut kind.command_power_raw),
            Some(b"range_factor") => fixed(&item.value, &mut kind.range_factor_raw),
            Some(b"speed_multiplier") => fixed(&item.value, &mut kind.speed_multiplier_raw),
            // Read fixed5 into a zeroed local, then assign it.
            Some(b"max_distance") => {
                let mut read = 0;
                fixed(&item.value, &mut read);
                kind.max_distance_raw = Some(read);
            }
            // `thermonuclear_bomb` selects the thermonuclear kind; other values do not.
            Some(b"nuke_type") => {
                kind.thermonuclear = value_keyword(item).as_deref() == Some("thermonuclear_bomb")
            }
            Some(b"essential_equipment") => equipment_pool(
                &item.value,
                registry,
                &mut kind.essential_equipment,
                &mut kind.nukes,
            )?,
            Some(b"additional_equipment") => equipment_pool(
                &item.value,
                registry,
                &mut kind.additional_equipment,
                &mut kind.additional_nukes,
            )?,
            Some(b"unit_requirements") => kind
                .unit_requirements
                .push(unit_requirement(&item.value, &what)?),
            _ => {}
        }
    }
    Ok(kind)
}

/// Read into one pool across every field of its key. `nukes` uses integer
/// scanning; a known equipment token sets its decimal amount, preserving
/// the position of an existing type. Other keys are reported by the reader.
/// Keys left undecided by an incomplete equipment database are refused.
fn equipment_pool(
    value: &Value,
    registry: Equipment<'_>,
    pool: &mut Vec<(String, i32)>,
    nukes: &mut i32,
) -> io::Result<()> {
    for item in fields(value) {
        let Some(key) = keyword(item) else {
            continue;
        };
        if key == b"nukes" {
            integer(&item.value, nukes);
            continue;
        }
        let Some(found) = registry(&key) else {
            return Err(invalid(format!(
                "equipment {}: the equipment database was lost before it, so whether its lookup succeeds is undecided",
                String::from_utf8_lossy(&key)
            )));
        };
        if found {
            let amount = crate::fixed5::decimal_i32(item.value.scalar().unwrap_or_default());
            match pool
                .iter_mut()
                .find(|(held, _)| held.as_bytes().eq_ignore_ascii_case(&key))
            {
                Some((_, held)) => *held = amount,
                None => pool.push((
                    String::from_utf8_lossy(parser::c_string(
                        item.key.as_deref().unwrap_or_default(),
                    ))
                    .into_owned(),
                    amount,
                )),
            }
        }
    }
    Ok(())
}

fn all<'a>(items: &'a [Item], key: &'a str) -> impl Iterator<Item = &'a Value> {
    items
        .iter()
        .filter(move |item| item.keyword_is(key))
        .map(|item| &item.value)
}

/// The `NDefines` used by raid arrow geometry, colour and unit movement.
/// Floats retain `f32` width; `speed_multiplier_raw` is fixed5 `i64`.
#[derive(Clone, Debug, PartialEq)]
pub struct RaidGraphicsDefines {
    /// `NMapMode.PREPARING/READY/ACTIVE_RAID_ARROW_COLOR`.
    pub preparing_color: [f32; 4],
    pub ready_color: [f32; 4],
    pub active_color: [f32; 4],
    /// `NGraphics.RAID_ARROW_BALLISTIC_*`.
    pub ballistic_shape: f32,
    pub ballistic_max_height: f32,
    pub ballistic_max_segment_length: f32,
    /// At least 1: smaller values are raised, including the integer
    /// getter's 0 for a missing or non-number value.
    pub ballistic_max_segments: i32,
    /// `NGraphics.RAID_ARROW_AIR_*`.
    pub air_height: f32,
    pub air_source_steepness: f32,
    pub air_source_length: f32,
    pub air_target_steepness: f32,
    pub air_target_length: f32,
    pub air_max_segment_length: f32,
    /// At least 1, with the same lower bound as the ballistic segment count.
    pub air_max_segments: i32,
    /// `NGraphics.RAID_ARROW_NAVAL_*` and `RAID_ARROW_LAND_*`.
    pub naval: PathArrowDefines,
    pub land: PathArrowDefines,
    /// `NGraphics.RAID_UNIT_ENTITY_BASE_SCALE` and `RAID_UNIT_ENTITY_OFFSET`.
    pub unit_base_scale: f32,
    pub unit_offset: [f32; 3],
    /// `NRaids.RAID_UNIT_SPEED_MULTIPLIER`, fixed5.
    pub speed_multiplier_raw: i64,
}

/// The path arrows' spline defines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathArrowDefines {
    /// At least 1: smaller values are raised, including the integer
    /// getter's 0 for a missing or non-number value.
    pub subdivisions: i32,
    pub sharp_turn_smoothness: f32,
    pub use_midpoints: bool,
}

impl RaidGraphicsDefines {
    pub fn read(defines: &crate::defines::Defines) -> io::Result<Self> {
        const GRAPHICS: &str = "NGraphics";
        let float = |name: &str| defines.number(GRAPHICS, name).map(|value| value as f32);
        // Segment and subdivision counts are raised to 1 when smaller.
        // The integer getter stores 0 for missing or non-number values;
        // a missing section remains an error.
        let at_least_one = |name: &str| defines.int32(GRAPHICS, name).map(|value| value.max(1));
        let floats = |section: &str, name: &str, count: usize| -> io::Result<Vec<f32>> {
            let values = defines.numbers(section, name)?;
            if values.len() != count {
                return Err(invalid(format!(
                    "NDefines.{section}.{name} has {} components, not {count}",
                    values.len()
                )));
            }
            Ok(values.into_iter().map(|value| value as f32).collect())
        };
        let color = |name: &str| -> io::Result<[f32; 4]> {
            let values = floats("NMapMode", name, 4)?;
            Ok([values[0], values[1], values[2], values[3]])
        };
        let path = |prefix: &str| -> io::Result<PathArrowDefines> {
            Ok(PathArrowDefines {
                subdivisions: at_least_one(&format!("{prefix}_SUBDIVISIONS"))?,
                sharp_turn_smoothness: float(&format!("{prefix}_SHARP_TURN_SMOOTHNESS"))?,
                use_midpoints: defines.boolean(GRAPHICS, &format!("{prefix}_USE_MIDPOINTS"))?,
            })
        };
        let offset = floats(GRAPHICS, "RAID_UNIT_ENTITY_OFFSET", 3)?;
        Ok(Self {
            preparing_color: color("PREPARING_RAID_ARROW_COLOR")?,
            ready_color: color("READY_RAID_ARROW_COLOR")?,
            active_color: color("ACTIVE_RAID_ARROW_COLOR")?,
            ballistic_shape: float("RAID_ARROW_BALLISTIC_SHAPE")?,
            ballistic_max_height: float("RAID_ARROW_BALLISTIC_MAX_HEIGHT")?,
            ballistic_max_segment_length: float("RAID_ARROW_BALLISTIC_MAX_SEGMENT_LENGTH")?,
            ballistic_max_segments: at_least_one("RAID_ARROW_BALLISTIC_MAX_SEGMENTS")?,
            air_height: float("RAID_ARROW_AIR_HEIGHT")?,
            air_source_steepness: float("RAID_ARROW_AIR_SLOPE_SOURCE_STEEPNESS")?,
            air_source_length: float("RAID_ARROW_AIR_SLOPE_SOURCE_LENGTH")?,
            air_target_steepness: float("RAID_ARROW_AIR_SLOPE_TARGET_STEEPNESS")?,
            air_target_length: float("RAID_ARROW_AIR_SLOPE_TARGET_LENGTH")?,
            air_max_segment_length: float("RAID_ARROW_AIR_MAX_SEGMENT_LENGTH")?,
            air_max_segments: at_least_one("RAID_ARROW_AIR_MAX_SEGMENTS")?,
            naval: path("RAID_ARROW_NAVAL")?,
            land: path("RAID_ARROW_LAND")?,
            unit_base_scale: float("RAID_UNIT_ENTITY_BASE_SCALE")?,
            unit_offset: [offset[0], offset[1], offset[2]],
            speed_multiplier_raw: defines.fixed5("NRaids", "RAID_UNIT_SPEED_MULTIPLIER")?,
        })
    }
}

/// Categories and types, each file already parsed, in load order.
pub fn parse(
    categories: &[parser::Document],
    types: &[parser::Document],
    equipment: Equipment<'_>,
) -> io::Result<Catalog> {
    let mut out = Catalog::default();
    for document in categories {
        // A container without a block is reported and entered nowhere;
        // the reader already noted it.
        for root in all(&document.items, "categories").filter(|root| root.items().is_some()) {
            for item in block(root, "raid categories")? {
                let name = key_str(item, "raid categories")?;
                let items = block(&item.value, name)?;
                // A repeated name resets the category and reads the later declaration
                // alone. Name tokens compare with the lexer's ASCII fold.
                out.categories
                    .retain(|held, _| !held.eq_ignore_ascii_case(name));
                out.categories
                    .insert(name.to_owned(), category(name, items)?);
            }
        }
    }
    let mut declared: Vec<String> = Vec::new();
    let mut declarations: BTreeMap<String, Vec<io::Result<RaidType>>> = BTreeMap::new();
    for document in types {
        // Type containers require a block, just as category containers do.
        for root in all(&document.items, "types").filter(|root| root.items().is_some()) {
            for item in block(root, "raid types")? {
                let name = key_str(item, "raid types")?;
                // Type names compare by folded token: a repeat in any case names
                // the same type.
                let first = match declared.iter().find(|seen| seen.eq_ignore_ascii_case(name)) {
                    Some(seen) => {
                        if !out.refused.contains(seen) {
                            out.refused.push(seen.clone());
                        }
                        seen.clone()
                    }
                    None => {
                        declared.push(name.to_owned());
                        name.to_owned()
                    }
                };
                let kind =
                    block(&item.value, name).and_then(|items| raid_type(name, items, equipment));
                match &kind {
                    Ok(kind) => out.types.push(kind.clone()),
                    Err(error) => out.unsupported.push((name.to_owned(), error.to_string())),
                }
                declarations.entry(first).or_default().push(kind);
            }
        }
    }
    let refused = &out.refused;
    let is_refused = |name: &str| refused.iter().any(|held| held.eq_ignore_ascii_case(name));
    out.types.retain(|kind| !is_refused(&kind.name));
    out.unsupported.retain(|(name, _)| !is_refused(name));
    for name in &declared {
        let refused = out.refused.contains(name);
        let unsupported = out.unsupported.iter().find(|(known, _)| known == name);
        if !refused && unsupported.is_none() {
            continue;
        }
        let parsed: Option<Vec<RaidType>> = declarations
            .remove(name)
            .unwrap_or_default()
            .into_iter()
            .map(Result::ok)
            .collect();
        out.uncertain.push(UncertainType {
            name: name.clone(),
            kind: if unsupported.is_some() {
                crate::DiagnosticKind::Unsupported
            } else {
                crate::DiagnosticKind::Refused
            },
            reason: match unsupported {
                Some((_, reason)) => format!("it cannot be read: {reason}"),
                None => "it is declared more than once".to_owned(),
            },
            declarations: parsed.unwrap_or_default(),
        });
    }
    // Find a type's category by its folded token.
    let names: Vec<String> = out.categories.keys().cloned().collect();
    for kind in out.types.iter_mut().chain(
        out.uncertain
            .iter_mut()
            .flat_map(|u| u.declarations.iter_mut()),
    ) {
        if let Some(category) = &mut kind.category
            && let Some(name) = names
                .iter()
                .find(|name| name.eq_ignore_ascii_case(category))
        {
            category.clone_from(name);
        }
        // Custom modifiers without configured formulas are reported and dropped.
        out.misreads
            .extend(kind.tooltip.misreads.iter().map(|note| {
                (
                    crate::DiagnosticKind::Misread,
                    format!("raid {}: {note}", kind.name),
                )
            }));
    }
    Ok(out)
}

pub fn read(layers: &ResolvedLayers) -> io::Result<Catalog> {
    // Where the equipment database cannot be followed to its end, the types
    // read before the loss are known and any other key is undecided.
    let database = crate::equipment::Database::load(layers)?;
    let types: std::collections::BTreeSet<Vec<u8>> = database
        .equipment
        .iter()
        .map(|kind| parser::c_string(&kind.name).to_ascii_lowercase())
        .collect();
    let complete = database.lost.is_none();
    let equipment = |key: &[u8]| {
        let found = types.contains(key);
        (found || complete).then_some(found)
    };
    let mut misreads = Vec::new();
    // Raids load after buildings.
    let names = layers.key_names()?;
    let mut documents = |folder: &str| -> io::Result<Vec<parser::Document>> {
        triggers::folder_files(layers, folder)?
            .iter()
            .map(|path| {
                let mut notes = Vec::new();
                let document =
                    reader::document(&layers.read(path)?, &equipment, &names, &mut notes)
                        .map_err(|error| invalid(format!("{path}: {error}")))?;
                misreads.extend(
                    notes
                        .into_iter()
                        .map(|(kind, note)| (kind, format!("{path}: {note}"))),
                );
                Ok(document)
            })
            .collect()
    };
    let categories = documents(CATEGORIES_DIR)?;
    let types = documents(RAIDS_DIR)?;
    let mut catalog = parse(&categories, &types, &equipment)?;
    misreads.append(&mut catalog.misreads);
    catalog.misreads = misreads;
    resolve_launch_buildings(&mut catalog, &crate::buildings::read(layers)?);
    Ok(catalog)
}

/// Resolve `building_types` against the building database, in database
/// order. A template is selected when its token or a tag appears in the
/// list, with ASCII case folding. Only provincial templates with province
/// cap at least 1 become launch buildings; state buildings are reported
/// and left out. Source enumeration uses these resolved types.
pub fn resolve_launch_buildings(catalog: &mut Catalog, buildings: &crate::buildings::Buildings) {
    let resolve = |kind: &mut RaidType, misreads: &mut Vec<crate::Diagnostic>| {
        let wanted = std::mem::take(&mut kind.starting_point.building_types);
        if wanted.is_empty() {
            return;
        }
        let held = |name: &str| wanted.iter().any(|want| want.eq_ignore_ascii_case(name));
        for name in &buildings.declaration_order {
            let building = &buildings.types[name];
            if !(held(name) || building.tags.iter().any(|tag| held(tag))) {
                continue;
            }
            if building.provincial() {
                kind.starting_point.building_types.push(name.clone());
            } else {
                misreads.push((
                    crate::DiagnosticKind::Misread,
                    format!(
                        "raid {}: state building {name} cannot be used as a raid source",
                        kind.name
                    ),
                ));
            }
        }
    };
    let mut misreads = Vec::new();
    for kind in catalog.types.iter_mut().chain(
        catalog
            .uncertain
            .iter_mut()
            .flat_map(|u| u.declarations.iter_mut()),
    ) {
        resolve(kind, &mut misreads);
    }
    catalog.misreads.extend(misreads);
}

#[cfg(test)]
mod tests;
