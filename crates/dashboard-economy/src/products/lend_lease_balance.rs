//! Lend-lease balance, prepared on its own (family `lend_lease_balance`):
//! the game's own cumulative lend-lease delivery histories, which the war
//! overview keeps per country and partner, in its two measures (industry and
//! fuel), sent and received.
//!
//! `WarLendLeaseHistories` holds every nonzero history of the countries the
//! recorded war rosters name, each the owner's own record of a partner. A
//! country's totals are the sums of its own records only, so a pair is never
//! counted from both sides. A country the table does not list has no value:
//! outside the war rosters its history is not recorded, so it stays unknown
//! rather than zero. Histories can predate the recording; a decrease or reset
//! is kept as recorded. Amounts are fixed5 and stay exact.

use super::keyed::{EconomyRun, HolderTag, Keyed, Observed, Published, Tags, branch_of};
use dashboard_base::{prepared::frame_changes::FrameChanges, products::Product};
use replay_data::rows::{TableId, tables};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io, sync::atomic::AtomicBool};
use ts_rs::TS;

pub const FAMILY: &str = "lend_lease_balance";
pub const QUERY: &str = "lend-lease-balance";
pub const TABLES: &[TableId] = &[
    TableId::Campaign,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WarLendLeaseHistories,
];
const WATCHED: [TableId; 3] = [
    TableId::Countries,
    TableId::CountryLinks,
    TableId::WarLendLeaseHistories,
];

/// The four recorded measures, in this order.
pub const MEASURES: usize = 4;

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
                for row in rows {
                    let (Some(owner), Some(target)) = (tags.slot(row.country), tags.slot(row.target))
                    else {
                        self.keyed
                            .refuse("a lend-lease history names a country with no identity");
                        continue;
                    };
                    self.keyed.tag(&tags, owner, hour);
                    self.keyed.tag(&tags, target, hour);
                    let values = [
                        row.industry_sent,
                        row.fuel_sent,
                        row.industry_received,
                        row.fuel_received,
                    ];
                    let total = totals.entry(owner).or_default();
                    for (measure, value) in values.into_iter().enumerate() {
                        total[measure] += i128::from(value);
                        observed.insert(
                            self.keyed.key(Field::Pair(owner, target, measure as u8)),
                            i128::from(value),
                        );
                    }
                }
                for (owner, total) in totals {
                    for (measure, value) in total.into_iter().enumerate() {
                        observed.insert(self.keyed.key(Field::Total(owner, measure as u8)), value);
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

    /// Bytes the next commit publishes and releases.
    pub fn bytes(&self) -> usize {
        self.keyed.pending()
    }

    pub fn flush(&mut self, writer: &mut store::Writer, branch: u64, cancelled: &AtomicBool) -> io::Result<()> {
        self.keyed.flush(writer, FAMILY, branch, cancelled)
    }

    #[cfg(test)]
    pub(crate) fn step(&mut self, frame: &DecodedFrame) {
        self.keyed.step(frame.hour).unwrap();
        self.observe(frame);
    }
}

pub struct LendLeaseBalance;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LendLeaseBalanceParams {
    pub branch_id: String,
    /// One holder's partners; every country's totals when absent.
    #[serde(default)]
    #[ts(optional)]
    pub holder: Option<String>,
}

/// A country's (or a partner's) cumulative histories over the branch, in
/// the reply's measure order: industry sent, fuel sent, industry received,
/// fuel received.
#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LendLeaseHistory {
    pub holder: String,
    pub tags: Vec<HolderTag>,
    pub values: Vec<Vec<EconomyRun>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LendLeaseBalanceReply {
    pub branch_id: String,
    pub last_hour: Option<i32>,
    /// Fixed-point denominator of every amount.
    pub scale: u32,
    /// Hours the histories were read complete ("1"): outside them nothing is
    /// known, and a country absent while they were read is outside the war
    /// rosters the game records histories for, so unknown too.
    pub known: Vec<EconomyRun>,
    /// Each country's totals over its own records, or, for one holder, its
    /// record of each partner.
    pub histories: Vec<LendLeaseHistory>,
    pub holder: Option<String>,
    pub warnings: Vec<String>,
}

impl Product for LendLeaseBalance {
    const KEY: &'static str = QUERY;
    const CONTENT_DEPENDENT: bool = false;
    type Params = LendLeaseBalanceParams;
    type Reply = LendLeaseBalanceReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if let Some(holder) = &params.holder {
            holder.parse::<u32>().map_err(|_| "Invalid holder")?;
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            LendLeaseBalanceParams::decl(),
            LendLeaseBalanceReply::decl(),
            LendLeaseHistory::decl(),
        ]
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
        let mut values = vec![Vec::new(); MEASURES];
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
