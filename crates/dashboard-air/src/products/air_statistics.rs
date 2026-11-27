
use dashboard_base::products::{Country, validate_country};
use dashboard_base::products::{LoadContext, Product};
use dashboard_base::{Source, catalogue, products::at_hour::{self, At, AtHour, Keys, Owner, Read}};
use crate::products::air_missions::{AirMissionTheatre, THEATRE_TABLES, theatre_wings, theatres_at};
use dashboard_follow::live::{Cells, Source as Cell};
use dashboard_query::identity::{Holder, TAG_UNLINKED};
use engine_air_history::cloop_history::air_stats;
use content_reader_localisation::localisation::Argument;
use replay_data::rows::{AirStatisticsRow, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[cfg(test)]
mod tests;

pub const PAIRS: usize = air_stats::CHANNELS.len() / 2;

const DETAIL_CHANNELS: [u8; PAIRS] = [1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 8, 12];

pub struct AirStatisticsAt;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirStatisticsParams {
    pub branch_id: String,
    pub country: Country,
    #[doc = " Pair index in recorded schema order, not the interface's button order."]
    pub channel: u8,
    #[doc = " Only these regions; every region the holder has statistics in when empty."]
    #[serde(default)]
    pub regions: Vec<u32>,
    #[doc = " Only regions of these air theatres at the hour; refused with the reason while membership is not known."]
    #[serde(default)]
    pub theatres: Vec<String>,
    #[doc = " The most active regions (ours plus the enemy's), when no region is chosen."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub top: Option<u32>,
}

#[derive(Default)]
pub(crate) struct Shown {
    pub regions: Option<BTreeSet<u32>>,
    pub top: Option<usize>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirStatisticsChannel {
    pub id: String,
    pub ours: String,
    pub enemy: String,
    pub divisor: i32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirStatisticsRegion {
    pub region: String,
    pub name: Option<String>,
    #[doc = " Why the name is not shown, without one."]
    pub missing: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirStatisticsReply {
    #[doc = " The hour of the read the rows hold; none when not known."]
    pub read_hour: Option<i32>,
    pub channels: Vec<AirStatisticsChannel>,
    pub regions: Vec<AirStatisticsRegion>,
    #[doc = " Why the statistics are not shown at the hour, when they are not."]
    pub missing: Option<String>,
    #[doc = " The country's air theatres and their member wings' regions at the hour; none where"]
    #[doc = " membership is not known."]
    pub theatres: Option<Vec<AirMissionTheatre>>,
    #[doc = " Why the theatres are not known at the hour."]
    pub theatres_missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AirStatisticsReply);

pub struct Names {
    catalogue: Result<Arc<catalogue::Catalogue>, String>,
    regions: Result<BTreeMap<u32, Result<String, String>>, String>,
    channels: Vec<AirStatisticsChannel>,
    warnings: Vec<String>,
}

fn divisor(channel: usize) -> i32 {
    if matches!(air_stats::CHANNELS[channel], air_stats::OUR_BUILDINGS_BOMBED | air_stats::ENEMY_BUILDINGS_BOMBED) {
        air_stats::BUILDINGS_BOMBED_DISPLAY_DIVISOR as i32
    } else {
        air_stats::DISPLAY_DIVISOR as i32
    }
}

fn names(source: &Source, load: &LoadContext) -> Names {
    let mut warnings = Vec::new();
    let cat = load.content().and_then(|options| catalogue::catalogue(source, options)).map_err(|error| format!("Recorded content stack unavailable; choose matching game and user folders ({error})"));
    let mut label = |channel: usize, enemy: bool| -> String {
        let key = format!("AIRVIEW_DETAILS_CHANNEL_{}", DETAIL_CHANNELS[channel] + if enemy { 12 } else { 0 });
        let text = cat.as_ref().map_err(Clone::clone).and_then(|cat| {
            if cat.localisation.get(&key).is_none() {
                return Err(format!("Localisation {key} is absent from the recorded content stack"));
            }
            let text = cat.localisation.localise_with(key.as_bytes(), &[("VAL", Argument::Text(""))]).map_err(|error| error.to_string())?;
            catalogue::plain_text(&text).map(|text| text.trim().to_owned())
        });
        text.unwrap_or_else(|why| {
            warnings.push(format!("Channel {channel} {} name unavailable: {why}", if enemy { "enemy" } else { "ours" }));
            "Channel name unavailable".to_owned()
        })
    };
    let channels = (0..PAIRS).map(|channel| AirStatisticsChannel { id: channel.to_string(), ours: label(channel, false), enemy: label(channel, true), divisor: divisor(channel) }).collect();
    let regions = cat.as_ref().map_err(Clone::clone).and_then(|cat| {
        warnings.extend(cat.verification.iter().cloned());
        warnings.extend(cat.warnings.iter().cloned());
        let content = dashboard_base::content::strategic_regions(&cat.content.layers).map_err(|error| error.to_string())?;
        warnings.extend(content.notes.iter().cloned());
        Ok(content.regions.keys().map(|&id| (id, dashboard_world::products::region_names::region_name(cat, &content, id))).collect())
    });
    Names { catalogue: cat, regions, channels, warnings }
}

fn channel_column(channel: usize) -> io::Result<usize> {
    let table = TableId::AirStatistics;
    (0..table.field_count())
        .filter(|&field| table.field_name(field).is_some_and(|name| name.starts_with("channels")))
        .nth(channel)
        .ok_or_else(|| dashboard_core::refused(format!("Air statistics channel {channel} is not in this build's schema")))
}

fn region(names: &Names, id: u32) -> AirStatisticsRegion {
    let (name, missing) = match names.regions.as_ref().map(|regions| regions.get(&id)) {
        Ok(Some(Ok(name))) => (Some(name.clone()), None),
        Ok(Some(Err(why))) => (None, Some(why.clone())),
        Ok(None) => (None, Some(format!("Region {id} absent from the recorded stack"))),
        Err(why) => (None, Some(why.clone())),
    };
    AirStatisticsRegion { region: id.to_string(), name, missing }
}

fn owners(frame: &DecodedFrame, owner: Owner, note: &mut Option<String>) -> Vec<u32> {
    let mut ids: Vec<u32> = match frame.complete::<tables::CountryLinks>() {
        Ok(links) => links.iter().filter(|link| link.canonical_slot == owner.slot).map(|link| link.id).collect(),
        Err(error) => {
            *note = Some(format!("Only the country's own id's air statistics are shown: its other link ids are not known at this hour ({error})"));
            Vec::new()
        }
    };
    ids.push(owner.id);
    ids.sort_unstable();
    ids.dedup();
    ids
}

pub(crate) fn reply(rows: io::Result<&[AirStatisticsRow]>, owners: &[u32], read_hour: Option<i32>, channel: usize, names: &Names, live: &Cells, shown: &Shown) -> io::Result<AirStatisticsReply> {
    let mut warnings = names.warnings.clone();
    let (ours, enemy) = (channel_column(channel)?, channel_column(channel + PAIRS)?);
    let (regions, missing) = match rows {
        Err(error) => (Vec::new(), Some(format!("Air statistics are not recorded at this hour: {error}"))),
        Ok(rows) => {
            let mut held: BTreeMap<u32, &AirStatisticsRow> = BTreeMap::new();
            let mut shared = 0usize;
            for row in rows.iter().filter(|row| owners.binary_search(&row.country).is_ok()) {
                let kept = held.entry(row.region).or_insert(row);
                if kept.country != row.country {
                    shared += 1;
                    if row.country < kept.country {
                        *kept = row;
                    }
                }
            }
            if shared > 0 {
                warnings.push(format!("{shared} air statistics rows of the country's other link ids not shown: a region they share shows the lowest id's row"));
            }
            let mut ranked: Vec<(i128, u32, &AirStatisticsRow)> = held
                .iter()
                .filter(|(id, _)| shown.regions.as_ref().is_none_or(|regions| regions.contains(id)))
                .map(|(&id, row)| (i128::from(row.channels[channel]).abs() + i128::from(row.channels[channel + PAIRS]).abs(), id, *row))
                .collect();
            ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            if let Some(top) = shown.top {
                ranked.truncate(top);
            }
            let mut regions = Vec::with_capacity(ranked.len());
            for (_, id, row) in ranked {
                let key = u128::from(row.country) << 32 | u128::from(id);
                let shown = region(names, id);
                live.value(&shown.region, "ours", Cell { table: TableId::AirStatistics, column: ours, key }, 1.0);
                live.value(&shown.region, "enemy", Cell { table: TableId::AirStatistics, column: enemy, key }, 1.0);
                regions.push(shown);
            }
            (regions, None)
        }
    };
    if let Err(why) = &names.regions {
        warnings.push(why.clone());
    }
    Ok(AirStatisticsReply { read_hour, channels: names.channels.clone(), regions, missing, theatres: None, theatres_missing: None, warnings })
}

impl AtHour for AirStatisticsAt {
    const KEY: &'static str = "air-statistics";
    const TITLE: &'static str = "Air statistics";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AirStatisticsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, channel, regions, "regions[]", theatres, "theatres[]", top];
    type Reply = AirStatisticsReply;
    type Context = Names;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)?;
        if usize::from(params.channel) >= PAIRS {
            return Err("Invalid air-statistics channel");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AirStatisticsParams::decl(), AirStatisticsReply::decl(), AirStatisticsChannel::decl(), AirStatisticsRegion::decl(), AirMissionTheatre::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&Country> {
        Some(&params.country)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = vec![TableId::AirStatistics, TableId::AirWings];
        tables.extend(THEATRE_TABLES);
        tables
    }

    fn reads(_params: &Self::Params, owner: Option<Owner>, step: usize, held: &DecodedFrame) -> Vec<Read> {
        match (owner, step) {
            (Some(_), 0) => THEATRE_TABLES.into_iter().map(Read::all).collect(),
            (Some(owner), 1) => {
                let ranges = owners(held, owner, &mut None).into_iter().map(|id| (u128::from(id) << 32, u128::from(id) << 32 | u128::from(u32::MAX))).collect();
                let mut reads = vec![Read { table: TableId::AirStatistics, keys: Keys::Ranges(ranges) }];
                reads.extend(theatre_wings(held, owner.slot));
                reads
            }
            _ => Vec::new(),
        }
    }

    fn valid_through(frame: &DecodedFrame, _params: &Self::Params, _at: &At<'_, Self::Context>) -> i32 {
        let dated = [TableId::AirStatistics, TableId::AirTheatres, TableId::AirTheatreGroups, TableId::AirTheatreMembers, TableId::CountryLinks, TableId::AirWings].iter().any(|table| {
            let view = &frame.tables[table.index()];
            !view.is_not_selected() && !matches!(view.held(), replay_frame::frame::TableView::Complete(_))
        });
        if dated { frame.hour } else { i32::MAX }
    }

    fn projection(params: &Self::Params, table: TableId, _plan: &[(Vec<Read>, i32)], _context: &Names) -> Option<Vec<usize>> {
        if table == TableId::AirStatistics {
            let channel = usize::from(params.channel);
            return Some(vec![channel_column(channel).ok()?, channel_column(channel + PAIRS).ok()?]);
        }
        crate::products::air_missions::theatre_columns(table)
    }

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(names(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Air statistics country is unavailable at this hour"))?;
        let read_hour = frame.tables[TableId::AirStatistics.index()].sampled_at();
        let mut note = None;
        let owners = owners(frame, owner, &mut note);
        let theatres = theatres_at(frame, owner.slot, &at.context.catalogue);
        let chosen: Option<BTreeSet<u32>> = (!params.regions.is_empty()).then(|| params.regions.iter().copied().collect());
        let in_theatres: Option<Result<BTreeSet<u32>, String>> = (!params.theatres.is_empty()).then(|| {
            theatres.as_ref().map_err(Clone::clone).map(|theatres| {
                theatres.iter().filter(|theatre| params.theatres.contains(&theatre.id)).flat_map(|theatre| theatre.regions.iter().filter_map(|region| region.parse().ok())).collect()
            })
        });
        let (regions, refused) = match (chosen, in_theatres) {
            (_, Some(Err(why))) => (Some(BTreeSet::new()), Some(format!("Air theatre membership is not recorded at this hour: {why}"))),
            (Some(chosen), Some(Ok(theatre))) => (Some(chosen.intersection(&theatre).copied().collect()), None),
            (chosen, Some(Ok(theatre))) => (chosen.or(Some(theatre)), None),
            (chosen, None) => (chosen, None),
        };
        let top = if params.regions.is_empty() { params.top.map(|top| usize::try_from(top).unwrap_or(usize::MAX)) } else { None };
        let mut reply = reply(frame.complete::<tables::AirStatistics>(), &owners, read_hour, usize::from(params.channel), at.context, at.live, &Shown { regions, top })?;
        reply.warnings.extend(note);
        if let Some(why) = refused {
            reply.missing.get_or_insert(why);
        }
        match theatres {
            Ok(theatres) => reply.theatres = Some(theatres),
            Err(why) => reply.theatres_missing = Some(why),
        }
        Ok(reply)
    }
}

pub struct AirStatisticsRegions;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirStatisticsRegionsParams {
    pub branch_id: String,
    pub country: Country,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirStatisticsRegionsReply {
    #[doc = " Every region the country held statistics in over the timeline, in region order."]
    pub regions: Vec<AirStatisticsRegion>,
    pub missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AirStatisticsRegionsReply);

pub(crate) fn regions_of(keys: impl IntoIterator<Item = u128>, ids: &BTreeSet<u32>) -> BTreeSet<u32> {
    keys.into_iter().filter(|&key| ids.contains(&((key >> 32) as u32))).map(|key| key as u32).collect()
}

impl Product for AirStatisticsRegions {
    const KEY: &'static str = "air-statistics-regions";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AirStatisticsRegionsParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country];
    type Reply = AirStatisticsRegionsReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_country(&params.country)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AirStatisticsRegionsParams::decl(), AirStatisticsRegionsReply::decl()]
    }

    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let names = names(source, load);
        let mut warnings = names.warnings.clone();
        let runs = query.holders.runs(&params.country);
        let unresolved = runs.iter().find(|run| matches!(run.holder, Holder::Unresolved)).map(|run| run.first)
            .or_else(|| query.holders.unknown.first().map(|&(first, ..)| first))
            .or_else(|| {
                if !matches!(params.country, Country::Slot(_)) {
                    return None;
                }
                query.holders.unlinked().iter().find_map(|&(start, end)| {
                    let mut first = start;
                    for held in &runs[runs.partition_point(|run| run.last < start)..] {
                        if held.first > first {
                            break;
                        }
                        if held.last >= end {
                            return None;
                        }
                        first = held.last + 1;
                    }
                    Some(first)
                })
            });
        let mut missing = unresolved.map(|hour| {
            query.holders.not_known(&params.country, hour).unwrap_or_else(|| format!("Who holds {} is not known: {TAG_UNLINKED}", params.country.key()))
        });
        let found = || runs.iter().filter_map(|run| match run.holder {
            Holder::Found { slot, id } => Some((slot, id)),
            Holder::Unresolved => None,
        });
        let slots: BTreeSet<u32> = found().map(|(slot, _)| slot).collect();
        let mut ids: BTreeSet<u32> = found().map(|(_, id)| id).collect();
        if !slots.is_empty() {
            let frame = at_hour::frame(&query, query.timeline.last, &[Read::all(TableId::CountryLinks)], "Air statistics");
            let reason = match frame {
                Ok(frame) => {
                    let links = dashboard_query::coverage::witnessed_rows::<tables::CountryLinks>(&frame);
                    ids.extend(links.rows.iter().filter(|link| slots.contains(&link.canonical_slot)).map(|link| link.id));
                    links.missing
                }
                Err(error) => Some(error.to_string()),
            };
            if let Some(reason) = reason {
                let reason = format!("Some of the country's other link ids are not known at the timeline's end ({reason})");
                missing.get_or_insert_with(|| reason.clone());
                warnings.push(reason);
            }
        }
        let recording = query.replay.recording();
        let mut held = BTreeSet::new();
        let mut read = BTreeSet::new();
        for stretch in &query.timeline.stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            if missing.is_none() {
                let table = TableId::AirStatistics;
                let statuses = dashboard_query::read::statuses(recording, stretch.run, table, stretch.first, stretch.last)?;
                if let Some(status) = statuses.iter().find(|status| status.status != replay_columnar::manifest::Status::Complete) {
                    let reasons = dashboard_query::read::status_reasons(recording, stretch.run, table, std::slice::from_ref(status))?;
                    let reason = match status.status {
                        replay_columnar::manifest::Status::Unavailable { reason } => reasons.get(&reason).map(String::as_str),
                        _ => None,
                    };
                    missing = dashboard_query::coverage::row_coverage(table, status.status, reason, false).into_missing();
                }
            }
            if read.insert(stretch.run) {
                let lifetimes = dashboard_query::cost::lifetimes(recording, stretch.run, TableId::AirStatistics)?;
                held.extend(regions_of(lifetimes.into_iter().map(|(key, ..)| key), &ids));
            }
        }
        if let Err(why) = &names.regions {
            warnings.push(why.clone());
        }
        Ok(AirStatisticsRegionsReply { regions: held.into_iter().map(|id| region(&names, id)).collect(), missing, warnings })
    }
}
