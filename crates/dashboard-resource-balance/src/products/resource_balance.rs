//! One country's strategic resources over a branch, for the Resource balance
//! widget: where each resource comes from and where it goes, and the trade
//! deals behind its imports and exports.
//!
//! Every value is a recorded pool (`CountryResources`), fixed point with
//! [`RESOURCE_DECIMALS`] places, kept exact. The pools are `produced`,
//! `adjustment`, `imported`, `used`, `special_total`, `production_demand`,
//! `to_export`, `base_export` and `exported`.
//!
//! - `used` is what the country has for use: produced + adjustment + imported
//!   minus `to_export`, with the game's rounding. What is taken off is the export
//!   reserve the trade law sets aside, separate from what buyers took (`exported`).
//! - `production_demand` is minus what the production lines need, met or not.
//! - `special_total` is what remains after the special-project reduction, which
//!   is never positive. Other consumers of that reduction are not identified.
//! - `special_total` + `production_demand` is the surplus, or its deficit
//!   when negative.
//!
//! `adjustment` is resources moved between a subject and its overlord: positive
//! for the overlord, negative for the subject. `base_export` (the reserve before
//! modifiers) is not shown.
//!
//! The pools and the deals are both read every hour (`replay::cadence`), so a
//! value shown at an hour is as of that hour. Values run over the consecutive
//! hours of the same country identity and end at a gap, an incomplete table or a
//! change of identity. The resource names come from `ResourceDefinitions`, read
//! every hour with the pools: they are content, fixed once loaded.
//!
//! The pools and the deals are captured as separate tables with their own
//! outcomes, so each keeps its own values: an hour whose deal table is missing
//! still shows the pools, and `deal_hours` says at which hours the deals are
//! known.
//!
//! Resources are told apart by their recorded index, not their token: the game
//! keeps every declaration as its own resource (see [`ResourceInfo`]).
//!
//! Trades are `ResourceExchanges` rows (a classic trade deal each) naming the
//! country as exporter or importer. `delivered` is the deal's daily amount.
//! Import and export deals can leave a remainder against the recorded pools,
//! so the widget shows that remainder as its own part.

pub use dashboard_base::products::resources::{RESOURCE_DECIMALS, ResourceInfo, ordinals};
use dashboard_base::products::{
    CountryParams, Product,
    resources::{Learned, resource_infos},
};
use dashboard_base::{products::country_groups::HourSpan, source::Source};
use dashboard_core::config::LoadContext;
use dashboard_text::metadata::IdentityTrack;
use replay::rows::{TableId, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, atomic::AtomicBool},
};
use ts_rs::TS;

#[cfg(test)]
use dashboard_text::metadata::validate_campaign;
#[cfg(test)]
use replay::{container::Replay, frame::FrameDecoder};
#[cfg(test)]
use std::{collections::VecDeque, sync::atomic::Ordering};

pub struct ResourceBalance;
pub mod prepared;
pub(crate) mod sample;

