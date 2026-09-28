//! One country's strategic resources over a branch, for the Resource balance
//! widget: where each resource comes from and where it goes, and the trade
//! deals behind its imports and exports.
//!
//! Every value is a recorded native pool (`CountryResources`), fixed point with
//! [`RESOURCE_DECIMALS`] places, kept exact. Against a fresh text save at a
//! full-refresh hour (W2 part-G, branch 1, hour 60827807) all 3,073 country
//! resources match the save's `resources` block exactly: `produced`,
//! `adjustment` (`transfer_overlord_subject`), `imported`, `used` (the first
//! `to_use` array), `special_total` (the first plus the second), and
//! `production_demand` (the third), `to_export`, `base_export` and `exported`.
//! What they mean:
//!
//! - `used` is what the country has for use: in 3,059 of the 3,073 it is exactly
//!   produced + adjustment + imported − `to_export`, and the rest differ only by
//!   the game's rounding. What is taken off is the export reserve the trade law
//!   sets aside (`to_export`), not what buyers took (`exported`), which differs
//!   in 266 of them.
//! - `production_demand` is minus what the production lines need, met or not:
//!   Germany's lines in the save use aluminium 11, steel 30, coal 17 and
//!   tungsten 1 and lack 5 rubber, and it holds −11, −30, −17, −1 and −5.
//! - `special_total` is what is left after the second array, which is never
//!   positive. The game names that step: its resource breakdown lists "Needed by
//!   Special Projects" between what it has for use and "Needed by Production
//!   Lines" (`RESOURCE_DETAILS_SURPLUS`), and special projects declare a
//!   `resource_cost`. Which other consumers it holds, if any, is not checked.
//! - `special_total` + `production_demand` is the game's surplus, or its deficit
//!   when negative.
//!
//! `adjustment` is resources moved between a subject and its overlord: positive
//! for the overlord, negative for the subject. `base_export` (the reserve before
//! modifiers) is not shown.
//!
//! The resource tables are read once a day in one slot and carried in between.
//! A read is an hour whose frame timed a walk of any table of that slot, as the
//! fresh-save oracle dates daily tables (`recorder::oracle::last_read_hours`).
//! Values start only at a read, are carried over the following hours of the same
//! country identity, and end at a gap, an incomplete table or a change of
//! identity; `reads` lets the widget say how old a carried value is.
//!
//! The pools and the deals are captured as separate tables with their own
//! outcomes, so each keeps its own values: a read whose deal table is missing
//! still shows the pools, and `deal_hours` says at which hours the deals are known.
//!
//! Resources are told apart by their native index, not their token: the game
//! keeps every declaration as its own resource (see [`ResourceInfo`]).
//!
//! Trades are `ResourceExchanges` rows (a classic trade deal each) naming the
//! country as exporter or importer. `delivered` is the deal's daily amount,
//! exact against the save's `delivered`. At the same hour a country's import
//! deals add up to `imported` in 3,055 of the 3,073 and its export deals to
//! `exported` in 3,065, so the widget shows any remainder as its own part.

