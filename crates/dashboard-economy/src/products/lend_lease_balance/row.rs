//! The family's row preparation and reply, as main prepared it from row frames: the oracle the
//! segment driver's publication is held to ([`super::columnar_tests`]).

use super::{
    FAMILY, LendLeaseBalanceParams, LendLeaseBalanceReply, LendLeaseHistory, MEASURES, ORPHAN,
    PARTNER, UNROSTERED,
};
use crate::products::keyed::{EconomyRun, Keyed, Observed, Published, Tags, branch_of};
use dashboard_base::prepared::frame_changes::FrameChanges;
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};

const WATCHED: [TableId; 4] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WarOwnedRelations,
    TableId::WarLendLeaseHistories,
];

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Field {
    /// The owner's record of one partner, in measure `2`.
    Pair(u32, u32, u8),
    /// The sum of the owner's records, in measure `1`.
    Total(u32, u8),
    /// 1 while the histories were read complete.
    Known,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Prepared {
    keyed: Keyed<Field, ()>,
}

impl Prepared {
    pub fn observe_changes(&mut self, frame: &DecodedFrame, changes: &FrameChanges) -> io::Result<()> {
        let gap = self.keyed.step(frame.hour)?;
        if !gap && WATCHED.iter().all(|table| !changes.rows_changed[table.index()]) {
            return Ok(());
        }
        self.observe(frame);
        Ok(())
    }

    pub fn observe(&mut self, frame: &DecodedFrame) {
        let hour = frame.hour;
        let mut observed = Observed::new();
        match (Tags::new(frame), frame.complete::<tables::WarLendLeaseHistories>()) {
            (Ok(tags), Ok(rows)) => {
                observed.insert(self.keyed.key(Field::Known), 1);
                let mut totals = BTreeMap::<u32, [i128; MEASURES]>::new();
                // Whether a record's owner, or which owners' partners, have no identity.
                let (mut orphaned, mut partial) = (false, BTreeSet::new());
                for row in rows {
                    let Some(owner) = tags.slot(row.country) else {
                        self.keyed.refuse(ORPHAN);
                        orphaned = true;
                        continue;
                    };
                    self.keyed.tag(&tags, owner, hour);
                    let values = [
                        row.industry_sent,
                        row.fuel_sent,
                        row.industry_received,
                        row.fuel_received,
                    ];
                    let total = totals.entry(owner).or_default();
                    for (measure, value) in values.into_iter().enumerate() {
                        total[measure] += i128::from(value);
                    }
                    // The owner's record still counts in its total when only the partner is unknown.
                    let Some(target) = tags.slot(row.target) else {
                        self.keyed.refuse(PARTNER);
                        partial.insert(owner);
                        continue;
                    };
                    self.keyed.tag(&tags, target, hour);
                    for (measure, value) in values.into_iter().enumerate() {
                        observed.insert(
                            self.keyed.key(Field::Pair(owner, target, measure as u8)),
                            i128::from(value),
                        );
                    }
                }
                // The table keeps only nonzero histories: a roster owner it does not list has
                // none, unless a record it might hold has no owner identity.
                let roster = if orphaned { None } else { self.roster(frame, &tags) };
                for &owner in roster.iter().flatten() {
                    if !totals.contains_key(&owner) {
                        self.keyed.tag(&tags, owner, hour);
                        totals.insert(owner, [0; MEASURES]);
                    }
                }
                for (owner, total) in totals {
                    for (measure, value) in total.into_iter().enumerate() {
                        observed.insert(self.keyed.key(Field::Total(owner, measure as u8)), value);
                    }
                }
                // And a record a roster owner no longer lists is zero, not ended.
                if let Some(roster) = &roster {
                    let zeroed: Vec<u64> = self
                        .keyed
                        .values
                        .active()
                        .filter(|key| !observed.contains_key(key))
                        .filter(|key| {
                            matches!(
                                self.keyed.meta.fields.get(key),
                                Some(Field::Pair(owner, ..))
                                    if roster.contains(owner) && !partial.contains(owner)
                            )
                        })
                        .collect();
                    for key in zeroed {
                        observed.insert(key, 0);
                    }
                }
            }
            (Err(error), _) => self
                .keyed
                .refuse(format!("country identities unavailable: {error}")),
            (_, Err(error)) => self
                .keyed
                .refuse(format!("lend-lease histories unavailable: {error}")),
        }
        self.keyed.commit(hour, &observed, |_| true);
    }

    /// The holders the hour's recorded war rosters name, whose histories the
    /// table keeps; `None` while the rosters are not read.
    fn roster(&mut self, frame: &DecodedFrame, tags: &Tags<'_>) -> Option<BTreeSet<u32>> {
        let rows = match frame.complete::<tables::WarOwnedRelations>() {
            Ok(rows) => rows,
            Err(error) => {
                self.keyed
                    .refuse(format!("war rosters unavailable, so unlisted totals are unknown: {error}"));
                return None;
            }
        };
        let mut roster = BTreeSet::new();
        for row in rows {
            for country in [row.owner, row.first_country, row.second_country] {
                match tags.slot(country) {
                    Some(slot) => {
                        roster.insert(slot);
                    }
                    None => self.keyed.refuse(UNROSTERED),
                }
            }
        }
        Some(roster)
    }

    pub fn flush(&mut self, writer: &mut store::Writer, branch: u64, cancelled: &AtomicBool) -> io::Result<()> {
        self.keyed.flush(writer, FAMILY, branch, cancelled)
    }

    pub(crate) fn step(&mut self, frame: &DecodedFrame) {
        self.keyed.step(frame.hour).unwrap();
        self.observe(frame);
    }
}

pub fn query(
    store: &store::Store,
    key: &store::SourceKey,
    params: &LendLeaseBalanceParams,
    through: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<LendLeaseBalanceReply> {
    let branch = branch_of(&params.branch_id)?;
    let holder = params
        .holder
        .as_deref()
        .map(str::parse::<u32>)
        .transpose()
        .map_err(|_| dashboard_base::invalid("Invalid lend-lease holder"))?;
    let mut reply = LendLeaseBalanceReply {
        branch_id: params.branch_id.clone(),
        last_hour: through,
        scale: 100_000,
        known: Vec::new(),
        histories: Vec::new(),
        holder: params.holder.clone(),
        warnings: Vec::new(),
    };
    let Some(through) = through else {
        return Ok(reply);
    };
    let Some(mut published) =
        Published::<Field, ()>::open(store, key, FAMILY, branch, through, cancelled)?
    else {
        return Ok(reply);
    };
    let mut wanted = BTreeMap::<u32, Vec<(u8, u64)>>::new();
    let mut known = None;
    for (&id, field) in &published.meta.fields {
        match (*field, holder) {
            (Field::Known, _) => known = Some(id),
            (Field::Total(owner, measure), None) => wanted.entry(owner).or_default().push((measure, id)),
            (Field::Pair(owner, target, measure), Some(holder)) if owner == holder => {
                wanted.entry(target).or_default().push((measure, id));
            }
            _ => {}
        }
    }
    let all = [(i32::MIN, i32::MAX)];
    if let Some(id) = known {
        reply.known = published.text(id, &all)?;
    }
    for (slot, keys) in wanted {
        published.check()?;
        let mut values: Vec<Vec<EconomyRun>> = vec![Vec::new(); MEASURES];
        for (measure, id) in keys {
            values[usize::from(measure)] = published.text(id, &all)?;
        }
        if values.iter().all(Vec::is_empty) {
            continue;
        }
        reply.histories.push(LendLeaseHistory {
            holder: slot.to_string(),
            tags: published.tags(slot),
            values,
        });
    }
    reply.warnings = published.warnings("Lend-lease balance");
    Ok(reply)
}

#[cfg(test)]
mod tests;
