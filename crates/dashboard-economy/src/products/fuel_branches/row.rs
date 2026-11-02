//! The row census, kept as the columnar path's parity oracle.

use super::{FAMILY, FuelBranch, FuelBranchCountry, FuelBranchesParams, FuelBranchesReply};
use crate::products::keyed::{Keyed, Observed, Published, Tags, branch_of};
use dashboard_base::prepared::frame_changes::FrameChanges;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};

const WATCHED: [TableId; 3] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryFuelConsumers,
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    Requested(u32, u32),
    Received(u32, u32),
    Priority(u32, u32),
    Read(u32),
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
    pub(super) keyed: Keyed<Field, ()>,
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
                let mut complete = true;
                let mut seen = BTreeMap::new();
                for row in rows {
                    let Some(slot) = tags.slot(row.country) else {
                        self.keyed.refuse(super::emit::UNIDENTIFIED);
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

    pub fn flush(&mut self, writer: &mut store::Writer, branch: u64, cancelled: &AtomicBool) -> io::Result<()> {
        self.keyed.flush(writer, FAMILY, branch, cancelled)
    }
}

pub fn reply(
    store: &store::Store,
    key: &store::SourceKey,
    params: &FuelBranchesParams,
    through: Option<i32>,
    labels: [Option<String>; 3],
    mut warnings: Vec<String>,
    cancelled: &AtomicBool,
) -> io::Result<FuelBranchesReply> {
    let branch = branch_of(&params.branch_id)?;
    let mut reply = super::query::unpublished(params, through, Vec::new());
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
