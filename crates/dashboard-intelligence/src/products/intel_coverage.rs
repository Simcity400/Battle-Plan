//! Independent daily national intel-network caches, not state-row sums.
//!
//! Names: `OWNER|TARGET|<measure>`, with both countries `TAG@slot` and measure
//! `coveredControlled`, `coveredCore`, `coveredWorth`, `controlledStrength`,
//! `coreStrength`, or `maxStrength`. A present zero is a real empty network.
//! Complete authenticated inventory, dated reads and admitted endpoints are
//! required for absence; a missing run is never numeric zero. Dated `rank|COUNTRY`
//! network counts and `member|COUNTRY` presence let playback select its own live
//! countries. Pair history keeps the same window shape and dated endpoints.

use dashboard_base::{
    prepared::prepass::Family,
    products::{Product, check_reply_budget, country_groups::HourSpan, validate_tag},
};
use dashboard_keyed::{
    Observer as KeyedObserver, Values,
    countries::{self, Countries},
    query::{KeyedWindow, Opened, Query},
};
use replay::container::TableFields;
use replay_data::{
    format::FormatVersion,
    rows::{TableId, tables},
};
use replay_frame::frame::DecodedFrame;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
use ts_rs::TS;

pub struct IntelCoverage;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum IntelCoverageView {
    #[default]
    Matrix,
    Pair,
}

