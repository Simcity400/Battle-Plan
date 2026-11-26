use super::{PowerSpendingParams, PowerSpendingReply, SpendCurrency, breakdown::unrecorded, naming, spending};
use dashboard_base::Source;
use dashboard_base::products::{Country, LoadContext, Product, breakdown, check_reply_budget, validate_country};
use dashboard_query::Query;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, sync::atomic::AtomicBool};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PowerSpendMarkersParams {
    pub branch_id: String,
    pub country: Country,
    pub currency: SpendCurrency,
    pub hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PowerSpendMarker {
    #[doc = " The latest payment's own hour; the hour it was read when its own hour was not recorded."]
    pub hour: i32,
    #[doc = " False when `hour` is the read hour: the payment was made by then."]
    pub dated: bool,
    #[doc = " With more than one payment: the earliest hour the payments other than the latest can be from."]
    pub since: Option<i32>,
    pub read: i32,
    pub payments: u32,
    #[doc = " Payments whose charge was not recorded."]
    pub unknown: u32,
    #[doc = " What the payments whose charge was recorded charged, fixed to `decimals`; none where not recorded."]
    pub charge_raw: Option<String>,
    #[doc = " The item as the game names it, or the payment's cause, item and target."]
    pub title: String,
    pub category: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PowerSpendUnrecorded {
    pub first_hour: i32,
    #[doc = " The gap's own last hour, which can be after the asked hour: a chart shows it through the hour it displays."]
    pub last_hour: i32,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct PowerSpendMarkersReply {
    pub hour: i32,
    #[doc = " The first later hour at which anything in the reply changes."]
    pub next_change: Option<i32>,
    pub decimals: u8,
    #[doc = " Every payment read by `hour`, in the order of their hours."]
    pub markers: Vec<PowerSpendMarker>,
    #[doc = " Hours by `hour` whose spending was not recorded: no marker there says nothing was spent."]
    pub unrecorded: Vec<PowerSpendUnrecorded>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(PowerSpendMarkersReply);

#[derive(Clone, Debug, PartialEq)]
struct MarkersKey(PowerSpendingParams);

pub(super) struct SpendMarkers {
    decimals: u8,
    markers: Vec<PowerSpendMarker>,
    reads: Vec<i32>,
    unrecorded: Vec<(i32, i32, String)>,
}

impl SpendMarkers {
    pub(super) fn at(&self, hour: i32) -> PowerSpendMarkersReply {
        let markers: Vec<PowerSpendMarker> = self.markers.iter().filter(|marker| marker.read <= hour).cloned().collect();
        let mut next = self.reads.get(self.reads.partition_point(|&read| read <= hour)).copied();
        let mut later = |at: i32| next = Some(next.map_or(at, |held| held.min(at)));
        let mut unrecorded = Vec::new();
        for (first, last, reason) in &self.unrecorded {
            if *first > hour {
                later(*first);
                break;
            }
            if *last >= hour {
                later(last.saturating_add(1));
            }
            unrecorded.push(PowerSpendUnrecorded { first_hour: *first, last_hour: *last, reason: reason.clone() });
        }
        PowerSpendMarkersReply { hour, next_change: next, decimals: self.decimals, markers, unrecorded, warnings: Vec::new() }
    }
}

pub(super) fn index(reply: &PowerSpendingReply, unread: Vec<(i32, i32, String)>) -> SpendMarkers {
    let categories: HashMap<&str, &str> = reply.categories.iter().map(|category| (category.id.as_str(), category.name.as_str())).collect();
    let observed: Vec<(i32, i32)> = reply.coverage.iter().filter(|span| span.covered).map(|span| (span.first_hour, span.last_hour)).collect();
    let observed_before = |read: i32| {
        let at = observed.partition_point(|&(first, _)| first < read);
        at.checked_sub(1).map(|at| observed[at].1.min(read - 1))
    };
    let mut markers = Vec::with_capacity(reply.payments.len());
    let mut latest: HashMap<u32, (Option<i32>, Option<&str>, Option<u32>)> = HashMap::new();
    for payment in &reply.payments {
        let Some(item) = reply.items.get(payment.item as usize) else { continue };
        let earliest = observed_before(payment.read).unwrap_or(reply.from);
        let newest = (payment.hour, payment.sequence.as_deref(), payment.ordinal);
        let renewed = latest.insert(payment.item, newest) != Some(newest);
        let own = payment.hour.filter(|&hour| renewed && hour >= earliest);
        let since = (payment.payments > 1).then_some(earliest);
        markers.push(PowerSpendMarker {
            hour: own.unwrap_or(payment.read),
            dated: own.is_some(),
            since,
            read: payment.read,
            payments: payment.payments,
            unknown: payment.unknown,
            charge_raw: payment.known_raw.clone().filter(|_| payment.unknown < payment.payments),
            title: item.name.clone().unwrap_or_else(|| item.action.clone()),
            category: categories.get(item.category.as_str()).map_or_else(|| item.category.clone(), |name| (*name).to_owned()),
        });
    }
    markers.sort_by(|a, b| (a.hour, a.read).cmp(&(b.hour, b.read)).then_with(|| a.title.cmp(&b.title)));
    let mut reads: Vec<i32> = markers.iter().map(|marker| marker.read).collect();
    reads.sort_unstable();
    reads.dedup();
    SpendMarkers { decimals: reply.decimals, markers, reads, unrecorded: unread }
}

pub(super) fn build(source: &Source, query: &Query<'_>, span: &PowerSpendingParams, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<SpendMarkers> {
    let naming = naming::context(source, context)?;
    let whole = spending(source, query, span, &naming, cancelled)?;
    let unread = unrecorded(&query.timeline, &query.holders, &span.country, &whole.coverage, whole.from, whole.to);
    Ok(index(&whole, unread))
}

pub struct PowerSpendMarkers;

impl Product for PowerSpendMarkers {
    const KEY: &'static str = "power-spend-markers";
    const CONTENT_DEPENDENT: bool = true;
    type Params = PowerSpendMarkersParams;
    const FIELDS: dashboard_base::products::Fields = dashboard_base::fields![branchId, country, currency, hour: Read];
    type Reply = PowerSpendMarkersReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![PowerSpendMarkersParams::decl(), PowerSpendMarkersReply::decl(), PowerSpendMarker::decl(), PowerSpendUnrecorded::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params.branch_id.parse::<u64>().map_err(|_| "Invalid power spending branch")?;
        validate_country(&params.country)
    }

    fn load(source: &Source, params: &Self::Params, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let query = dashboard_base::queries::query(source, params.branch_id.parse().map_err(|_| dashboard_core::invalid("Invalid power spending branch"))?)?;
        let span = PowerSpendingParams {
            branch_id: params.branch_id.clone(),
            country: params.country.clone(),
            currency: params.currency,
            from: query.timeline.first,
            to: query.timeline.last,
        };
        let held = breakdown::held(source, context, MarkersKey(span.clone()), || build(source, &query, &span, context, cancelled))?;
        let reply = held.at(params.hour.min(query.timeline.last));
        check_reply_budget(&reply)?;
        Ok(reply)
    }
}
