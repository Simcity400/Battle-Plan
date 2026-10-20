//! One country's retained Combat Log months at a reached hour: the manpower each
//! army was charged in each calendar month, by the log's three source channels,
//! and the equipment each month's ledgers hold.
//!
//! The game keeps one log per ordinary army and, in it, one entry per calendar
//! month a loss was written in; a later write in the same month changes that
//! entry. The rows at an hour are therefore each retained month's latest
//! version, never a sum of snapshots. Losses are written only to the army a
//! division belongs to, never again to its army group, so the armies' logs are
//! disjoint. The three manpower counters are the log's ground, air and navy
//! source channels, which add up; they are not killed, wounded or captured.
//! Months the game has pruned, or never wrote, are absent: absence is unknown,
//! not zero. Equipment amounts join the months the game keeps now, so a
//! pruned month's lingering rows never return; an older month that left its
//! country's latest read keeps its latest witnessed amounts ([`ledgers`]).
//!
//! A log joins its army by the army's full reference; its month rows carry the
//! reference's id. Logs whose army no longer exists (they wait for the game's
//! daily prune) and logs of army groups are refused and counted, as the game's
//! own Combat Log page leaves them out.
use super::reached;
use dashboard_base::{
    catalogue,
    prepared::query::progressive,
    products::{Product, ReplyWarnings, check_reply_budget, equipment},
    source::Source,
};
use dashboard_core::config::LoadContext;
use dashboard_text::{
    catalogue::{Catalogue, View},
    metadata::{TaggedCountry, check_cancel, link_slot, tagged_country},
    scripted_text::stored_text,
};
use engine_land_inputs::ncombat_log_cequipment_loss::month_bucket;
use replay_data::rows::{COMBAT_LOG_VARIANT_TYPE, CombatLogMonthRow, OrdersGroupRow, TableId, tables};
use replay_frame::frame::{DecodedFrame, TableView};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

pub struct ArmyLosses;

/// The Combat Log's equipment ledgers: its own losses in combat, its three
/// attrition modes (which partition attrition and add up), what was recovered,
/// and what the army dealt to its enemies.
pub const COMBAT_LEDGER: u8 = 0;
pub const ATTRITION_LEDGERS: [u8; 3] = [1, 2, 3];
pub const RECOVERED_LEDGER: u8 = 4;
pub const ENEMY_LEDGER: u8 = 5;

const TABLES: [TableId; 9] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CombatLogGroups,
    TableId::CombatLogManpower,
    TableId::CombatLogMonths,
    TableId::CombatLogEquipment,
    TableId::CombatLogEnemyEquipment,
    TableId::OrdersGroups,
    TableId::SliceReads,
];

/// The month an equipment row keys its entry by, from the entry's first write.
fn month_key(date: i32) -> u16 {
    month_bucket(date) as u16
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArmyLossesParams {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
}

/// An army whose Combat Log the country keeps.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossArmy {
    /// The army's full reference, which identifies it in the reply.
    pub reference: String,
    /// Position in the country's list of logs.
    pub ordinal: u32,
    pub name: Option<String>,
    pub name_missing: Option<String>,
    /// The army group holding it now, by full reference; none for an army
    /// with no army group.
    pub parent: Option<String>,
    pub parent_name: Option<String>,
    /// Why the army group is unknown, when the army names one that was not read.
    pub parent_missing: Option<String>,
}

/// One retained month of one army's manpower.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossManpower {
    pub army: String,
    pub ordinal: u32,
    /// The game's month: twelve per year plus the zero-based month.
    pub month: i32,
    /// The hour of the month's first write.
    pub date: i32,
    /// Ground, air and navy source channels, signed as stored.
    pub losses: [i32; 3],
}

/// One design's amount in one retained ledger month.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossEquipment {
    pub army: String,
    pub ledger: u8,
    pub month: i32,
    pub date: i32,
    pub variant: String,
    pub name: Option<String>,
    /// The design's equipment archetype and its in-game name.
    pub archetype: Option<String>,
    pub archetype_name: Option<String>,
    /// Signed fixed-point amount with five decimals, as stored.
    pub amount_raw: String,
}

