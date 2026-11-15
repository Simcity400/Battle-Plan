//! One country's retained Combat Log months at an hour: the manpower each army was charged in each
//! calendar month by the log's three source channels, and the equipment each month's ledgers hold.
//!
//! The game keeps one log per ordinary army and in it one entry per calendar month a loss was
//! written in; a later write that month changes the entry, so the rows at an hour are each
//! retained month's latest version. Losses are written only to the army a division belongs to,
//! never again to its army group, so the logs are disjoint. Months the game pruned or never wrote
//! are absent: unknown, never zero. An older month whose rows left the country's read (a read
//! lists an array's last two entries) is read from `CombatLogDropped`, the equipment-loss
//! writer's copy of the rows the game dropped ([`dropped`], the one place it is read).
//!
//! Every amount is a live cell of its row: the reply carries armies, months and designs; the
//! numbers are painted from followed cells at the head.

use super::{
    LoadContext, ReplyWarnings,
    at_hour::{At, AtHour, Read},
};
use crate::{
    Source,
    catalogue::{self, Catalogue, View},
};
use dashboard_follow::live::Source as Cell;
use dashboard_query::identity::Holder;
use dashboard_text::{
    metadata::link_slot,
    scripted_text::stored_text,
    variant_names::{self, Tags, Texts},
};
use engine_land_inputs::ncombat_log_cequipment_loss::month_bucket;
use replay_data::rows::{COMBAT_LOG_VARIANT_TYPE, CombatLogDroppedRow, CombatLogEquipmentRow, CombatLogMonthRow, OrdersGroupRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::Arc,
};
use ts_rs::TS;

/// The Combat Log's equipment ledgers: losses in combat (0), the three attrition modes (1 to 3),
/// recovered (4), and what the army dealt to its enemies.
pub const ENEMY_LEDGER: u8 = 5;

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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArmyLossesParams {
    pub branch_id: String,
    pub tag: String,
    #[serde(default)]
    pub view: LossView,
}

/// What a reply holds of the retained months: the one measure the chart draws, or every part of
/// one month for its drill. The armies are in every reply.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum LossView {
    /// Every retained month's manpower.
    #[default]
    Manpower,
    /// Every retained month of these equipment ledgers.
    Equipment { ledgers: Vec<u8> },
    /// The game's month `month` (twelve per year plus the zero-based month): its manpower and
    /// every ledger.
    Month { month: i32 },
}

impl LossView {
    fn manpower(&self, month: i32) -> bool {
        match self {
            Self::Manpower => true,
            Self::Equipment { .. } => false,
            Self::Month { month: asked } => *asked == month,
        }
    }

    fn ledger(&self, ledger: u8, month: i32) -> bool {
        match self {
            Self::Manpower => false,
            Self::Equipment { ledgers } => ledgers.contains(&ledger),
            Self::Month { month: asked } => *asked == month,
        }
    }
}

/// An army whose Combat Log the country keeps.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossArmy {
    /// The army's full reference, which identifies it in the reply.
    pub reference: String,
    pub ordinal: u32,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    pub parent: Option<String>,
    pub parent_name: Option<String>,
    pub parent_missing: Option<String>,
}

/// One retained month of one army's manpower; its ground, air and navy channels are the live
/// cells `(key, "ground" | "air" | "navy")`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossManpower {
    pub army: String,
    pub ordinal: u32,
    /// The game's month: twelve per year plus the zero-based month.
    pub month: i32,
    pub date: i32,
    pub key: String,
}

/// One design in one retained ledger month; its signed fixed5 amount is the live cell
/// `(key, "amount")`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossEquipment {
    pub army: String,
    pub ledger: u8,
    pub month: i32,
    pub date: i32,
    /// Its design's place in the reply's `designs`.
    pub design: u32,
    pub key: String,
}

/// A design the reply's equipment names, once however many rows name it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossDesign {
    pub variant: String,
    pub name: Option<String>,
    pub archetype: Option<String>,
    pub archetype_name: Option<String>,
}

/// One retained ledger month: in `empty_months` one its latest read covered without amounts; in
/// `unknown_months` one whose amounts are unknown, with why.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossMonth {
    pub army: String,
    pub ledger: u8,
    pub month: i32,
    pub date: i32,
    pub reason: Option<String>,
}

/// Why each part could not be read at this hour; none when it was.
#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossesAvailability {
    pub armies: Option<String>,
    pub manpower: Option<String>,
    pub months: Option<String>,
    pub own_equipment: Option<String>,
    pub enemy_equipment: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossesReply {
    pub tag: String,
    /// No country holds the tag at this hour.
    pub absent: bool,
    pub armies: Vec<LossArmy>,
    pub manpower: Vec<LossManpower>,
    pub equipment: Vec<LossEquipment>,
    pub designs: Vec<ArmyLossDesign>,
    pub empty_months: Vec<LossMonth>,
    pub unknown_months: Vec<LossMonth>,
    pub availability: ArmyLossesAvailability,
    pub warnings: Vec<String>,
}

