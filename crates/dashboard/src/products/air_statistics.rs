//! One country's air statistics at an hour, ours against the enemy's per strategic region for one
//! of the twelve channel pairs: the game's current year window as its last read before the hour
//! holds it. A rows-at-an-hour product ([`AtHour`]): its structure is the regions with a row and
//! their names; each region's two words are live cells (`ours`, `enemy`) read as recorded, a
//! region without a row absent, never zero. The holder's rows are those of every raw country
//! linked to its slot; a region two of them hold shows the lowest id's row (one unit a region).
//! `air-statistics-regions` names every region the country held statistics in over the timeline,
//! so a region not recorded at the hour shown can still be chosen.

use super::{LoadContext, Product};
use crate::{
    Source, catalogue,
    products::{
        air_missions::{AirMissionTheatre, THEATRE_TABLES, theatre_wings, theatres_at},
        at_hour::{self, At, AtHour, Keys, Owner, Read},
    },
};
use dashboard_base::products::validate_tag;
use dashboard_follow::live::{Cells, Source as Cell};
use engine_air_history::cloop_history::air_stats;
use installed_content_localisation::localisation::Argument;
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

/// The recorded channels: twelve of ours, then the same twelve of the enemy's.
pub const PAIRS: usize = air_stats::CHANNELS.len() / 2;

/// Interface detail-channel ordinals for the schema's twelve pairs: disruption precedes ground
/// equipment losses in the interface, but not in stored rows.
const DETAIL_CHANNELS: [u8; PAIRS] = [1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 8, 12];

pub struct AirStatisticsAt;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirStatisticsParams {
    pub branch_id: String,
    pub tag: String,
    /// Pair index in recorded schema order, not the interface's button order.
    pub channel: u8,
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

/// A region with statistics at the hour; its raw signed words are the live cells (`region`,
/// `ours`) and (`region`, `enemy`).
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirStatisticsRegion {
    pub region: String,
    pub name: Option<String>,
    /// Why the name is not shown, without one.
    pub missing: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirStatisticsReply {
    /// The hour of the read the rows hold; none when not known.
    pub read_hour: Option<i32>,
    pub channels: Vec<AirStatisticsChannel>,
    pub regions: Vec<AirStatisticsRegion>,
    /// Why the statistics are not shown at the hour, when they are not.
    pub missing: Option<String>,
    /// The country's air theatres and their member wings' regions at the hour; none where
    /// membership is not known.
    pub theatres: Option<Vec<AirMissionTheatre>>,
    /// Why the theatres are not known at the hour.
    pub theatres_missing: Option<String>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AirStatisticsReply);

/// The recorded content a reply names with: every strategic region's name and the channels.
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
        Ok(content.regions.keys().map(|&id| (id, super::air_ledger::region_name(cat, &content, id))).collect())
    });
    Names { catalogue: cat, regions, channels, warnings }
}

/// The table's column holding recorded channel `channel`.
fn channel_column(channel: usize) -> io::Result<usize> {
    let table = TableId::AirStatistics;
    (0..table.field_count())
        .filter(|&field| table.field_name(field).is_some_and(|name| name.starts_with("channels")))
        .nth(channel)
        .ok_or_else(|| dashboard_core::refused(format!("Air statistics channel {channel} is not in this build's schema")))
}

/// Region `id` as the content names it, or why it has no name.
fn region(names: &Names, id: u32) -> AirStatisticsRegion {
    let (name, missing) = match names.regions.as_ref().map(|regions| regions.get(&id)) {
        Ok(Some(Ok(name))) => (Some(name.clone()), None),
        Ok(Some(Err(why))) => (None, Some(why.clone())),
        Ok(None) => (None, Some(format!("Region {id} absent from the recorded stack"))),
        Err(why) => (None, Some(why.clone())),
    };
    AirStatisticsRegion { region: id.to_string(), name, missing }
}

/// The raw countries whose rows are `owner`'s at `frame`'s hour, ascending: every id linked to its
/// slot and its own; only its own where the links are not recorded, which `note` then names.
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