fn default_shown() -> usize {
    12
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IntelCoverageParams {
    pub branch_id: String,
    #[serde(default)]
    pub view: IntelCoverageView,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default = "default_shown")]
    pub shown: usize,
    // Also every country with a network in or from one shown: those `tags` name, else the most active.
    #[serde(default)]
    pub touching: bool,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub first_hour: Option<i32>,
    #[serde(default)]
    pub last_hour: Option<i32>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct IntelCoverageReply {
    pub window: KeyedWindow,
    pub warnings: Vec<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Bindings {
    countries: Vec<(u32, u32, Vec<u8>)>,
    links: Vec<(u32, u32)>,
    raw: BTreeMap<u32, String>,
}
impl Bindings {
    fn read(frame: &DecodedFrame) -> Option<Self> {
        let countries = frame.complete::<tables::Countries>().ok()?;
        let links = frame.complete::<tables::CountryLinks>().ok()?;
        let joined = Countries::read(frame)?;
        let valid_slots: BTreeSet<_> = countries
            .iter()
            .filter(|row| key_tag(&row.tag).is_some())
            .map(|row| row.canonical_slot)
            .collect();
        Some(Self {
            countries: countries
                .iter()
                .map(|r| (r.id, r.canonical_slot, r.tag.clone()))
                .collect(),
            links: links.iter().map(|r| (r.id, r.canonical_slot)).collect(),
            raw: links
                .iter()
                .filter_map(|r| {
                    let slot = joined.slot(r.id)?;
                    valid_slots
                        .contains(&slot)
                        .then(|| joined.raw(r.id))
                        .flatten()
                        .map(|part| (r.id, part))
                })
                .collect(),
        })
    }
    fn same(&self, frame: &DecodedFrame) -> bool {
        frame.complete::<tables::Countries>().is_ok_and(|rows| {
            self.countries
                .iter()
                .map(|(id, slot, tag)| (*id, *slot, tag.as_slice()))
                .eq(rows
                    .iter()
                    .map(|r| (r.id, r.canonical_slot, r.tag.as_slice())))
        }) && frame.complete::<tables::CountryLinks>().is_ok_and(|rows| {
            self.links
                .iter()
                .copied()
                .eq(rows.iter().map(|r| (r.id, r.canonical_slot)))
        })
    }
}

fn key_tag(bytes: &[u8]) -> Option<&str> {
    std::str::from_utf8(bytes)
        .ok()
        .filter(|tag| !tag.is_empty() && !tag.contains(['|', '\0']))
}

/// Sparse membership and inventory certificates, not a world-pair cube.
#[derive(Default, Serialize, Deserialize)]
pub struct IntelCoverageObserver {
    bindings: Option<Bindings>,
    inventory: Vec<(i32, bool)>,
    refusals: BTreeMap<String, u64>,
}

pub static FAMILY: Family = dashboard_keyed::block::family::<IntelCoverageObserver>(
    "intel-network-coverage",
    "intel-network-coverage.extract",
    "intel-network-coverage.sequential",
    "intel-network-coverage.fallback",
);

fn transition(events: &mut Vec<(i32, bool)>, hour: i32, value: bool) {
    if events.last().is_none_or(|(_, held)| *held != value) {
        events.push((hour, value));
    }
}
fn at(events: &[(i32, bool)], hour: i32) -> bool {
    events[..events.partition_point(|(start, _)| *start <= hour)]
        .last()
        .is_some_and(|(_, value)| *value)
}

impl KeyedObserver for IntelCoverageObserver {
    const NAME: &'static str = "intel-network-coverage";
    const TABLES: &'static [TableId] = &[
        TableId::IntelNetworkSummaries,
        TableId::CountryLinks,
        TableId::Countries,
    ];
    const READ: TableId = TableId::IntelNetworkSummaries;
    fn family() -> &'static Family {
        &FAMILY
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::all(Self::READ).when_read(),
            TableFields::all(TableId::CountryLinks),
            TableFields::named(TableId::Countries, &["canonical_slot", "tag"])?,
        ])
    }
    fn changed(changes: &dashboard_base::prepared::frame_changes::FrameChanges) -> bool {
        changes.slot_read[Self::READ.index()]
            || Self::TABLES
                .iter()
                .any(|table| changes.rows_changed[table.index()])
    }
    fn observe(
        &mut self,
        format: FormatVersion,
        frame: &DecodedFrame,
        values: &mut Values,
    ) -> io::Result<bool> {
        let Ok(rows) = frame.complete::<tables::IntelNetworkSummaries>() else {
            return Ok(false);
        };
        if dashboard_text::metadata::slot_read(format, &frame.metrics, Self::READ, frame.hour) {
            self.bindings = Bindings::read(frame);
        }
        let Some(bindings) = self.bindings.as_ref().filter(|b| b.same(frame)) else {
            return Ok(false);
        };
        let mut live = BTreeSet::new();
        let mut certified = true;
        for (raw, slot, tag) in &bindings.countries {
            if *slot == 0 {
                continue;
            }
            let Some(tag) = key_tag(tag) else {
                certified = false;
                *self.refusals.entry("Country tag cannot be represented by the keyed name grammar (country observations)".into()).or_default() += 1;
                continue;
            };
            let part = countries::part(tag, *slot);
            if bindings.raw.get(raw) != Some(&part) {
                certified = false;
                continue;
            }
            live.insert(part);
        }
        for part in &live {
            values.push(format!("member|{part}"), 1);
        }
        let mut pairs = BTreeMap::new();
        let mut resolved = Vec::new();
        for row in rows {
            let pair = bindings
                .raw
                .get(&row.owner_country)
                .zip(bindings.raw.get(&row.target_country));
            let Some((owner, target)) = pair else {
                certified = false;
                *self
                    .refusals
                    .entry(
                        "Intel network endpoint identity unavailable (network observations)".into(),
                    )
                    .or_default() += 1;
                continue;
            };
            if owner == target || !live.contains(owner) || !live.contains(target) {
                certified = false;
                *self.refusals.entry("Intel network inventory has unauthenticated endpoints (network observations)".into()).or_default()+=1;
            }
            *pairs.entry((owner, target)).or_insert(0_u32) += 1;
            resolved.push((owner, target, row));
        }
        for (owner, target, row) in resolved {
            // Aliases of one canonical pair: which of their summaries is the pair's is unknown.
            if pairs[&(owner, target)] > 1 {
                certified = false;
                *self.refusals.entry("Intel network summaries repeat one canonical endpoint pair; its values are unknown (network observations)".into()).or_default() += 1;
                continue;
            }
            // names: OWNER|TARGET|measure; all six independent signed caches survive.
            for (measure, value) in [
                (
                    "coveredControlled",
                    i128::from(row.covered_controlled_states),
                ),
                ("coveredCore", i128::from(row.covered_core_states)),
                ("coveredWorth", i128::from(row.covered_worth)),
                (
                    "controlledStrength",
                    i128::from(row.controlled_strength_sum),
                ),
                ("coreStrength", i128::from(row.core_strength_sum)),
                ("maxStrength", i128::from(row.max_state_strength)),
            ] {
                values.push(format!("{owner}|{target}|{measure}"), value);
            }
        }
        transition(&mut self.inventory, frame.hour, certified);
        Ok(true)
    }
}

