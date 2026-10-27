//! Daily occupation reserve occurrences, independent of occupation-law evaluation:
//! the foreign garrison manpower each country's occupation status holds in reserve,
//! by the country that sent it.
//!
//! Every country owns one occupation status, and the walk that reads each status's
//! reserve writes one OccupationSettings row for it. Those rows, from the same
//! supplying read, admit the holders: an admitted holder without occurrences holds
//! none. A read without that inventory is not an observation.
//!
//! Names: `TAG@slot|holder` (1 while admitted), `TAG@slot|<ordinal>|amount` and
//! `TAG@slot|<ordinal>|source`. Source values index `sources` (decimal id ->
//! `TAG@slot`). An ordinal is list order, not a transfer or a lifetime. Positive
//! amounts alone determine race order; negatives stay signed.

use dashboard_base::{
    prepared::prepass::Family,
    products::{Product, check_reply_budget, validate_tag},
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

pub struct OccupationReserve;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum OccupationReserveView {
    #[default]
    Race,
    Holder,
}

#[derive(Clone, Debug, Deserialize, Serialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OccupationReserveParams {
    pub branch_id: String,
    #[serde(default)]
    pub view: OccupationReserveView,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub first_hour: Option<i32>,
    #[serde(default)]
    pub last_hour: Option<i32>,
    #[serde(default)]
    pub top: Option<usize>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ReserveHolder {
    pub tag: String,
    pub slot: u32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OccupationReserveReply {
    pub window: KeyedWindow,
    pub holders: Vec<ReserveHolder>,
    pub sources: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

const UNADMITTED: &str = "Reserve holders unavailable: occupation statuses not recorded at the supplying read (reads)";
const UNJOINED: &str =
    "Reserve holder identities unavailable: countries not recorded at the supplying read (reads)";
const UNSPELLED: &str =
    "Country tag cannot be represented by the keyed name grammar (holder observations)";
const UNNAMED: &str = "Reserve holder identity unavailable (holder observations)";

/// A supplying read's identity join and admitted holders, not the later playback hour's.
#[derive(Default, Serialize, Deserialize)]
struct Bindings {
    countries: Vec<(u32, u32, Vec<u8>)>,
    links: Vec<(u32, u32)>,
    raw: BTreeMap<u32, String>,
    holders: Vec<String>,
    /// Admitted holders the read cannot name: [tag outside the grammar, identity unresolved].
    unnamed: [u64; 2],
}

impl Bindings {
    fn read(frame: &DecodedFrame) -> Result<Self, &'static str> {
        let admitted = frame
            .complete::<tables::OccupationSettings>()
            .map_err(|_| UNADMITTED)?;
        let countries = frame
            .complete::<tables::Countries>()
            .map_err(|_| UNJOINED)?;
        let links = frame
            .complete::<tables::CountryLinks>()
            .map_err(|_| UNJOINED)?;
        let joined = Countries::read(frame).ok_or(UNJOINED)?;
        let valid_slots: BTreeSet<_> = countries
            .iter()
            .filter(|row| key_tag(&row.tag).is_some())
            .map(|row| row.canonical_slot)
            .collect();
        let raw: BTreeMap<u32, String> = links
            .iter()
            .filter_map(|r| {
                let slot = joined.slot(r.id)?;
                valid_slots
                    .contains(&slot)
                    .then(|| joined.raw(r.id))
                    .flatten()
                    .map(|part| (r.id, part))
            })
            .collect();
        let mut holders = Vec::new();
        let mut unnamed = [0; 2];
        for row in admitted {
            match raw.get(&row.country) {
                Some(part) => holders.push(part.clone()),
                None if joined.raw(row.country).is_some() => unnamed[0] += 1,
                None => unnamed[1] += 1,
            }
        }
        Ok(Self {
            countries: countries
                .iter()
                .map(|r| (r.id, r.canonical_slot, r.tag.clone()))
                .collect(),
            links: links.iter().map(|r| (r.id, r.canonical_slot)).collect(),
            raw,
            holders,
            unnamed,
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

#[derive(Default, Serialize, Deserialize)]
pub struct OccupationReserveObserver {
    bindings: Option<Bindings>,
    sources: BTreeMap<String, u32>,
    holders: BTreeMap<String, i32>,
    refusals: BTreeMap<String, u64>,
}

pub static FAMILY: Family = dashboard_keyed::block::family::<OccupationReserveObserver>(
    "occupation-reserve",
    "occupation-reserve.extract",
    "occupation-reserve.sequential",
    "occupation-reserve.fallback",
);

impl KeyedObserver for OccupationReserveObserver {
    const NAME: &'static str = "occupation-reserve";
    const TABLES: &'static [TableId] = &[
        TableId::CountryOccupationReserveManpower,
        TableId::OccupationSettings,
        TableId::CountryLinks,
        TableId::Countries,
    ];
    const READ: TableId = TableId::CountryOccupationReserveManpower;
    fn family() -> &'static Family {
        &FAMILY
    }
    fn fields() -> io::Result<Vec<TableFields>> {
        Ok(vec![
            TableFields::all(Self::READ).when_read(),
            // The shared occupation walk records its read on this table alone.
            TableFields::keys(TableId::OccupationSettings).when_read(),
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
        let Ok(rows) = frame.complete::<tables::CountryOccupationReserveManpower>() else {
            return Ok(false);
        };
        if dashboard_text::metadata::slot_read(format, &frame.metrics, Self::READ, frame.hour) {
            self.bindings = match Bindings::read(frame) {
                Ok(bindings) => Some(bindings),
                Err(reason) => {
                    *self.refusals.entry(reason.into()).or_default() += 1;
                    None
                }
            };
        }
        let Some(bindings) = self.bindings.as_ref().filter(|b| b.same(frame)) else {
            return Ok(false);
        };
        for holder in &bindings.holders {
            values.push(format!("{holder}|holder"), 1);
            self.holders.entry(holder.clone()).or_insert(frame.hour);
        }
        for (reason, count) in [UNSPELLED, UNNAMED].into_iter().zip(bindings.unnamed) {
            if count > 0 {
                *self.refusals.entry(reason.into()).or_default() += count;
            }
        }
        for row in rows {
            let Some(holder) = bindings.raw.get(&row.country) else {
                *self
                    .refusals
                    .entry("Reserve holder identity unavailable (occurrence observations)".into())
                    .or_default() += 1;
                continue;
            };
            // names: holder|ordinal|amount and holder|ordinal|source; repeats stay separate.
            values.push(
                format!("{holder}|{}|amount", row.ordinal),
                i128::from(row.amount),
            );
            let Some(source) = bindings.raw.get(&row.source_country) else {
                *self.refusals.entry("Reserve source identity unavailable; amounts retained (occurrence observations)".into()).or_default() += 1;
                continue;
            };
            let next = u32::try_from(self.sources.len())
                .ok()
                .and_then(|id| id.checked_add(1))
                .ok_or_else(|| {
                    io::Error::other("Reserve source dictionary exceeds its id range")
                })?;
            let id = *self.sources.entry(source.clone()).or_insert(next);
            values.push(format!("{holder}|{}|source", row.ordinal), i128::from(id));
        }
        Ok(true)
    }
}

impl Product for OccupationReserve {
    const KEY: &'static str = "occupation-reserve-manpower";
    const CONTENT_DEPENDENT: bool = false;
    type Params = OccupationReserveParams;
    type Reply = OccupationReserveReply;
    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        params
            .branch_id
            .parse::<u64>()
            .map_err(|_| "Invalid branch")?;
        if params
            .first_hour
            .zip(params.last_hour)
            .is_some_and(|(a, b)| a > b)
        {
            return Err("Invalid reserve window");
        }
        if params.top == Some(0) {
            return Err("Invalid reserve top count");
        }
        for tag in params.tags.iter().flatten().chain(params.tag.iter()) {
            validate_tag(tag)?;
            if tag.contains('|') {
                return Err("Country selection contains a name separator");
            }
        }
        if matches!(params.view, OccupationReserveView::Holder) && params.tag.is_none() {
            return Err("Holder view requires a country");
        }
        Ok(())
    }
    fn types() -> Vec<String> {
        vec![
            OccupationReserveParams::decl(),
            OccupationReserveReply::decl(),
            OccupationReserveView::decl(),
            ReserveHolder::decl(),
            dashboard_keyed::query::KeyedSeries::decl(),
            KeyedWindow::decl(),
            dashboard_base::products::country_groups::HourSpan::decl(),
        ]
    }
}

fn selected(part: &str, selectors: &[String]) -> bool {
    selectors.iter().any(|selector| {
        selector == part || countries::split(part).is_some_and(|(tag, _)| tag == selector)
    })
}

/// Read only the selected holders' prepared runs. `tag` may be `TAG@slot` to
/// identify one holder; a bare tag retains distinct tag-reuse series.
pub fn query(query: &Query<'_>) -> io::Result<serde_json::Value> {
    dashboard_text::metadata::check_cancel(query.cancelled)?;
    let params: OccupationReserveParams = serde_json::from_value(query.params.clone())
        .map_err(|error: serde_json::Error| dashboard_core::invalid(error.to_string()))?;
    OccupationReserve::validate(&params).map_err(dashboard_core::invalid)?;
    let (first, last) = query.window()?;
    let reply = reply(
        query.open::<OccupationReserveObserver>()?,
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
    mut opened: Option<Opened<'_, OccupationReserveObserver>>,
    params: &OccupationReserveParams,
    first: i32,
    last: i32,
    through: Option<i32>,
    cancelled: &AtomicBool,
) -> io::Result<OccupationReserveReply> {
    let mut reply = OccupationReserveReply {
        window: KeyedWindow {
            first_hour: first,
            last_hour: last,
            ..Default::default()
        },
        holders: Vec::new(),
        sources: BTreeMap::new(),
        warnings: Vec::new(),
    };
    if first > last.min(through.unwrap_or(last)) {
        return Ok(reply);
    }
    let Some(opened) = opened.as_mut() else {
        return Ok(reply);
    };
    let selectors = if matches!(params.view, OccupationReserveView::Holder) {
        params.tag.iter().cloned().collect::<Vec<_>>()
    } else {
        params.tags.clone().unwrap_or_default()
    };
    let restricted = matches!(params.view, OccupationReserveView::Holder) || params.tags.is_some();
    let mut holders: Vec<String> = opened
        .scan
        .observer
        .holders
        .iter()
        .filter(|(part, start)| {
            **start <= last.min(through.unwrap_or(last))
                && (!restricted || selected(part, &selectors))
        })
        .map(|(part, _)| part.clone())
        .collect();
    if matches!(params.view, OccupationReserveView::Race)
        && let Some(top) = params.top
    {
        let end = last.min(through.unwrap_or(last));
        let candidates: BTreeSet<_> = holders.iter().cloned().collect();
        let keys: Vec<_> = opened
            .scan
            .keyed()
            .filter_map(|(name, key)| {
                let (part, tail) = name.split_once('|')?;
                let admission = tail == "holder";
                ((admission || tail.ends_with("|amount")) && candidates.contains(part))
                    .then_some((part.to_owned(), admission, key))
            })
            .collect();
        dashboard_text::metadata::check_cancel(cancelled)?;
        let mut requested = vec![dashboard_keyed::KNOWN, dashboard_keyed::READ];
        requested.extend(keys.iter().map(|(_, _, key)| *key));
        let values = opened.values_at(&requested, end)?;
        let mut totals = BTreeMap::<String, i128>::new();
        if values[0].is_some() && values[1].is_some() {
            // An admitted holder without entries ranks as a proven zero.
            for ((part, admission, _), value) in keys.iter().zip(&values[2..]) {
                if let Some(value) = value {
                    let total = totals.entry(part.clone()).or_default();
                    if !admission {
                        *total += (*value).max(0);
                    }
                }
            }
        }
        holders.retain(|part| totals.contains_key(part));
        holders.sort_by(|a, b| totals[b].cmp(&totals[a]).then_with(|| a.cmp(b)));
        holders.truncate(top);
    }
    let chosen: BTreeSet<_> = holders.iter().cloned().collect();
    reply.window = opened.window(first, last, through, |name| {
        name.split_once('|')
            .is_some_and(|(part, _)| chosen.contains(part))
    })?;
    // A carried baseline or a recording gap has no supplying-read certificate.
    retain_dated(&mut reply.window);
    let ids: BTreeSet<_> = reply
        .window
        .series
        .iter()
        .filter(|s| s.name.ends_with("|source"))
        .flat_map(|s| s.runs.iter().map(|r| r.2.as_str()))
        .collect();
    reply.sources = opened
        .scan
        .observer
        .sources
        .iter()
        .filter(|(_, id)| ids.contains(id.to_string().as_str()))
        .map(|(part, id)| (id.to_string(), part.clone()))
        .collect();
    reply.holders = holders
        .iter()
        .filter_map(|part| {
            countries::split(part).map(|(tag, slot)| ReserveHolder {
                tag: tag.to_owned(),
                slot,
            })
        })
        .collect();
    reply.warnings.extend(
        opened
            .scan
            .observer
            .refusals
            .iter()
            .map(|(reason, count)| format!("{reason}: {count}.")),
    );
    if reply.window.known.is_empty() {
        reply.warnings.push(
            "Dated complete reserve observations unavailable in the requested window (1).".into(),
        );
    }
    Ok(reply)
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

#[cfg(test)]
mod tests;