use super::{CountryParams, LoadContext, Product};
use crate::{
    Source,
    catalogue::{self, Catalogue},
    metadata::{link_slot, slot_read, tagged_country, validate_campaign},
    products::country_groups::HourSpan,
};
use replay::{
    container::Replay,
    frame::FrameDecoder,
    rows::{TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

pub struct ResourceBalance;

/// Decimal places of every recorded resource amount.
pub const RESOURCE_DECIMALS: u8 = 5;
/// Runs kept per country before the scan refuses, far above a long campaign's
/// daily changes.
const MAX_RUNS: usize = 400_000;
const CACHED_COUNTRIES: usize = 8;
/// Longest side of a returned resource icon.
const ICON_EDGE: u32 = 48;
/// The trade view's resource row and its icon button, the element the game gives
/// each resource's `icon_frame` (`interface/countrytradeview.gui`).
const TRADE_VIEW_GUI: &str = "interface/countrytradeview.gui";
const RESOURCE_ROW: &str = "resources_info_entry";
const RESOURCE_ICON: &str = "resources_icon";

/// One recorded read of a country's pools of one resource, held over
/// consecutive hours while it stays unchanged. Raw values are exact decimal
/// strings with [`RESOURCE_DECIMALS`] places.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoolRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// The run follows a gap, a missing read or another country identity.
    pub starts_segment: bool,
    /// Extracted in the country's states (`produced`).
    pub produced_raw: String,
    /// Moved from subjects (positive) or to the overlord (negative) (`adjustment`).
    pub subjects_raw: String,
    pub imported_raw: String,
    /// Set aside for export by the trade law (`to_export`).
    pub to_export_raw: String,
    /// What buyers took (`exported`).
    pub exported_raw: String,
    /// What the country has for use (`used`).
    pub available_raw: String,
    /// What is left for use after special projects (`special_total`).
    pub after_projects_raw: String,
    /// Minus what production lines need, met or not (`production_demand`).
    pub production_raw: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResourceSeries {
    /// The resource's native index (`ResourceDefinitions.id`).
    pub id: u32,
    pub runs: Vec<PoolRun>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum TradeDirection {
    Import,
    Export,
}

/// One trade deal over consecutive hours while its terms stay unchanged.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct TradeRun {
    /// The deal's native handle, as a decimal string.
    pub deal: String,
    /// The resource's native index.
    pub resource: u32,
    pub direction: TradeDirection,
    /// The other country's tag.
    pub partner: String,
    /// The daily amount delivered.
    pub delivered_raw: String,
    /// Civilian factories the importer pays.
    pub factories: i32,
    pub convoys_assigned: i32,
    pub convoys_required: i32,
    pub first_hour: i32,
    pub last_hour: i32,
    pub starts_segment: bool,
}

/// A recorded resource, as content names and pictures it. The game keeps every
/// declaration of a resource as its own resource, even one repeating a token
/// (`CStrategicResourceDatabase::load_file@140AB2ED0` interns each declared key and
/// appends a new item and key with no check for an earlier one), so resources are
/// told apart by their native index and a repeated token is numbered.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    /// The native index (`ResourceDefinitions.id`).
    pub id: u32,
    pub token: String,
    /// Which declaration of its token this is, from 1, in the game's order.
    pub ordinal: u32,
    pub name: Option<String>,
    /// A PNG data URL; null when content has no icon for it.
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBalanceReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    /// Every recorded resource, in the game's order.
    pub resources: Vec<ResourceInfo>,
    /// One per resource, in `resources` order.
    pub series: Vec<ResourceSeries>,
    /// Hours the country's resources were read, ascending: a value shown at a
    /// later hour is as of the latest read before it.
    pub reads: Vec<i32>,
    /// Hours whose deals are known, as ascending spans: from a read of the deal
    /// table over the hours it is carried. The deal table is captured apart from
    /// the pools, so pools shown at an hour outside these spans have unknown deals,
    /// which is not the same as no deals.
    pub deal_hours: Vec<HourSpan>,
    pub trades: Vec<TradeRun>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ScannedBalance {
    /// Resources in the game's order: native index and token.
    pub resources: Vec<(u32, String)>,
    pub series: BTreeMap<u32, Vec<PoolRun>>,
    pub reads: Vec<i32>,
    pub deal_hours: Vec<HourSpan>,
    pub trades: Vec<TradeRun>,
    /// Rows naming a resource the recorded definitions or tokens do not name.
    pub unnamed: usize,
    /// Deals whose other country the recorded links do not resolve.
    pub unresolved: usize,
}

/// The same recorded pools, whatever hours they cover.
fn same_pools(a: &PoolRun, b: &PoolRun) -> bool {
    [
        (&a.produced_raw, &b.produced_raw),
        (&a.subjects_raw, &b.subjects_raw),
        (&a.imported_raw, &b.imported_raw),
        (&a.to_export_raw, &b.to_export_raw),
        (&a.exported_raw, &b.exported_raw),
        (&a.available_raw, &b.available_raw),
        (&a.after_projects_raw, &b.after_projects_raw),
        (&a.production_raw, &b.production_raw),
    ]
    .into_iter()
    .all(|(a, b)| a == b)
}

/// The same deal on the same terms, whatever hours they cover.
fn same_terms(a: &TradeRun, b: &TradeRun) -> bool {
    (
        &a.deal,
        &a.resource,
        a.direction,
        &a.partner,
        &a.delivered_raw,
        a.factories,
        a.convoys_assigned,
        a.convoys_required,
    ) == (
        &b.deal,
        &b.resource,
        b.direction,
        &b.partner,
        &b.delivered_raw,
        b.factories,
        b.convoys_assigned,
        b.convoys_required,
    )
}

/// Adds an hour to ascending spans, joining it to the last span when it follows it.
fn mark(spans: &mut Vec<HourSpan>, hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) == Some(hour) => span.last_hour = hour,
        Some(span) if span.last_hour >= hour => {}
        _ => spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        }),
    }
}

