//! Fuel by military branch, prepared on its own (family `fuel_branches`):
//! each country's fuel consumers as the game's fuel status holds them, in
//! native array order: the fuel each requested and received at the latest
//! read, its priority, and that read's hour.
//!
//! `CountryFuelConsumers` is read in its daily slot; its requested and
//! received amounts are fixed5 consumer state at that read, not a rate and
//! not the completed day's settled receipts (the fuel family keeps those).
//! An hour is observed when the table's rows changed, it was read, the
//! country identities changed, or it follows a gap; between reads every
//! value holds with its supplying read's hour. A read that is not complete
//! leaves the consumers unknown, never zero; one with a consumer whose
//! country has no identity keeps the others but proves no country has none.

use super::keyed::{
    EconomyRun, HolderTag, Keyed, Observed, Published, Tags, branch_of,
};
use dashboard_base::{
    Source,
    catalogue,
    prepared::frame_changes::FrameChanges,
    products::{LoadContext, Product},
};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

pub const FAMILY: &str = "fuel_branches";
pub const QUERY: &str = "fuel-branches";
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryFuelConsumers,
];
const WATCHED: [TableId; 3] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryFuelConsumers,
];

/// What one series key holds.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    /// Fixed5 fuel the consumer requested at the read.
    Requested(u32, u32),
    /// Fixed5 fuel the consumer received at the read.
    Received(u32, u32),
    Priority(u32, u32),
    /// The hour of the read supplying the holder's consumers.
    Read(u32),
    /// 1 while the consumer table was read complete with every consumer's
    /// country identified: a country without consumers then requested
    /// nothing; outside it nothing is known.
    Known,
}

