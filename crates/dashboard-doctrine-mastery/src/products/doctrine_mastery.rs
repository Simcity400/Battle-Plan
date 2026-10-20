//! Doctrine mastery, bank and the independent source/effective rates, in fixed5.
//!
//! Names: `TAG@slot|folder|track|measure`, where folder and track are the recorded
//! folder/track ordinals and measure is mastery, bank, daily, effective, rewards
//! or sub. `TAG@slot|folder|grand` is the selected grand doctrine. Token values are
//! observer-interned ids (zero means no selection); `tokens` spells known ids.
//! `TAG@slot|present` witnesses an assigned doctrine slot, even with no folders.
//! `TAG@slot|folder|completed|ordinal` preserves the ordered completed tokens,
//! without dating unlocks. A witnessed folder with none of those names is empty.
//! Folder/track ids in the content tree match these native ordered indices;
//! grand/sub/reward ids are authored tokens. Raw mastery/rate/threshold text is
//! fixed5 (100,000 raw units per unit); rewards is an unscaled signed count.

mod content;
mod observer;
#[cfg(test)]
mod tests;

pub use observer::{DoctrineMasteryObserver, FAMILY};

use dashboard_base::products::{Product, check_reply_budget, validate_tag};
use dashboard_keyed::query::{KeyedSeries, KeyedWindow, Query};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};
use ts_rs::TS;

pub struct DoctrineMastery;

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DoctrineMasteryParams {
    pub branch_id: String,
    pub tag: String,
    pub first_hour: Option<i32>,
    pub last_hour: Option<i32>,
    /// Selected transient description scope. Omit for prepared history; numbers
    /// retain the same window and names in either request.
    pub hour: Option<i32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineMasteryReward {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub threshold: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineMasterySubdoctrine {
    pub id: String,
    /// Recorded interned identity; absent for unwitnessed authored alternatives.
    pub token_id: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub rewards: Option<Vec<DoctrineMasteryReward>>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineMasteryTrack {
    pub id: String,
    pub name: Option<String>,
    pub subdoctrines: Vec<DoctrineMasterySubdoctrine>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineMasteryGrand {
    pub id: String,
    /// Recorded interned identity; absent for unwitnessed authored alternatives.
    pub token_id: Option<String>,
    pub name: Option<String>,
    pub icon: Option<String>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineMasteryFolder {
    pub id: String,
    pub name: Option<String>,
    pub icon: Option<String>,
    pub grand: Vec<DoctrineMasteryGrand>,
    pub tracks: Vec<DoctrineMasteryTrack>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DoctrineMasteryReply {
    pub window: KeyedWindow,
    pub folders: Vec<DoctrineMasteryFolder>,
    pub tokens: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

impl Product for DoctrineMastery {
    const KEY: &'static str = "doctrine-mastery";
    const CONTENT_DEPENDENT: bool = true;
    type Params = DoctrineMasteryParams;
    type Reply = DoctrineMasteryReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        validate_tag(&params.tag)?;
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid doctrine mastery branch")?;
        if params
            .first_hour
            .zip(params.last_hour)
            .is_some_and(|(first, last)| first > last)
        {
            return Err("Invalid doctrine mastery window");
        }
        Ok(())
    }

    fn types() -> Vec<String> {
        vec![
            DoctrineMasteryParams::decl(),
            DoctrineMasteryReply::decl(),
            KeyedWindow::decl(),
            KeyedSeries::decl(),
            dashboard_base::products::country_groups::HourSpan::decl(),
            DoctrineMasteryFolder::decl(),
            DoctrineMasteryGrand::decl(),
            DoctrineMasteryTrack::decl(),
            DoctrineMasterySubdoctrine::decl(),
            DoctrineMasteryReward::decl(),
        ]
    }
}

/// The selected country's prepared window. Only an explicit `hour` asks the
/// shared frame cache for reached description operands.
pub fn query(query: &Query<'_>) -> io::Result<serde_json::Value> {
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    let params: DoctrineMasteryParams = serde_json::from_value(query.params.clone())
        .map_err(|error| dashboard_core::invalid(error.to_string()))?;
    DoctrineMastery::validate(&params).map_err(dashboard_core::invalid)?;
    if params.branch_id.parse::<u64>().ok() != Some(query.branch) {
        return Err(dashboard_core::invalid("Doctrine mastery branch differs"));
    }
    let (first, last) = query.window()?;
    let mut reply = DoctrineMasteryReply {
        window: KeyedWindow {
            first_hour: first,
            last_hour: last,
            ..Default::default()
        },
        folders: Vec::new(),
        tokens: BTreeMap::new(),
        warnings: Vec::new(),
    };
    if let Some(mut opened) = query.open::<DoctrineMasteryObserver>()? {
        reply.window = opened.window(first, last, query.through(), |name| {
            selected(name, &params.tag)
        })?;
        content::present(query, &opened.scan.observer, &mut reply)?;
        for (owner, refusals) in &opened.scan.observer.refusals {
            if owner.is_empty()
                || dashboard_keyed::countries::split(owner)
                    .is_some_and(|(tag, _)| tag == params.tag)
            {
                reply.warnings.extend(refusals.iter().map(|(why, count)| {
                    format!("{count} doctrine mastery observations refused: {why}")
                }));
            }
        }
    }
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    check_reply_budget(&reply)?;
    serde_json::to_value(reply).map_err(io::Error::other)
}

fn selected(name: &str, tag: &str) -> bool {
    name.split('|')
        .next()
        .and_then(dashboard_keyed::countries::split)
        .is_some_and(|(known, _)| known == tag)
}