/// Scans one branch for one country's resources and trades.
pub fn scan(
    replay: &Replay,
    branch: u64,
    tag: &str,
    cancelled: &AtomicBool,
) -> io::Result<ScannedBalance> {
    let selected = [
        TableId::Campaign,
        TableId::Countries,
        TableId::CountryLinks,
        TableId::LexicalTokens,
        TableId::ResourceDefinitions,
        TableId::CountryResources,
        TableId::ResourceExchanges,
    ];
    let cancel = || io::Error::new(io::ErrorKind::Interrupted, "Resource request cancelled");
    let budget = || io::Error::other("Resource history exceeds its memory budget");
    let mut result = ScannedBalance::default();
    let mut ids: BTreeMap<u32, String> = BTreeMap::new();
    // The identity and hour last seen, and whether the pools and the deals are
    // open (carried) at it. The two tables are captured apart, so each has its own.
    let mut last: Option<((u32, u32), i32)> = None;
    let mut open = false;
    let mut trades_open = false;
    let mut open_trades: BTreeMap<u64, usize> = BTreeMap::new();
    let mut runs = 0_usize;
    for block in replay.index.iter().filter(|block| block.branch == branch) {
        if cancelled.load(Ordering::Acquire) {
            return Err(cancel());
        }
        let mut decoder = FrameDecoder::for_tables(&selected);
        let Ok(frames) = replay.read_block_selected(block, &selected) else {
            continue;
        };
        for frame in frames {
            if cancelled.load(Ordering::Acquire) {
                return Err(cancel());
            }
            let Ok(decoded) = decoder.advance(&frame) else {
                break;
            };
            if validate_campaign(decoded, replay).is_err() {
                break;
            }
            let hour = frame.hour;
            let identity = decoded
                .complete::<tables::Countries>()
                .ok()
                .zip(decoded.complete::<tables::CountryLinks>().ok())
                .and_then(|(countries, links)| {
                    tagged_country(countries, links, tag)
                        .found()
                        .map(|(country, slot)| ((slot, country.id), links))
                });
            let continues = identity.is_some_and(|(identity, _)| {
                last.is_some_and(|(seen, at)| seen == identity && at.checked_add(1) == Some(hour))
            });
            last = identity.map(|(identity, _)| (identity, hour));
            let rows = decoded.complete::<tables::CountryResources>().ok();
            let exchanges = decoded.complete::<tables::ResourceExchanges>().ok();
            // Each table ends its own values where it is missing; the other keeps going.
            if !continues || rows.is_none() {
                open = false;
            }
            if !continues || exchanges.is_none() {
                trades_open = false;
                open_trades.clear();
            }
            let Some(((slot, _), links)) = identity else {
                continue;
            };
            if !slot_read(&decoded.metrics, TableId::CountryResources) {
                // Carried: the last read still holds, but only after a read of this identity.
                if open {
                    for series in result.series.values_mut() {
                        if let Some(run) = series.last_mut()
                            && run.last_hour.checked_add(1) == Some(hour)
                        {
                            run.last_hour = hour;
                        }
                    }
                }
                if trades_open {
                    for index in open_trades.values() {
                        result.trades[*index].last_hour = hour;
                    }
                    mark(&mut result.deal_hours, hour);
                }
                continue;
            }
            let (Ok(definitions), Ok(lexical)) = (
                decoded.complete::<tables::ResourceDefinitions>(),
                decoded.complete::<tables::LexicalTokens>(),
            ) else {
                open = false;
                trades_open = false;
                open_trades.clear();
                continue;
            };
            for definition in definitions {
                if ids.contains_key(&definition.id) {
                    continue;
                }
                let Some(token) = lexical
                    .binary_search_by_key(&definition.token, |row| row.token)
                    .ok()
                    .and_then(|index| std::str::from_utf8(&lexical[index].literal).ok())
                    .filter(|token| !token.is_empty())
                else {
                    continue;
                };
                ids.insert(definition.id, token.to_owned());
                result.resources.push((definition.id, token.to_owned()));
                result.resources.sort_by_key(|(id, _)| *id);
            }
            if let Some(rows) = rows {
                result.reads.push(hour);
                let was_open = open;
                open = true;
                for row in rows
                    .iter()
                    .filter(|row| link_slot(links, row.country) == Some(slot))
                {
                    if !ids.contains_key(&row.resource) {
                        result.unnamed += 1;
                        continue;
                    }
                    let values = PoolRun {
                        first_hour: hour,
                        last_hour: hour,
                        starts_segment: false,
                        produced_raw: row.produced.to_string(),
                        subjects_raw: row.adjustment.to_string(),
                        imported_raw: row.imported.to_string(),
                        to_export_raw: row.to_export.to_string(),
                        exported_raw: row.exported.to_string(),
                        available_raw: row.used.to_string(),
                        after_projects_raw: row.special_total.to_string(),
                        production_raw: row.production_demand.to_string(),
                    };
                    let series = result.series.entry(row.resource).or_default();
                    match series.last_mut() {
                        Some(run)
                            if was_open
                                && run.last_hour.checked_add(1) == Some(hour)
                                && same_pools(run, &values) =>
                        {
                            run.last_hour = hour
                        }
                        previous => {
                            let starts_segment = !(was_open
                                && previous
                                    .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour)));
                            runs += 1;
                            if runs > MAX_RUNS {
                                return Err(budget());
                            }
                            series.push(PoolRun {
                                starts_segment,
                                ..values
                            });
                        }
                    }
                }
            }
            let Some(exchanges) = exchanges else {
                continue;
            };
            mark(&mut result.deal_hours, hour);
            trades_open = true;
            // Deals this read still lists continue; the others ended.
            let countries = decoded.complete::<tables::Countries>()?;
            let mut still = BTreeMap::new();
            for exchange in exchanges {
                let (exporter, importer) = (
                    link_slot(links, exchange.exporter),
                    link_slot(links, exchange.importer),
                );
                let (direction, partner) = if exporter == Some(slot) {
                    (TradeDirection::Export, importer)
                } else if importer == Some(slot) {
                    (TradeDirection::Import, exporter)
                } else {
                    continue;
                };
                if !ids.contains_key(&exchange.resource) {
                    result.unnamed += 1;
                    continue;
                }
                let Some(partner) = partner.and_then(|partner| {
                    countries
                        .iter()
                        .find(|country| country.canonical_slot == partner)
                        .and_then(|country| std::str::from_utf8(&country.tag).ok())
                }) else {
                    result.unresolved += 1;
                    continue;
                };
                let terms = TradeRun {
                    deal: exchange.id.to_string(),
                    resource: exchange.resource,
                    direction,
                    partner: partner.to_owned(),
                    delivered_raw: exchange.delivered.to_string(),
                    factories: exchange.assigned_factories,
                    convoys_assigned: exchange.convoys_assigned,
                    convoys_required: exchange.convoys_required,
                    first_hour: hour,
                    last_hour: hour,
                    starts_segment: false,
                };
                match open_trades.get(&exchange.id) {
                    Some(&index)
                        if result.trades[index].last_hour.checked_add(1) == Some(hour)
                            && same_terms(&result.trades[index], &terms) =>
                    {
                        result.trades[index].last_hour = hour;
                        still.insert(exchange.id, index);
                    }
                    previous => {
                        runs += 1;
                        if runs > MAX_RUNS {
                            return Err(budget());
                        }
                        let continued = previous.is_some_and(|&index| {
                            result.trades[index].last_hour.checked_add(1) == Some(hour)
                        });
                        result.trades.push(TradeRun {
                            starts_segment: !continued,
                            ..terms
                        });
                        still.insert(exchange.id, result.trades.len() - 1);
                    }
                }
            }
            open_trades = still;
        }
    }
    Ok(result)
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<ScannedBalance>)>,
    content: Option<ContentCache>,
}

