//! Convoy health, prepared on its own (family `convoys`): each country's
//! delivery efficiency by strategic sea region, the trade efficiency of each
//! resource deal it imports, and the lend-lease it receives by sea.
//!
//! - Regional efficiency (`RegionalConvoyEfficiency`, fixed5) is read only
//!   for countries taking part in a bad naval route, in each sea region of
//!   its path. A country or region outside that is not observed: never 100%
//!   and never zero.
//! - A deal's trade efficiency is the product of its two recorded fixed5
//!   factors, widened and truncated as the game compares it
//!   ([`engine_resources::resource_map::efficiency_product`]). Each deal
//!   stays its own row: no per-country figure is made from them.
//! - Lend-lease convoy clients (category 4) record their route's bad flag
//!   and convoys, but no numeric delivery efficiency: that stays unknown.

use super::keyed::{EconomyRun, HolderTag, Keyed, Observed, Published, branch_of};
use dashboard_base::{
    Source, catalogue,
    prepared::frame_changes::FrameChanges,
    products::{LoadContext, Product, resources::Learned},
};
use replay_data::rows::{RowCodec, TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

pub const FAMILY: &str = "convoys";
pub const QUERY: &str = "convoy-health";
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::RegionalConvoyEfficiency,
    TableId::ResourceExchanges,
    TableId::ConvoyClients,
];
const WATCHED: [TableId; 7] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::LexicalTokens,
    TableId::ResourceDefinitions,
    TableId::RegionalConvoyEfficiency,
    TableId::ResourceExchanges,
    TableId::ConvoyClients,
];
/// The convoy client category of a lend-lease shipment.
const LEND_LEASE: u8 = 4;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    /// A country's efficiency in a sea region.
    Regional(u32, u32),
    /// An importer's deal: its exporter's holder, absent while unidentified.
    DealExporter(u32, u64),
    DealResource(u32, u64),
    /// The efficiency product and its two factors.
    DealEfficiency(u32, u64),
    DealFactor(u32, u64, u8),
    /// The deal's convoys assigned (0) and required (1).
    DealConvoys(u32, u64, u8),
    /// A receiver's lend-lease client: the giver's holder, absent while unidentified.
    LeaseGiver(u32, u32),
    /// 1 while the client's route draws as bad.
    LeaseBad(u32, u32),
    LeaseConvoys(u32, u32, u8),
    /// 1 while a table (0 regional efficiency, 1 resource deals, 2 convoy
    /// clients) was read complete with every row's holder identified: without
    /// it absence proves nothing.
    Known(u8),
}