impl Field {
    fn holder(&self) -> u32 {
        match *self {
            Self::Requested(slot, _)
            | Self::Received(slot, _)
            | Self::Priority(slot, _)
            | Self::Read(slot) => slot,
            Self::Known => u32::MAX,
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    keyed: Keyed<Field, ()>,
    /// Each holder's latest read hour, held between reads.
    reads: BTreeMap<u32, i32>,
}

impl Prepared {
    pub fn observe_changes(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()> {
        let gap = self.keyed.step(frame.hour)?;
        let index = TableId::CountryFuelConsumers.index();
        let read = changes.table_read[index] || changes.slot_read[index];
        if !gap && !read && WATCHED.iter().all(|table| !changes.rows_changed[table.index()]) {
            return Ok(());
        }
        self.observe(frame, read || gap);
        Ok(())
    }

    /// One hour whose rows changed, read or follows a gap.
    pub fn observe(&mut self, frame: &DecodedFrame, read: bool) {
        let hour = frame.hour;
        let mut observed = Observed::new();
        let tags = match Tags::new(frame) {
            Ok(tags) => tags,
            Err(error) => {
                self.keyed.refuse(format!("country identities unavailable: {error}"));
                self.reads.clear();
                self.keyed.commit(hour, &observed, |_| true);
                return;
            }
        };
        match frame.complete::<tables::CountryFuelConsumers>() {
            Ok(rows) => {
                let known = self.keyed.key(Field::Known);
                // An unidentified consumer may be any country's, so no absence is proven.
                let mut complete = true;
                let mut seen = BTreeMap::new();
                for row in rows {
                    let Some(slot) = tags.slot(row.country) else {
                        self.keyed.refuse("a fuel consumer's country has no identity");
                        complete = false;
                        continue;
                    };
                    self.keyed.tag(&tags, slot, hour);
                    seen.insert(slot, ());
                    for (field, value) in [
                        (Field::Requested(slot, row.index), i128::from(row.requested_raw)),
                        (Field::Received(slot, row.index), i128::from(row.received_raw)),
                        (Field::Priority(slot, row.index), i128::from(row.priority)),
                    ] {
                        observed.insert(self.keyed.key(field), value);
                    }
                }
                if complete {
                    observed.insert(known, 1);
                }
                self.reads.retain(|slot, _| seen.contains_key(slot));
                for &slot in seen.keys() {
                    let at = if read { hour } else { *self.reads.get(&slot).unwrap_or(&hour) };
                    self.reads.insert(slot, at);
                    observed.insert(self.keyed.key(Field::Read(slot)), i128::from(at));
                }
            }
            Err(error) => {
                self.keyed.refuse(format!("fuel consumers unavailable: {error}"));
                self.reads.clear();
            }
        }
        self.keyed.commit(hour, &observed, |_| true);
    }

    /// Bytes the next commit publishes and releases.
    pub fn bytes(&self) -> usize {
        self.keyed.pending()
    }

    pub fn flush(&mut self, writer: &mut store::Writer, branch: u64, cancelled: &AtomicBool) -> io::Result<()> {
        self.keyed.flush(writer, FAMILY, branch, cancelled)
    }
}

pub struct FuelBranches;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FuelBranchesParams {
    pub branch_id: String,
    /// The countries; every country when empty.
    pub tags: Vec<String>,
}

/// One consumer (military branch) of one country over the branch.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelBranch {
    /// Its native consumer index.
    pub index: u32,
    /// Fixed5 fuel requested at each read.
    pub requested: Vec<EconomyRun>,
    /// Fixed5 fuel received at each read.
    pub received: Vec<EconomyRun>,
    pub priority: Vec<EconomyRun>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelBranchCountry {
    pub holder: String,
    pub tags: Vec<HolderTag>,
    /// The supplying read's hour.
    pub read: Vec<EconomyRun>,
    pub branches: Vec<FuelBranch>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FuelBranchesReply {
    pub branch_id: String,
    /// Through the last prepared hour; later hours are still being prepared.
    pub last_hour: Option<i32>,
    /// Each native consumer index's label from the recorded content; null
    /// where content does not name it.
    pub consumer_names: Vec<Option<String>>,
    /// Fixed-point denominator of the amounts.
    pub scale: u32,
    /// Hours the consumer table was read complete with every consumer's
    /// country identified ("1").
    pub known: Vec<EconomyRun>,
    pub countries: Vec<FuelBranchCountry>,
    pub warnings: Vec<String>,
}

impl Product for FuelBranches {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = true;
    type Params = FuelBranchesParams;
    type Reply = FuelBranchesReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        for tag in &params.tags {
            dashboard_base::products::validate_tag(tag)?;
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            super::keyed::EconomyRun::decl(),
            super::keyed::HolderTag::decl(),
            FuelBranchesParams::decl(),
            FuelBranchesReply::decl(),
            FuelBranchCountry::decl(),
            FuelBranch::decl(),
        ]
    }
}

/// The content labels of the consumers, where the recorded stack names them.
pub fn labels(source: &Source, context: &LoadContext, warnings: &mut Vec<String>) -> [Option<String>; 3] {
    let catalogue = match context
        .content()
        .and_then(|options| catalogue::catalogue(source, options))
    {
        Ok(catalogue) => Some(catalogue),
        Err(error) => {
            warnings.push(format!("3 fuel branch labels cannot be resolved: {error}"));
            None
        }
    };
    dashboard_country_fuel::products::country_fuel::consumer_names(catalogue.as_deref(), warnings)
}

pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    params: &FuelBranchesParams,
    through: Option<i32>,
    labels: [Option<String>; 3],
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<FuelBranchesReply> {
    let branch = branch_of(&params.branch_id)?;
    let mut reply = FuelBranchesReply {
        branch_id: params.branch_id.clone(),
        last_hour: through,
        consumer_names: Vec::new(),
        scale: 100_000,
        known: Vec::new(),
        countries: Vec::new(),
        warnings: Vec::new(),
    };
    let Some(through) = through else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let Some(mut published) =
        Published::<Field, ()>::open(store, key, FAMILY, branch, through, cancelled)?
    else {
        reply.warnings = warnings;
        return Ok(reply);
    };
    let mut by_holder = BTreeMap::<u32, Vec<(u64, Field)>>::new();
    let mut known = None;
    for (&id, field) in &published.meta.fields {
        if *field == Field::Known {
            known = Some(id);
            continue;
        }
        by_holder.entry(field.holder()).or_default().push((id, field.clone()));
    }
    if let Some(id) = known {
        reply.known = published.text(id, &[(i32::MIN, i32::MAX)])?;
    }
    let mut indexes = std::collections::BTreeSet::new();
    for (slot, spans) in published.holder_spans(&params.tags) {
        published.check()?;
        let mut country = FuelBranchCountry {
            holder: slot.to_string(),
            tags: published.tags(slot),
            read: Vec::new(),
            branches: Vec::new(),
        };
        let mut branches = BTreeMap::<u32, FuelBranch>::new();
        for (id, field) in by_holder.get(&slot).into_iter().flatten() {
            let runs = published.text(*id, &spans)?;
            let index = match *field {
                Field::Read(_) => {
                    country.read = runs;
                    continue;
                }
                Field::Requested(_, index) | Field::Received(_, index) | Field::Priority(_, index) => index,
                Field::Known => continue,
            };
            let branch = branches.entry(index).or_insert_with(|| FuelBranch {
                index,
                requested: Vec::new(),
                received: Vec::new(),
                priority: Vec::new(),
            });
            match field {
                Field::Requested(..) => branch.requested = runs,
                Field::Received(..) => branch.received = runs,
                _ => branch.priority = runs,
            }
        }
        branches.retain(|_, branch| !branch.requested.is_empty() || !branch.received.is_empty());
        if branches.is_empty() {
            continue;
        }
        indexes.extend(branches.keys().copied());
        country.branches = branches.into_values().collect();
        reply.countries.push(country);
    }
    let last = indexes.last().map_or(0, |last| *last as usize + 1).max(labels.len());
    reply.consumer_names = (0..last).map(|index| labels.get(index).cloned().flatten()).collect();
    let beyond = indexes.iter().filter(|&&index| index as usize >= labels.len()).count();
    if beyond > 0 {
        warnings.push(format!("{beyond} fuel consumers beyond the fuel panel's three have no content label"));
    }
    warnings.extend(published.warnings("Fuel by branch"));
    reply.warnings = warnings;
    Ok(reply)
}

#[cfg(test)]
mod tests;