/// Names and icons, and the standing warnings, for the catalogue they were read from.
struct ContentCache {
    catalogue: Arc<Catalogue>,
    /// By token and which declaration of it (from 1).
    resources: BTreeMap<(String, u32), Declared>,
    warnings: Vec<String>,
}

/// A declaration's name and icon from content.
#[derive(Clone)]
struct Declared {
    name: Option<String>,
    icon: Option<String>,
}

/// The icon sprite of the trade view's resource row, and each content declaration's
/// frame in it: per token, its declarations in load order.
struct Icons {
    texture: String,
    frames: u32,
    icon_frames: BTreeMap<String, Vec<Option<u32>>>,
}

fn icons(catalogue: &Catalogue) -> io::Result<(Icons, Vec<String>)> {
    let layers = &catalogue.content.layers;
    let gui = installed_content::gui::read(layers, TRADE_VIEW_GUI)?;
    let name = installed_content::gui::find(&gui, "containerWindowType", RESOURCE_ROW)
        .and_then(|row| installed_content::gui::find_child(row, &["buttonType", "iconType"], RESOURCE_ICON))
        .and_then(installed_content::gui::sprite)
        .ok_or_else(|| {
            crate::invalid(format!(
                "{TRADE_VIEW_GUI} declares no {RESOURCE_ICON} with a sprite in {RESOURCE_ROW}"
            ))
        })?;
    let sprite =
        catalogue.sprites.sprites.get(name).ok_or_else(|| {
            crate::invalid(format!("{name} is not declared in the recorded content"))
        })?;
    let texture = sprite
        .texture
        .clone()
        .ok_or_else(|| crate::invalid(format!("{name} has no texture")))?;
    let (resources, warnings) = installed_content::resources::read(layers)?;
    let mut icon_frames: BTreeMap<String, Vec<Option<u32>>> = BTreeMap::new();
    for resource in resources {
        icon_frames
            .entry(resource.token)
            .or_default()
            .push(resource.icon_frame);
    }
    Ok((
        Icons {
            texture,
            frames: sprite.frames,
            icon_frames,
        },
        warnings,
    ))
}