/// One retained ledger month: in `empty_months`, one a read covered without
/// amounts; in `unknown_months`, one whose amounts were not witnessed.
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LossMonth {
    pub army: String,
    pub ledger: u8,
    pub month: i32,
    pub date: i32,
}

/// Why each part could not be read at this hour; none when it was.
#[derive(Clone, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossesAvailability {
    pub armies: Option<String>,
    pub manpower: Option<String>,
    pub months: Option<String>,
    pub own_equipment: Option<String>,
    pub enemy_equipment: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ArmyLossesReply {
    pub branch_id: String,
    pub hour: i32,
    pub tag: String,
    /// No country holds the tag at this hour.
    pub absent: bool,
    pub armies: Vec<LossArmy>,
    pub manpower: Vec<LossManpower>,
    pub equipment: Vec<LossEquipment>,
    /// Ledger months with no design amounts, kept apart from known amounts.
    pub empty_months: Vec<LossMonth>,
    /// Retained ledger months whose amounts are unknown, never zero.
    pub unknown_months: Vec<LossMonth>,
    pub availability: ArmyLossesAvailability,
    pub warnings: Vec<String>,
}
impl ReplyWarnings for ArmyLossesReply {
    fn warnings_mut(&mut self) -> &mut Vec<String> {
        &mut self.warnings
    }
}

fn unavailable(frame: &DecodedFrame, id: TableId) -> Option<String> {
    match &*frame.tables[id.index()] {
        TableView::Complete(_) => None,
        TableView::Unavailable { reason } => Some(reason.clone()),
        TableView::Truncated { .. } => Some("incomplete read".into()),
    }
}

impl Product for ArmyLosses {
    const KEY: &'static str = "army-losses";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = ArmyLossesParams;
    type Reply = ArmyLossesReply;
    fn types() -> Vec<String> {
        vec![
            ArmyLossesParams::decl(),
            ArmyLossesReply::decl(),
            LossArmy::decl(),
            LossManpower::decl(),
            LossEquipment::decl(),
            LossMonth::decl(),
            ArmyLossesAvailability::decl(),
        ]
    }
    fn validate(p: &Self::Params) -> Result<(), &'static str> {
        dashboard_base::products::validate_tag(&p.tag)?;
        p.branch_id
            .parse::<u64>()
            .map(|_| ())
            .map_err(|_| "Invalid branch")
    }
    fn load(
        source: &Source,
        p: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let branch = p
            .branch_id
            .parse::<u64>()
            .map_err(|_| io::Error::other("Invalid branch"))?;
        let reply = reached::cached(
            source,
            context,
            Self::KEY,
            branch,
            p.hour,
            &p.tag,
            cancelled,
            || {
                let frame = progressive::frame_at(source, branch, p.hour, &TABLES, cancelled)?
                    .ok_or_else(|| io::Error::other("This hour is not recorded"))?;
                let mut warnings = Vec::new();
                let cat = context.content.as_ref().and_then(|options| {
                    catalogue::catalogue(source, options)
                        .map_err(|why| warnings.push(format!("Combat Log names unavailable: {why}")))
                        .ok()
                });
                let naming = progressive::latest_read(
                    source,
                    branch,
                    frame.hour,
                    TableId::VariantDefinitions,
                    replay_data::cadence::STAGGER_PERIOD,
                    &equipment::VariantCatalogue::TABLES,
                    cancelled,
                )?;
                let mut history = ledgers::Recorded::new(source, branch, frame.hour, cancelled);
                let mut reply =
                    build(&frame, &p.tag, cat.as_ref(), naming.as_deref(), &mut history, cancelled)?;
                reply.branch_id.clone_from(&p.branch_id);
                reply.warnings.splice(0..0, warnings);
                Ok(reply)
            },
        )?;
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}