impl Field {
    fn holder(self) -> u32 {
        match self {
            Self::Regional(slot, _)
            | Self::DealExporter(slot, _)
            | Self::DealResource(slot, _)
            | Self::DealEfficiency(slot, _)
            | Self::DealFactor(slot, ..)
            | Self::DealConvoys(slot, ..)
            | Self::LeaseGiver(slot, _)
            | Self::LeaseBad(slot, _)
            | Self::LeaseConvoys(slot, ..) => slot,
            Self::Known(_) => u32::MAX,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Extra {
    /// Every resource the definitions named, by native index.
    pub resources: Learned,
}

/// What an hour changed of one census table in the fields the census reads: the table read
/// whole, or the keys of the rows written and deleted.
pub enum Edit {
    Whole,
    Rows { written: Vec<u128>, deleted: Vec<u128> },
}

/// The census tables, in the order a whole read reads them.
const CENSUS: [TableId; 3] = [
    TableId::RegionalConvoyEfficiency,
    TableId::ResourceExchanges,
    TableId::ConvoyClients,
];

/// Country identities at one read, as a whole read reads them: native links to canonical
/// slots, and each slot's tag (its last country's), none where empty or not UTF-8.
#[derive(Default)]
struct Holders {
    links: HashMap<u32, u32>,
    tags: HashMap<u32, Option<String>>,
}

impl Holders {
    fn read(frame: &DecodedFrame) -> io::Result<Self> {
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(io::Error::other)?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(io::Error::other)?;
        let mut tags = HashMap::with_capacity(countries.len());
        for row in countries {
            let tag = std::str::from_utf8(&row.tag)
                .ok()
                .filter(|tag| !tag.is_empty())
                .map(str::to_owned);
            tags.insert(row.canonical_slot, tag);
        }
        Ok(Self {
            links: links
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect(),
            tags,
        })
    }

    fn slot(&self, raw: u32) -> Option<u32> {
        self.links.get(&raw).copied()
    }

    fn tag(&self, slot: u32) -> Option<String> {
        self.tags.get(&slot).cloned().flatten()
    }
}

/// One row as the census reads it: its values by field, the holders it tags, and whether a
/// holder it names has no identity: its own (`orphan`: the row is skipped and leaves its table
/// unknown) or its partner's (`stray`: only that field is missing).
#[derive(Default)]
struct Read {
    fields: Vec<(Field, i128)>,
    tags: Vec<u32>,
    orphan: bool,
    stray: bool,
}

fn read_regional(row: &replay_data::rows::RegionalConvoyEfficiencyRow, holders: &Holders) -> Read {
    let Some(slot) = holders.slot(row.country) else {
        return Read {
            orphan: true,
            ..Read::default()
        };
    };
    Read {
        fields: vec![(Field::Regional(slot, row.region), i128::from(row.efficiency))],
        tags: vec![slot],
        ..Read::default()
    }
}

fn read_deal(row: &replay_data::rows::ResourceExchangeRow, holders: &Holders) -> Read {
    let Some(importer) = holders.slot(row.importer) else {
        return Read {
            orphan: true,
            ..Read::default()
        };
    };
    let id = row.id;
    let mut read = Read {
        tags: vec![importer],
        ..Read::default()
    };
    // The importer's deal keeps its numbers; only its exporter is unknown.
    match holders.slot(row.exporter) {
        Some(exporter) => {
            read.tags.push(exporter);
            read.fields
                .push((Field::DealExporter(importer, id), i128::from(exporter)));
        }
        None => read.stray = true,
    }
    read.fields.extend([
        (Field::DealResource(importer, id), i128::from(row.resource)),
        (
            Field::DealEfficiency(importer, id),
            engine_resources::resource_map::efficiency_product(row.efficiency_a, row.efficiency_b),
        ),
        (Field::DealFactor(importer, id, 0), i128::from(row.efficiency_a)),
        (Field::DealFactor(importer, id, 1), i128::from(row.efficiency_b)),
        (Field::DealConvoys(importer, id, 0), i128::from(row.convoys_assigned)),
        (Field::DealConvoys(importer, id, 1), i128::from(row.convoys_required)),
    ]);
    read
}

fn read_lease(row: &replay_data::rows::ConvoyClientRow, holders: &Holders) -> Read {
    if row.category != LEND_LEASE {
        return Read::default();
    }
    let Some(receiver) = holders.slot(row.other) else {
        return Read {
            orphan: true,
            ..Read::default()
        };
    };
    let handle = row.handle;
    let giver = holders.slot(row.owner);
    let mut read = Read {
        stray: giver.is_none(),
        ..Read::default()
    };
    read.tags.extend(giver);
    read.tags.push(receiver);
    if let Some(giver) = giver {
        read.fields
            .push((Field::LeaseGiver(receiver, handle), i128::from(giver)));
    }
    read.fields.extend([
        (Field::LeaseBad(receiver, handle), i128::from(row.bad)),
        (Field::LeaseConvoys(receiver, handle, 0), i128::from(row.convoys_assigned)),
        (Field::LeaseConvoys(receiver, handle, 1), i128::from(row.convoys_required)),
    ]);
    read
}

/// What the census holds of one row: its values by key, and whether it is an orphan or names a
/// stray partner.
struct Row {
    values: Vec<u64>,
    orphan: bool,
    stray: bool,
}

/// What the census holds of one table at its last read.
#[derive(Default)]
struct Table {
    complete: bool,
    rows: HashMap<u128, Row>,
    orphans: u64,
    strays: u64,
}

impl Table {
    /// Whether its [`Field::Known`] value is held: it was read complete, every row's holder
    /// identified.
    fn known(&self) -> bool {
        self.complete && self.orphans == 0
    }
}

/// What the census held at its last read, from which an hour's changed rows go on: the holders,
/// each table's rows, and each field's contributing rows by key (a whole read keeps the value
/// of the highest key, its last write).
#[derive(Default)]
struct Held {
    holders: Holders,
    tables: [Table; 3],
    fields: HashMap<u64, BTreeMap<u128, i128>>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    keyed: Keyed<Field, Extra>,
    /// Rebuilt by a whole read: after a resume, a gap or a change of the holders.
    #[serde(skip)]
    held: Option<Box<Held>>,
}

impl Prepared {
    /// What changed since its last save ([`Keyed::save_changes`]).
    pub fn save_changes(&mut self, whole: bool) -> io::Result<Vec<Vec<u8>>> {
        self.keyed.save_changes(whole)
    }

    /// The census its saves' pieces hold, in order; its holders are read whole at its next hour.
    pub fn resumed(pieces: &[Vec<u8>]) -> io::Result<Self> {
        let mut prepared = Self::default();
        for piece in pieces {
            prepared.keyed.apply_changes(piece)?;
        }
        Ok(prepared)
    }

    pub fn observe_changes(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()> {
        self.observe_edits(frame, changes, None)
    }

    /// One hour, as hourly preparation observes it: nothing where no watched table changed;
    /// else only the rows `edits` names, where it names them and the holders held, the census
    /// read whole otherwise.
    pub fn observe_edits(
        &mut self,
        frame: &DecodedFrame,
        changes: &FrameChanges,
        edits: Option<&[(TableId, Edit)]>,
    ) -> io::Result<()> {
        let gap = self.keyed.step(frame.hour)?;
        if !gap && WATCHED.iter().all(|table| !changes.rows_changed[table.index()]) {
            return Ok(());
        }
        let edits = edits.filter(|edits| {
            !gap
                && edits
                    .iter()
                    .all(|(table, _)| !matches!(table, TableId::Countries | TableId::CountryLinks))
        });
        match (edits, self.held.take()) {
            (Some(edits), Some(mut held)) => {
                self.observe_rows(frame, &mut held, edits);
                self.held = Some(held);
            }
            _ => self.observe(frame),
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn step(&mut self, frame: &DecodedFrame) {
        self.keyed.step(frame.hour).unwrap();
        self.observe(frame);
    }

    /// Learns the resources the definitions name, as each read does.
    fn learn(&mut self, frame: &DecodedFrame) {
        if let (Ok(definitions), Ok(lexical)) = (
            frame.complete::<tables::ResourceDefinitions>(),
            frame.complete::<tables::LexicalTokens>(),
        ) {
            let resources = &mut self.keyed.meta.extra.resources;
            let before = super::keyed::json_len(&*resources);
            if resources.learn(definitions, lexical) {
                let bytes = super::keyed::json_len(&*resources).saturating_sub(before);
                self.keyed.note_change(bytes);
            }
        }
    }

    /// The census read whole: every table's rows, every value observed and every value it no
    /// longer shows ended.
    fn observe(&mut self, frame: &DecodedFrame) {
        let hour = frame.hour;
        self.learn(frame);
        let holders = match Holders::read(frame) {
            Ok(holders) => holders,
            Err(error) => {
                self.keyed
                    .refuse(format!("country identities unavailable: {error}"));
                self.keyed.commit(hour, &Observed::new(), |_| true);
                self.held = None;
                return;
            }
        };
        let mut held = Box::new(Held {
            holders,
            ..Held::default()
        });
        let mut was = HashMap::new();
        for at in 0..CENSUS.len() {
            self.read_table(frame, &mut held, at, &mut was);
            self.refusals(frame, &held, at);
        }
        let mut observed = Observed::new();
        for (&key, rows) in &held.fields {
            if let Some(&value) = rows.values().next_back() {
                observed.insert(key, value);
            }
        }
        for (at, table) in held.tables.iter().enumerate() {
            if table.known()
                && let Some(key) = self.keyed.lookup(&Field::Known(at as u8))
            {
                observed.insert(key, 1);
            }
        }
        self.keyed.commit(hour, &observed, |_| true);
        self.held = Some(held);
    }

    /// The hour's changed rows: each table's rows `edits` names read again, the others held;
    /// every table's refusals counted again, as each read counts them.
    fn observe_rows(&mut self, frame: &DecodedFrame, held: &mut Held, edits: &[(TableId, Edit)]) {
        let hour = frame.hour;
        if edits
            .iter()
            .any(|(table, _)| matches!(table, TableId::ResourceDefinitions | TableId::LexicalTokens))
        {
            self.learn(frame);
        }
        // Each field the hour may have changed, with whether it held a value before.
        let mut was: HashMap<u64, bool> = HashMap::new();
        for at in 0..CENSUS.len() {
            let known = held.tables[at].known();
            match edits.iter().find(|(table, _)| *table == CENSUS[at]) {
                None => {}
                Some((_, Edit::Rows { written, deleted }))
                    if held.tables[at].complete && Self::complete(frame, at) =>
                {
                    let mut gone: Vec<u128> = written.iter().chain(deleted).copied().collect();
                    gone.sort_unstable();
                    gone.dedup();
                    for key in gone {
                        Self::remove_row(held, at, key, &mut was);
                    }
                    let mut written = written.clone();
                    written.sort_unstable();
                    written.dedup();
                    for key in written {
                        self.add_row(frame, held, at, key, &mut was);
                    }
                }
                Some(_) => {
                    let keys: Vec<u128> = held.tables[at].rows.keys().copied().collect();
                    for key in keys {
                        Self::remove_row(held, at, key, &mut was);
                    }
                    self.read_table(frame, held, at, &mut was);
                }
            }
            self.refusals(frame, held, at);
            if held.tables[at].known() != known
                && let Some(key) = self.keyed.lookup(&Field::Known(at as u8))
            {
                if known {
                    self.keyed.values.observe(key, hour, None, false);
                } else {
                    self.keyed
                        .values
                        .observe_if_changed(key, hour, Some(1), false, false);
                }
            }
        }
        for (key, held_before) in was {
            match held.fields.get(&key).and_then(|rows| rows.values().next_back()) {
                Some(&value) => {
                    self.keyed
                        .values
                        .observe_if_changed(key, hour, Some(value), false, false);
                }
                None if held_before => self.keyed.values.observe(key, hour, None, false),
                None => {}
            }
        }
    }

    /// Whether census table `at` holds every row this hour.
    fn complete(frame: &DecodedFrame, at: usize) -> bool {
        match at {
            0 => frame
                .complete::<tables::RegionalConvoyEfficiency>()
                .is_ok(),
            1 => frame.complete::<tables::ResourceExchanges>().is_ok(),
            _ => frame.complete::<tables::ConvoyClients>().is_ok(),
        }
    }

    /// Reads census table `at` whole into `held`, its fields' keys given in the order a whole
    /// read gives them: the table's [`Field::Known`] first, then each row's in row order.
    fn read_table(
        &mut self,
        frame: &DecodedFrame,
        held: &mut Held,
        at: usize,
        was: &mut HashMap<u64, bool>,
    ) {
        held.tables[at].complete = Self::complete(frame, at);
        if !held.tables[at].complete {
            return;
        }
        self.keyed.key(Field::Known(at as u8));
        let holders = &held.holders;
        let reads: Vec<(u128, Read)> = match at {
            0 => frame
                .complete::<tables::RegionalConvoyEfficiency>()
                .map(|rows| {
                    rows.iter()
                        .map(|row| (row.key(), read_regional(row, holders)))
                        .collect()
                })
                .unwrap_or_default(),
            1 => frame
                .complete::<tables::ResourceExchanges>()
                .map(|rows| {
                    rows.iter()
                        .map(|row| (row.key(), read_deal(row, holders)))
                        .collect()
                })
                .unwrap_or_default(),
            _ => frame
                .complete::<tables::ConvoyClients>()
                .map(|rows| {
                    rows.iter()
                        .map(|row| (row.key(), read_lease(row, holders)))
                        .collect()
                })
                .unwrap_or_default(),
        };
        for (key, read) in reads {
            self.hold(frame.hour, held, at, key, read, was);
        }
    }

    /// Holds `read`, census table `at`'s row `key`, and tags its holders; `was` notes each field
    /// it names with whether that field held a value before.
    fn hold(
        &mut self,
        hour: i32,
        held: &mut Held,
        at: usize,
        key: u128,
        read: Read,
        was: &mut HashMap<u64, bool>,
    ) {
        for &slot in &read.tags {
            self.keyed.tag_text(slot, held.holders.tag(slot), hour);
        }
        let mut values = Vec::with_capacity(read.fields.len());
        for (field, value) in read.fields {
            let id = self.keyed.key(field);
            let rows = held.fields.entry(id).or_default();
            was.entry(id).or_insert(!rows.is_empty());
            rows.insert(key, value);
            values.push(id);
        }
        let table = &mut held.tables[at];
        table.orphans += u64::from(read.orphan);
        table.strays += u64::from(read.stray);
        table.rows.insert(
            key,
            Row {
                values,
                orphan: read.orphan,
                stray: read.stray,
            },
        );
    }

    /// Reads census table `at`'s row `key` again from the frame, where it holds one.
    fn add_row(
        &mut self,
        frame: &DecodedFrame,
        held: &mut Held,
        at: usize,
        key: u128,
        was: &mut HashMap<u64, bool>,
    ) {
        fn find<R: RowCodec>(rows: &[R], key: u128) -> Option<&R> {
            rows.binary_search_by_key(&key, RowCodec::key)
                .ok()
                .map(|at| &rows[at])
        }
        let holders = &held.holders;
        let read = match at {
            0 => frame
                .complete::<tables::RegionalConvoyEfficiency>()
                .ok()
                .and_then(|rows| find(rows, key))
                .map(|row| read_regional(row, holders)),
            1 => frame
                .complete::<tables::ResourceExchanges>()
                .ok()
                .and_then(|rows| find(rows, key))
                .map(|row| read_deal(row, holders)),
            _ => frame
                .complete::<tables::ConvoyClients>()
                .ok()
                .and_then(|rows| find(rows, key))
                .map(|row| read_lease(row, holders)),
        };
        if let Some(read) = read {
            self.hold(frame.hour, held, at, key, read, was);
        }
    }

    /// Forgets census table `at`'s row `key`; `was` notes each field it named with whether
    /// that field held a value before.
    fn remove_row(held: &mut Held, at: usize, key: u128, was: &mut HashMap<u64, bool>) {
        let table = &mut held.tables[at];
        let Some(row) = table.rows.remove(&key) else {
            return;
        };
        table.orphans -= u64::from(row.orphan);
        table.strays -= u64::from(row.stray);
        for id in row.values {
            let Some(rows) = held.fields.get_mut(&id) else {
                continue;
            };
            was.entry(id).or_insert(!rows.is_empty());
            rows.remove(&key);
            if rows.is_empty() {
                held.fields.remove(&id);
            }
        }
    }

    /// Counts census table `at`'s refusals at this read, as a whole read names them: each row
    /// naming a holder with no identity, or the table unavailable.
    fn refusals(&mut self, frame: &DecodedFrame, held: &Held, at: usize) {
        let table = &held.tables[at];
        if !table.complete {
            let error = match at {
                0 => frame
                    .complete::<tables::RegionalConvoyEfficiency>()
                    .err()
                    .map(|error| format!("regional convoy efficiency unavailable: {error}")),
                1 => frame
                    .complete::<tables::ResourceExchanges>()
                    .err()
                    .map(|error| format!("resource deals unavailable: {error}")),
                _ => frame
                    .complete::<tables::ConvoyClients>()
                    .err()
                    .map(|error| format!("convoy clients unavailable: {error}")),
            };
            if let Some(error) = error {
                self.keyed.refuse(error);
            }
            return;
        }
        let (reason, times) = match at {
            0 => ("a regional efficiency's country has no identity", table.orphans),
            1 => (
                "a resource deal names a country with no identity",
                table.orphans + table.strays,
            ),
            _ => (
                "a lend-lease convoy client names a country with no identity",
                table.orphans + table.strays,
            ),
        };
        self.keyed.refuse_times(reason, times);
    }

    /// Bytes the next commit publishes and releases.
    pub fn bytes(&self) -> usize {
        self.keyed.pending()
    }

    pub fn flush(&mut self, writer: &mut store::Writer, branch: u64, cancelled: &AtomicBool) -> io::Result<()> {
        self.keyed.flush(writer, FAMILY, branch, cancelled)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum ConvoyView {
    Regions,
    Trade,
}

pub struct ConvoyHealth;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConvoyHealthParams {
    pub branch_id: String,
    pub view: ConvoyView,
    /// The one country shown.
    pub tag: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RegionEfficiency {
    pub region: u32,
    /// The region's in-game name; null where content does not name it.
    pub name: Option<String>,
    /// Fixed5 efficiency at each read.
    pub efficiency: Vec<EconomyRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DealEfficiency {
    /// The exchange's native handle, hexadecimal.
    pub id: String,
    pub exporter: Vec<EconomyRun>,
    /// The resource's native index.
    pub resource: Vec<EconomyRun>,
    /// The fixed5 efficiency product.
    pub efficiency: Vec<EconomyRun>,
    /// Its two fixed5 factors.
    pub factors: Vec<Vec<EconomyRun>>,
    /// Convoys assigned and required.
    pub convoys: Vec<Vec<EconomyRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LeaseConvoy {
    /// The client's native handle.
    pub handle: String,
    pub giver: Vec<EconomyRun>,
    /// "1" while its route draws as bad.
    pub bad: Vec<EconomyRun>,
    /// Convoys assigned and required.
    pub convoys: Vec<Vec<EconomyRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConvoyResource {
    pub id: u32,
    pub token: String,
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConvoyHealthReply {
    pub branch_id: String,
    pub view: ConvoyView,
    pub tag: String,
    pub last_hour: Option<i32>,
    /// Fixed-point denominator of every efficiency.
    pub scale: u32,
    /// Hours each table was read complete ("1"): regional efficiency,
    /// resource deals and convoy clients.
    pub known: Vec<Vec<EconomyRun>>,
    pub regions: Vec<RegionEfficiency>,
    pub deals: Vec<DealEfficiency>,
    pub leases: Vec<LeaseConvoy>,
    pub resources: Vec<ConvoyResource>,
    pub holders: BTreeMap<String, Vec<HolderTag>>,
    pub warnings: Vec<String>,
}

impl Product for ConvoyHealth {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = true;
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = ConvoyHealthParams;
    type Reply = ConvoyHealthReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        dashboard_base::products::validate_tag(&params.tag)
    }
    fn types() -> Vec<String> {
        vec![
            ConvoyView::decl(),
            ConvoyHealthParams::decl(),
            ConvoyHealthReply::decl(),
            RegionEfficiency::decl(),
            DealEfficiency::decl(),
            LeaseConvoy::decl(),
            ConvoyResource::decl(),
        ]
    }
}

/// Strategic regions' in-game names through the recorded content.
pub fn region_names(source: &Source, context: &LoadContext, warnings: &mut Vec<String>) -> BTreeMap<u32, String> {
    let catalogue = match context
        .content()
        .and_then(|options| catalogue::catalogue(source, options))
    {
        Ok(catalogue) => catalogue,
        Err(error) => {
            warnings.push(format!("Sea region names unavailable: {error}"));
            return BTreeMap::new();
        }
    };
    let regions = match dashboard_base::content::strategic_regions(&catalogue.content.layers) {
        Ok(regions) => regions,
        Err(error) => {
            warnings.push(format!("Sea region names unavailable: {error}"));
            return BTreeMap::new();
        }
    };
    warnings.extend(regions.notes.iter().cloned());
    let mut names = BTreeMap::new();
    let mut refused = 0_usize;
    for (&id, region) in &regions.regions {
        match catalogue.localised(&region.name_key, dashboard_text::catalogue::View::Plain) {
            Some(name) => {
                names.insert(id, name);
            }
            None => refused += 1,
        }
    }
    if refused > 0 {
        warnings.push(format!("{refused} sea region names are absent from the recorded content"));
    }
    names
}

/// Resource names through the recorded content.
pub type ResourceNames<'a> = &'a dyn Fn(&[(u32, String)]) -> (BTreeMap<u32, String>, Vec<String>);

#[allow(clippy::too_many_arguments)]
pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    params: &ConvoyHealthParams,
    through: Option<i32>,
    regions: &BTreeMap<u32, String>,
    resources: ResourceNames<'_>,
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<ConvoyHealthReply> {
    let branch = branch_of(&params.branch_id)?;
    let mut reply = ConvoyHealthReply {
        branch_id: params.branch_id.clone(),
        view: params.view,
        tag: params.tag.clone(),
        last_hour: through,
        scale: 100_000,
        known: vec![Vec::new(); 3],
        regions: Vec::new(),
        deals: Vec::new(),
        leases: Vec::new(),
        resources: Vec::new(),
        holders: BTreeMap::new(),
        warnings: Vec::new(),
    };
    let Some(through) = through else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let Some(mut published) =
        Published::<Field, Extra>::open(store, key, FAMILY, branch, through, cancelled)?
    else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let spans = published.holder_spans(std::slice::from_ref(&params.tag));
    let mut fields = BTreeMap::<(u64, u8), Vec<(u64, Field)>>::new();
    let mut known = Vec::new();
    for (&id, field) in &published.meta.fields {
        if let Field::Known(table) = *field {
            known.push((table, id));
            continue;
        }
        if !spans.contains_key(&field.holder()) {
            continue;
        }
        let owner = match (*field, params.view) {
            (Field::Regional(_, region), ConvoyView::Regions) => (u64::from(region), 0),
            (
                Field::DealExporter(_, deal)
                | Field::DealResource(_, deal)
                | Field::DealEfficiency(_, deal)
                | Field::DealFactor(_, deal, _)
                | Field::DealConvoys(_, deal, _),
                ConvoyView::Trade,
            ) => (deal, 1),
            (
                Field::LeaseGiver(_, handle)
                | Field::LeaseBad(_, handle)
                | Field::LeaseConvoys(_, handle, _),
                ConvoyView::Trade,
            ) => (u64::from(handle), 2),
            _ => continue,
        };
        fields.entry(owner).or_default().push((id, *field));
    }
    for (table, id) in known {
        if let Some(runs) = reply.known.get_mut(usize::from(table)) {
            *runs = published.text(id, &[(i32::MIN, i32::MAX)])?;
        }
    }
    let mut holders = std::collections::BTreeSet::new();
    let mut unnamed = 0_usize;
    let mut regional = BTreeMap::<u32, Vec<EconomyRun>>::new();
    for ((owner, kind), keys) in fields {
        published.check()?;
        let mut deal = DealEfficiency {
            id: format!("{owner:016x}"),
            exporter: Vec::new(),
            resource: Vec::new(),
            efficiency: Vec::new(),
            factors: vec![Vec::new(), Vec::new()],
            convoys: vec![Vec::new(), Vec::new()],
        };
        let mut lease = LeaseConvoy {
            handle: owner.to_string(),
            giver: Vec::new(),
            bad: Vec::new(),
            convoys: vec![Vec::new(), Vec::new()],
        };
        for (id, field) in keys {
            let holder_spans = spans.get(&field.holder()).map(Vec::as_slice).unwrap_or_default();
            let runs = published.text(id, holder_spans)?;
            match field {
                Field::Regional(..) => regional.entry(owner as u32).or_default().extend(runs),
                Field::DealExporter(..) => deal.exporter = runs,
                Field::DealResource(..) => deal.resource = runs,
                Field::DealEfficiency(..) => deal.efficiency = runs,
                Field::DealFactor(_, _, which) => deal.factors[usize::from(which.min(1))] = runs,
                Field::DealConvoys(_, _, which) => deal.convoys[usize::from(which.min(1))] = runs,
                Field::LeaseGiver(..) => lease.giver = runs,
                Field::LeaseBad(..) => lease.bad = runs,
                Field::LeaseConvoys(_, _, which) => lease.convoys[usize::from(which.min(1))] = runs,
                Field::Known(_) => {}
            }
        }
        match kind {
            1 if !deal.efficiency.is_empty() || !deal.exporter.is_empty() => {
                holders.extend(deal.exporter.iter().filter_map(|run| run.2.parse::<u32>().ok()));
                reply.deals.push(deal);
            }
            // Every recorded client has a route flag; its giver may be unidentified.
            2 if !lease.bad.is_empty() => {
                holders.extend(lease.giver.iter().filter_map(|run| run.2.parse::<u32>().ok()));
                reply.leases.push(lease);
            }
            _ => {}
        }
    }
    for (region, mut efficiency) in regional {
        efficiency.sort_by_key(|run| run.0);
        let name = regions.get(&region).cloned();
        if name.is_none() {
            unnamed += 1;
        }
        reply.regions.push(RegionEfficiency {
            region,
            name,
            efficiency,
        });
    }
    if unnamed > 0 {
        warnings.push(format!("{unnamed} sea regions have no in-game name; their ids are shown"));
    }
    if params.view == ConvoyView::Trade {
        let list = published.meta.extra.resources.list();
        let (names, notes) = resources(&list);
        warnings.extend(notes);
        reply.resources = list
            .into_iter()
            .map(|(id, token)| ConvoyResource {
                id,
                name: names.get(&id).cloned(),
                token,
            })
            .collect();
    }
    holders.extend(spans.keys().copied());
    reply.holders = holders
        .into_iter()
        .map(|slot| (slot.to_string(), published.tags(slot)))
        .collect();
    warnings.extend(published.warnings("Convoy health"));
    reply.warnings = warnings;
    Ok(reply)
}

pub mod served;
#[cfg(test)]
mod tests;