impl ReplyWarnings for ArmyLossesReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

/// What naming reads of the content.
pub struct Content {
    catalogue: Result<Arc<Catalogue>, String>,
    units: Option<Arc<installed_content_core::equipment::Database>>,
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

/// The rows of the game's dropped Combat Log lines the hour holds for countries `ours` keeps: the
/// one read of `CombatLogDropped`.
pub fn dropped<'a>(frame: &'a DecodedFrame, ours: &dyn Fn(u32) -> bool) -> Vec<&'a CombatLogDroppedRow> {
    frame.complete::<tables::CombatLogDropped>().map(|rows| rows.iter().filter(|row| ours(row.country)).collect()).unwrap_or_default()
}

/// Each array's first-write hours of the entries any read lists (its last two), by
/// (country, group, kind).
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

/// What is known of one retained own-ledger month at the hour.
#[derive(Debug, PartialEq, Eq)]
pub enum Witness<'a> {
    /// Rows the latest read listed.
    Listed(Vec<&'a CombatLogEquipmentRow>),
    /// Rows the game dropped from the listing, as the recording kept them.
    Dropped(Vec<&'a CombatLogDroppedRow>),
    Empty,
    Unknown(&'static str),
}

/// One month's witness: its entry's rows (matched by first write) listed at the hour, else kept
/// from the listing, else none where its country's latest read lists the entry (an array's last
/// two), else unknown.
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
        .filter(|row| (row.country, row.group, row.kind) == (month.country, month.group, month.kind) && matches!(row.date, replay_data::rows::Recorded::Value(date) if date == month.date))
        .collect();
    if !kept.is_empty() {
        return Witness::Dropped(kept);
    }
    let newer = dates[dates.partition_point(|&date| date <= month.date)..].iter().take_while(|&&date| date <= read).count();
    if newer <= 1 { Witness::Empty } else { Witness::Unknown(UNWITNESSED) }
}

/// Design names and archetypes from the hour's definitions: the country tags built once a reply
/// and each variant named and listed once, however many rows name it.
struct Designs<'f> {
    frame: &'f DecodedFrame,
    content: &'f Content,
    tags: Option<Tags<'f>>,
    listed: Vec<ArmyLossDesign>,
    places: HashMap<u64, u32>,
    unnamed: BTreeSet<u64>,
}

impl<'f> Designs<'f> {
    fn new(frame: &'f DecodedFrame, content: &'f Content) -> Self {
        let tags = match (frame.complete::<tables::Countries>(), frame.complete::<tables::CountryLinks>()) {
            (Ok(countries), Ok(links)) => Some(Tags::new(countries, links, frame.complete::<tables::CountryAppearance>().ok())),
            _ => None,
        };
        Self { frame, content, tags, listed: Vec::new(), places: HashMap::new(), unnamed: BTreeSet::new() }
    }

    /// `variant`'s place in the list, named the first time it is asked.
    fn design(&mut self, variant: u64) -> u32 {
        if let Some(&place) = self.places.get(&variant) {
            return place;
        }
        let (name, archetype, archetype_name) = self.read(variant);
        if name.is_none() {
            self.unnamed.insert(variant);
        }
        let place = self.listed.len() as u32;
        self.listed.push(ArmyLossDesign { variant: variant.to_string(), name, archetype, archetype_name });
        self.places.insert(variant, place);
        place
    }

    /// A design's name, archetype and archetype name.
    fn read(&self, variant: u64) -> (Option<String>, Option<String>, Option<String>) {
        let (frame, content) = (self.frame, self.content);
        let (Some(tags), Ok(lexical), Ok(definitions)) = (&self.tags, frame.complete::<tables::LexicalTokens>(), frame.complete::<tables::VariantDefinitions>()) else {
            return (None, None, None);
        };
        let Ok(at) = definitions.binary_search_by_key(&variant, |row| row.id) else {
            return (None, None, None);
        };
        let raw = variant_names::raw_variant(&definitions[at], frame.hour, tags, lexical);
        let texts: Option<&dyn Texts> = content.catalogue.as_ref().ok().map(|catalogue| catalogue.as_ref() as &dyn Texts);
        let (name, _) = variant_names::variant_names(&raw, texts);
        let archetype = content.units.as_ref().and_then(|units| {
            let ordinal = units.named(&raw.token)?.archetype?;
            units.equipment.get(ordinal)?.name_str().ok().map(str::to_owned)
        });
        let archetype_name = archetype.as_deref().and_then(|key| content.catalogue.as_ref().ok()?.localised(key, View::Plain)).and_then(|text| catalogue::plain_text(&text).ok());
        (name, archetype, archetype_name)
    }
}

