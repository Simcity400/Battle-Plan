use dashboard_base::products::Country;
use dashboard_base::products::{LoadContext, ReplyWarnings, at_hour::{self, At, AtHour, Keys, Read, ReadsAt}};
use dashboard_base::{Source, catalogue::{self, Catalogue, View}};
use dashboard_follow::live::Source as Cell;
use dashboard_query::identity::Holder;
use dashboard_text::{
    metadata::link_slot,
    scripted_text::stored_text,
    variant_names::{self, Tags, Texts},
};
use engine_land_inputs::ncombat_log_cequipment_loss::month_bucket;
use replay_data::rows::{COMBAT_LOG_VARIANT_TYPE, CombatLogDroppedRow, CombatLogEquipmentRow, CombatLogMonthRow, OrdersGroupRow, Recorded, TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const ENEMY_LEDGER: u8 = 5;
const EQUIPMENT_SCALE: u32 = 100_000;
const HOUR_EPOCH: i64 = 43_800_000;
const MONTH_START_DAYS: [i64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];

const TABLES: [TableId; 13] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::LexicalTokens,
    TableId::VariantDefinitions,
    TableId::CombatLogGroups,
    TableId::CombatLogManpower,
    TableId::CombatLogMonths,
    TableId::CombatLogEquipment,
    TableId::CombatLogEnemyEquipment,
    TableId::OrdersGroups,
    TableId::SliceReads,
    TableId::CombatLogDropped,
];

const NOT_READ: &str = "Combat Log equipment of this hour was not read";
const NO_READ_YET: &str = "Combat Log equipment months of a country not read yet are unknown";
const AFTER_READ: &str = "Combat Log equipment months written after their country's latest read are unknown";
const UNWITNESSED: &str = "Combat Log equipment months whose rows the recording did not keep are unknown";
const DROPPED_UNRECORDED: &str = "Combat Log dropped equipment amount was not recorded";

const INDEPENDENT: &str = "independent";
const UNRESOLVED: &str = "unresolved";
const UNCLASSIFIED: &str = "unclassified";
const OTHER: &str = "other";

pub fn first_hour_of_month(month: i32) -> i32 {
    let m = i64::from(month);
    let year = m.div_euclid(12);
    let idx = m.rem_euclid(12) as usize;
    (HOUR_EPOCH + (year * 365 + MONTH_START_DAYS[idx]) * 24) as i32
}