/// Which declaration of its token each resource is, from 1, in the game's order:
/// the game appends every declaration, so the k-th native resource with a token is
/// its k-th declaration in load order.
pub fn ordinals(resources: &[(u32, String)]) -> Vec<u32> {
    let mut seen: BTreeMap<&str, u32> = BTreeMap::new();
    resources
        .iter()
        .map(|(_, token)| {
            let count = seen.entry(token.as_str()).or_default();
            *count += 1;
            *count
        })
        .collect()
}

/// Names and icons for `wanted` declarations (token, ordinal) from content: the name
/// the game's resource texts use (`state_resource_<token>`) and the declaration's
/// frame of the trade view's icon.
fn resource_content(
    catalogue: &Catalogue,
    wanted: &[(String, u32)],
    cached: &mut BTreeMap<(String, u32), Declared>,
    warnings: &mut Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let missing: Vec<&(String, u32)> = wanted
        .iter()
        .filter(|key| !cached.contains_key(*key))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let icons = match icons(catalogue) {
        Ok((icons, notes)) => {
            warnings.extend(notes);
            Some(icons)
        }
        Err(error) => {
            warnings.push(format!("Resource icons are unavailable: {error}"));
            None
        }
    };
    for (token, ordinal) in missing {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Resource request cancelled",
            ));
        }
        let name = catalogue
            .localise(&format!("state_resource_{token}"))
            .text()
            .filter(|name| !name.is_empty() && name.len() <= 256);
        let icon = match icons.as_ref().map(|icons| {
            let frame = icons
                .icon_frames
                .get(token.as_str())
                .and_then(|frames| frames.get(*ordinal as usize - 1))
                .copied()
                .flatten();
            (icons, frame)
        }) {
            Some((icons, Some(frame))) => match catalogue::texture_frame_png(
                &catalogue.content.layers,
                &icons.texture,
                icons.frames,
                frame - 1,
                ICON_EDGE,
            ) {
                Ok(icon) => Some(icon),
                Err(error) => {
                    warnings.push(format!("{token}: icon unavailable: {error}"));
                    None
                }
            },
            Some((_, None)) => {
                warnings.push(format!(
                    "{token}: the recorded content gives no usable icon frame."
                ));
                None
            }
            None => None,
        };
        if name.is_none() && *ordinal == 1 {
            warnings.push(format!("{token}: no name in the recorded content."));
        }
        cached.insert((token.clone(), *ordinal), Declared { name, icon });
    }
    Ok(())
}

impl Product for ResourceBalance {
    const KEY: &'static str = "resource-balance";
    type Params = CountryParams;
    type Reply = ResourceBalanceReply;

