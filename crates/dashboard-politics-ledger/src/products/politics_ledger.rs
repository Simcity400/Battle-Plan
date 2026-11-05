//! Recorded per-country political and command power: the spending, balance and government
//! changes the capture recorded, as widgets ask for them. Prices
//! are native evaluated operands, never reconstructed content prices or balance falls.
//!
//! A segment operator reads each hour's changed native rows and, at an item's own hour, what
//! only that hour's frame says of it ([`facts`]); the emit accounts them into the family's series
//! publication, each row and item as replies show it, named with the recorded content ([`emit`],
//! [`present`], [`records`]); a reply reads them back ([`query`]).

pub mod emit;
pub mod facts;
mod items;
mod present;
pub mod query;
mod records;
mod rows;
pub mod served;
mod tally;
#[cfg(test)]
mod tests;

use dashboard_base::products::idea_format::{LocalisedText, ModifierFormat};
use dashboard_base::products::idea_history::IdeaInfo;
use dashboard_base::products::politics_content::{PoliticsContent, recorded_politics_content};
use dashboard_base::products::{CountryParams, Product};
use dashboard_base::source::Source;
pub use present::{LedgerArt, present_icons};
use replay_data::rows::*;
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use ts_rs::TS;

/// The recorded content a preparation names items with, or why it cannot.
pub type PreparedContent = Result<Option<Arc<PoliticsContent>>, String>;

/// The recorded mod stack's politics content, as preparation and replies name items with it.
pub fn prepared_content(
    source: &Source,
    options: Option<&dashboard_core::config::ContentOptions>,
) -> PreparedContent {
    let content = options
        .map(|options| recorded_politics_content(source, options))
        .transpose()
        .map_err(|reason| reason.to_string())?;
    if let Some(content) = &content {
        let _ = content.prepare_power_content();
    }
    Ok(content)
}

pub struct PoliticsLedger;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum LedgerView {
    Spending,
    Balance,
    Governments,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PoliticsLedgerParams {
    pub branch_id: String,
    pub tag: String,
    pub view: LedgerView,
}

impl PoliticsLedgerParams {
    fn country(&self) -> CountryParams {
        CountryParams {
            branch_id: self.branch_id.clone(),
            tag: self.tag.clone(),
        }
    }
}

/// A recorded item identity, as presentation and holders read it.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
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

/// A payment as its item's presentation reads it.
#[derive(Clone, Debug)]
pub struct PaidRow {
    pub currency: String,
    pub cause: String,
    pub item: LedgerItem,
    /// The payment took power: a negative requested amount.
    pub charged: bool,
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
    /// Event-dated lexical group names; never borrowed from a later party census.
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub before_token: Option<String>,
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub after_token: Option<String>,
    pub covered: bool,
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

/// A native application: where keyboard hover stops.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerBalanceChange {
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub country_id: u32,
    pub hour: i32,
    pub read_hour: i32,
    pub currency: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerIdeaTooltips {
    pub ideas: Vec<IdeaInfo>,
    pub artwork: Vec<dashboard_core::dto::Artwork>,
    pub modifiers: Vec<ModifierFormat>,
    pub texts: Vec<LocalisedText>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSpendChange {
    /// Native date of this contribution, separate from its supplying read.
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub hour: Option<i32>,
    pub read_hour: i32,
    pub charge_raw: Option<String>,
    pub realized_delta_raw: Option<String>,
    pub occurrences: u32,
    pub covered: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSpend {
    /// Recorded country link identity, also CountryRow.id.
    #[serde(default)]
    #[ts(optional, as = "Option<_>")]
    pub country_id: u32,
    pub id: String,
    pub sequence: String,
    pub ordinal: u32,
    pub hour: i32,
    pub read_hour: i32,
    pub country_tag: String,
    pub country_name: Option<String>,
    pub currency: String,
    pub category: String,
    pub item_id: String,
    pub charge_raw: Option<String>,
    pub realized_delta_raw: Option<String>,
    pub covered: bool,
    pub occurrences: u32,
    /// Upkeep has one row per country and day; each contribution stays dated by its own read.
    pub changes: Vec<LedgerSpendChange>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSpendItem {
    pub id: String,
    /// Specific-item identity includes the recorded target and template.
    pub group_id: String,
    pub name: Option<String>,
    /// Plain display text, using only the recorded cause, item and target.
    pub action: String,
    /// Drawn when a reply is served, from the request's selection ([`Self::icon_art`]).
    pub icon: Option<String>,
    /// What the icon is drawn from, taken when a reply is served.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub icon_art: Option<LedgerArt>,
    pub tooltip_token: Option<String>,
    pub character: Option<u32>,
    pub target_tag: Option<String>,
    pub target_name: Option<String>,
    pub unknown: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSpendCategory {
    pub id: String,
    pub name: String,
    pub identified: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerSpendCoverage {
    pub read_hour: i32,
    pub since_hour: Option<i32>,
    pub through_hour: Option<i32>,
    pub political: bool,
    pub command: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerCommandHolder {
    pub id: String,
    /// Recorded object domain, cause or writer site; never a guessed holder.
    pub kind: String,
    pub name: Option<String>,
    pub amount_raw: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerCommandHold {
    pub read_hour: i32,
    pub through_hour: i32,
    pub total_raw: Option<String>,
    pub holders: Vec<LedgerCommandHolder>,
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
    pub governments: Vec<LedgerGovernment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<_>")]
    pub daily: Vec<LedgerDaily>,
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
    pub government_marks: Option<dashboard_base::prepared_chart::MarkSummary>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, TS)]
#[serde(rename_all = "camelCase")]
pub struct LedgerPartSummary {
    pub id: String,
    pub label: String,
    pub summary: dashboard_base::prepared_chart::ChartSummary,
}

/// A recurring subtotal for one item within a game month or a requested membership interval.
/// It has no list of individual paid days.
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
    const SERVED: Option<&'static dashboard_base::prepared::served::Plan> = Some(&served::PLAN);
    type Params = PoliticsLedgerParams;
    type Reply = PoliticsLedgerReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.country().validate()
    }
    fn types() -> Vec<String> {
        vec![
            PoliticsLedgerParams::decl(),
            LedgerView::decl(),
            LedgerGovernment::decl(),
            LedgerDaily::decl(),
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
}