fn group_name(row: &OrdersGroupRow, cat: Option<&Arc<Catalogue>>) -> Result<String, String> {
    let bytes = row
        .name
        .require("OrdersGroups.name")?
        .as_ref()
        .ok_or("The army's name was not read")?;
    let cat = cat.ok_or("Army names need the recorded content stack")?;
    stored_text(&cat.localisation, &cat.recorded_language, bytes, "Army name")
        .and_then(|name| catalogue::plain_text(&name))
}

/// Count each refused row under its reason.
fn refuse(refused: &mut BTreeMap<&'static str, usize>, reason: &'static str) {
    *refused.entry(reason).or_default() += 1;
}

/// The reply from one reached frame, naming designs from the supplying read of
/// their definitions; `history` finds older months' rows in earlier reads.
pub fn build(
    frame: &DecodedFrame,
    tag: &str,
    cat: Option<&Arc<Catalogue>>,
    naming: Option<&DecodedFrame>,
    history: &mut dyn ledgers::History,
    cancelled: &AtomicBool,
) -> io::Result<ArmyLossesReply> {
    let mut reply = ArmyLossesReply {
        branch_id: frame.branch.to_string(),
        hour: frame.hour,
        tag: tag.to_owned(),
        absent: false,
        armies: Vec::new(),
        manpower: Vec::new(),
        equipment: Vec::new(),
        empty_months: Vec::new(),
        unknown_months: Vec::new(),
        availability: ArmyLossesAvailability {
            armies: unavailable(frame, TableId::CombatLogGroups)
                .or_else(|| unavailable(frame, TableId::OrdersGroups)),
            manpower: unavailable(frame, TableId::CombatLogManpower),
            months: unavailable(frame, TableId::CombatLogMonths),
            own_equipment: unavailable(frame, TableId::CombatLogEquipment)
                .or_else(|| unavailable(frame, TableId::SliceReads)),
            enemy_equipment: unavailable(frame, TableId::CombatLogEnemyEquipment),
        },
        warnings: Vec::new(),
    };
    let countries = frame.complete::<tables::Countries>()?;
    let links = frame.complete::<tables::CountryLinks>()?;
    let slot = match tagged_country(countries, links, tag) {
        TaggedCountry::Found(_, slot) => slot,
        TaggedCountry::Absent => {
            reply.absent = true;
            return Ok(reply);
        }
        TaggedCountry::Unresolved => {
            return Err(io::Error::other(format!(
                "{tag} has no resolved country identity at this hour"
            )));
        }
    };
    let mut refused = BTreeMap::new();
    let ours = |raw: u32, refused: &mut BTreeMap<&'static str, usize>| match link_slot(links, raw) {
        Some(found) => found == slot,
        None => {
            refuse(refused, "Combat Log rows name a country with no recorded binding");
            false
        }
    };
    let orders: BTreeMap<u64, &OrdersGroupRow> = frame
        .complete::<tables::OrdersGroups>()
        .map(|rows| rows.iter().map(|row| (row.handle, row)).collect())
        .unwrap_or_default();
    // The country's logs by the id their month rows carry, each joined to its army.
    let mut armies = BTreeMap::<(u32, u32), String>::new();
    if reply.availability.armies.is_none() {
        for row in frame.complete::<tables::CombatLogGroups>()? {
            if !ours(row.country, &mut refused) {
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
            let (name, name_missing) = match group_name(army, cat) {
                Ok(name) => (Some(name), None),
                Err(why) => {
                    refuse(&mut refused, "Army name unavailable");
                    (None, Some(why))
                }
            };
            let (parent, parent_name, parent_missing) = match army.parent {
                0 => (None, None, None),
                parent => match orders.get(&parent) {
                    Some(group) => (Some(parent.to_string()), group_name(group, cat).ok(), None),
                    None => {
                        refuse(&mut refused, "Army group of an army was not read");
                        (None, None, Some("Its army group was not read at this hour".to_owned()))
                    }
                },
            };
            armies.insert((row.country, (row.group >> 32) as u32), row.group.to_string());
            reply.armies.push(LossArmy {
                reference: row.group.to_string(),
                ordinal: row.ordinal,
                name,
                name_missing,
                parent,
                parent_name,
                parent_missing,
            });
        }
    }
    let army_of = |country: u32, group: u32, refused: &mut BTreeMap<&'static str, usize>| {
        let army = armies.get(&(country, group)).cloned();
        if army.is_none() && reply_armies_known(&reply.availability) {
            refuse(refused, "Combat Log month rows of a log that was left out");
        }
        army
    };
    for row in frame.complete::<tables::CombatLogManpower>().unwrap_or(&[]) {
        if !ours(row.country, &mut refused) {
            continue;
        }
        if let Some(army) = army_of(row.country, row.group, &mut refused) {
            reply.manpower.push(LossManpower {
                army,
                ordinal: row.ordinal,
                month: month_bucket(row.date),
                date: row.date,
                losses: [row.losses_0, row.losses_1, row.losses_2],
            });
        }
    }
    check_cancel(cancelled)?;
    let designs = Designs::new(naming, cat);
    let mut unnamed = BTreeSet::new();
    let months: Vec<_> = frame
        .complete::<tables::CombatLogMonths>()
        .unwrap_or(&[])
        .iter()
        .filter(|row| ours(row.country, &mut refused))
        .collect();
    let month = |header: &CombatLogMonthRow| LossMonth {
        army: armies[&(header.country, header.group)].clone(),
        ledger: header.kind,
        month: month_bucket(header.date),
        date: header.date,
    };
    let design_row = |unnamed: &mut BTreeSet<u64>, header: &CombatLogMonthRow, variant: u64, amount: i64| {
        let (name, archetype, archetype_name) = designs.design(variant, unnamed);
        LossEquipment {
            army: armies[&(header.country, header.group)].clone(),
            ledger: header.kind,
            month: month_bucket(header.date),
            date: header.date,
            variant: variant.to_string(),
            name,
            archetype,
            archetype_name,
            amount_raw: amount.to_string(),
        }
    };
    // Own ledgers: rows join the months the game keeps now; a pruned month's
    // rows that linger until its country's next read are left out.
    if reply.availability.months.is_none()
        && let Ok(rows) = frame.complete::<tables::CombatLogEquipment>()
    {
        let kept: BTreeSet<_> = months
            .iter()
            .map(|header| (header.country, header.group, header.kind, month_key(header.date)))
            .collect();
        for row in rows {
            if ours(row.country, &mut refused)
                && army_of(row.country, row.group, &mut refused).is_some()
                && !kept.contains(&(row.country, row.group, row.kind, row.month))
            {
                refuse(&mut refused, "Combat Log equipment rows of a month the game no longer keeps are left out");
            }
        }
    }
    let own: Vec<&CombatLogMonthRow> = months
        .iter()
        .copied()
        .filter(|header| header.kind != ENEMY_LEDGER && armies.contains_key(&(header.country, header.group)))
        .collect();
    for (&header, witness) in own.iter().zip(ledgers::witnesses(frame, &own, history, cancelled)?) {
        match witness {
            ledgers::Witness::Rows(rows) => {
                for row in rows {
                    let variant = (u64::from(row.variant) << 32) | u64::from(COMBAT_LOG_VARIANT_TYPE);
                    reply.equipment.push(design_row(&mut unnamed, header, variant, row.amount_raw));
                }
            }
            ledgers::Witness::Empty => reply.empty_months.push(month(header)),
            ledgers::Witness::Unknown(why) => {
                if why != ledgers::NOT_READ {
                    refuse(&mut refused, why);
                }
                reply.unknown_months.push(month(header));
            }
        }
    }
    // Dealt losses are read every hour with their months; an unread table
    // leaves each such month unknown.
    let enemy = frame.complete::<tables::CombatLogEnemyEquipment>().ok();
    if let Some(rows) = enemy {
        for row in rows {
            if ours(row.country, &mut refused)
                && army_of(row.country, row.group, &mut refused).is_some()
                && !months.iter().any(|m| {
                    m.country == row.country && m.group == row.group && m.kind == ENEMY_LEDGER && m.ordinal == row.month
                })
            {
                refuse(&mut refused, "Enemy equipment rows without their month");
            }
        }
    }
    for &header in months
        .iter()
        .filter(|header| header.kind == ENEMY_LEDGER && armies.contains_key(&(header.country, header.group)))
    {
        let Some(rows) = enemy else {
            reply.unknown_months.push(month(header));
            continue;
        };
        let start = rows.partition_point(|row| (row.country, row.group, row.month) < (header.country, header.group, header.ordinal));
        let held: Vec<_> = rows[start..]
            .iter()
            .take_while(|row| (row.country, row.group, row.month) == (header.country, header.group, header.ordinal))
            .collect();
        if held.is_empty() {
            reply.empty_months.push(month(header));
        }
        for row in held {
            reply.equipment.push(design_row(&mut unnamed, header, row.variant, row.amount_raw));
        }
    }
    if !unnamed.is_empty() {
        reply.warnings.push(format!(
            "{} recorded equipment designs have no resolved name",
            unnamed.len()
        ));
    }
    reply
        .warnings
        .extend(refused.into_iter().map(|(reason, count)| format!("{reason} ({count})")));
    Ok(reply)
}