impl Product for IntelCoverage {
    const KEY: &'static str = "intel-network-coverage";
    const CONTENT_DEPENDENT: bool = false;
    type Params = IntelCoverageParams;
    type Reply = IntelCoverageReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        if params.shown == 0 {
            return Err("Invalid countries shown");
        }
        if params
            .first_hour
            .zip(params.last_hour)
            .is_some_and(|(a, b)| a > b)
        {
            return Err("Invalid intel coverage window");
        }
        for tag in params
            .tags
            .iter()
            .flatten()
            .chain(params.owner.iter())
            .chain(params.target.iter())
        {
            validate_tag(tag)?;
            if tag.contains('|') {
                return Err("Country selection contains a name separator");
            }
        }
        if matches!(params.view, IntelCoverageView::Pair)
            && (params.owner.is_none() || params.target.is_none())
        {
            return Err("Pair view requires owner and target");
        }
        if params.touching && matches!(params.view, IntelCoverageView::Pair) {
            return Err("Networks with any country belong to the matrix view");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            IntelCoverageParams::decl(),
            IntelCoverageReply::decl(),
            IntelCoverageView::decl(),
            dashboard_keyed::query::KeyedSeries::decl(),
            KeyedWindow::decl(),
            dashboard_base::products::country_groups::HourSpan::decl(),
        ]
    }
}

fn selected(part: &str, selector: &str) -> bool {
    part == selector || countries::split(part).is_some_and(|(tag, _)| tag == selector)
}

/// Every country with a dated network in or from a chosen one.
fn counterparties(
    evidence: &KeyedWindow,
    chosen: impl Fn(&str) -> bool,
    cancelled: &AtomicBool,
) -> io::Result<BTreeSet<String>> {
    let mut linked = BTreeSet::new();
    for series in &evidence.series {
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mut parts = series.name.split('|');
        let (Some(owner), Some(target), Some("coveredControlled")) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if chosen(owner) {
            linked.insert(target.to_owned());
        }
        if chosen(target) {
            linked.insert(owner.to_owned());
        }
    }
    Ok(linked)
}

pub fn query(query: &Query<'_>) -> io::Result<serde_json::Value> {
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    let params: IntelCoverageParams = serde_json::from_value(query.params.clone())
        .map_err(|error: serde_json::Error| dashboard_core::invalid(error.to_string()))?;
    IntelCoverage::validate(&params).map_err(dashboard_core::invalid)?;
    let (first, last) = query.window()?;
    let reply = reply(
        query.open::<IntelCoverageObserver>()?,
        &params,
        first,
        last,
        query.through(),
        query.cancelled,
    )?;
    check_reply_budget(&reply)?;
    serde_json::to_value(reply).map_err(io::Error::other)
}

