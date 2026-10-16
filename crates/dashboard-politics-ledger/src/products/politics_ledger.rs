//! Recorded per-country mutations. Prices are native evaluated operands,
//! never reconstructed content prices or balance falls. Spending presentation
//! shares the prepared native accounting with the raw ledger.
mod spending;
#[cfg(any(test, feature = "test-support"))]
use super::LoadContext;
use super::{CountryParams, Product};
use crate::Source;
use replay::{frame::DecodedFrame, rows::*};
use serde::{Deserialize, Serialize};
pub use spending::prepared;
pub use spending::{
    LedgerCommandHold, LedgerCommandHolder, LedgerIdeaTooltips, LedgerSpend, LedgerSpendCategory,
    LedgerSpendChange, LedgerSpendCoverage, LedgerSpendItem,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::{AtomicBool, Ordering},
};
use ts_rs::TS;
pub struct PoliticsLedger;
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum LedgerView {
    #[default]
    Raw,
    Spending,
    Balance,
    Governments,
}
#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoliticsLedgerParams {
    pub branch_id: String,
    pub tag: String,
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub view: LedgerView,
}
impl PoliticsLedgerParams {
    fn country(&self) -> CountryParams {
        CountryParams {
            branch_id: self.branch_id.clone(),
            tag: self.tag.clone(),
        }
    }
    #[cfg(any(test, feature = "test-support"))]
    fn branch(&self) -> u64 {
        self.branch_id.parse().expect("validated branch id")
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerItem {
    pub action: String,
    pub token: Option<u32>,
    pub key: Vec<u8>,
    pub slot_key: Vec<u8>,
    pub character: Option<u32>,
    pub object_kind: String,
    pub object_reference: Option<String>,
    pub target_country: Option<u32>,
    pub target_state: Option<u32>,
    pub target_region: Option<u32>,
    pub origin_kind: String,
    pub origin_token: Option<u32>,
    pub origin_key: Vec<u8>,
}
impl From<&PowerIdentity> for LedgerItem {
    fn from(item: &PowerIdentity) -> Self {
        Self {
            action: format!("{:?}", item.action),
            token: item.item_token,
            key: item.item_key.clone(),
            slot_key: item.slot_key.clone(),
            character: item.character,
            object_kind: format!("{:?}", item.object_kind),
            object_reference: item.object_reference.map(|value| value.to_string()),
            target_country: item.target_country,
            target_state: item.target_state,
            target_region: item.target_region,
            origin_kind: format!("{:?}", item.origin_kind),
            origin_token: item.origin_token,
            origin_key: item.origin_key.clone(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerMutation {
    pub country: u32,
    pub instance: u32,
    pub sequence: String,
    pub epoch: String,
    pub hour: i32,
    pub read_hour: i32,
    pub currency: String,
    pub cause: String,
    pub operation: String,
    pub site: String,
    pub amount_raw: Option<String>,
    pub realized_delta_raw: String,
    pub before_raw: String,
    pub after_raw: String,
    pub item: LedgerItem,
    pub covered: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerGovernment {
    pub country: u32,
    pub instance: u32,
    pub sequence: String,
    pub epoch: String,
    pub hour: i32,
    pub read_hour: i32,
    pub cause: String,
    pub before_party: Option<u32>,
    pub after_party: Option<u32>,
    pub before_ideology: Option<u32>,
    pub after_ideology: Option<u32>,
    /// Event-dated lexical group names; never borrowed from a later party census.
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub before_token: Option<String>,
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub after_token: Option<String>,
    pub actor_country: Option<u32>,
    pub origin_kind: String,
    pub origin_token: Option<u32>,
    pub origin_key: Vec<u8>,
    pub covered: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerCoverage {
    pub country: Option<u32>,
    pub instance: Option<u32>,
    pub read_hour: i32,
    pub since_hour: Option<i32>,
    pub through_hour: Option<i32>,
    pub epoch: Option<String>,
    pub streams: u8,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerDaily {
    pub country: u32,
    pub instance: u32,
    pub epoch: String,
    pub day_hour: i32,
    pub read_hour: i32,
    /// The native daily PP and CP tuples may reach different supplying reads.
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub pp_read_hour: Option<i32>,
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub cp_read_hour: Option<i32>,
    pub pp_gross_raw: Option<String>,
    pub pp_cost_raw: Option<String>,
    pub pp_income_raw: Option<String>,
    pub pp_realized_raw: Option<String>,
    pub pp_sequence: Option<String>,
    pub cp_income_raw: Option<String>,
    pub cp_realized_raw: Option<String>,
    pub cp_sequence: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerDailyCost {
    pub country: u32,
    pub instance: u32,
    pub epoch: String,
    pub day_hour: i32,
    pub read_hour: i32,
    pub covered: bool,
    pub pp_sequence: String,
    pub ordinal: u32,
    pub component: String,
    pub amount_raw: String,
    pub cause: String,
    pub item: LedgerItem,
}
/// A native application, distinct from its evaluated charge and daily rate.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerBalanceChange {
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub country_id: u32,
    pub hour: i32,
    pub read_hour: i32,
    pub currency: String,
    pub amount_raw: Option<String>,
    pub realized_delta_raw: String,
    pub action: String,
    pub covered: bool,
    pub reason: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct PoliticsLedgerReply {
    pub branch_id: String,
    pub tag: String,
    pub decimals: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub mutations: Vec<LedgerMutation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub governments: Vec<LedgerGovernment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub daily: Vec<LedgerDaily>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub daily_costs: Vec<LedgerDailyCost>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub coverage: Vec<LedgerCoverage>,
    pub warnings: Vec<String>,
    pub spending: Vec<LedgerSpend>,
    pub spend_items: Vec<LedgerSpendItem>,
    pub spend_categories: Vec<LedgerSpendCategory>,
    pub spending_coverage: Vec<LedgerSpendCoverage>,
    pub idea_tooltips: Option<LedgerIdeaTooltips>,
    pub command_holds: Vec<LedgerCommandHold>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub balance_changes: Vec<LedgerBalanceChange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub recurring: Vec<LedgerRecurringSpend>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub gains: Vec<LedgerGain>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub recurring_days: Vec<LedgerMonthDays>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub part_summaries: Vec<LedgerPartSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub government_marks: Option<crate::prepared_chart::MarkSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerPartSummary {
    pub id: String,
    pub label: String,
    pub summary: crate::prepared_chart::ChartSummary,
}

/// A prepared recurring subtotal for one item within a game month or a
/// requested membership interval. It has no list of individual paid days.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerRecurringSpend {
    pub spend: LedgerSpend,
    pub first_hour: i32,
    pub last_hour: i32,
    pub days: u32,
    pub count: u32,
    pub known_raw: String,
    pub known_count: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerMonthDays {
    pub currency: String,
    pub category: String,
    pub month: Option<i32>,
    pub days: u32,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerGain {
    pub currency: String,
    pub gained_raw: String,
    pub lost_raw: String,
    pub complete: bool,
}
impl Product for PoliticsLedger {
    const KEY: &'static str = "politics-ledger";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PoliticsLedgerParams;
    type Reply = PoliticsLedgerReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.country().validate()
    }
    fn types() -> Vec<String> {
        vec![
            PoliticsLedgerParams::decl(),
            LedgerView::decl(),
            LedgerItem::decl(),
            LedgerMutation::decl(),
            LedgerGovernment::decl(),
            LedgerCoverage::decl(),
            LedgerDaily::decl(),
            LedgerDailyCost::decl(),
            LedgerBalanceChange::decl(),
            LedgerSpend::decl(),
            LedgerSpendChange::decl(),
            LedgerSpendItem::decl(),
            LedgerSpendCategory::decl(),
            LedgerSpendCoverage::decl(),
            LedgerIdeaTooltips::decl(),
            LedgerCommandHold::decl(),
            LedgerCommandHolder::decl(),
            PoliticsLedgerReply::decl(),
            LedgerRecurringSpend::decl(),
            LedgerGain::decl(),
            LedgerMonthDays::decl(),
            LedgerPartSummary::decl(),
        ]
    }
    #[cfg(any(test, feature = "test-support"))]
    fn load(
        source: &Source,
        params: &Self::Params,
        context: &LoadContext,
        cancelled: &AtomicBool,
    ) -> io::Result<Self::Reply> {
        spending::load(source, params, context, cancelled)
    }
}
fn coverage(row: &PoliticsCaptureCoverageRow, read_hour: i32) -> LedgerCoverage {
    LedgerCoverage {
        country: Some(row.country),
        instance: Some(row.instance),
        read_hour,
        since_hour: Some(row.since_hour),
        through_hour: Some(row.through_hour),
        epoch: Some(row.epoch.to_string()),
        streams: row.streams,
        reason: None,
    }
}
#[cfg(test)]
mod tests;