    fn types() -> Vec<String> {
        vec![
            PoolRun::decl(),
            ResourceSeries::decl(),
            TradeDirection::decl(),
            TradeRun::decl(),
            ResourceInfo::decl(),
            ResourceBalanceReply::decl(),
        ]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.validate()
    }

    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        let key = (params.branch(), params.tag.clone());
        let state = source.state::<Cache>();
        let cached = {
            let cache = state.lock().unwrap_or_else(|error| error.into_inner());
            cache
                .entries
                .iter()
                .find(|(cached, _)| *cached == key)
                .map(|(_, scanned)| Arc::clone(scanned))
        };
        let scanned = match cached {
            Some(scanned) => scanned,
            None => {
                let scanned = Arc::new(scan(
                    &source.replay(),
                    params.branch(),
                    &params.tag,
                    cancelled,
                )?);
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                cache.entries.retain(|(cached, _)| *cached != key);
                cache.entries.push_back((key, Arc::clone(&scanned)));
                while cache.entries.len() > CACHED_COUNTRIES {
                    cache.entries.pop_front();
                }
                scanned
            }
        };
        let mut warnings = Vec::new();
        let ordinals = ordinals(&scanned.resources);
        let declarations: Vec<(String, u32)> = scanned
            .resources
            .iter()
            .zip(&ordinals)
            .map(|((_, token), ordinal)| (token.clone(), *ordinal))
            .collect();
        let info = |index: usize, declared: Option<&Declared>| {
            let (id, token) = &scanned.resources[index];
            ResourceInfo {
                id: *id,
                token: token.clone(),
                ordinal: ordinals[index],
                name: declared.and_then(|declared| declared.name.clone()),
                icon: declared.and_then(|declared| declared.icon.clone()),
            }
        };
        // Without the game folders the bars still draw, named by token and without icons.
        let resources = match context
            .content()
            .and_then(|content| catalogue::catalogue(source, content))
        {
            Ok(catalogue) => {
                let mut cache = state.lock().unwrap_or_else(|error| error.into_inner());
                if !cache
                    .content
                    .as_ref()
                    .is_some_and(|cached| Arc::ptr_eq(&cached.catalogue, &catalogue))
                {
                    let mut standing = catalogue.verification.clone();
                    standing.extend(catalogue.warnings.iter().cloned());
                    cache.content = Some(ContentCache {
                        catalogue: Arc::clone(&catalogue),
                        resources: BTreeMap::new(),
                        warnings: standing,
                    });
                }
                let content = cache.content.as_mut().expect("content cached");
                resource_content(
                    &catalogue,
                    &declarations,
                    &mut content.resources,
                    &mut content.warnings,
                    cancelled,
                )?;
                warnings.extend(content.warnings.iter().cloned());
                let resources: Vec<ResourceInfo> = declarations
                    .iter()
                    .enumerate()
                    .map(|(index, key)| info(index, content.resources.get(key)))
                    .collect();
                if resources.iter().any(|resource| resource.icon.is_some()) {
                    warnings.push("Widget artwork comes from the recorded content stack; visual assets are outside content hash coverage.".to_owned());
                }
                resources
            }
            Err(error) => {
                warnings.push(format!("Resource names and icons are unavailable: {error}"));
                (0..scanned.resources.len())
                    .map(|index| info(index, None))
                    .collect()
            }
        };
        // Each declaration is its own resource in the game, so each is shown, numbered.
        for (index, (_, token)) in scanned.resources.iter().enumerate() {
            if ordinals[index] == 2 {
                let count = scanned
                    .resources
                    .iter()
                    .filter(|(_, other)| other == token)
                    .count();
                warnings.push(format!(
                    "{token} is declared {count} times; the game keeps each declaration as its own resource, so each is shown and numbered."
                ));
            }
        }
        if scanned.unnamed > 0 {
            warnings.push(format!(
                "{} recorded rows name a resource the recorded definitions do not; they are left out.",
                scanned.unnamed
            ));
        }
        if scanned.unresolved > 0 {
            warnings.push(format!(
                "{} recorded trade reads name another country the recorded links do not resolve; they are left out.",
                scanned.unresolved
            ));
        }
        if scanned.reads.is_empty() {
            warnings.push(format!(
                "{}: no recorded resources in this branch",
                params.tag
            ));
        }
        Ok(ResourceBalanceReply {
            branch_id: params.branch_id.clone(),
            tag: params.tag.clone(),
            decimals: RESOURCE_DECIMALS,
            series: scanned
                .resources
                .iter()
                .map(|(id, _)| ResourceSeries {
                    id: *id,
                    runs: scanned.series.get(id).cloned().unwrap_or_default(),
                })
                .collect(),
            resources,
            reads: scanned.reads.clone(),
            deal_hours: scanned.deal_hours.clone(),
            trades: scanned.trades.clone(),
            warnings,
        })
    }
}