fn reply(
    mut opened: Option<Opened<'_, IntelCoverageObserver>>,
    params: &IntelCoverageParams,
    first: i32,
    last: i32,
    through: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<IntelCoverageReply> {
    let mut reply = IntelCoverageReply {
        window: KeyedWindow {
            first_hour: first,
            last_hour: last,
            ..Default::default()
        },
        warnings: Vec::new(),
    };
    if first > last.min(through.unwrap_or(last)) {
        return Ok(reply);
    }
    let Some(opened) = opened.as_mut() else {
        return Ok(reply);
    };
    let end = last.min(through.unwrap_or(last));
    let pair = matches!(params.view, IntelCoverageView::Pair);
    let tagged = |part: &str| {
        params
            .tags
            .as_ref()
            .is_some_and(|tags| tags.iter().any(|tag| selected(part, tag)))
    };
    let mut evidence = opened.window(first, last, through, |name| {
        if let Some(part) = name.strip_prefix("member|") {
            if pair {
                selected(part, params.owner.as_deref().expect("validated owner"))
                    || selected(part, params.target.as_deref().expect("validated target"))
            } else {
                params.touching || params.tags.is_none() || tagged(part)
            }
        } else {
            !pair && name.ends_with("|coveredControlled")
        }
    })?;
    retain_dated(&mut evidence);
    let inventory = inventory_spans(&opened.scan.observer.inventory, first, end);
    let mut known = intersect_spans(&evidence.known, &inventory);
    let ranking = if pair {
        for selector in [params.owner.as_deref(), params.target.as_deref()] {
            let selector = selector.expect("validated endpoint");
            let mut spans: Vec<_> = evidence
                .series
                .iter()
                .filter_map(|series| {
                    let part = series.name.strip_prefix("member|")?;
                    selected(part, selector).then_some(&series.runs)
                })
                .flatten()
                .map(|run| HourSpan {
                    first_hour: run.0,
                    last_hour: run.1,
                })
                .collect();
            spans.sort_unstable_by_key(|span| (span.first_hour, span.last_hour));
            known = intersect_spans(&known, &spans);
        }
        Vec::new()
    } else if params.touching && params.tags.is_some() {
        Vec::new()
    } else {
        ranking::series(&evidence, &known, cancelled)?
    };
    let mut chosen = ranking::shown_anywhere(&ranking, params.shown, cancelled)?;
    if params.touching {
        let linked = counterparties(
            &evidence,
            |part: &str| chosen.contains(part) || tagged(part),
            cancelled,
        )?;
        chosen.extend(linked);
    }
    let included = |part: &str| chosen.contains(part) || tagged(part);
    reply.window = opened.window(first, last, through, |name| {
        let mut parts = name.split('|');
        let (Some(owner), Some(target), Some(_)) = (parts.next(), parts.next(), parts.next())
        else {
            return false;
        };
        if pair {
            selected(owner, params.owner.as_deref().expect("validated owner"))
                && selected(target, params.target.as_deref().expect("validated target"))
        } else {
            included(owner) && included(target)
        }
    })?;
    retain_dated(&mut reply.window);
    // Present values and membership stay exact independently of absence proof.
    reply.window.known = known;
    reply
        .window
        .series
        .extend(evidence.series.into_iter().filter(|series| {
            series
                .name
                .strip_prefix("member|")
                .is_some_and(|part| !params.touching || included(part))
        }));
    reply.window.series.extend(ranking);
    let state = &opened.scan.observer;
    reply.warnings.extend(
        state
            .refusals
            .iter()
            .map(|(reason, count)| format!("{reason}: {count}.")),
    );
    if reply.window.known.is_empty() {
        reply.warnings.push("Complete dated network inventory unavailable for these endpoints; present runs remain exact, missing pairs are unknown (1).".into());
    }
    Ok(reply)
}

fn inventory_spans(events: &[(i32, bool)], first: i32, last: i32) -> Vec<HourSpan> {
    let mut boundaries = vec![first];
    boundaries.extend(
        events
            .iter()
            .map(|(hour, _)| *hour)
            .filter(|hour| first < *hour && *hour <= last),
    );
    boundaries
        .iter()
        .enumerate()
        .filter_map(|(index, start)| {
            at(events, *start).then_some(HourSpan {
                first_hour: *start,
                last_hour: boundaries.get(index + 1).map_or(last, |next| next - 1),
            })
        })
        .collect()
}

fn intersect_spans(
    left: &[dashboard_base::products::country_groups::HourSpan],
    right: &[dashboard_base::products::country_groups::HourSpan],
) -> Vec<dashboard_base::products::country_groups::HourSpan> {
    let mut out: Vec<dashboard_base::products::country_groups::HourSpan> = Vec::new();
    let (mut a, mut b) = (0, 0);
    while a < left.len() && b < right.len() {
        let first = left[a].first_hour.max(right[b].first_hour);
        let last = left[a].last_hour.min(right[b].last_hour);
        if first <= last {
            if let Some(held) = out.last_mut().filter(|held| {
                held.last_hour
                    .checked_add(1)
                    .is_some_and(|next| next >= first)
            }) {
                held.last_hour = held.last_hour.max(last);
            } else {
                out.push(dashboard_base::products::country_groups::HourSpan {
                    first_hour: first,
                    last_hour: last,
                });
            }
        }
        if left[a].last_hour <= right[b].last_hour {
            a += 1;
        } else {
            b += 1;
        }
    }
    out
}

fn retain_dated(window: &mut KeyedWindow) {
    let reads: Vec<_> = window
        .reads
        .iter()
        .map(
            |(first, last, _)| dashboard_base::products::country_groups::HourSpan {
                first_hour: *first,
                last_hour: *last,
            },
        )
        .collect();
    window.known = intersect_spans(&window.known, &reads);
    for series in &mut window.series {
        let mut runs = Vec::new();
        for run in &series.runs {
            let start = window.known.partition_point(|span| span.last_hour < run.0);
            for span in window.known[start..]
                .iter()
                .take_while(|span| span.first_hour <= run.1)
            {
                let first = run.0.max(span.first_hour);
                let last = run.1.min(span.last_hour);
                runs.push((first, last, run.2.clone(), run.3 || first > run.0));
            }
        }
        series.runs = runs;
    }
    window.series.retain(|series| !series.runs.is_empty());
}

mod ranking;

#[cfg(test)]
mod tests;