/// The run of `rows` (in key order) of one Combat Log array: country, group and ledger.
fn array_rows<'a>(rows: &'a [CombatLogEquipmentRow], array: (u32, u32, u8)) -> &'a [CombatLogEquipmentRow] {
    let start = rows.partition_point(|row| (row.country, row.group, row.kind) < array);
    let len = rows[start..].partition_point(|row| (row.country, row.group, row.kind) == array);
    &rows[start..start + len]
}

impl AtHour for ArmyLosses {
    const KEY: &'static str = "army-losses";
    const TITLE: &'static str = "Army losses";
    const CONTENT_DEPENDENT: bool = true;
    type Params = ArmyLossesParams;
    type Reply = ArmyLossesReply;
    type Context = Content;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::CountryParams { branch_id: params.branch_id.clone(), tag: params.tag.clone() }.validate()
    }

    fn types() -> Vec<String> {
        vec![
            ArmyLossesParams::decl(),
            ArmyLossesReply::decl(),
            LossArmy::decl(),
            LossManpower::decl(),
            LossView::decl(),
            LossEquipment::decl(),
            ArmyLossDesign::decl(),
            LossMonth::decl(),
            ArmyLossesAvailability::decl(),
        ]
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        TABLES.to_vec()
    }

    fn reads(_params: &Self::Params, _owner: Option<super::at_hour::Owner>, step: usize, _held: &DecodedFrame) -> Vec<Read> {
        if step == 0 { TABLES.iter().map(|&table| Read::all(table)).collect() } else { Vec::new() }
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        let catalogue = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|why| format!("Combat Log names unavailable: {why}"));
        let units = catalogue.as_ref().ok().and_then(|catalogue| catalogue::equipment_units(catalogue).ok());
        Ok(Content { catalogue, units })
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let content = at.context;
        let mut reply = ArmyLossesReply {
            tag: params.tag.clone(),
            absent: false,
            armies: Vec::new(),
            manpower: Vec::new(),
            equipment: Vec::new(),
            designs: Vec::new(),
            empty_months: Vec::new(),
            unknown_months: Vec::new(),
            availability: ArmyLossesAvailability {
                armies: unavailable(frame, TableId::CombatLogGroups).or_else(|| unavailable(frame, TableId::OrdersGroups)),
                manpower: unavailable(frame, TableId::CombatLogManpower),
                months: unavailable(frame, TableId::CombatLogMonths),
                own_equipment: unavailable(frame, TableId::CombatLogEquipment).or_else(|| unavailable(frame, TableId::SliceReads)),
                enemy_equipment: unavailable(frame, TableId::CombatLogEnemyEquipment),
            },
            warnings: content.catalogue.as_ref().err().cloned().into_iter().collect(),
        };
        let slot = match at.holders.at(&params.tag, at.hour).map(|run| run.holder) {
            Some(Holder::Found { slot, .. }) => slot,
            Some(Holder::Unresolved) => return Err(dashboard_core::refused(format!("{} has no resolved country identity at this hour", params.tag))),
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
        let mut armies = BTreeMap::<(u32, u32), String>::new();
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
                armies.insert((row.country, (row.group >> 32) as u32), row.group.to_string());
                reply.armies.push(LossArmy { reference: row.group.to_string(), ordinal: row.ordinal, name, name_missing, parent, parent_name, parent_missing });
            }
        }
        let known_armies = reply.availability.armies.is_none();
        let army_of = |country: u32, group: u32, refused: &mut BTreeMap<&'static str, usize>| {
            let army = armies.get(&(country, group)).cloned();
            if army.is_none() && known_armies {
                refuse(refused, "Combat Log month rows of a log that was left out");
            }
            army
        };
        let channels = ["losses_0", "losses_1", "losses_2"].map(|name| dashboard_declare::field(TableId::CombatLogManpower, name));
        let amount = |table: TableId, name: &str| dashboard_declare::field(table, name);
        for row in frame.complete::<tables::CombatLogManpower>().unwrap_or(&[]) {
            if !ours(row.country) || !params.view.manpower(month_bucket(row.date)) {
                continue;
            }
            if let Some(army) = army_of(row.country, row.group, &mut refused) {
                let key = format!("m:{}", row.key());
                for (channel, column) in ["ground", "air", "navy"].into_iter().zip(&channels) {
                    if let Ok(column) = column {
                        at.live.value(&key, channel, Cell { table: TableId::CombatLogManpower, column: *column, key: row.key() }, 1.0);
                    }
                }
                reply.manpower.push(LossManpower { army, ordinal: row.ordinal, month: month_bucket(row.date), date: row.date, key });
            }
        }
        let months: Vec<&CombatLogMonthRow> = frame.complete::<tables::CombatLogMonths>().map(|rows| rows.iter().filter(|row| ours(row.country)).collect()).unwrap_or_default();
        let mut designs = Designs::new(frame, content);
        let month_of = |header: &CombatLogMonthRow, reason: Option<&str>| LossMonth {
            army: armies.get(&(header.country, header.group)).cloned().unwrap_or_default(),
            ledger: header.kind,
            month: month_bucket(header.date),
            date: header.date,
            reason: reason.map(str::to_owned),
        };
        let design_row = |header: &CombatLogMonthRow, variant: u64, key: String, designs: &mut Designs<'_>| LossEquipment {
            army: armies.get(&(header.country, header.group)).cloned().unwrap_or_default(),
            ledger: header.kind,
            month: month_bucket(header.date),
            date: header.date,
            design: designs.design(variant),
            key,
        };
        let view = &params.view;
        let own: Vec<&CombatLogMonthRow> = months.iter().copied().filter(|header| header.kind != ENEMY_LEDGER && armies.contains_key(&(header.country, header.group))).collect();
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
        let live_amount = amount(TableId::CombatLogEquipment, "amount_raw");
        let dropped_amount = amount(TableId::CombatLogDropped, "amount");
        for &header in &own {
            if !view.ledger(header.kind, month_bucket(header.date)) {
                continue;
            }
            let Some(rows) = listed else {
                reply.unknown_months.push(month_of(header, Some(NOT_READ)));
                continue;
            };
            let array = (header.country, header.group, header.kind);
            let kept_rows = kept.get(&array).map_or(&[][..], Vec::as_slice);
            match witness(header, reads.get(&header.country).copied(), array_rows(rows, array), kept_rows, &arrays[&array]) {
                Witness::Listed(rows) => {
                    for row in rows {
                        let key = format!("e:{}", row.key());
                        if let Ok(column) = live_amount {
                            at.live.value(&key, "amount", Cell { table: TableId::CombatLogEquipment, column, key: row.key() }, 1.0);
                        }
                        let variant = u64::from(row.variant) << 32 | u64::from(COMBAT_LOG_VARIANT_TYPE);
                        reply.equipment.push(design_row(header, variant, key, &mut designs));
                    }
                }
                Witness::Dropped(rows) => {
                    for row in rows {
                        let key = format!("d:{}", row.key());
                        if let Ok(column) = dropped_amount {
                            at.live.value(&key, "amount", Cell { table: TableId::CombatLogDropped, column, key: row.key() }, 1.0);
                        }
                        let variant = u64::from(row.variant) << 32 | u64::from(COMBAT_LOG_VARIANT_TYPE);
                        reply.equipment.push(design_row(header, variant, key, &mut designs));
                    }
                }
                Witness::Empty => reply.empty_months.push(month_of(header, None)),
                Witness::Unknown(why) => reply.unknown_months.push(month_of(header, Some(why))),
            }
        }
        let enemy = frame.complete::<tables::CombatLogEnemyEquipment>().ok();
        let enemy_amount = amount(TableId::CombatLogEnemyEquipment, "amount_raw");
        for &header in months.iter().filter(|header| header.kind == ENEMY_LEDGER && armies.contains_key(&(header.country, header.group))) {
            if !view.ledger(header.kind, month_bucket(header.date)) {
                continue;
            }
            let Some(rows) = enemy else {
                reply.unknown_months.push(month_of(header, reply.availability.enemy_equipment.as_deref()));
                continue;
            };
            let start = rows.partition_point(|row| (row.country, row.group, row.month) < (header.country, header.group, header.ordinal));
            let held: Vec<_> = rows[start..].iter().take_while(|row| (row.country, row.group, row.month) == (header.country, header.group, header.ordinal)).collect();
            if held.is_empty() {
                reply.empty_months.push(month_of(header, None));
            }
            for row in held {
                let key = format!("x:{}", row.key());
                if let Ok(column) = enemy_amount {
                    at.live.value(&key, "amount", Cell { table: TableId::CombatLogEnemyEquipment, column, key: row.key() }, 1.0);
                }
                reply.equipment.push(design_row(header, row.variant, key, &mut designs));
            }
        }
        if unbound.get() != 0 {
            refused.insert("Combat Log rows name a country with no recorded binding", unbound.get());
        }
        if !designs.unnamed.is_empty() {
            reply.warnings.push(format!("{} recorded equipment designs have no resolved name", designs.unnamed.len()));
        }
        reply.designs = designs.listed;
        reply.warnings.extend(refused.into_iter().map(|(reason, count)| format!("{reason} ({count})")));
        Ok(reply)
    }
}

#[cfg(test)]
mod tests;