pub fn month_hours(month: i32) -> (i32, i32) {
    (first_hour_of_month(month), first_hour_of_month(month + 1) - 1)
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LossStack {
    #[default]
    Channel,
    Type,
    ArmyGroup,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArmyLossesParams {
    pub branch_id: String,
    pub country: Country,
    #[serde(default)]
    pub view: LossView,
    #[serde(default)]
    pub stack: Option<LossStack>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", tag = "kind", try_from = "LossViewAsked")]
pub enum LossView {
    #[default]
    Manpower,
    Equipment { ledgers: Vec<u8> },
    Month { month: i32, #[serde(default)] ledgers: Option<Vec<u8>> },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LossViewAsked {
    kind: LossKind,
    ledgers: Option<Vec<u8>>,
    month: Option<i32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum LossKind {
    Manpower,
    Equipment,
    Month,
}

impl TryFrom<LossViewAsked> for LossView {
    type Error = &'static str;

    fn try_from(asked: LossViewAsked) -> Result<Self, Self::Error> {
        match (asked.kind, asked.ledgers, asked.month) {
            (LossKind::Manpower, _, _) => Ok(Self::Manpower),
            (LossKind::Equipment, Some(ledgers), _) => Ok(Self::Equipment { ledgers }),
            (LossKind::Month, ledgers, Some(month)) => Ok(Self::Month { month, ledgers }),
            (LossKind::Equipment, None, _) => Err("An equipment loss view names its ledgers"),
            (LossKind::Month, _, None) => Err("A month loss view names its month"),
        }
    }
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LossArmy {
    pub reference: String,
    pub ordinal: u32,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub parent: Option<String>,
    pub parent_name: Option<String>,
    pub parent_missing: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossPart {
    pub key: String,
    pub label: String,
    pub other_count: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossMonthBar {
    pub month: i32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub live_row: Option<String>,
    pub missing: Option<String>,
    pub values: Vec<String>,
    pub total: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossRollup {
    pub unit_scale: u32,
    pub wrap_i32: bool,
    pub parts: Vec<ArmyLossPart>,
    pub months: Vec<ArmyLossMonthBar>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossMonthDesign {
    pub name: Option<String>,
    pub archetype: Option<String>,
    pub archetype_name: Option<String>,
    pub amount_raw: Option<String>,
    pub live_row: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossMonthLedger {
    pub ledger: u8,
    pub kept: bool,
    pub missing: Option<String>,
    pub total_raw: Option<String>,
    pub live_row: Option<String>,
    pub designs: Vec<ArmyLossMonthDesign>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossMonthArmy {
    pub army: LossArmy,
    pub manpower: Option<[i32; 3]>,
    pub manpower_live_row: Option<String>,
    pub selected_total_raw: Option<String>,
    pub selected_live_row: Option<String>,
    pub selected_missing: Option<String>,
    pub ledgers: Vec<ArmyLossMonthLedger>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossMonthDetail {
    pub month: i32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub missing: Option<String>,
    pub armies: Vec<ArmyLossMonthArmy>,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossesAvailability {
    pub armies: Option<String>,
    pub manpower: Option<String>,
    pub months: Option<String>,
    pub own_equipment: Option<String>,
    pub enemy_equipment: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossesReply {
    pub tag: String,
    pub absent: bool,
    pub rollup: Option<ArmyLossRollup>,
    pub month_detail: Option<ArmyLossMonthDetail>,
    pub availability: ArmyLossesAvailability,
    pub warnings: Vec<String>,
}

impl ReplyWarnings for ArmyLossesReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

pub struct Content {
    catalogue: Result<Arc<Catalogue>, String>,
    units: Option<Arc<content_reader_key_names::equipment::Database>>,
}

pub struct ArmyLosses;

fn unavailable(frame: &DecodedFrame, id: TableId) -> Option<String> {
    match &*frame.tables[id.index()] {
        TableView::Complete(_) => None,
        TableView::Unavailable { reason } => Some(reason.clone()),
        TableView::NotRecorded { mode } => Some(format!("not recorded in a {mode} recording")),
        TableView::Truncated { .. } => Some("incomplete read".into()),
    }
}

fn group_name(row: &OrdersGroupRow, content: &Content) -> Result<String, String> {
    let bytes = row.name.require("name of orders groups")?.as_ref().ok_or("The army's name was not read")?;
    let catalogue = content.catalogue.as_ref().map_err(Clone::clone)?;
    stored_text(&catalogue.localisation, &catalogue.recorded_language, bytes, "Army name").and_then(|name| catalogue::plain_text(&name))
}

fn refuse(refused: &mut BTreeMap<&'static str, usize>, reason: &'static str) {
    *refused.entry(reason).or_default() += 1;
}

pub fn dropped<'a>(frame: &'a DecodedFrame, ours: &dyn Fn(u32) -> bool) -> Vec<&'a CombatLogDroppedRow> {
    frame.complete::<tables::CombatLogDropped>().map(|rows| rows.iter().filter(|row| ours(row.country)).collect()).unwrap_or_default()
}

fn recent_from(months: &[&CombatLogMonthRow]) -> BTreeMap<(u32, u32, u8), Vec<i32>> {
    let mut arrays = BTreeMap::<(u32, u32, u8), Vec<i32>>::new();
    for month in months {
        arrays.entry((month.country, month.group, month.kind)).or_default().push(month.date);
    }
    for dates in arrays.values_mut() {
        dates.sort_unstable();
    }
    arrays
}

#[derive(Debug, PartialEq, Eq)]
pub enum Witness<'a> {
    Listed(Vec<&'a CombatLogEquipmentRow>),
    Dropped(Vec<&'a CombatLogDroppedRow>),
    Empty,
    Unknown(&'static str),
}

pub fn witness<'a>(month: &CombatLogMonthRow, read: Option<i32>, listed: &'a [CombatLogEquipmentRow], dropped: &[&'a CombatLogDroppedRow], dates: &[i32]) -> Witness<'a> {
    let rows: Vec<&CombatLogEquipmentRow> = listed.iter().filter(|row| (row.country, row.group, row.kind, row.date) == (month.country, month.group, month.kind, month.date)).collect();
    if !rows.is_empty() {
        return Witness::Listed(rows);
    }
    let Some(read) = read else { return Witness::Unknown(NO_READ_YET) };
    if month.date > read {
        return Witness::Unknown(AFTER_READ);
    }
    let kept: Vec<&CombatLogDroppedRow> = dropped
        .iter()
        .copied()
        .filter(|row| (row.country, row.group, row.kind) == (month.country, month.group, month.kind) && matches!(row.date, Recorded::Value(date) if date == month.date))
        .collect();
    if !kept.is_empty() {
        return Witness::Dropped(kept);
    }
    let newer = dates[dates.partition_point(|&date| date <= month.date)..].iter().take_while(|&&date| date <= read).count();
    if newer <= 1 { Witness::Empty } else { Witness::Unknown(UNWITNESSED) }
}

#[derive(Clone, Debug)]
struct DesignInfo {
    name: Option<String>,
    archetype: Option<String>,
    archetype_name: Option<String>,
}

struct Designs<'f> {
    frame: &'f DecodedFrame,
    content: &'f Content,
    tags: Option<Tags<'f>>,
    cache: HashMap<u64, DesignInfo>,
    unnamed: BTreeSet<u64>,
}

impl<'f> Designs<'f> {
    fn new(frame: &'f DecodedFrame, content: &'f Content) -> Self {
        let tags = match (frame.complete::<tables::Countries>(), frame.complete::<tables::CountryLinks>()) {
            (Ok(countries), Ok(links)) => Some(Tags::new(countries, links, frame.complete::<tables::CountryAppearance>().ok())),
            _ => None,
        };
        Self { frame, content, tags, cache: HashMap::new(), unnamed: BTreeSet::new() }
    }

    fn design(&mut self, variant: u64) -> DesignInfo {
        if let Some(info) = self.cache.get(&variant) {
            return info.clone();
        }
        let info = self.read(variant);
        if info.name.is_none() {
            self.unnamed.insert(variant);
        }
        self.cache.insert(variant, info.clone());
        info
    }

    fn read(&self, variant: u64) -> DesignInfo {
        let (frame, content) = (self.frame, self.content);
        let (Some(tags), Ok(lexical), Ok(definitions)) = (&self.tags, frame.complete::<tables::LexicalTokens>(), frame.complete::<tables::VariantDefinitions>()) else {
            return DesignInfo { name: None, archetype: None, archetype_name: None };
        };
        let Ok(at) = definitions.binary_search_by_key(&variant, |row| row.id) else {
            return DesignInfo { name: None, archetype: None, archetype_name: None };
        };
        let raw = variant_names::raw_variant(&definitions[at], frame.hour, tags, lexical);
        let texts: Option<&dyn Texts> = content.catalogue.as_ref().ok().map(|catalogue| catalogue.as_ref() as &dyn Texts);
        let (name, _) = variant_names::variant_names(&raw, texts);
        let archetype = content.units.as_ref().and_then(|units| {
            let ordinal = units.named(&raw.token)?.archetype?;
            units.equipment.get(ordinal)?.name_str().ok().map(str::to_owned)
        });
        let archetype_name = archetype.as_deref().and_then(|key| content.catalogue.as_ref().ok()?.localised(key, View::Plain)).and_then(|text| catalogue::plain_text(&text).ok());
        DesignInfo { name, archetype, archetype_name }
    }
}

fn array_rows<'a>(rows: &'a [CombatLogEquipmentRow], array: (u32, u32, u8)) -> &'a [CombatLogEquipmentRow] {
    let start = rows.partition_point(|row| (row.country, row.group, row.kind) < array);
    let len = rows[start..].partition_point(|row| (row.country, row.group, row.kind) == array);
    &rows[start..start + len]
}

fn army_group_key_label(army: &LossArmy) -> (String, String) {
    match (&army.parent, &army.parent_missing) {
        (Some(parent), _) => (parent.clone(), army.parent_name.clone().unwrap_or_else(|| "Army group name unavailable".into())),
        (None, None) => (INDEPENDENT.into(), "Independent armies".into()),
        (None, Some(_)) => (UNRESOLVED.into(), "Army group not recorded".into()),
    }
}

fn add_value(left: i128, right: i128, wrap_i32: bool) -> i128 {
    if wrap_i32 {
        i128::from((left as i32).wrapping_add(right as i32))
    } else {
        left + right
    }
}

#[derive(Default)]
pub(super) struct RollupBuilder {
    unit_scale: u32,
    wrap_i32: bool,
    fixed_parts: Option<Vec<(String, String)>>,
    labels: BTreeMap<String, String>,
    recorded: BTreeSet<i32>,
    unknown: BTreeMap<i32, String>,
    entries: BTreeMap<i32, BTreeMap<String, (i128, Vec<Cell>)>>,
}

impl RollupBuilder {
    pub(super) fn new(unit_scale: u32, wrap_i32: bool, fixed_parts: Option<Vec<(String, String)>>) -> Self {
        let mut labels = BTreeMap::new();
        if let Some(parts) = &fixed_parts {
            for (k, v) in parts {
                labels.insert(k.clone(), v.clone());
            }
        }
        Self { unit_scale, wrap_i32, fixed_parts, labels, ..Self::default() }
    }

    pub(super) fn mark_recorded(&mut self, month: i32) {
        self.recorded.insert(month);
    }

    pub(super) fn mark_unknown(&mut self, month: i32, reason: String) {
        self.unknown.entry(month).or_insert(reason);
    }

    pub(super) fn add(&mut self, month: i32, key: String, label: String, amount: i128, cell: Option<Cell>) {
        self.recorded.insert(month);
        self.labels.entry(key.clone()).or_insert(label);
        let entry = self.entries.entry(month).or_default().entry(key).or_insert((0, Vec::new()));
        entry.0 = add_value(entry.0, amount, self.wrap_i32);
        if let Some(cell) = cell {
            entry.1.push(cell);
        }
    }

    pub(super) fn finish(self, limit: usize, live: &dashboard_follow::live::Cells) -> ArmyLossRollup {
        let mut totals: BTreeMap<String, i128> = BTreeMap::new();
        for (&month, map) in &self.entries {
            if self.unknown.contains_key(&month) {
                continue;
            }
            for (key, (val, _)) in map {
                *totals.entry(key.clone()).or_default() += val.abs();
            }
        }
        let ordered: Vec<String> = match &self.fixed_parts {
            Some(fixed) => fixed.iter().map(|(k, _)| k.clone()).collect(),
            None => {
                let mut keys: Vec<String> = totals.keys().cloned().collect();
                keys.sort_by(|a, b| totals[b].cmp(&totals[a]).then_with(|| a.cmp(b)));
                keys
            }
        };
        let (kept, folded): (Vec<String>, BTreeSet<String>) = if self.fixed_parts.is_none() && ordered.len() > limit {
            let keep = limit.saturating_sub(1);
            (ordered[..keep].to_vec(), ordered[keep..].iter().cloned().collect())
        } else {
            (ordered, BTreeSet::new())
        };
        let mut parts: Vec<ArmyLossPart> = kept
            .iter()
            .map(|key| ArmyLossPart { key: key.clone(), label: self.labels.get(key).cloned().unwrap_or_else(|| key.clone()), other_count: 0 })
            .collect();
        if !folded.is_empty() {
            parts.push(ArmyLossPart { key: OTHER.into(), label: format!("Other ({})", folded.len()), other_count: folded.len() as u32 });
        }
        let all_months: BTreeSet<i32> = self.recorded.iter().copied().chain(self.unknown.keys().copied()).collect();
        let mut months = Vec::with_capacity(all_months.len());
        for month in all_months {
            let (first_hour, last_hour) = month_hours(month);
            if let Some(reason) = self.unknown.get(&month) {
                months.push(ArmyLossMonthBar {
                    month,
                    first_hour,
                    last_hour,
                    live_row: None,
                    missing: Some(reason.clone()),
                    values: Vec::new(),
                    total: "0".into(),
                });
                continue;
            }
            let row_name = format!("r:{month}");
            let mut values = Vec::with_capacity(parts.len());
            let mut month_total: i128 = 0;
            let mut all_cells: Vec<Cell> = Vec::new();
            let shown = self.entries.get(&month);
            for (idx, part) in parts.iter().enumerate() {
                let mut part_val: i128 = 0;
                let mut part_cells: Vec<Cell> = Vec::new();
                if let Some(shown) = shown {
                    if part.key == OTHER {
                        for folded_key in &folded {
                            if let Some((val, cells)) = shown.get(folded_key) {
                                part_val = add_value(part_val, *val, self.wrap_i32);
                                part_cells.extend_from_slice(cells);
                            }
                        }
                    } else if let Some((val, cells)) = shown.get(&part.key) {
                        part_val = *val;
                        part_cells.extend_from_slice(cells);
                    }
                }
                if !part_cells.is_empty() {
                    live.sum(&row_name, &format!("p{idx}"), &part_cells, 1.0);
                    all_cells.extend_from_slice(&part_cells);
                }
                month_total = add_value(month_total, part_val, self.wrap_i32);
                values.push(part_val.to_string());
            }
            if !all_cells.is_empty() {
                live.sum(&row_name, "total", &all_cells, 1.0);
            }
            months.push(ArmyLossMonthBar {
                month,
                first_hour,
                last_hour,
                live_row: (!all_cells.is_empty()).then_some(row_name),
                missing: None,
                values,
                total: month_total.to_string(),
            });
        }
        ArmyLossRollup { unit_scale: self.unit_scale, wrap_i32: self.wrap_i32, parts, months }
    }
}

#[derive(Default)]
pub(super) struct MonthLedgerAcc {
    pub kept: bool,
    pub missing: Option<String>,
    pub total: i128,
    pub cells: Vec<Cell>,
    pub designs: Vec<ArmyLossMonthDesign>,
}

pub(super) fn summarize_selected_ledgers(
    army_ref: &str,
    wanted: Option<&[u8]>,
    detail_missing: Option<&str>,
    ledgers: &BTreeMap<u8, MonthLedgerAcc>,
    live: &dashboard_follow::live::Cells,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(wanted) = wanted else {
        return (None, None, None);
    };
    if let Some(reason) = detail_missing {
        return (None, None, Some(reason.to_owned()));
    }
    let matched: Vec<&MonthLedgerAcc> = wanted.iter().filter_map(|l| ledgers.get(l)).collect();
    if matched.is_empty() {
        return (None, None, Some("No equipment ledger row kept for this army in this month.".into()));
    }
    let mut total: i128 = 0;
    let mut cells: Vec<Cell> = Vec::new();
    for la in matched {
        if !la.kept || la.missing.is_some() {
            return (None, None, Some(la.missing.clone().unwrap_or_else(|| NOT_READ.into())));
        }
        total += la.total;
        cells.extend_from_slice(&la.cells);
    }
    let live_row = (!cells.is_empty()).then(|| {
        let row_name = format!("sel:{army_ref}");
        live.sum(&row_name, "amount", &cells, 1.0);
        row_name
    });
    (Some(total.to_string()), live_row, None)
}

pub(super) fn month_detail_missing(availability: &ArmyLossesAvailability, wanted: Option<&[u8]>) -> Option<String> {
    if let Some(reason) = &availability.armies {
        return Some(reason.clone());
    }
    match wanted {
        None => availability.manpower.clone(),
        Some(ledgers) => {
            if let Some(reason) = &availability.months {
                return Some(reason.clone());
            }
            let needs_own = ledgers.iter().any(|&l| l != ENEMY_LEDGER);
            let needs_enemy = ledgers.contains(&ENEMY_LEDGER);
            if needs_own && let Some(reason) = &availability.own_equipment {
                return Some(reason.clone());
            }
            if needs_enemy && let Some(reason) = &availability.enemy_equipment {
                return Some(reason.clone());
            }
            None
        }
    }
}

struct MonthArmyAcc {
    army: LossArmy,
    manpower: Option<[i32; 3]>,
    manpower_live_row: Option<String>,
    ledgers: BTreeMap<u8, MonthLedgerAcc>,
}

pub(super) fn advance_keeps(
    frame: &DecodedFrame,
    params: &ArmyLossesParams,
    holders: &dashboard_query::identity::Holders,
    hour: i32,
    edits: &dashboard_base::products::at_hour::Edits,
    live: &dashboard_follow::live::Live,
    held_parts_count: Option<usize>,
) -> io::Result<bool> {
    if !edits.replaced.is_empty() {
        return Ok(false);
    }
    let Some(Holder::Found { slot, .. }) = holders.at(&params.country, hour).map(|run| run.holder) else {
        return Ok(false);
    };
    let Ok(links) = frame.complete::<tables::CountryLinks>() else {
        return Ok(false);
    };
    let ours = |raw: u32| link_slot(links, raw) == Some(slot);
    let mp_mask = dashboard_base::products::at_hour::field_mask(TableId::CombatLogManpower, &["losses_0", "losses_1", "losses_2"]);
    let eq_mask = dashboard_base::products::at_hour::field_mask(TableId::CombatLogEquipment, &["amount_raw"]);
    let enemy_mask = dashboard_base::products::at_hour::field_mask(TableId::CombatLogEnemyEquipment, &["amount_raw"]);
    let dropped_mask = dashboard_base::products::at_hour::field_mask(TableId::CombatLogDropped, &["amount"]);
    let has_live = |table: TableId, mask: u64, key: u128| -> bool {
        let mut bits = mask;
        while bits != 0 {
            let col = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            if !live.groups.iter().any(|g| g.table == table && g.columns.contains(&col) && g.keys.binary_search(&key).is_ok()) {
                return false;
            }
        }
        true
    };
    let mut our_edited = false;
    for (&table, keys) in &edits.tables {
        let allowed = match table {
            TableId::CombatLogManpower => mp_mask,
            TableId::CombatLogEquipment => eq_mask,
            TableId::CombatLogEnemyEquipment => enemy_mask,
            TableId::CombatLogDropped => dropped_mask,
            _ => return Ok(false),
        };
        if keys.values().any(|&mask| mask & !allowed != 0) {
            return Ok(false);
        }
        match table {
            TableId::CombatLogManpower => {
                let Ok(rows) = frame.complete::<tables::CombatLogManpower>() else { return Ok(false) };
                for (&key, &mask) in keys {
                    let Ok(idx) = rows.binary_search_by_key(&key, |r| r.key()) else { return Ok(false) };
                    if !ours(rows[idx].country) {
                        continue;
                    }
                    if !has_live(table, mask, key) {
                        return Ok(false);
                    }
                    our_edited = true;
                }
            }
            TableId::CombatLogEquipment => {
                let Ok(rows) = frame.complete::<tables::CombatLogEquipment>() else { return Ok(false) };
                for (&key, &mask) in keys {
                    let Ok(idx) = rows.binary_search_by_key(&key, |r| r.key()) else { return Ok(false) };
                    if !ours(rows[idx].country) {
                        continue;
                    }
                    if !has_live(table, mask, key) {
                        return Ok(false);
                    }
                    our_edited = true;
                }
            }
            TableId::CombatLogEnemyEquipment => {
                let Ok(rows) = frame.complete::<tables::CombatLogEnemyEquipment>() else { return Ok(false) };
                for (&key, &mask) in keys {
                    let Ok(idx) = rows.binary_search_by_key(&key, |r| r.key()) else { return Ok(false) };
                    if !ours(rows[idx].country) {
                        continue;
                    }
                    if !has_live(table, mask, key) {
                        return Ok(false);
                    }
                    our_edited = true;
                }
            }
            TableId::CombatLogDropped => {
                let Ok(rows) = frame.complete::<tables::CombatLogDropped>() else { return Ok(false) };
                for (&key, &mask) in keys {
                    let Ok(idx) = rows.binary_search_by_key(&key, |r| r.key()) else { return Ok(false) };
                    if !ours(rows[idx].country) {
                        continue;
                    }
                    if !matches!(rows[idx].amount, Recorded::Value(_)) || !has_live(table, mask, key) {
                        return Ok(false);
                    }
                    our_edited = true;
                }
            }
            _ => return Ok(false),
        }
    }
    if !our_edited {
        return Ok(true);
    }
    match &params.view {
        LossView::Month { .. } => Ok(true),
        LossView::Manpower if matches!(params.stack.unwrap_or(LossStack::Channel), LossStack::Channel | LossStack::Type) => Ok(true),
        _ => Ok(held_parts_count.is_some_and(|n| n <= 1)),
    }
}

impl AtHour for ArmyLosses {
    const KEY: &'static str = "army-losses";
    const TITLE: &'static str = "Army losses";
    const CONTENT_DEPENDENT: bool = true;
    const CLOCK: dashboard_base::products::at_hour::Clock = dashboard_base::products::at_hour::Clock::InputsOnly;
    type Params = ArmyLossesParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, view, "view.ledgers[]", "view.month", stack, limit];
    type Reply = ArmyLossesReply;
    type Context = Content;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        dashboard_base::products::CountryParams { branch_id: params.branch_id.clone(), country: params.country.clone() }.validate()?;
        if params.limit == Some(0) {
            return Err("Invalid army losses stack limit");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            LossStack::decl(),
            ArmyLossesParams::decl(),
            ArmyLossesReply::decl(),
            LossArmy::decl(),
            LossView::decl(),
            ArmyLossPart::decl(),
            ArmyLossMonthBar::decl(),
            ArmyLossRollup::decl(),
            ArmyLossMonthDesign::decl(),
            ArmyLossMonthLedger::decl(),
            ArmyLossMonthArmy::decl(),
            ArmyLossMonthDetail::decl(),
            ArmyLossesAvailability::decl(),
        ]
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn reads_in(params: &Self::Params, _owner: Option<at_hour::Owner>, stage: usize, held: &DecodedFrame, at: &ReadsAt<'_, Self::Context>) -> Vec<Read> {
        let equipment = !matches!(params.view, LossView::Manpower);
        match stage {
            0 => {
                let mut reads = vec![Read::all(TableId::CountryLinks), Read::all(TableId::OrdersGroups)];
                if equipment { reads.extend([TableId::Countries, TableId::CountryAppearance, TableId::LexicalTokens].map(Read::all)); }
                reads
            }
            1 => {
                let slot = at.holders.at(&params.country, at.hour).and_then(|run| match run.holder { Holder::Found { slot, .. } => Some(slot), _ => None });
                let links = held.complete::<tables::CountryLinks>().unwrap_or(&[]);
                let foreign: Vec<_> = links.iter().filter(|link| Some(link.canonical_slot) != slot).map(|link| u128::from(link.id)).collect();
                let logs = [
                    (TableId::CombatLogGroups, 64, true),
                    (TableId::CombatLogManpower, 64, !matches!(params.view, LossView::Equipment { .. })),
                    (TableId::CombatLogMonths, 72, equipment),
                    (TableId::CombatLogEquipment, 88, equipment),
                    (TableId::CombatLogEnemyEquipment, 96, equipment),
                    (TableId::CombatLogDropped, 88, equipment),
                ];
                let mut reads: Vec<_> = logs.into_iter().map(|(table, bits, needed)| {
                    let mut ranges = Vec::new();
                    if needed && slot.is_some() {
                        let mut first = 0;
                        for &id in &foreign {
                            if first < id { ranges.push((first << bits, (id << bits) - 1)); }
                            first = id + 1;
                        }
                        if first <= u128::from(u32::MAX) { ranges.push((first << bits, (u128::from(u32::MAX) << bits) | ((1u128 << bits) - 1))); }
                    }
                    Read { table, keys: Keys::Ranges(ranges) }
                }).collect();
                let table = (TableId::CombatLogEquipment.index() as u128) << 32;
                reads.push(Read::these(TableId::SliceReads, links.iter().filter(|link| equipment && Some(link.canonical_slot) == slot)
                    .map(|link| table | u128::from(link.id))));
                reads
            }
            2 if equipment => {
                let variant = |id| (u128::from(id) << 32) | u128::from(COMBAT_LOG_VARIANT_TYPE);
                let own = held.complete::<tables::CombatLogEquipment>().unwrap_or(&[]).iter().map(|row| variant(row.variant));
                let kept = held.complete::<tables::CombatLogDropped>().unwrap_or(&[]).iter().map(|row| variant(row.variant));
                let enemy = held.complete::<tables::CombatLogEnemyEquipment>().unwrap_or(&[]).iter().map(|row| u128::from(row.variant));
                vec![Read::these(TableId::VariantDefinitions, own.chain(kept).chain(enemy))]
            }
            _ => Vec::new(),
        }
    }

    fn plan_changes(stage: usize, edits: &at_hour::Edits, _held: &DecodedFrame, _at: &ReadsAt<'_, Self::Context>) -> bool {
        match stage {
            1 => edits.fields(TableId::CountryLinks, u64::MAX),
            2 => [TableId::CombatLogEquipment, TableId::CombatLogDropped].iter().any(|table| edits.replaced.contains(table) || edits.tables.get(table).is_some_and(|rows| rows.values().any(|&fields| fields == u64::MAX)))
                || edits.fields(TableId::CombatLogEnemyEquipment, at_hour::field_mask(TableId::CombatLogEnemyEquipment, &["variant"])),
            _ => false,
        }
    }

    fn columns(table: TableId) -> Option<Vec<usize>> {
        if table == TableId::OrdersGroups {
            Some(at_hour::field_columns(table, &["is_army_group", "parent", "name"]))
        } else {
            super::divisions::naming_columns(table)
        }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let catalogue = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|why| format!("Combat Log names unavailable: {why}"));
        let units = catalogue.as_ref().ok().and_then(|catalogue| catalogue::equipment_units(catalogue).ok());
        Ok(Content { catalogue, units })
    }

    fn advance(
        frame: &DecodedFrame,
        params: &Self::Params,
        at: &At<'_, Self::Context>,
        edits: &dashboard_base::products::at_hour::Edits,
        live: &dashboard_follow::live::Live,
        data: &mut dashboard_base::products::at_hour::Held<'_>,
    ) -> io::Result<bool> {
        let parts_count = data
            .get("rollup")
            .and_then(|rollup| rollup.get("parts"))
            .and_then(serde_json::Value::as_array)
            .map(Vec::len);
        advance_keeps(frame, params, at.holders, at.hour, edits, live, parts_count)
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let content = at.context;
        let mut reply = ArmyLossesReply {
            tag: at.holders.name(&params.country, at.hour),
            absent: false,
            rollup: None,
            month_detail: None,
            availability: ArmyLossesAvailability {
                armies: unavailable(frame, TableId::CombatLogGroups).or_else(|| unavailable(frame, TableId::OrdersGroups)),
                manpower: unavailable(frame, TableId::CombatLogManpower),
                months: unavailable(frame, TableId::CombatLogMonths),
                own_equipment: unavailable(frame, TableId::CombatLogEquipment).or_else(|| unavailable(frame, TableId::SliceReads)),
                enemy_equipment: unavailable(frame, TableId::CombatLogEnemyEquipment),
            },
            warnings: content.catalogue.as_ref().err().cloned().into_iter().collect(),
        };
        let slot = match at.holders.at(&params.country, at.hour).map(|run| run.holder) {
            Some(Holder::Found { slot, .. }) => slot,
            Some(Holder::Unresolved) => return Err(dashboard_core::refused(at.holders.not_known(&params.country, at.hour).unwrap_or_else(|| at.holders.say(&params.country, at.hour, |name| format!("{name} has no resolved country identity at this hour"))))),
            None => {
                reply.absent = true;
                return Ok(reply);
            }
        };
        let links = frame.complete::<tables::CountryLinks>()?;
        let mut refused = BTreeMap::new();
        let unbound = std::cell::Cell::new(0usize);
        let ours = |raw: u32| match link_slot(links, raw) {
            Some(found) => found == slot,
            None => {
                unbound.set(unbound.get() + 1);
                false
            }
        };
        let orders: BTreeMap<u64, &OrdersGroupRow> = frame.complete::<tables::OrdersGroups>().map(|rows| rows.iter().map(|row| (row.handle, row)).collect()).unwrap_or_default();
        let mut armies_by_key = BTreeMap::<(u32, u32), LossArmy>::new();
        if reply.availability.armies.is_none() {
            for row in frame.complete::<tables::CombatLogGroups>()? {
                if !ours(row.country) {
                    continue;
                }
                let Some(army) = orders.get(&row.group) else {
                    refuse(&mut refused, "Combat Log army no longer exists and is left out, as the game's page leaves it");
                    continue;
                };
                if army.is_army_group != 0 {
                    refuse(&mut refused, "Combat Log kept for an army group is left out; losses are written to armies");
                    continue;
                }
                let (name, name_missing) = match group_name(army, content) {
                    Ok(name) => (Some(name), None),
                    Err(why) => {
                        refuse(&mut refused, "Army name unavailable");
                        (None, Some(why))
                    }
                };
                let (parent, parent_name, parent_missing) = match army.parent {
                    0 => (None, None, None),
                    parent => match orders.get(&parent) {
                        Some(group) => (Some(parent.to_string()), group_name(group, content).ok(), None),
                        None => {
                            refuse(&mut refused, "Army group of an army was not read");
                            (None, None, Some("Its army group was not read at this hour".to_owned()))
                        }
                    },
                };
                armies_by_key.insert(
                    (row.country, (row.group >> 32) as u32),
                    LossArmy { reference: row.group.to_string(), ordinal: row.ordinal, name, name_missing, parent, parent_name, parent_missing },
                );
            }
        }
        let known_armies = reply.availability.armies.is_none();
        let channels = ["losses_0", "losses_1", "losses_2"].map(|name| dashboard_declare::field(TableId::CombatLogManpower, name));
        let amount = |table: TableId, name: &str| dashboard_declare::field(table, name);
        let limit = params.limit.unwrap_or(8).clamp(1, 32) as usize;
        let mut designs = Designs::new(frame, content);

        match &params.view {
            LossView::Manpower => {
                if reply.availability.armies.is_none() && reply.availability.manpower.is_none() {
                    let stack = match params.stack.unwrap_or(LossStack::Channel) {
                        LossStack::ArmyGroup => LossStack::ArmyGroup,
                        LossStack::Channel | LossStack::Type => LossStack::Channel,
                    };
                    let fixed = (stack == LossStack::Channel).then(|| {
                        vec![("ground".into(), "Ground".into()), ("air".into(), "Air".into()), ("navy".into(), "Navy".into())]
                    });
                    let mut rollup = RollupBuilder::new(1, true, fixed);
                    for row in frame.complete::<tables::CombatLogManpower>().unwrap_or(&[]) {
                        if !ours(row.country) {
                            continue;
                        }
                        let Some(army) = armies_by_key.get(&(row.country, row.group)) else {
                            if known_armies {
                                refuse(&mut refused, "Combat Log month rows of a log that was left out");
                            }
                            continue;
                        };
                        let month = month_bucket(row.date);
                        let vals = [i128::from(row.losses_0), i128::from(row.losses_1), i128::from(row.losses_2)];
                        if stack == LossStack::Channel {
                            for (((key, label), val), col) in [("ground", "Ground"), ("air", "Air"), ("navy", "Navy")].into_iter().zip(vals).zip(&channels) {
                                let cell = col.as_ref().ok().map(|&column| Cell { table: TableId::CombatLogManpower, column, key: row.key() });
                                rollup.add(month, key.into(), label.into(), val, cell);
                            }
                        } else {
                            let (key, label) = army_group_key_label(army);
                            for (val, col) in vals.into_iter().zip(&channels) {
                                let cell = col.as_ref().ok().map(|&column| Cell { table: TableId::CombatLogManpower, column, key: row.key() });
                                rollup.add(month, key.clone(), label.clone(), val, cell);
                            }
                        }
                    }
                    reply.rollup = Some(rollup.finish(limit, at.live));
                }
            }
            LossView::Equipment { ledgers } => {
                let needs_own = ledgers.iter().any(|&l| l != ENEMY_LEDGER);
                let needs_enemy = ledgers.contains(&ENEMY_LEDGER);
                let available = reply.availability.armies.is_none()
                    && reply.availability.months.is_none()
                    && (!needs_own || reply.availability.own_equipment.is_none())
                    && (!needs_enemy || reply.availability.enemy_equipment.is_none());
                if available {
                    let stack = match params.stack.unwrap_or(LossStack::Type) {
                        LossStack::ArmyGroup => LossStack::ArmyGroup,
                        LossStack::Type | LossStack::Channel => LossStack::Type,
                    };
                    let mut rollup = RollupBuilder::new(EQUIPMENT_SCALE, false, None);
                    let months: Vec<&CombatLogMonthRow> = frame.complete::<tables::CombatLogMonths>().map(|rows| rows.iter().filter(|row| ours(row.country)).collect()).unwrap_or_default();
                    let own: Vec<&CombatLogMonthRow> = months.iter().copied().filter(|header| header.kind != ENEMY_LEDGER && armies_by_key.contains_key(&(header.country, header.group))).collect();
                    let listed = frame.complete::<tables::CombatLogEquipment>().ok();
                    let reads: BTreeMap<u32, i32> = frame
                        .complete::<tables::SliceReads>()
                        .map(|rows| rows.iter().filter(|row| row.table == TableId::CombatLogEquipment.index() as u32).map(|row| (row.owner, row.sample_hour)).collect())
                        .unwrap_or_default();
                    let mut kept = BTreeMap::<(u32, u32, u8), Vec<&CombatLogDroppedRow>>::new();
                    for row in dropped(frame, &ours) {
                        kept.entry((row.country, row.group, row.kind)).or_default().push(row);
                    }
                    let arrays = recent_from(&own);
                    let live_amount = amount(TableId::CombatLogEquipment, "amount_raw").ok();
                    let dropped_amount = amount(TableId::CombatLogDropped, "amount").ok();
                    for &header in &own {
                        let month = month_bucket(header.date);
                        if !ledgers.contains(&header.kind) {
                            continue;
                        }
                        let Some(rows) = listed else {
                            rollup.mark_unknown(month, NOT_READ.into());
                            continue;
                        };
                        let army = &armies_by_key[&(header.country, header.group)];
                        let array = (header.country, header.group, header.kind);
                        let kept_rows = kept.get(&array).map_or(&[][..], Vec::as_slice);
                        match witness(header, reads.get(&header.country).copied(), array_rows(rows, array), kept_rows, &arrays[&array]) {
                            Witness::Listed(rows) => {
                                for row in rows {
                                    let variant = u64::from(row.variant) << 32 | u64::from(COMBAT_LOG_VARIANT_TYPE);
                                    let (key, label) = if stack == LossStack::Type {
                                        let info = designs.design(variant);
                                        match info.archetype {
                                            Some(token) => (format!("archetype:{token}"), info.archetype_name.unwrap_or_else(|| "Equipment name unavailable".into())),
                                            None => (UNCLASSIFIED.into(), "Equipment type unavailable".into()),
                                        }
                                    } else {
                                        army_group_key_label(army)
                                    };
                                    let cell = live_amount.map(|column| Cell { table: TableId::CombatLogEquipment, column, key: row.key() });
                                    rollup.add(month, key, label, i128::from(row.amount_raw), cell);
                                }
                            }
                            Witness::Dropped(rows) => {
                                for row in rows {
                                    let Recorded::Value(val) = row.amount else {
                                        rollup.mark_unknown(month, DROPPED_UNRECORDED.into());
                                        continue;
                                    };
                                    let variant = u64::from(row.variant) << 32 | u64::from(COMBAT_LOG_VARIANT_TYPE);
                                    let (key, label) = if stack == LossStack::Type {
                                        let info = designs.design(variant);
                                        match info.archetype {
                                            Some(token) => (format!("archetype:{token}"), info.archetype_name.unwrap_or_else(|| "Equipment name unavailable".into())),
                                            None => (UNCLASSIFIED.into(), "Equipment type unavailable".into()),
                                        }
                                    } else {
                                        army_group_key_label(army)
                                    };
                                    let cell = dropped_amount.map(|column| Cell { table: TableId::CombatLogDropped, column, key: row.key() });
                                    rollup.add(month, key, label, i128::from(val), cell);
                                }
                            }
                            Witness::Empty => rollup.mark_recorded(month),
                            Witness::Unknown(why) => rollup.mark_unknown(month, why.into()),
                        }
                    }
                    if needs_enemy {
                        let enemy = frame.complete::<tables::CombatLogEnemyEquipment>().ok();
                        let enemy_amount = amount(TableId::CombatLogEnemyEquipment, "amount_raw").ok();
                        for &header in months.iter().filter(|header| header.kind == ENEMY_LEDGER && armies_by_key.contains_key(&(header.country, header.group))) {
                            let month = month_bucket(header.date);
                            let Some(rows) = enemy else {
                                rollup.mark_unknown(month, reply.availability.enemy_equipment.clone().unwrap_or_else(|| NOT_READ.into()));
                                continue;
                            };
                            let army = &armies_by_key[&(header.country, header.group)];
                            let start = rows.partition_point(|row| (row.country, row.group, row.month) < (header.country, header.group, header.ordinal));
                            let held: Vec<_> = rows[start..].iter().take_while(|row| (row.country, row.group, row.month) == (header.country, header.group, header.ordinal)).collect();
                            if held.is_empty() {
                                rollup.mark_recorded(month);
                            }
                            for row in held {
                                let (key, label) = if stack == LossStack::Type {
                                    let info = designs.design(row.variant);
                                    match info.archetype {
                                        Some(token) => (format!("archetype:{token}"), info.archetype_name.unwrap_or_else(|| "Equipment name unavailable".into())),
                                        None => (UNCLASSIFIED.into(), "Equipment type unavailable".into()),
                                    }
                                } else {
                                    army_group_key_label(army)
                                };
                                let cell = enemy_amount.map(|column| Cell { table: TableId::CombatLogEnemyEquipment, column, key: row.key() });
                                rollup.add(month, key, label, i128::from(row.amount_raw), cell);
                            }
                        }
                    }
                    reply.rollup = Some(rollup.finish(limit, at.live));
                }
            }
            LossView::Month { month: asked_month, ledgers: wanted_ledgers } => {
                let asked = *asked_month;
                let detail_missing = month_detail_missing(&reply.availability, wanted_ledgers.as_deref());
                let mut accs = BTreeMap::<String, MonthArmyAcc>::new();
                fn entry_of<'a>(accs: &'a mut BTreeMap<String, MonthArmyAcc>, army: &LossArmy) -> &'a mut MonthArmyAcc {
                    accs.entry(army.reference.clone()).or_insert_with(|| MonthArmyAcc {
                        army: army.clone(),
                        manpower: None,
                        manpower_live_row: None,
                        ledgers: BTreeMap::new(),
                    })
                }
                for row in frame.complete::<tables::CombatLogManpower>().unwrap_or(&[]) {
                    if !ours(row.country) || month_bucket(row.date) != asked {
                        continue;
                    }
                    let Some(army) = armies_by_key.get(&(row.country, row.group)) else {
                        if known_armies {
                            refuse(&mut refused, "Combat Log month rows of a log that was left out");
                        }
                        continue;
                    };
                    let key = format!("m:{}", row.key());
                    for (channel, column) in ["ground", "air", "navy"].into_iter().zip(&channels) {
                        if let Ok(column) = column {
                            at.live.value(&key, channel, Cell { table: TableId::CombatLogManpower, column: *column, key: row.key() }, 1.0);
                        }
                    }
                    let acc = entry_of(&mut accs, army);
                    acc.manpower = Some([row.losses_0, row.losses_1, row.losses_2]);
                    acc.manpower_live_row = Some(key);
                }
                let months: Vec<&CombatLogMonthRow> = frame.complete::<tables::CombatLogMonths>().map(|rows| rows.iter().filter(|row| ours(row.country)).collect()).unwrap_or_default();
                let own: Vec<&CombatLogMonthRow> = months.iter().copied().filter(|header| header.kind != ENEMY_LEDGER && armies_by_key.contains_key(&(header.country, header.group))).collect();
                let listed = frame.complete::<tables::CombatLogEquipment>().ok().filter(|_| reply.availability.months.is_none());
                let reads: BTreeMap<u32, i32> = frame
                    .complete::<tables::SliceReads>()
                    .map(|rows| rows.iter().filter(|row| row.table == TableId::CombatLogEquipment.index() as u32).map(|row| (row.owner, row.sample_hour)).collect())
                    .unwrap_or_default();
                let mut kept = BTreeMap::<(u32, u32, u8), Vec<&CombatLogDroppedRow>>::new();
                for row in dropped(frame, &ours) {
                    kept.entry((row.country, row.group, row.kind)).or_default().push(row);
                }
                let arrays = recent_from(&own);
                let live_amount = amount(TableId::CombatLogEquipment, "amount_raw").ok();
                let dropped_amount = amount(TableId::CombatLogDropped, "amount").ok();
                for &header in &own {
                    if month_bucket(header.date) != asked {
                        continue;
                    }
                    let army = &armies_by_key[&(header.country, header.group)];
                    let ledger_acc = entry_of(&mut accs, army).ledgers.entry(header.kind).or_default();
                    let Some(rows) = listed else {
                        ledger_acc.missing.get_or_insert_with(|| NOT_READ.into());
                        continue;
                    };
                    let array = (header.country, header.group, header.kind);
                    let kept_rows = kept.get(&array).map_or(&[][..], Vec::as_slice);
                    match witness(header, reads.get(&header.country).copied(), array_rows(rows, array), kept_rows, &arrays[&array]) {
                        Witness::Listed(rows) => {
                            ledger_acc.kept = true;
                            for row in rows {
                                let key = format!("e:{}", row.key());
                                if let Some(column) = live_amount {
                                    let cell = Cell { table: TableId::CombatLogEquipment, column, key: row.key() };
                                    at.live.value(&key, "amount", cell, 1.0);
                                    ledger_acc.cells.push(cell);
                                }
                                let variant = u64::from(row.variant) << 32 | u64::from(COMBAT_LOG_VARIANT_TYPE);
                                let info = designs.design(variant);
                                ledger_acc.total += i128::from(row.amount_raw);
                                ledger_acc.designs.push(ArmyLossMonthDesign {
                                    name: info.name,
                                    archetype: info.archetype,
                                    archetype_name: info.archetype_name,
                                    amount_raw: Some(row.amount_raw.to_string()),
                                    live_row: Some(key),
                                });
                            }
                        }
                        Witness::Dropped(rows) => {
                            ledger_acc.kept = true;
                            for row in rows {
                                let key = format!("d:{}", row.key());
                                let amount_raw = match row.amount {
                                    Recorded::Value(val) => {
                                        ledger_acc.total += i128::from(val);
                                        if let Some(column) = dropped_amount {
                                            let cell = Cell { table: TableId::CombatLogDropped, column, key: row.key() };
                                            at.live.value(&key, "amount", cell, 1.0);
                                            ledger_acc.cells.push(cell);
                                        }
                                        Some(val.to_string())
                                    }
                                    Recorded::NotRecorded => {
                                        ledger_acc.missing.get_or_insert_with(|| DROPPED_UNRECORDED.into());
                                        None
                                    }
                                };
                                let variant = u64::from(row.variant) << 32 | u64::from(COMBAT_LOG_VARIANT_TYPE);
                                let info = designs.design(variant);
                                ledger_acc.designs.push(ArmyLossMonthDesign {
                                    name: info.name,
                                    archetype: info.archetype,
                                    archetype_name: info.archetype_name,
                                    amount_raw,
                                    live_row: Some(key),
                                });
                            }
                        }
                        Witness::Empty => {
                            ledger_acc.kept = true;
                        }
                        Witness::Unknown(why) => {
                            ledger_acc.missing.get_or_insert_with(|| why.into());
                        }
                    }
                }
                let enemy = frame.complete::<tables::CombatLogEnemyEquipment>().ok();
                let enemy_amount = amount(TableId::CombatLogEnemyEquipment, "amount_raw").ok();
                for &header in months.iter().filter(|header| header.kind == ENEMY_LEDGER && armies_by_key.contains_key(&(header.country, header.group))) {
                    if month_bucket(header.date) != asked {
                        continue;
                    }
                    let army = &armies_by_key[&(header.country, header.group)];
                    let ledger_acc = entry_of(&mut accs, army).ledgers.entry(header.kind).or_default();
                    let Some(rows) = enemy else {
                        ledger_acc.missing.get_or_insert_with(|| reply.availability.enemy_equipment.clone().unwrap_or_else(|| NOT_READ.into()));
                        continue;
                    };
                    ledger_acc.kept = true;
                    let start = rows.partition_point(|row| (row.country, row.group, row.month) < (header.country, header.group, header.ordinal));
                    let held: Vec<_> = rows[start..].iter().take_while(|row| (row.country, row.group, row.month) == (header.country, header.group, header.ordinal)).collect();
                    for row in held {
                        let key = format!("x:{}", row.key());
                        if let Some(column) = enemy_amount {
                            let cell = Cell { table: TableId::CombatLogEnemyEquipment, column, key: row.key() };
                            at.live.value(&key, "amount", cell, 1.0);
                            ledger_acc.cells.push(cell);
                        }
                        let info = designs.design(row.variant);
                        ledger_acc.total += i128::from(row.amount_raw);
                        ledger_acc.designs.push(ArmyLossMonthDesign {
                            name: info.name,
                            archetype: info.archetype,
                            archetype_name: info.archetype_name,
                            amount_raw: Some(row.amount_raw.to_string()),
                            live_row: Some(key),
                        });
                    }
                }
                let mut detail_armies: Vec<ArmyLossMonthArmy> = accs
                    .into_values()
                    .map(|acc| {
                        let (selected_total_raw, selected_live_row, selected_missing) = summarize_selected_ledgers(
                            &acc.army.reference,
                            wanted_ledgers.as_deref(),
                            detail_missing.as_deref(),
                            &acc.ledgers,
                            at.live,
                        );
                        let ledgers = acc
                            .ledgers
                            .into_iter()
                            .map(|(ledger, la)| {
                                let valid = la.kept && la.missing.is_none();
                                let live_row = (valid && !la.cells.is_empty()).then(|| {
                                    let row_name = format!("l:{}:{ledger}", acc.army.reference);
                                    at.live.sum(&row_name, "amount", &la.cells, 1.0);
                                    row_name
                                });
                                ArmyLossMonthLedger {
                                    ledger,
                                    kept: la.kept,
                                    missing: la.missing,
                                    total_raw: valid.then(|| la.total.to_string()),
                                    live_row,
                                    designs: la.designs,
                                }
                            })
                            .collect();
                        ArmyLossMonthArmy {
                            army: acc.army,
                            manpower: acc.manpower,
                            manpower_live_row: acc.manpower_live_row,
                            selected_total_raw,
                            selected_live_row,
                            selected_missing,
                            ledgers,
                        }
                    })
                    .collect();
                detail_armies.sort_by_key(|entry| entry.army.ordinal);
                let (first_hour, last_hour) = month_hours(asked);
                reply.month_detail = Some(ArmyLossMonthDetail {
                    month: asked,
                    first_hour,
                    last_hour,
                    missing: detail_missing,
                    armies: detail_armies,
                });
            }
        }
        if unbound.get() != 0 {
            refused.insert("Combat Log rows name a country with no recorded binding", unbound.get());
        }
        if !designs.unnamed.is_empty() {
            reply.warnings.push(format!("{} recorded equipment designs have no resolved name", designs.unnamed.len()));
        }
        reply.warnings.extend(refused.into_iter().map(|(reason, count)| format!("{reason} ({count})")));
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
