
use super::Named;
use dashboard_base::{Source, products::{LoadContext, Product}};
use dashboard_query::{
    log::{self, Cell},
    series::{MissingSpan, PAGE_LIMIT},
};
use dashboard_text::metadata::link_slot;
use replay_data::rows::{TableId, TagHolderRow, ThreatEventRow, ThreatSourceRow, tables};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub struct ThreatEvents;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ThreatEventsParams {
    pub branch_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ThreatAdded {
    pub hour: i32,
    #[doc = " In contribution units (100,000 a percentage point of tension)."]
    pub added_raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ThreatEvent {
    #[doc = " The hour the game created it."]
    pub hour: i32,
    #[doc = " The country that caused it, or the tag that last held its slot; null when none is known."]
    pub tag: Option<String>,
    pub remembered: bool,
    pub reason: String,
    pub added: Vec<ThreatAdded>,
    pub first_read: i32,
    pub last_read: i32,
    #[doc = " It is still listed at the timeline's end."]
    pub open: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct ThreatEventsReply {
    pub branch_id: String,
    #[doc = " By creation hour, then first read."]
    pub events: Vec<ThreatEvent>,
    #[doc = " Hours of the timeline it does not hold, with why."]
    pub missing: Vec<MissingSpan>,
    pub warnings: Vec<String>,
}

dashboard_base::reply_warnings!(ThreatEventsReply);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct Identity {
    pub country: u32,
    pub second: u32,
    pub date: i32,
    pub reason: Vec<u8>,
    pub place: u32,
}

pub(super) enum Step {
    Stretch { first: i32, last: i32, listed: Vec<(Identity, i64, Option<u32>)> },
    Row { hour: i32, kind: u8, identity: Identity, final_raw: i64, slot: Option<u32> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Joined {
    pub identity: Identity,
    pub slot: Option<u32>,
    pub added: Vec<(i32, i64)>,
    pub first_read: i32,
    pub last_read: Option<i32>,
}

pub(super) fn join(steps: impl IntoIterator<Item = Step>) -> (Vec<Joined>, usize) {
    let mut events: Vec<Joined> = Vec::new();
    let mut open: HashMap<Identity, usize> = HashMap::new();
    let mut stretch_last = i32::MIN;
    let mut unmatched = 0;
    let added = |event: &mut Joined, hour: i32, value: i64| {
        if event.added.last().is_none_or(|&(_, last)| last != value) {
            event.added.push((hour, value));
        }
    };
    for step in steps {
        match step {
            Step::Stretch { first, last, listed } => {
                let now: std::collections::HashSet<&Identity> = listed.iter().map(|(identity, ..)| identity).collect();
                open.retain(|identity, at| {
                    let kept = now.contains(identity);
                    if !kept {
                        events[*at].last_read = Some(stretch_last);
                    }
                    kept
                });
                for (identity, value, slot) in listed {
                    match open.get(&identity) {
                        Some(&at) => added(&mut events[at], first, value),
                        None => {
                            open.insert(identity.clone(), events.len());
                            events.push(Joined { identity, slot, added: vec![(first, value)], first_read: first, last_read: None });
                        }
                    }
                }
                stretch_last = last;
            }
            Step::Row { hour, kind, identity, final_raw, slot } => match (kind, open.get(&identity).copied()) {
                (ThreatEventRow::DROPPED, Some(at)) => {
                    events[at].last_read = Some(hour - 1);
                    open.remove(&identity);
                }
                (ThreatEventRow::DROPPED, None) => unmatched += 1,
                (_, Some(at)) => added(&mut events[at], hour, final_raw),
                (kind, None) => {
                    if kind == ThreatEventRow::CHANGED {
                        unmatched += 1;
                    }
                    open.insert(identity.clone(), events.len());
                    events.push(Joined { identity, slot, added: vec![(hour, final_raw)], first_read: hour, last_read: None });
                }
            },
        }
    }
    (events, unmatched)
}

fn listed(sources: &[ThreatSourceRow], slot_of: impl Fn(u32) -> Option<u32>) -> Vec<(Identity, i64, Option<u32>)> {
    let mut out: Vec<(Identity, i64, Option<u32>)> = Vec::with_capacity(sources.len());
    for source in sources {
        let place = out
            .iter()
            .filter(|(held, ..)| held.country == source.country && held.second == source.second_country && held.date == source.date && held.reason == source.reason)
            .count() as u32;
        out.push((
            Identity { country: source.country, second: source.second_country, date: source.date, reason: source.reason.clone(), place },
            source.final_raw,
            slot_of(source.country),
        ));
    }
    out
}

fn number(cells: &[Cell], column: usize) -> Option<i64> {
    match cells.get(column)? {
        Cell::Number(value) => Some(*value),
        _ => None,
    }
}

fn bytes(cells: &[Cell], column: usize) -> Option<Vec<u8>> {
    match cells.get(column)? {
        Cell::Text(text) => Some(text.clone().into_bytes()),
        Cell::List(bytes) => Some(bytes.clone()),
        _ => None,
    }
}

fn row_step(row: &log::Row) -> Option<Step> {
    let slot = (row.key >> 64) as u32;
    let field = |name: &str| dashboard_declare::field(TableId::ThreatEvents, name).ok();
    let cells = &row.cells;
    Some(Step::Row {
        hour: row.hour,
        kind: (row.key >> 96) as u8,
        identity: Identity {
            country: u32::try_from(number(cells, field("country")?)?).ok()?,
            second: u32::try_from(number(cells, field("second_country")?)?).ok()?,
            date: i32::try_from(number(cells, field("date")?)?).ok()?,
            reason: bytes(cells, field("reason")?)?,
            place: u32::try_from(number(cells, field("place")?)?).ok()?,
        },
        final_raw: number(cells, field("final_raw")?)?,
        slot: (slot != TagHolderRow::NONE).then_some(slot),
    })
}

impl Product for ThreatEvents {
    const KEY: &'static str = "threat-events";
    const CONTENT_DEPENDENT: bool = false;
    type Params = ThreatEventsParams;
    type Reply = ThreatEventsReply;

    #[cfg(any(test, feature = "export-types"))]
    fn types() -> Vec<String> {
        vec![ThreatEventsParams::decl(), ThreatAdded::decl(), ThreatEvent::decl(), ThreatEventsReply::decl()]
    }

    fn validate(params: &Self::Params) -> Result<(), &'static str> {
        super::branch(&params.branch_id)
    }

    fn load(source: &Source, params: &Self::Params, _context: &LoadContext, cancelled: &AtomicBool) -> io::Result<Self::Reply> {
        let branch = params.branch_id.parse::<u64>().map_err(|_| dashboard_core::invalid("Invalid branch"))?;
        let query = dashboard_base::queries::query(source, branch)?;
        let timeline = &query.timeline;
        let mut warnings = Vec::new();
        let declared = dashboard_declare::log("threat-events").ok_or_else(|| dashboard_core::invalid("The threat events log is not declared"))?;
        let reader = log::Reader::new(&query, declared)?;
        let selection = reader.select(&[], timeline.first, timeline.last)?;
        let mut rows: Vec<log::Row> = Vec::new();
        let mut cursor = Some(0);
        while let Some(at) = cursor {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let page = reader.page(&selection.patterns, selection.groups.len(), timeline.first, timeline.last, at, PAGE_LIMIT)?;
            cursor = page.cursor();
            rows.extend(page.rows.into_iter().map(|paged| paged.row));
        }
        let recording = query.replay.recording();
        let mut steps: Vec<Step> = Vec::new();
        let mut rows = rows.into_iter().peekable();
        let mut unreadable = 0usize;
        for stretch in &timeline.stretches {
            dashboard_text::metadata::check_cancel(cancelled)?;
            let reads = [replay_lanes::at::Read { table: TableId::ThreatSources, columns: None, keys: replay_lanes::at::Keys::All }, replay_lanes::at::Read { table: TableId::CountryLinks, columns: None, keys: replay_lanes::at::Keys::All }];
            let frame = dashboard_query::cost::storage(|| replay_lanes::at::frame(recording, stretch.run, stretch.first, &reads))?;
            let listed = match frame.complete::<tables::ThreatSources>() {
                Ok(sources) => {
                    let links = frame.complete::<tables::CountryLinks>().ok();
                    listed(sources, |country| links.and_then(|links| link_slot(links, country)))
                }
                Err(error) => {
                    warnings.push(format!("Threat sources listed at hour {} are not known: {error}", stretch.first));
                    Vec::new()
                }
            };
            while let Some(row) = rows.next_if(|row| row.hour < stretch.first) {
                steps.extend(row_step(&row).or_else(|| {
                    unreadable += 1;
                    None
                }));
            }
            steps.push(Step::Stretch { first: stretch.first, last: stretch.last, listed });
            while let Some(row) = rows.next_if(|row| row.hour <= stretch.last) {
                steps.extend(row_step(&row).or_else(|| {
                    unreadable += 1;
                    None
                }));
            }
        }
        let last = timeline.stretches.last().map_or(timeline.last, |stretch| stretch.last);
        let (joined, unmatched) = join(steps);
        if unreadable > 0 {
            warnings.push(format!("{unreadable} threat event rows hold a cell that was not recorded and are not shown"));
        }
        if unmatched > 0 {
            warnings.push(format!("{unmatched} threat event rows change or drop a source no read listed"));
        }
        let named = Named::new(&query.holders, timeline.first);
        let mut events: Vec<ThreatEvent> = joined
            .into_iter()
            .map(|event| {
                let tag = event.slot.and_then(|slot| named.slot(slot, event.first_read));
                ThreatEvent {
                    hour: event.identity.date,
                    tag: tag.map(|(tag, _)| tag.to_owned()),
                    remembered: tag.is_some_and(|(_, remembered)| remembered),
                    reason: dashboard_text::catalogue::display_text(&event.identity.reason).unwrap_or_else(|why| {
                        let raw = String::from_utf8_lossy(&event.identity.reason).into_owned();
                        warnings.push(format!("Threat reason {raw:?} {why}"));
                        raw
                    }),
                    added: event.added.iter().map(|&(hour, value)| ThreatAdded { hour, added_raw: value.to_string() }).collect(),
                    first_read: event.first_read,
                    last_read: event.last_read.unwrap_or(last),
                    open: event.last_read.is_none(),
                }
            })
            .collect();
        events.sort_by_key(|event| (event.hour, event.first_read));
        let missing = timeline
            .unheld
            .iter()
            .map(|gap| MissingSpan { first_hour: gap.first, last_hour: gap.last, reason: gap.reason.clone() })
            .collect();
        let reply = ThreatEventsReply { branch_id: params.branch_id.clone(), events, missing, warnings };
        dashboard_base::products::check_reply_budget(&reply)?;
        Ok(reply)
    }
}
