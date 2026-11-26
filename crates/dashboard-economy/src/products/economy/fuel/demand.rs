
use dashboard_base::products::{Country, validate_country};
use super::{Content, DETAIL_HOURS, FuelConsumptionAt, reason};
use dashboard_base::{Source, products::LoadContext};
use dashboard_base::products::Product;
use dashboard_query::{Query, identity::Holder};
use engine_fuel::fuel_demand::daily_required;
use replay_data::rows::{
    CountryFuelRow, FuelAwardRecipientRow, FuelDemandFleet, FuelDemandGroup, FuelDemandPartRow, FuelDemandRead, FuelDemandUnit, FuelDemandWing, Recorded,
    TableId, assemble_award_recipients, assemble_demand_parts, tables,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub(crate) fn holder_id(query: &Query<'_>, tag: &Country, hour: i32) -> io::Result<u32> {
    match query.holders.at(tag, hour).map(|run| run.holder) {
        Some(Holder::Found { id, .. }) => Ok(id),
        _ => Err(dashboard_core::refused(query.holders.not_known(tag, hour).unwrap_or_else(|| "Fuel country is unavailable at this hour".to_owned()))),
    }
}

pub(crate) fn window_span(window: i32) -> io::Result<(i32, i32)> {
    let first = window.checked_mul(DETAIL_HOURS).ok_or_else(|| dashboard_core::invalid("Invalid fuel window"))?;
    let last = first.checked_add(DETAIL_HOURS - 1).ok_or_else(|| dashboard_core::invalid("Invalid fuel window"))?;
    Ok((first, last))
}

#[derive(Clone, Debug)]
pub struct ReadRow {
    pub hour: i32,
    pub last: i32,
    pub starts_segment: bool,
    pub row: CountryFuelRow,
    pub parts: Result<Vec<FuelDemandPartRow>, &'static str>,
}

pub(crate) struct Reads {
    pub reads: Vec<ReadRow>,
    pub warnings: Vec<String>,
}

const PARTS_UNAVAILABLE: &str = "fuel demand parts are not recorded at this read";
const RECIPIENTS_UNAVAILABLE: &str = "fuel award recipients are not recorded at this read";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Detail {
    DemandParts,
    AwardRecipients,
}

fn typed<T: replay_data::rows::TableSchema>(view: &replay_columnar::store::TableView<'_>, key: u128, slot: u32, format: u16) -> io::Result<Option<T::Row>>
where
    T::Row: Clone,
{
    let rows = replay_frame::frame::cells_rows(T::ID, view, &[(key, slot)], format)?;
    Ok(rows.rows::<T>().and_then(|rows| rows.first().cloned()))
}

pub(crate) fn country_reads(query: &Query<'_>, id: u32, first: i32, last: i32, detail: Detail, cancelled: &AtomicBool) -> io::Result<Reads> {
    let recording = query.replay.recording();
    let key = u128::from(id);
    let mut out = Reads { reads: Vec::new(), warnings: Vec::new() };
    for stretch in query.timeline.within(first, last.saturating_add(super::DAY_HOURS)) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let (run, from, to) = (stretch.run, stretch.first, stretch.last);
        if from > last {
            continue;
        }
        let mut hours: Vec<i32> = Vec::new();
        let stored = recording.run(run)?;
        for (start, end) in replay_lanes::lanes::reads(recording, run, TableId::CountryFuel, from, to)? {
            hours.extend(start..=end);
        }
        if from <= to && stored.ordinal(from).is_some() && hours.first() != Some(&from) {
            hours.insert(0, from);
        }
        hours.retain(|&hour| hour <= to && stored.ordinal(hour).is_some());
        hours.sort_unstable();
        hours.dedup();
        if hours.is_empty() {
            continue;
        }
        let asks: Vec<(i32, u128)> = hours.iter().filter(|&&hour| hour <= last).map(|&hour| (hour, key)).collect();
        let mut rows: BTreeMap<i32, CountryFuelRow> = BTreeMap::new();
        replay_lanes::lanes::at_each(recording, run, TableId::CountryFuel, None, &asks, &mut |hour, key, row| {
            if let Some((view, slot)) = row
                && let Some(row) = typed::<tables::CountryFuel>(&view, key, slot, recording.format())?
            {
                rows.insert(hour, row);
            }
            Ok(())
        })?;
        if rows.is_empty() {
            continue;
        }
        let last_read = |start: i32| replay_lanes::lanes::reads(recording, run, TableId::CountryFuel, start, from).map(|reads| reads.last().map(|&(_, last)| last));
        let read = match hours[0] == from {
            true => match last_read(from.saturating_sub(super::DAY_HOURS).max(stored.first_hour))? {
                Some(read) => Some(read),
                None => last_read(stored.first_hour)?,
            },
            false => None,
        };
        let mut held: BTreeMap<i32, Vec<FuelDemandPartRow>> = BTreeMap::new();
        let mut unavailable = BTreeSet::new();
        if detail == Detail::DemandParts {
            let (low, high) = (key << 83, (key << 83) | ((1u128 << 83) - 1));
            let position = replay_data::rows::column::<FuelDemandPartRow>("position");
            let lives = replay_lanes::lanes::history(recording, run, TableId::FuelDemandParts, &[position], replay_lanes::lanes::Keys::Range(low, high), hours[0], to.min(last))?;
            let mut asks: Vec<(i32, u128)> = Vec::new();
            for &hour in rows.keys() {
                for (part, steps) in &lives.keys {
                    let at = steps.partition_point(|step| step.hour <= hour);
                    if at > 0 && steps[at - 1].cells.is_some() {
                        asks.push((hour, *part));
                    }
                }
                held.insert(hour, Vec::new());
            }
            dashboard_text::metadata::check_cancel(cancelled)?;
            replay_lanes::lanes::at_each(recording, run, TableId::FuelDemandParts, None, &asks, &mut |hour, key, row| {
                match row {
                    Some((view, slot)) => {
                        if let Some(part) = typed::<tables::FuelDemandParts>(&view, key, slot, recording.format())? {
                            held.entry(hour).or_default().push(part);
                        }
                    }
                    None => {
                        unavailable.insert(hour);
                    }
                }
                Ok(())
            })?;
        } else if detail == Detail::AwardRecipients && recording.predates(TableId::FuelAwardRecipients.index()).is_none() {
            let (low, high) = (key << 83, (key << 83) | ((1u128 << 83) - 1));
            let position = replay_data::rows::column::<FuelAwardRecipientRow>("position");
            let lives = replay_lanes::lanes::history(recording, run, TableId::FuelAwardRecipients, &[position], replay_lanes::lanes::Keys::Range(low, high), hours[0], to.min(last))?;
            let mut award_held: BTreeMap<i32, Vec<FuelAwardRecipientRow>> = BTreeMap::new();
            let mut award_unavailable = BTreeSet::new();
            let mut asks: Vec<(i32, u128)> = Vec::new();
            for &hour in rows.keys() {
                let complete = lives
                    .statuses
                    .iter()
                    .find(|run| run.first <= hour && hour <= run.last)
                    .is_some_and(|run| run.status == replay_columnar::manifest::Status::Complete);
                if !complete {
                    award_unavailable.insert(hour);
                    continue;
                }
                for (part, steps) in &lives.keys {
                    let at = steps.partition_point(|step| step.hour <= hour);
                    if at > 0 && steps[at - 1].cells.is_some() {
                        asks.push((hour, *part));
                    }
                }
                award_held.insert(hour, Vec::new());
            }
            dashboard_text::metadata::check_cancel(cancelled)?;
            replay_lanes::lanes::at_each(recording, run, TableId::FuelAwardRecipients, None, &asks, &mut |hour, key, row| {
                match row {
                    Some((view, slot)) => {
                        if let Some(part) = typed::<tables::FuelAwardRecipients>(&view, key, slot, recording.format())? {
                            award_held.entry(hour).or_default().push(part);
                        }
                    }
                    None => {
                        award_unavailable.insert(hour);
                    }
                }
                Ok(())
            })?;
            for (&written, row) in &mut rows {
                let read_hour = if written == from {
                    read.or_else(|| super::read_hour(row, None)).filter(|&hour| hour <= from).unwrap_or(written)
                } else {
                    written
                };
                let recipients = if award_unavailable.contains(&written) {
                    Err(RECIPIENTS_UNAVAILABLE)
                } else {
                    Ok(award_held.get(&written).map_or(&[][..], Vec::as_slice))
                };
                assemble_award_recipients(std::slice::from_mut(row), recipients, read_hour);
            }
        }
        let cut = query.timeline.at(from).is_some_and(|whole| whole.first < from);
        let walk = Walk { from, to, last, cut, read };
        stretch_reads(&walk, &hours, &mut rows, &mut held, &unavailable, &mut out.reads);
    }
    Ok(out)
}

pub(crate) struct Walk {
    pub from: i32,
    pub to: i32,
    pub last: i32,
    pub cut: bool,
    pub read: Option<i32>,
}

pub(crate) fn stretch_reads(
    walk: &Walk,
    hours: &[i32],
    rows: &mut BTreeMap<i32, CountryFuelRow>,
    held: &mut BTreeMap<i32, Vec<FuelDemandPartRow>>,
    unavailable: &BTreeSet<i32>,
    out: &mut Vec<ReadRow>,
) {
    let mut previous: Option<i32> = None;
    for (index, &written) in hours.iter().enumerate() {
        if written > walk.last {
            break;
        }
        let next = hours.get(index + 1).map_or(walk.to, |next| next - 1);
        let Some(row) = rows.remove(&written) else {
            previous = None;
            continue;
        };
        let parts = if unavailable.contains(&written) { Err(PARTS_UNAVAILABLE) } else { Ok(held.remove(&written).unwrap_or_default()) };
        let in_force = written == walk.from;
        let hour = if in_force { walk.read.or_else(|| super::read_hour(&row, None)).filter(|&read| read <= walk.from).unwrap_or(written) } else { written };
        let starts_segment = match previous {
            Some(before) => before + 1 != hour,
            None => !(in_force && walk.cut),
        };
        out.push(ReadRow { hour, last: next, starts_segment, row, parts });
        previous = Some(next);
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Span<'a> {
    Read { read: &'a ReadRow, first: i32, last: i32 },
    Gap { first: i32, last: i32 },
}

pub(crate) fn spans(reads: &[ReadRow], first: i32, last: i32, cut_last: bool) -> Vec<Span<'_>> {
    let mut out = Vec::with_capacity(reads.len() + 2);
    let mut next = Some(first);
    let mut shown: Option<i32> = None;
    for read in reads.iter().filter(|read| read.last >= first && read.hour <= last) {
        let Some(from) = next else { break };
        if read.hour > from {
            out.push(Span::Gap { first: from, last: read.hour - 1 });
        }
        let start = shown.map_or(read.hour, |end| read.hour.max(end + 1));
        let end = if cut_last { read.last.min(last) } else { read.last };
        if end < start {
            continue;
        }
        out.push(Span::Read { read, first: start, last: end });
        shown = Some(end);
        next = end.checked_add(1);
    }
    if let Some(from) = next
        && from <= last
    {
        out.push(Span::Gap { first: from, last });
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Wing {
    pub reference: u64,
    pub owner: Option<u32>,
    pub owner_reason: Vec<u8>,
    pub pool_ordinal: u32,
    pub wing_ordinal: u32,
    pub name: Option<Vec<u8>>,
    pub name_reason: Vec<u8>,
    pub rate: Result<i64, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub list: u8,
    pub ordinal: u32,
    pub base: u64,
    pub container_owner: Option<u32>,
    pub capacity_raw: Option<i64>,
    pub demilitarized: Option<u8>,
    pub reason: Vec<u8>,
    pub wings: Vec<Wing>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Pack {
    pub divisions: Vec<FuelDemandUnit>,
    pub task_forces: Vec<FuelDemandUnit>,
    pub sites: Vec<Site>,
    pub groups: Vec<FuelDemandGroup>,
    pub fleets: Vec<FuelDemandFleet>,
    pub group_reason: Vec<u8>,
}

fn wing_of(pack: &FuelDemandRead, wing: &FuelDemandWing) -> Wing {
    Wing {
        reference: wing.reference,
        owner: wing.owner,
        owner_reason: wing.owner_reason.clone(),
        pool_ordinal: wing.pool_ordinal,
        wing_ordinal: wing.wing_ordinal,
        name: wing.name.clone(),
        name_reason: wing.name_reason.clone(),
        rate: match reason(&wing.reason) {
            Some(why) => Err(why),
            None => pack.wing_rate(wing).map_err(str::to_owned),
        },
    }
}

impl Pack {
    pub fn of(pack: &FuelDemandRead) -> Self {
        let sites = pack
            .sites
            .iter()
            .map(|site| Site {
                list: site.list,
                ordinal: site.ordinal,
                base: site.base,
                container_owner: site.container_owner,
                capacity_raw: site.capacity_raw,
                demilitarized: site.demilitarized,
                reason: site.reason.clone(),
                wings: site.wings.iter().map(|wing| wing_of(pack, wing)).collect(),
            })
            .collect();
        Self {
            divisions: pack.divisions.clone(),
            task_forces: pack.task_forces.clone(),
            sites,
            groups: pack.groups.clone(),
            fleets: pack.fleets.clone(),
            group_reason: pack.group_reason.clone(),
        }
    }
}

pub fn pack_at(row: &CountryFuelRow, parts: Result<&[FuelDemandPartRow], &'static str>, hour: i32) -> Result<FuelDemandRead, String> {
    let mut rows = [row.clone()];
    assemble_demand_parts(&mut rows, parts);
    let [row] = rows;
    let Recorded::Value(pack) = row.demand else { return Err("Individual current fuel consumption was not recorded".into()) };
    if pack.hour.at(hour) != hour {
        return Err("The current fuel consumption was not read with this fuel read".into());
    }
    if let Some(why) = reason(&pack.reason) {
        return Err(why);
    }
    Ok(pack)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Key {
    Unit(u32, usize, u64, u64, u32, Option<[u32; 3]>),
    Wing(usize, usize, u8, u32, u64, u32, u32, u64, Option<u32>, Option<u32>),
}

pub struct Occurrence<'a> {
    pub key: Key,
    pub kind: u32,
    pub reference: u64,
    pub parent: u64,
    pub ordinal: u32,
    pub owners: Option<[u32; 3]>,
    pub wing_owner: Option<u32>,
    pub group: Option<u64>,
    pub topology_reason: &'a [u8],
    pub site: Option<FuelDemandLocation>,
    pub rate: Result<i64, String>,
    pub name: Option<&'a [u8]>,
    pub name_reason: &'a [u8],
}

fn unit(kind: u32, index: usize, row: &FuelDemandUnit) -> Occurrence<'_> {
    Occurrence {
        key: Key::Unit(kind, index, row.reference, row.parent, row.ordinal, row.owners),
        kind,
        reference: row.reference,
        parent: row.parent,
        ordinal: row.ordinal,
        owners: row.owners,
        wing_owner: None,
        group: row.group,
        topology_reason: &row.group_reason,
        site: None,
        rate: row.rate_raw.ok_or_else(|| reason(&row.reason).unwrap_or_else(|| "The current fuel rate was not recorded".into())),
        name: row.name.as_deref(),
        name_reason: &row.name_reason,
    }
}

pub fn wing_key(site_index: usize, index: usize, site: &Site, row: &Wing) -> Key {
    Key::Wing(site_index, index, site.list, site.ordinal, site.base, row.pool_ordinal, row.wing_ordinal, row.reference, row.owner, site.container_owner)
}

fn wing(pack: &Pack, site_index: usize, index: usize) -> Option<Occurrence<'_>> {
    let site = pack.sites.get(site_index)?;
    let row = site.wings.get(index)?;
    Some(Occurrence {
        key: wing_key(site_index, index, site, row),
        kind: 1,
        reference: row.reference,
        parent: site.base,
        ordinal: row.wing_ordinal,
        owners: None,
        wing_owner: row.owner,
        group: None,
        topology_reason: &row.owner_reason,
        site: Some(FuelDemandLocation {
            list: site.list,
            site_ordinal: site.ordinal,
            base: site.base.to_string(),
            container_owner: site.container_owner,
            pool_ordinal: row.pool_ordinal,
            wing_ordinal: row.wing_ordinal,
            reason: reason(&site.reason),
        }),
        rate: row.rate.clone(),
        name: row.name.as_deref(),
        name_reason: &row.name_reason,
    })
}

pub fn occurrences(pack: &Pack) -> impl Iterator<Item = Occurrence<'_>> {
    pack.divisions
        .iter()
        .enumerate()
        .map(|(index, row)| unit(0, index, row))
        .chain(pack.task_forces.iter().enumerate().map(|(index, row)| unit(2, index, row)))
        .chain(pack.sites.iter().enumerate().flat_map(move |(site, item)| (0..item.wings.len()).filter_map(move |index| wing(pack, site, index))))
}

pub fn occurrence<'a>(pack: &'a Pack, key: &Key) -> Option<Occurrence<'a>> {
    let row = match key {
        Key::Unit(0, index, ..) => unit(0, *index, pack.divisions.get(*index)?),
        Key::Unit(2, index, ..) => unit(2, *index, pack.task_forces.get(*index)?),
        Key::Wing(site, index, ..) => wing(pack, *site, *index)?,
        Key::Unit(..) => return None,
    };
    (&row.key == key).then_some(row)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandLocation {
    pub list: u8,
    pub site_ordinal: u32,
    pub base: String,
    pub container_owner: Option<u32>,
    pub pool_ordinal: u32,
    pub wing_ordinal: u32,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandPart {
    pub key: String,
    #[doc = " 0 army, 1 air, 2 navy: the fuel consumer it counts toward."]
    pub kind: u32,
    pub reference: String,
    pub parent: String,
    pub ordinal: u32,
    pub owners: Option<[u32; 3]>,
    pub wing_owner: Option<u32>,
    pub group: Option<String>,
    pub topology_reason: Option<String>,
    pub site: Option<FuelDemandLocation>,
    pub amount_raw: Option<String>,
    pub reason: Option<String>,
    pub recorded_name: Option<String>,
    pub name_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandCommand {
    pub reference: String,
    pub parent: String,
    pub country: u32,
    pub army_group: bool,
    pub members: Vec<String>,
    pub children: Vec<String>,
    pub recorded_name: Option<String>,
    pub name_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandFleetName {
    pub reference: String,
    pub recorded_name: Option<String>,
    pub name_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandAirSite {
    pub list: u8,
    pub ordinal: u32,
    pub base: String,
    pub container_owner: Option<u32>,
    pub capacity_raw: Option<String>,
    pub demilitarized: Option<u8>,
    pub reason: Option<String>,
    #[doc = " Its wings' consumer keys."]
    pub wings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FuelDemandParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " Any hour the read holds: the reply names the read."]
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandReply {
    pub tag: String,
    #[doc = " The country the request named, which a reply is matched to: a slot follows its country through any tag."]
    pub asked: dashboard_base::products::Country,
    #[doc = " The read the pack is of."]
    pub read_hour: i32,
    pub country: u32,
    pub country_instance: Option<u32>,
    #[doc = " The families' sums the read gives, as `country-fuel` shows them."]
    pub consumption: FuelConsumptionAt,
    pub parts: Vec<FuelDemandPart>,
    pub commands: Vec<FuelDemandCommand>,
    pub fleets: Vec<FuelDemandFleetName>,
    pub sites: Vec<FuelDemandAirSite>,
    pub topology_reason: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FuelDemandReply);

fn key_text(key: &Key) -> io::Result<String> {
    serde_json::to_string(key).map_err(|error| dashboard_core::invalid(error.to_string()))
}

pub fn part_of(row: Occurrence<'_>, content: &Content) -> io::Result<FuelDemandPart> {
    let (recorded_name, name_reason) = content.stored(row.name, row.name_reason, "fuel consumer's stored name");
    let (amount_raw, why) = match row.rate {
        Ok(rate) => (Some(daily_required(rate).to_string()), None),
        Err(why) => (None, Some(why)),
    };
    Ok(FuelDemandPart {
        key: key_text(&row.key)?,
        kind: row.kind,
        reference: row.reference.to_string(),
        parent: row.parent.to_string(),
        ordinal: row.ordinal,
        owners: row.owners,
        wing_owner: row.wing_owner,
        group: row.group.map(|group| group.to_string()),
        topology_reason: reason(row.topology_reason),
        site: row.site,
        amount_raw,
        reason: why,
        recorded_name,
        name_reason,
    })
}

pub fn consumption(pack: &FuelDemandRead, parts: &Pack) -> FuelConsumptionAt {
    let rates = pack.native_rates();
    let rates = [rates[0], rates[2], rates[1]];
    let causes = [&pack.army_reason, &pack.air_reason, &pack.navy_reason];
    let mut known = [0_i128; 3];
    let mut positive = [0_i128; 3];
    for row in occurrences(parts) {
        if let Ok(rate) = row.rate {
            let value = i128::from(daily_required(rate));
            known[row.kind as usize] += value;
            positive[row.kind as usize] += value.max(0);
        }
    }
    FuelConsumptionAt {
        native_daily_raw: rates.map(|rate| rate.ok().map(|rate| daily_required(rate).to_string())),
        known_daily_raw: known.map(|value| Some(value.to_string())),
        positive_daily_raw: positive.map(|value| Some(value.to_string())),
        reasons: std::array::from_fn(|at| rates[at].err().map(|message| reason(causes[at]).unwrap_or_else(|| message.to_owned()))),
    }
}

fn counted(reasons: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut counts = BTreeMap::<String, usize>::new();
    for why in reasons {
        *counts.entry(why).or_default() += 1;
    }
    counts.into_iter().map(|(why, count)| format!("{count} current fuel consumption operands refused: {why}")).collect()
}

pub fn demand_reply(asked: &dashboard_base::products::Country, tag: &str, read: &FuelDemandRead, hour: i32, country: u32, content: &Content, cancelled: &AtomicBool) -> io::Result<FuelDemandReply> {
    let pack = Pack::of(read);
    let mut parts = Vec::new();
    for row in occurrences(&pack) {
        dashboard_text::metadata::check_cancel(cancelled)?;
        parts.push(part_of(row, content)?);
    }
    let commands: Vec<FuelDemandCommand> = pack
        .groups
        .iter()
        .map(|group| {
            let (recorded_name, name_reason) = content.stored(group.name.as_deref(), &group.name_reason, "army's stored name");
            FuelDemandCommand {
                reference: group.reference.to_string(),
                parent: group.parent.to_string(),
                country: group.country,
                army_group: group.army_group != 0,
                members: group.members.iter().map(u64::to_string).collect(),
                children: group.children.iter().map(u64::to_string).collect(),
                recorded_name,
                name_reason,
            }
        })
        .collect();
    let fleets: Vec<FuelDemandFleetName> = pack
        .fleets
        .iter()
        .map(|fleet| {
            let (recorded_name, name_reason) = content.stored(fleet.name.as_deref(), &fleet.name_reason, "fleet's stored name");
            FuelDemandFleetName { reference: fleet.reference.to_string(), recorded_name, name_reason }
        })
        .collect();
    let mut sites = Vec::new();
    for (index, site) in pack.sites.iter().enumerate() {
        let wings = site.wings.iter().enumerate().map(|(at, wing)| key_text(&wing_key(index, at, site, wing))).collect::<io::Result<Vec<_>>>()?;
        sites.push(FuelDemandAirSite {
            list: site.list,
            ordinal: site.ordinal,
            base: site.base.to_string(),
            container_owner: site.container_owner,
            capacity_raw: site.capacity_raw.map(|value| value.to_string()),
            demilitarized: site.demilitarized,
            reason: reason(&site.reason),
            wings,
        });
    }
    let consumption = consumption(read, &pack);
    let topology_reason = reason(&pack.group_reason);
    let mut warnings = counted(
        parts
            .iter()
            .flat_map(|part| [&part.reason, &part.name_reason, &part.topology_reason])
            .flatten()
            .cloned()
            .chain(sites.iter().filter_map(|site| site.reason.clone()))
            .chain(commands.iter().filter_map(|group| group.name_reason.clone()))
            .chain(fleets.iter().filter_map(|fleet| fleet.name_reason.clone()))
            .chain(consumption.reasons.iter().flatten().cloned())
            .chain(topology_reason.clone()),
    );
    warnings.extend(content.warnings.iter().cloned());
    Ok(FuelDemandReply {
        tag: tag.to_owned(),
        asked: asked.clone(),
        read_hour: hour,
        country,
        country_instance: read.country_instance,
        consumption,
        parts,
        commands,
        fleets,
        sites,
        topology_reason,
        warnings,
    })
}

fn branch(id: &str) -> io::Result<u64> {
    id.parse().map_err(|_| dashboard_core::invalid("Invalid branch"))
}

pub struct CountryFuelDemand;

impl Product for CountryFuelDemand {
    const KEY: &'static str = "country-fuel-demand";
    const CONTENT_DEPENDENT: bool = true;
    const INTERACTIVE: bool = true;
    type Params = FuelDemandParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read];
    type Reply = FuelDemandReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![
            FuelDemandParams::decl(),
            FuelDemandReply::decl(),
            FuelDemandLocation::decl(),
            FuelDemandPart::decl(),
            FuelDemandCommand::decl(),
            FuelDemandFleetName::decl(),
            FuelDemandAirSite::decl(),
        ]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let id = holder_id(&query, &params.country, params.hour)?;
        let key = u128::from(id);
        use dashboard_base::products::at_hour::{Keys, Read, frame};
        let frame = frame(
            &query,
            params.hour,
            &[
                Read { table: TableId::CountryFuel, keys: Keys::These(vec![key]) },
                Read { table: TableId::FuelDemandParts, keys: Keys::Range(key << 83, (key << 83) | ((1u128 << 83) - 1)) },
            ],
            "Fuel",
        )?;
        let fuel = frame.complete::<tables::CountryFuel>().map_err(|error| dashboard_core::refused(format!("Fuel is not recorded at this hour: {error}")))?;
        let row = fuel.iter().find(|row| row.country == id).ok_or_else(|| dashboard_core::refused("No fuel status is recorded for this country at this hour"))?;
        let read_hour = super::read_hour(row, frame.tables[TableId::CountryFuel.index()].sampled_at())
            .ok_or_else(|| dashboard_core::refused("The fuel read holding this hour is not known"))?;
        let parts = frame.complete::<tables::FuelDemandParts>().map_err(|_| PARTS_UNAVAILABLE);
        let pack = pack_at(row, parts, read_hour).map_err(dashboard_core::refused)?;
        let content = Content::read(source, load);
        let reply = demand_reply(&params.country, &query.holders.name(&params.country, params.hour), &pack, read_hour, id, &content, cancelled)?;
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FuelDemandHistoryParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " The hour the consumers were picked at: the country is its holder then."]
    pub hour: i32,
    pub window: i32,
    #[doc = " The consumers' keys, as `country-fuel-demand` gives them."]
    pub keys: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
    pub amount_raw: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandHistoryPart {
    pub key: String,
    pub runs: Vec<FuelDemandRun>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct FuelDemandHistoryReply {
    pub tag: String,
    pub country: u32,
    pub window: i32,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " In the order asked."]
    pub parts: Vec<FuelDemandHistoryPart>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(FuelDemandHistoryReply);

const NO_READ: &str = "Current fuel consumption was not recorded in the selected native country/recording segment";

pub fn demand_runs(reads: &[ReadRow], keys: &[Key], first: i32, last: i32) -> Vec<Vec<FuelDemandRun>> {
    let spans = spans(reads, first, last, true);
    let mut out: Vec<Vec<FuelDemandRun>> = keys.iter().map(|_| Vec::with_capacity(spans.len())).collect();
    for span in spans {
        let (read, first_hour, last_hour) = match span {
            Span::Gap { first, last } => {
                for runs in &mut out {
                    runs.push(FuelDemandRun { first_hour: first, last_hour: last, starts_segment: true, amount_raw: None, reason: Some(NO_READ.to_owned()) });
                }
                continue;
            }
            Span::Read { read, first, last } => (read, first, last),
        };
        let pack = pack_at(&read.row, read.parts.as_deref().map_err(|why| *why), read.hour).map(|pack| Pack::of(&pack));
        for (key, runs) in keys.iter().zip(&mut out) {
            let (amount_raw, why) = match &pack {
                Err(why) => (None, Some(why.clone())),
                Ok(pack) => match occurrence(pack, key) {
                    None => (None, Some("This consumer is not in the pack at this read".to_owned())),
                    Some(row) => match row.rate {
                        Ok(rate) => (Some(daily_required(rate).to_string()), None),
                        Err(why) => (None, Some(why)),
                    },
                },
            };
            runs.push(FuelDemandRun { first_hour, last_hour, starts_segment: read.starts_segment, amount_raw, reason: why });
        }
    }
    out
}

pub struct CountryFuelDemandHistory;

impl Product for CountryFuelDemandHistory {
    const KEY: &'static str = "country-fuel-demand-history";
    const CONTENT_DEPENDENT: bool = false;
    type Params = FuelDemandHistoryParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, hour: Read, window, keys];
    type Reply = FuelDemandHistoryReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)?;
        if params.keys.is_empty() { Err("Choose a fuel consumer") } else { Ok(()) }
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![FuelDemandHistoryParams::decl(), FuelDemandHistoryReply::decl(), FuelDemandHistoryPart::decl(), FuelDemandRun::decl()]
    }

    fn load(source: &Source, params: &Self::Params, _load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let keys = params
            .keys
            .iter()
            .map(|key| serde_json::from_str::<Key>(key).map_err(|_| dashboard_core::invalid("Invalid fuel consumer key")))
            .collect::<io::Result<Vec<Key>>>()?;
        let query = dashboard_base::queries::query(source, branch(&params.branch_id)?)?;
        let id = holder_id(&query, &params.country, params.hour)?;
        let (first, last) = window_span(params.window)?;
        let reads = country_reads(&query, id, first, last, Detail::DemandParts, cancelled)?;
        let parts = demand_runs(&reads.reads, &keys, first, last).into_iter().zip(&params.keys).map(|(runs, key)| FuelDemandHistoryPart { key: key.clone(), runs }).collect();
        let reply = FuelDemandHistoryReply { tag: query.holders.name(&params.country, params.hour), country: id, window: params.window, first_hour: first, last_hour: last, parts, warnings: reads.warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