fn reply_armies_known(availability: &ArmyLossesAvailability) -> bool {
    availability.armies.is_none()
}

/// Design names and archetypes from the supplying read of their definitions.
struct Designs<'a> {
    read: Option<(
        &'a DecodedFrame,
        equipment::Tags<'a>,
        &'a [replay_data::rows::VariantDefinitionRow],
        &'a [replay_data::rows::LexicalTokenRow],
    )>,
    cat: Option<&'a Arc<Catalogue>>,
    types: Option<Arc<installed_content_core::equipment::Database>>,
}
impl<'a> Designs<'a> {
    fn new(read: Option<&'a DecodedFrame>, cat: Option<&'a Arc<Catalogue>>) -> Self {
        Self {
            read: read.and_then(|read| {
                Some((
                    read,
                    equipment::Tags::new(
                        read.complete::<tables::Countries>().ok()?,
                        read.complete::<tables::CountryLinks>().ok()?,
                        read.complete::<tables::CountryAppearance>().ok(),
                    ),
                    read.complete::<tables::VariantDefinitions>().ok()?,
                    read.complete::<tables::LexicalTokens>().ok()?,
                ))
            }),
            cat,
            types: cat.and_then(|cat| equipment::units_of(cat).ok()),
        }
    }
    fn design(&self, variant: u64, unnamed: &mut BTreeSet<u64>) -> (Option<String>, Option<String>, Option<String>) {
        let raw = self.read.as_ref().and_then(|(read, tags, definitions, lexical)| {
            let at = definitions.binary_search_by_key(&variant, |r| r.id).ok()?;
            Some(equipment::raw_variant(&definitions[at], read.hour, tags, lexical))
        });
        let Some(raw) = raw else {
            unnamed.insert(variant);
            return (None, None, None);
        };
        let (name, _) =
            equipment::variant_names(&raw, self.cat.map(|c| c.as_ref() as &dyn equipment::Texts));
        if name.is_none() {
            unnamed.insert(variant);
        }
        let archetype = self
            .types
            .as_ref()
            .and_then(|types| equipment::archetype_of(types, &raw.token).ok())
            .map(|(key, _, _)| key);
        let archetype_name = archetype.as_ref().and_then(|key| {
            self.cat
                .and_then(|cat| cat.localised(key, View::Plain))
                .and_then(|text| catalogue::plain_text(&text).ok())
        });
        (name, archetype, archetype_name)
    }
}

pub mod ledgers;
#[cfg(test)]
mod tests;