#[cfg(test)]
const CACHED_COUNTRIES: usize = 8;
/// A country's recorded pools of one resource over consecutive hours while they
/// stay unchanged. Raw values are exact decimal strings with
/// [`RESOURCE_DECIMALS`] places.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoolRun {
    pub first_hour: i32,
    pub last_hour: i32,
    /// The run follows a gap, a missing table or another country identity.
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

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResourceBalanceReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    /// Every recorded resource, in the game's order.
    pub resources: Vec<ResourceInfo>,
    /// One per resource, in `resources` order.
    pub series: Vec<ResourceSeries>,
    /// Hours whose deals are known, as ascending spans: the hours the deal table
    /// was recorded. It is captured apart from the pools, so pools shown at an
    /// hour outside these spans have unknown deals, which is not the same as no
    /// deals.
    pub deal_hours: Vec<HourSpan>,
    pub trades: Vec<TradeRun>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ScannedBalance {
    /// Resources in the game's order: native index and token.
    pub resources: Vec<(u32, String)>,
    pub series: BTreeMap<u32, Vec<PoolRun>>,
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

/// Current native operands only; refreshed by their supplying table's frame fact.
#[derive(Clone)]
struct PoolOperand {
    country: u32,
    resource: u32,
    values: [i64; 8],
}
impl From<&replay::rows::CountryResourceRow> for PoolOperand {
    fn from(row: &replay::rows::CountryResourceRow) -> Self {
        Self {
            country: row.country,
            resource: row.resource,
            values: [
                row.produced,
                row.adjustment,
                row.imported,
                row.to_export,
                row.exported,
                row.used,
                row.special_total,
                row.production_demand,
            ],
        }
    }
}
impl PoolOperand {
    fn run(&self, hour: i32) -> PoolRun {
        PoolRun {
            first_hour: hour,
            last_hour: hour,
            starts_segment: false,
            produced_raw: self.values[0].to_string(),
            subjects_raw: self.values[1].to_string(),
            imported_raw: self.values[2].to_string(),
            to_export_raw: self.values[3].to_string(),
            exported_raw: self.values[4].to_string(),
            available_raw: self.values[5].to_string(),
            after_projects_raw: self.values[6].to_string(),
            production_raw: self.values[7].to_string(),
        }
    }
}
#[derive(Clone)]
struct DealOperand {
    id: u64,
    exporter: u32,
    importer: u32,
    resource: u32,
    delivered: i64,
    assigned_factories: i32,
    convoys_assigned: i32,
    convoys_required: i32,
}
impl From<&replay::rows::ResourceExchangeRow> for DealOperand {
    fn from(row: &replay::rows::ResourceExchangeRow) -> Self {
        Self {
            id: row.id,
            exporter: row.exporter,
            importer: row.importer,
            resource: row.resource,
            delivered: row.delivered,
            assigned_factories: row.assigned_factories,
            convoys_assigned: row.convoys_assigned,
            convoys_required: row.convoys_required,
        }
    }
}

fn resource_values(run: &PoolRun) -> io::Result<[i128; 11]> {
    let raw = |value: &str| {
        value
            .parse::<i128>()
            .map_err(|_| dashboard_core::invalid("Malformed recorded resource pool"))
    };
    let subjects = raw(&run.subjects_raw)?;
    let produced = raw(&run.produced_raw)?;
    let imported = raw(&run.imported_raw)?;
    let projects = raw(&run.available_raw)? - raw(&run.after_projects_raw)?;
    let production = -raw(&run.production_raw)?;
    let reserve = raw(&run.to_export_raw)?;
    let supply = produced + subjects.max(0) + imported;
    let demand = projects + production + (-subjects).max(0) + reserve;
    let net = raw(&run.after_projects_raw)? - production;
    Ok([
        supply,
        demand,
        produced,
        subjects.max(0),
        imported,
        -projects,
        -production,
        -(-subjects).max(0),
        -reserve,
        net - (supply - demand),
        net,
    ])
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
pub(crate) fn mark(spans: &mut Vec<HourSpan>, hour: i32) {
    match spans.last_mut() {
        Some(span) if span.last_hour.checked_add(1) == Some(hour) => span.last_hour = hour,
        _ => spans.push(HourSpan {
            first_hour: hour,
            last_hour: hour,
        }),
    }
}

/// Scans one branch for one country's resources and trades.
#[cfg(test)]
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
    let mut scan = Scan {
        selected: Some(tag.to_owned()),
        ..Default::default()
    };
    scan.countries
        .insert(tag.to_owned(), CountryScan::default());
    dashboard_core::blocks::scan_blocks(
        replay,
        replay.index.iter().filter(|block| block.branch == branch),
        &selected,
        cancelled,
        |blocks| {
            for (_, frames) in blocks {
                if cancelled.load(Ordering::Acquire) {
                    return Err(cancel());
                }
                let mut decoder = FrameDecoder::for_tables(&selected);
                let Ok(frames) = frames else {
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
                    scan.observe(decoded)?;
                }
            }
            Ok(())
        },
    )?;
    Ok(scan.countries.remove(tag).unwrap_or_default().result)
}

#[derive(Default, Serialize, Deserialize)]
struct CountryScan {
    result: ScannedBalance,
    learned: Learned,
    names_ready: bool,
    track: IdentityTrack,
    pools_open: bool,
    deals_open: bool,
    open_trades: BTreeMap<u64, usize>,
    diagnostic_rate: [usize; 3],
    materialized: Option<i32>,
    pool_keys: std::collections::BTreeSet<u32>,
    #[serde(skip)]
    dirty_pools: std::collections::BTreeSet<u32>,
}
impl CountryScan {
    fn advance(&mut self, from: i32, through: i32) {
        self.track.advance(through);
        if self.pools_open {
            for resource in &self.pool_keys {
                let runs = self
                    .result
                    .series
                    .get_mut(resource)
                    .expect("open resource pool");
                if let Some(run) = runs.last_mut().filter(|run| run.last_hour == from) {
                    run.last_hour = through;
                }
            }
        }
        if self.deals_open {
            if let Some(span) = self.result.deal_hours.last_mut() {
                span.last_hour = through;
            }
            for &index in self.open_trades.values() {
                self.result.trades[index].last_hour = through;
            }
        }
        let hours = usize::try_from(i64::from(through) - i64::from(from))
            .expect("accepted resource frontier");
        self.result.unnamed += hours * (self.diagnostic_rate[0] + self.diagnostic_rate[1]);
        self.result.unresolved += hours * self.diagnostic_rate[2];
    }
    fn seal(&mut self, through: i32) {
        if let Some(from) = self.materialized.filter(|&from| from < through) {
            self.advance(from, through);
        }
        self.materialized = Some(through);
    }
    #[allow(clippy::too_many_arguments)]
    fn observe(
        &mut self,
        decoded: &replay::frame::DecodedFrame,
        identity: Option<(u32, u32)>,
        pools: Option<&Vec<&PoolOperand>>,
        deals: Option<&Vec<&DealOperand>>,
        slots: &std::collections::HashMap<u32, u32>,
        partners: &BTreeMap<u32, Option<String>>,
        names: Option<&Learned>,
        names_changed: bool,
        changed: [bool; 2],
    ) -> io::Result<()> {
        let hour = decoded.hour;
        self.dirty_pools.clear();
        if changed[0] {
            self.dirty_pools.extend(self.pool_keys.iter().copied());
        }
        let before = [self.result.unnamed, self.result.unresolved];
        let continues = self.track.step(identity, hour);
        // Each table's values go on only from its previous hour of the same
        // identity; where one is missing, the other keeps going.
        let pools_continue = continues && self.pools_open;
        if !(continues && self.deals_open) {
            self.open_trades.clear();
        }
        if changed[0] {
            self.pools_open = false;
        }
        if changed[1] {
            self.deals_open = false;
        }
        self.materialized = Some(hour);
        let Some((slot, _)) = identity else {
            self.pools_open = false;
            self.deals_open = false;
            self.pool_keys.clear();
            self.diagnostic_rate = [0; 3];
            return Ok(());
        };
        let Some(names) = names else {
            self.pools_open = false;
            self.deals_open = false;
            self.pool_keys.clear();
            self.open_trades.clear();
            self.diagnostic_rate = [0; 3];
            return Ok(());
        };
        if names_changed || !self.names_ready {
            if self.learned.learn_names(names) {
                self.result.resources = self.learned.list();
            }
            self.names_ready = true;
        }
        if changed[0] {
            self.pool_keys.clear();
            if let Some(rows) = pools {
                self.pools_open = true;
                for &row in rows {
                    if !self.learned.contains(row.resource) {
                        self.result.unnamed += 1;
                        continue;
                    }
                    let values = row.run(hour);
                    self.pool_keys.insert(row.resource);
                    self.dirty_pools.insert(row.resource);
                    let series = self.result.series.entry(row.resource).or_default();
                    let follows = pools_continue
                        && series
                            .last()
                            .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour));
                    match series.last_mut() {
                        Some(run) if follows && same_pools(run, &values) => run.last_hour = hour,
                        _ => {
                            series.push(PoolRun {
                                starts_segment: !follows,
                                ..values
                            });
                        }
                    }
                }
            }
            self.diagnostic_rate[0] = self.result.unnamed - before[0];
        } else if self.pools_open {
            for resource in &self.pool_keys {
                let run = self
                    .result
                    .series
                    .get_mut(resource)
                    .and_then(|runs| runs.last_mut());
                if let Some(run) = run.filter(|run| run.last_hour == hour - 1) {
                    run.last_hour = hour;
                }
            }
            self.result.unnamed += self.diagnostic_rate[0];
        }
        let before_deals = [self.result.unnamed, self.result.unresolved];
        if !changed[1] {
            if self.deals_open {
                mark(&mut self.result.deal_hours, hour);
                for &index in self.open_trades.values() {
                    self.result.trades[index].last_hour = hour;
                }
            }
            self.result.unnamed += self.diagnostic_rate[1];
            self.result.unresolved += self.diagnostic_rate[2];
            return Ok(());
        }
        self.diagnostic_rate[1..].fill(0);
        let Some(exchanges) = deals else {
            self.open_trades.clear();
            return Ok(());
        };
        mark(&mut self.result.deal_hours, hour);
        self.deals_open = true;
        // Deals this hour still lists continue; the others ended.

        let mut still = BTreeMap::new();
        for &exchange in exchanges {
            let (exporter, importer) = (
                slots.get(&exchange.exporter).copied(),
                slots.get(&exchange.importer).copied(),
            );
            let (direction, partner) = if exporter == Some(slot) {
                (TradeDirection::Export, importer)
            } else if importer == Some(slot) {
                (TradeDirection::Import, exporter)
            } else {
                continue;
            };
            if !self.learned.contains(exchange.resource) {
                self.result.unnamed += 1;
                continue;
            }
            let Some(partner) =
                partner.and_then(|partner| partners.get(&partner).and_then(|tag| tag.as_deref()))
            else {
                self.result.unresolved += 1;
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
            // An open deal was listed at the previous hour.
            match self.open_trades.get(&exchange.id) {
                Some(&index) if same_terms(&self.result.trades[index], &terms) => {
                    self.result.trades[index].last_hour = hour;
                    still.insert(exchange.id, index);
                }
                previous => {
                    self.result.trades.push(TradeRun {
                        starts_segment: previous.is_none(),
                        ..terms
                    });
                    still.insert(exchange.id, self.result.trades.len() - 1);
                }
            }
        }
        self.open_trades = still;
        self.diagnostic_rate[1] = self.result.unnamed - before_deals[0];
        self.diagnostic_rate[2] = self.result.unresolved - before_deals[1];
        Ok(())
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(crate) struct Scan {
    selected: Option<String>,
    countries: BTreeMap<String, CountryScan>,
    frontier: Option<i32>,
    active: std::collections::BTreeSet<String>,
    #[serde(skip)]
    country_reads: Option<dashboard_base::products::territory::CountryReads>,
    #[serde(skip)]
    inputs_ready: bool,
    #[serde(skip)]
    pool_operands: Option<Vec<PoolOperand>>,
    #[serde(skip)]
    deal_operands: Option<Vec<DealOperand>>,
    #[serde(skip)]
    current_names: Option<Learned>,
    #[serde(skip)]
    pool_slots: std::collections::BTreeSet<u32>,
    #[serde(skip)]
    slot_tags: BTreeMap<u32, Vec<String>>,
    #[serde(skip)]
    availability_changed: [bool; 2],
    #[serde(skip)]
    deal_slots: std::collections::BTreeSet<u32>,
    #[cfg(test)]
    #[serde(skip)]
    row_visits: usize,
    #[cfg(test)]
    #[serde(skip)]
    history_visits: usize,
}
impl Scan {
    fn advance(&mut self, hour: i32) {
        self.frontier = Some(hour);
    }
    fn seal(&mut self) {
        if let Some(through) = self.frontier {
            for tag in &self.active {
                self.countries
                    .get_mut(tag)
                    .expect("active resource country")
                    .seal(through);
            }
        }
    }
    #[cfg(test)]
    fn observe(&mut self, frame: &replay::frame::DecodedFrame) -> io::Result<()> {
        self.country_reads = frame
            .complete::<tables::Countries>()
            .ok()
            .map(|rows| dashboard_base::products::territory::CountryReads::new(frame, rows));
        self.refresh_inputs(frame, [true; 3]);
        self.observe_resolved(frame, true, [true; 2], true)
            .map(|_| ())
    }
    fn refresh_inputs(&mut self, frame: &replay::frame::DecodedFrame, changed: [bool; 3]) {
        self.availability_changed = [false; 2];
        if changed[0] || !self.inputs_ready {
            let available = self.pool_operands.is_some();
            self.pool_operands = frame
                .complete::<tables::CountryResources>()
                .ok()
                .map(|rows| {
                    #[cfg(test)]
                    {
                        self.row_visits += rows.len();
                    }
                    rows.iter().map(PoolOperand::from).collect()
                });
            self.availability_changed[0] = available != self.pool_operands.is_some();
        }
        if changed[1] || !self.inputs_ready {
            let available = self.deal_operands.is_some();
            self.deal_operands = frame
                .complete::<tables::ResourceExchanges>()
                .ok()
                .map(|rows| {
                    #[cfg(test)]
                    {
                        self.row_visits += rows.len();
                    }
                    rows.iter().map(DealOperand::from).collect()
                });
            self.availability_changed[1] = available != self.deal_operands.is_some();
        }
        if changed[2] || !self.inputs_ready {
            self.current_names = frame
                .complete::<tables::ResourceDefinitions>()
                .ok()
                .zip(frame.complete::<tables::LexicalTokens>().ok())
                .map(|(definitions, lexical)| {
                    #[cfg(test)]
                    {
                        self.row_visits += definitions.len();
                    }
                    let mut names = Learned::default();
                    names.learn(definitions, lexical);
                    names
                });
        }
        self.inputs_ready = true;
    }
    fn observe_resolved(
        &mut self,
        frame: &replay::frame::DecodedFrame,
        names_changed: bool,
        changed: [bool; 2],
        holders_changed: bool,
    ) -> io::Result<BTreeMap<String, [bool; 2]>> {
        let empty_slots = Default::default();
        let slots = self.country_reads.as_ref().map_or(
            &empty_slots,
            dashboard_base::products::territory::CountryReads::slots,
        );
        let empty_identities = BTreeMap::new();
        let identities = self.country_reads.as_ref().map_or(
            &empty_identities,
            dashboard_base::products::territory::CountryReads::holders,
        );
        let empty_partners = BTreeMap::new();
        let partners = self.country_reads.as_ref().map_or(
            &empty_partners,
            dashboard_base::products::territory::CountryReads::partners,
        );
        if holders_changed {
            self.slot_tags.clear();
            for (tag, &identity) in identities {
                if self
                    .selected
                    .as_deref()
                    .is_some_and(|selected| selected != tag)
                {
                    continue;
                }
                self.countries.entry(tag.clone()).or_default();
                if let Some((slot, _)) = identity {
                    self.slot_tags.entry(slot).or_default().push(tag.clone());
                }
            }
        }
        let mut pools = BTreeMap::<u32, Vec<&PoolOperand>>::new();
        if changed[0] || holders_changed {
            for row in self.pool_operands.iter().flatten() {
                if let Some(&slot) = slots.get(&row.country) {
                    pools.entry(slot).or_default().push(row);
                }
            }
        }
        let mut deals = BTreeMap::<u32, Vec<&DealOperand>>::new();
        if changed[1] || holders_changed {
            for row in self.deal_operands.iter().flatten() {
                let exporter = slots.get(&row.exporter).copied();
                let importer = slots.get(&row.importer).copied();
                for slot in [exporter, importer]
                    .into_iter()
                    .flatten()
                    .collect::<std::collections::BTreeSet<_>>()
                {
                    deals.entry(slot).or_default().push(row);
                }
            }
        }
        let empty_pools = Vec::new();
        let empty_deals = Vec::new();
        let mut touched = BTreeMap::<String, [bool; 2]>::new();
        for (index, supplying_slots) in [
            pools
                .keys()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            deals.keys().copied().collect(),
        ]
        .into_iter()
        .enumerate()
        {
            if !changed[index] {
                continue;
            }
            let previous = if index == 0 {
                &self.pool_slots
            } else {
                &self.deal_slots
            };
            for slot in supplying_slots.iter().chain(previous.iter()) {
                for tag in self.slot_tags.get(slot).into_iter().flatten() {
                    touched.entry(tag.clone()).or_default()[index] = true;
                }
            }
            if self.availability_changed[index] {
                for tags in self.slot_tags.values() {
                    for tag in tags {
                        touched.entry(tag.clone()).or_default()[index] = true;
                    }
                }
            }
        }
        if holders_changed || names_changed {
            touched.extend(
                identities
                    .keys()
                    .filter(|tag| {
                        self.selected
                            .as_deref()
                            .is_none_or(|selected| selected == tag.as_str())
                    })
                    .cloned()
                    .map(|tag| (tag, [true; 2])),
            );
            touched.extend(
                self.active
                    .iter()
                    .chain(self.selected.iter())
                    .cloned()
                    .map(|tag| (tag, [true; 2])),
            );
        }
        for (tag, &flags) in &touched {
            #[cfg(test)]
            {
                self.history_visits += 1;
            }
            let country = self
                .countries
                .get_mut(tag)
                .expect("resource observed country");
            if let Some(through) = self.frontier {
                country.seal(through);
            }
            let identity = identities.get(tag).copied().flatten();
            let slot = identity.map(|identity| identity.0);
            country.observe(
                frame,
                identity,
                self.pool_operands.as_ref().map(|_| {
                    slot.and_then(|slot| pools.get(&slot))
                        .unwrap_or(&empty_pools)
                }),
                self.deal_operands.as_ref().map(|_| {
                    slot.and_then(|slot| deals.get(&slot))
                        .unwrap_or(&empty_deals)
                }),
                slots,
                partners,
                self.current_names.as_ref(),
                names_changed,
                flags,
            )?;
        }
        self.frontier = Some(frame.hour);
        if changed[0] || holders_changed {
            self.pool_slots = pools.keys().copied().collect();
        }
        if changed[1] || holders_changed {
            self.deal_slots = deals.keys().copied().collect();
        }
        if holders_changed {
            self.active = self.slot_tags.values().flatten().cloned().collect();
        }
        Ok(touched)
    }
}

#[cfg(test)]
#[derive(Default)]
struct Cache {
    entries: VecDeque<((u64, String), Arc<ScannedBalance>)>,
}

impl Product for ResourceBalance {
    const KEY: &'static str = "resource-balance";
    const CONTENT_DEPENDENT: bool = true;
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

    #[cfg(test)]
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
        reply(source, params, context, &scanned, cancelled)
    }
}

pub(crate) fn reply(
    source: &Source,
    params: &CountryParams,
    context: &LoadContext,
    scanned: &ScannedBalance,
    cancelled: &AtomicBool,
) -> io::Result<ResourceBalanceReply> {
    // Without game folders resources keep their tokens and no icons.
    let (resources, mut warnings) = resource_infos(source, context, &scanned.resources, cancelled)?;
    if scanned.unnamed > 0 {
        warnings.push(format!(
            "{} recorded rows name a resource the recorded definitions do not; they are left out.",
            scanned.unnamed
        ));
    }
    if scanned.unresolved > 0 {
        warnings.push(format!(
                "{} recorded trade rows name another country the recorded links do not resolve; they are left out.",
                scanned.unresolved
            ));
    }
    if scanned.series.is_empty() {
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
        deal_hours: scanned.deal_hours.clone(),
        trades: scanned.trades.clone(),
        warnings,
    })
}

#[cfg(test)]
mod tests;