/// The reply at `rows`' hour for pair `channel`: each region's row of the raw countries `owners`
/// (ascending), the lowest id's where several hold it, in region order, its two words named in
/// `live`.
pub(crate) fn reply(rows: io::Result<&[AirStatisticsRow]>, owners: &[u32], read_hour: Option<i32>, channel: usize, names: &Names, live: &Cells) -> io::Result<AirStatisticsReply> {
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
            let mut regions = Vec::with_capacity(held.len());
            for (&id, row) in &held {
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
    type Reply = AirStatisticsReply;
    type Context = Names;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_tag(&params.tag)?;
        if usize::from(params.channel) >= PAIRS {
            return Err("Invalid air-statistics channel");
        }
        Ok(())
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AirStatisticsParams::decl(), AirStatisticsReply::decl(), AirStatisticsChannel::decl(), AirStatisticsRegion::decl(), AirMissionTheatre::decl()]
    }

    fn owner(params: &Self::Params) -> Option<&str> {
        Some(&params.tag)
    }

    fn tables(_params: &Self::Params) -> Vec<TableId> {
        let mut tables = vec![TableId::AirStatistics, TableId::AirWings];
        tables.extend(THEATRE_TABLES);
        tables
    }

    /// The theatre tables (the links among them), then the holder's ids' rows alone (the table is
    /// keyed by raw country above region) and the wings of its theatres' members.
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

    fn context(source: &Source, load: &LoadContext) -> io::Result<Self::Context> {
        Ok(names(source, load))
    }

    fn reply(frame: &DecodedFrame, params: &Self::Params, at: &At<'_, Self::Context>) -> io::Result<Self::Reply> {
        let owner = at.owner.ok_or_else(|| dashboard_core::refused("Air statistics country is unavailable at this hour"))?;
        let read_hour = frame.tables[TableId::AirStatistics.index()].sampled_at();
        let mut note = None;
        let owners = owners(frame, owner, &mut note);
        let mut reply = reply(frame.complete::<tables::AirStatistics>(), &owners, read_hour, usize::from(params.channel), at.context, at.live)?;
        reply.warnings.extend(note);
        match theatres_at(frame, owner.slot, &at.context.catalogue) {
            Ok(theatres) => reply.theatres = Some(theatres),
            Err(why) => reply.theatres_missing = Some(why),
        }
        Ok(reply)
    }
}

pub struct AirStatisticsRegions;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirStatisticsRegionsParams {
    pub branch_id: String,
    pub tag: String,
}

#[derive(Clone, Debug, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct AirStatisticsRegionsReply {
    /// Every region the country held statistics in over the timeline, in region order.
    pub regions: Vec<AirStatisticsRegion>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(AirStatisticsRegionsReply);

/// The regions of `keys` (raw country above region) whose country is one of `ids`.
pub(crate) fn regions_of(keys: impl IntoIterator<Item = u128>, ids: &BTreeSet<u32>) -> BTreeSet<u32> {
    keys.into_iter().filter(|&key| ids.contains(&((key >> 32) as u32))).map(|key| key as u32).collect()
}

impl Product for AirStatisticsRegions {
    const KEY: &'static str = "air-statistics-regions";
    const CONTENT_DEPENDENT: bool = true;
    type Params = AirStatisticsRegionsParams;
    type Reply = AirStatisticsRegionsReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        validate_tag(&params.tag)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![AirStatisticsRegionsParams::decl(), AirStatisticsRegionsReply::decl()]
    }

    /// From each run's key lifetimes (summaries, no row decoded): the keys of the ids the tag's
    /// holders had, and of the ids linked to their slots at the timeline's last hour.
    fn load(source: &Source, params: &Self::Params, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = crate::answer::query(source, branch)?;
        let names = names(source, load);
        let mut warnings = names.warnings.clone();
        let runs = query.holders.tags.get(&params.tag).map(Vec::as_slice).unwrap_or_default();
        let found = || runs.iter().filter_map(|run| match run.holder {
            dashboard_query::identity::Holder::Found { slot, id } => Some((slot, id)),
            dashboard_query::identity::Holder::Unresolved => None,
        });
        let slots: BTreeSet<u32> = found().map(|(slot, _)| slot).collect();
        let mut ids: BTreeSet<u32> = found().map(|(_, id)| id).collect();
        if !slots.is_empty() {
            let frame = at_hour::frame(&query, query.timeline.last, &[Read::all(TableId::CountryLinks)], "Air statistics");
            match frame.and_then(|frame| frame.complete::<tables::CountryLinks>().map(|links| links.iter().filter(|link| slots.contains(&link.canonical_slot)).map(|link| link.id).collect::<Vec<_>>())) {
                Ok(linked) => ids.extend(linked),
                Err(error) => warnings.push(format!("Regions of the country's other link ids are not listed: its links are not known at the timeline's end ({error})")),
            }
        }
        let recording = query.replay.recording();
        let mut held = BTreeSet::new();
        let mut read = BTreeSet::new();
        for stretch in &query.timeline.stretches {
            crate::metadata::check_cancel(cancelled)?;
            if read.insert(stretch.run) {
                let lifetimes = dashboard_query::cost::lifetimes(recording, stretch.run, TableId::AirStatistics)?;
                held.extend(regions_of(lifetimes.into_iter().map(|(key, ..)| key), &ids));
            }
        }
        if let Err(why) = &names.regions {
            warnings.push(why.clone());
        }
        Ok(AirStatisticsRegionsReply { regions: held.into_iter().map(|id| region(&names, id)).collect(), warnings })
    }
}
