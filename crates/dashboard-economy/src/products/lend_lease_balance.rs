//! Lend-lease balance (family `lend_lease_balance`): the game's own cumulative lend-lease
//! delivery histories, which the war overview keeps per country and partner, in its two measures
//! (industry and fuel), sent and received, prepared on the segment driver ([`facts`], [`emit`])
//! and read from its series publication ([`query`]).
//!
//! `WarLendLeaseHistories` holds every nonzero history of the countries the recorded war rosters
//! (`WarOwnedRelations`: each owner and both sides) name, each the owner's own record of a
//! partner. A country's totals are the sums of its own records only, so a pair is never counted
//! from both sides. A roster country the complete table does not list has exact zero totals, and
//! a record it no longer lists is zero. A country outside the rosters, or while they are not
//! read, has no value: its history is not recorded, so it stays unknown rather than zero.
//! Histories can predate the recording; a decrease or reset is kept as recorded. Amounts are
//! fixed5 and stay exact.

use super::keyed::{EconomyRun, HolderTag};
use dashboard_base::products::Product;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub mod emit;
pub mod facts;
pub mod query;
pub mod served;

#[cfg(test)]
mod columnar_tests;
#[cfg(test)]
mod row;

pub const FAMILY: &str = "lend_lease_balance";
pub const QUERY: &str = "lend-lease-balance";

/// The four recorded measures, in this order: industry sent, fuel sent, industry received, fuel
/// received.
pub const MEASURES: usize = 4;

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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
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

/// The refusal a history with no owner identity counts once a read.
pub const ORPHAN: &str = "a lend-lease history names a country with no identity";
/// The refusal a history with no partner identity counts once a read.
pub const PARTNER: &str = "a lend-lease history names a partner with no identity";
/// The refusal each roster country with no identity counts once a read.
pub const UNROSTERED: &str = "a war roster names a country with no identity";
