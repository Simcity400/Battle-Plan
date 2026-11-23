
use dashboard_base::products::validate_country;
use dashboard_base::products::Country;
use dashboard_base::{products::{LoadContext, Product}, queries::country_ids::Names};
use dashboard_world::products::subjects::settle;
use super::lend_lease_balance::SCALE;
use dashboard_base::{Source, span::HourSpan};
use dashboard_query::identity::Holder;
use replay_lanes::lanes::{self, Cell, Keys};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct LendLeasePartners;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum LendLeaseMeasure {
    Industry,
    Fuel,
}

impl LendLeaseMeasure {
    fn columns(self) -> [&'static str; 2] {
        match self {
            Self::Industry => ["industry_sent", "industry_received"],
            Self::Fuel => ["fuel_sent", "fuel_received"],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LendLeasePartnersParams {
    pub branch_id: String,
    pub holder: Country,
    pub measure: LendLeaseMeasure,
    #[doc = " The hours read; the whole timeline when absent."]
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from: Option<i32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub to: Option<i32>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LendLeaseTag {
    pub first_hour: i32,
    pub last_hour: i32,
    pub tag: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LendLeasePartner {
    pub slot: u32,
    pub tags: Vec<LendLeaseTag>,
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<Array<[number, number, string]>>"))]
    pub values: Vec<Vec<(i32, i32, String)>>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct LendLeasePartnersReply {
    pub holder: String,
    pub scale: u32,
    pub first_hour: i32,
    pub last_hour: i32,
    #[doc = " Hours the histories were read while the tag had a holder: outside them nothing is known."]
    pub known: Vec<HourSpan>,
    #[doc = " The holder's own two totals of the measure over its partners, from its first partner on."]
    #[cfg_attr(any(test, feature = "export-types"), ts(type = "Array<Array<[number, number, string]>>"))]
    pub totals: Vec<Vec<(i32, i32, String)>>,
    pub partners: Vec<LendLeasePartner>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(LendLeasePartnersReply);

type Pieces = Vec<(i32, i32, i64)>;

pub(crate) fn summed(pieces: &[(i32, i32, i64)], domain: &[(i32, i32)]) -> Vec<(i32, i32, i128)> {
    let mut steps: Vec<(i64, i128)> = Vec::with_capacity(2 * (pieces.len() + domain.len()));
    for &(a, b, value) in pieces {
        steps.push((i64::from(a), i128::from(value)));
        steps.push((i64::from(b) + 1, -i128::from(value)));
    }
    for &(a, b) in domain {
        steps.push((i64::from(a), 0));
        steps.push((i64::from(b) + 1, 0));
    }
    steps.sort_unstable_by_key(|&(hour, _)| hour);
    let mut out: Vec<(i32, i32, i128)> = Vec::new();
    let (mut sum, mut at, mut span) = (0i128, 0, 0);
    while at < steps.len() {
        let hour = steps[at].0;
        while at < steps.len() && steps[at].0 == hour {
            sum += steps[at].1;
            at += 1;
        }
        let Some(&(next, _)) = steps.get(at) else { break };
        while span < domain.len() && i64::from(domain[span].1) < hour {
            span += 1;
        }
        if span == domain.len() || i64::from(domain[span].0) > hour {
            continue;
        }
        let (a, b) = (hour as i32, (next - 1) as i32);
        match out.last_mut() {
            Some(held) if held.2 == sum && i64::from(held.1) + 1 == i64::from(a) => held.1 = b,
            _ => out.push((a, b, sum)),
        }
    }
    out
}

fn from_on(spans: &[(i32, i32)], from: i32) -> Vec<(i32, i32)> {
    spans.iter().filter(|&&(_, b)| b >= from).map(|&(a, b)| (a.max(from), b)).collect()
}

fn both(a: &[(i32, i32)], b: &[(i32, i32)]) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    for &(first, last) in a {
        for &(from, to) in b {
            let (x, y) = (first.max(from), last.min(to));
            if x <= y {
                out.push((x, y));
            }
        }
    }
    out.sort_unstable();
    out
}

fn texts(runs: Vec<(i32, i32, i128)>) -> Vec<(i32, i32, String)> {
    runs.into_iter().map(|(a, b, value)| (a, b, value.to_string())).collect()
}

type Gathered = BTreeMap<u32, ([Pieces; 2], i32)>;

impl Product for LendLeasePartners {
    const KEY: &'static str = "lend-lease-partners";
    const CONTENT_DEPENDENT: bool = false;
    type Params = LendLeasePartnersParams;
    type Reply = LendLeasePartnersReply;

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid branch")?;
        if let (Some(from), Some(to)) = (params.from, params.to)
            && from > to
        {
            return Err("Invalid lend-lease hours");
        }
        validate_country(&params.holder)
    }

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![LendLeasePartnersParams::decl(), LendLeasePartnersReply::decl(), LendLeaseMeasure::decl(), LendLeasePartner::decl(), LendLeaseTag::decl(), HourSpan::decl()]
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let from = params.from.unwrap_or(query.timeline.first).max(query.timeline.first);
        let to = params.to.unwrap_or(query.timeline.last).min(query.timeline.last);
        let recording = query.replay.recording();
        let names = Names::new(&query.holders);
        let mut columns = [0usize; 2];
        for (column, name) in columns.iter_mut().zip(params.measure.columns()) {
            *column = dashboard_declare::field(TableId::WarLendLeaseHistories, name)?;
        }
        let mut sorted = columns;
        sorted.sort_unstable();
        let at: [usize; 2] = columns.map(|column| sorted.iter().position(|&held| held == column).unwrap_or_default());
        let mut known: Vec<(i32, i32)> = Vec::new();
        let mut partners: Gathered = BTreeMap::new();
        let mut unlinked = 0usize;
        for stretch in query.timeline.within(from, to) {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let run = stretch.run;
            let read: Vec<(i32, i32)> = dashboard_query::cost::storage(|| lanes::statuses(recording, run, TableId::WarLendLeaseHistories, stretch.first, stretch.last))?
                .into_iter()
                .filter(|held| replay_recording::columnar_frames::available(held.status))
                .map(|held| (held.first, held.last))
                .collect();
            let links = dashboard_base::queries::links(source, run)?;
            for holder in query.holders.within(&params.holder, stretch.first, stretch.last) {
                let Holder::Found { slot, .. } = holder.holder else { continue };
                let held = both(&read, &[(holder.first.max(stretch.first), holder.last.min(stretch.last))]);
                if held.is_empty() {
                    continue;
                }
                known.extend(&held);
                let mut ids: BTreeMap<u32, Vec<(i32, i32)>> = BTreeMap::new();
                for (&raw, runs) in &links.slots {
                    for &(a, b, linked) in runs {
                        if linked == Some(i64::from(slot)) {
                            ids.entry(raw).or_default().push((a, b));
                        }
                    }
                }
                for (raw, spans) in ids {
                    let spans = both(&spans, &held);
                    let (Some(&(low, _)), Some(&(_, high))) = (spans.first(), spans.last()) else { continue };
                    let owner = u128::from(raw) << 32;
                    let history = dashboard_query::cost::storage(|| {
                        lanes::history(recording, run, TableId::WarLendLeaseHistories, &sorted, Keys::Range(owner, owner | u128::from(u32::MAX)), low, high)
                    })?;
                    for (key, steps) in &history.keys {
                        let target = *key as u32;
                        for (index, step) in steps.iter().enumerate() {
                            let Some(cells) = step.cells.as_deref() else { continue };
                            let last = steps.get(index + 1).map_or(history.to, |next| next.hour - 1);
                            for (a, b) in both(&[(step.hour, last)], &spans) {
                                for (partner, x, y) in links.within(target, a, b) {
                                    let Some(partner) = partner else {
                                        unlinked += 1;
                                        continue;
                                    };
                                    let entry = partners.entry(partner).or_insert_with(|| (Default::default(), i32::MAX));
                                    entry.1 = entry.1.min(x);
                                    for (measure, &column) in at.iter().enumerate() {
                                        if let Some(Cell::Number(value)) = cells.get(column) {
                                            entry.0[measure].push((x, y, *value));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        known.sort_unstable();
        let mut warnings = Vec::new();
        if unlinked > 0 {
            warnings.push(format!("{unlinked} stretches of lend-lease history name a partner with no identity; they are left out"));
        }
        let first_row = partners.values().map(|(_, first)| *first).min();
        let mut totals: [Pieces; 2] = Default::default();
        let mut out = Vec::with_capacity(partners.len());
        for (slot, (pieces, first)) in partners {
            let domain = from_on(&known, first);
            let values: Vec<Vec<(i32, i32, i128)>> = pieces.iter().map(|pieces| summed(pieces, &domain)).collect();
            for (measure, runs) in values.iter().enumerate() {
                totals[measure].extend(runs.iter().filter_map(|&(a, b, value)| i64::try_from(value).ok().map(|value| (a, b, value))));
            }
            let mut tags: Vec<LendLeaseTag> = Vec::new();
            for &(a, b) in &domain {
                for (tag, x, y) in names.slot_tags(slot, a, b) {
                    let tag = tag.map(str::to_owned);
                    match tags.last_mut() {
                        Some(held) if held.tag == tag && i64::from(held.last_hour) + 1 == i64::from(x) => held.last_hour = y,
                        _ => tags.push(LendLeaseTag { first_hour: x, last_hour: y, tag }),
                    }
                }
            }
            out.push(LendLeasePartner { slot, tags, values: values.into_iter().map(texts).collect() });
        }
        let domain = first_row.map(|first| from_on(&known, first)).unwrap_or_default();
        Ok(LendLeasePartnersReply {
            holder: query.holders.name(&params.holder, to),
            scale: SCALE,
            first_hour: from,
            last_hour: to,
            known: settle(known.into_iter().map(|(first_hour, last_hour)| HourSpan { first_hour, last_hour }).collect()),
            totals: totals.iter().map(|pieces| texts(summed(pieces, &domain))).collect(),
            partners: out,
            warnings,
        })
    }
}
