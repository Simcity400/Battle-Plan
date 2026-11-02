//! The row census the family was prepared by before the segment driver, kept as the oracle
//! the columnar preparation is held to: one country's resources and trades over a branch,
//! observed frame by frame from whole tables.

use super::{PoolRun, ScannedBalance, TradeDirection, TradeRun, mark};
use dashboard_base::products::{resources::Learned, territory::CountryReads};
use dashboard_text::metadata::{IdentityTrack, validate_campaign};
use replay::container::Replay;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::{DecodedFrame, FrameDecoder};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::atomic::{AtomicBool, Ordering},
};

/// The same recorded pools, whatever hours they cover.
pub(crate) fn same_pools(a: &PoolRun, b: &PoolRun) -> bool {
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

#[derive(Clone)]
struct PoolOperand {
    country: u32,
    resource: u32,
    values: [i64; 8],
}

impl From<&replay_data::rows::CountryResourceRow> for PoolOperand {
    fn from(row: &replay_data::rows::CountryResourceRow) -> Self {
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

impl From<&replay_data::rows::ResourceExchangeRow> for DealOperand {
    fn from(row: &replay_data::rows::ResourceExchangeRow) -> Self {
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

/// The resources recorded definitions and tokens name this hour.
fn read_names(frame: &DecodedFrame) -> Option<Learned> {
    frame
        .complete::<tables::ResourceDefinitions>()
        .ok()
        .zip(frame.complete::<tables::LexicalTokens>().ok())
        .map(|(definitions, lexical)| {
            let mut names = Learned::default();
            names.learn(definitions, lexical);
            names
        })
}

#[derive(Default)]
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
    pool_keys: BTreeSet<u32>,
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
        hour: i32,
        identity: Option<(u32, u32)>,
        pools: Option<&Vec<&PoolOperand>>,
        deals: Option<&Vec<&DealOperand>>,
        slots: &HashMap<u32, u32>,
        partners: &BTreeMap<u32, Option<String>>,
        names: Option<&Learned>,
        names_changed: bool,
        changed: [bool; 2],
    ) {
        let before = [self.result.unnamed, self.result.unresolved];
        let continues = self.track.step(identity, hour);
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
            return;
        };
        let Some(names) = names else {
            self.pools_open = false;
            self.deals_open = false;
            self.pool_keys.clear();
            self.open_trades.clear();
            self.diagnostic_rate = [0; 3];
            return;
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
                    let series = self.result.series.entry(row.resource).or_default();
                    let follows = pools_continue
                        && series
                            .last()
                            .is_some_and(|run| run.last_hour.checked_add(1) == Some(hour));
                    match series.last_mut() {
                        Some(run) if follows && same_pools(run, &values) => run.last_hour = hour,
                        _ => series.push(PoolRun {
                            starts_segment: !follows,
                            ..values
                        }),
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
            return;
        }
        self.diagnostic_rate[1..].fill(0);
        let Some(exchanges) = deals else {
            self.open_trades.clear();
            return;
        };
        mark(&mut self.result.deal_hours, hour);
        self.deals_open = true;
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
    }
}

/// The census of one selected country, every frame observed from its whole tables.
#[derive(Default)]
struct Scan {
    selected: String,
    country: CountryScan,
    frontier: Option<i32>,
}

impl Scan {
    fn observe(&mut self, frame: &DecodedFrame) {
        let reads = frame
            .complete::<tables::Countries>()
            .ok()
            .map(|rows| CountryReads::new(frame, rows));
        let pools: Option<Vec<PoolOperand>> = frame
            .complete::<tables::CountryResources>()
            .ok()
            .map(|rows| rows.iter().map(PoolOperand::from).collect());
        let deals: Option<Vec<DealOperand>> = frame
            .complete::<tables::ResourceExchanges>()
            .ok()
            .map(|rows| rows.iter().map(DealOperand::from).collect());
        let names = read_names(frame);
        let empty_slots = HashMap::new();
        let slots = reads.as_ref().map_or(&empty_slots, |reads| &**reads.slots());
        let empty_partners = BTreeMap::new();
        let partners = reads.as_ref().map_or(&empty_partners, CountryReads::partners);
        let identity = reads
            .as_ref()
            .and_then(|reads| reads.holders().get(&self.selected).copied().flatten());
        let slot = identity.map(|(slot, _)| slot);
        let pools = pools.as_ref().map(|rows| {
            rows.iter()
                .filter(|row| slot.is_some() && slots.get(&row.country).copied() == slot)
                .collect::<Vec<_>>()
        });
        let deals = deals.as_ref().map(|rows| {
            rows.iter()
                .filter(|row| {
                    slot.is_some()
                        && (slots.get(&row.exporter).copied() == slot
                            || slots.get(&row.importer).copied() == slot)
                })
                .collect::<Vec<_>>()
        });
        if let Some(through) = self.frontier {
            self.country.seal(through);
        }
        self.country.observe(
            frame.hour,
            identity,
            pools.as_ref(),
            deals.as_ref(),
            slots,
            partners,
            names.as_ref(),
            true,
            [true; 2],
        );
        self.frontier = Some(frame.hour);
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
    let mut scan = Scan {
        selected: tag.to_owned(),
        ..Default::default()
    };
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
                    scan.observe(decoded);
                }
            }
            Ok(())
        },
    )?;
    if let Some(through) = scan.frontier {
        scan.country.seal(through);
    }
    Ok(scan.country.result)
}
