use crate::products::subjects::{self, RunSubjects};
use dashboard_base::{
    Source,
    products::Country,
    queries::country_ids::RunLinks,
    span::HourSpan,
};
use dashboard_query::{
    Query,
    identity::Holder,
    series::{Ask, Line, Span, hour_of},
    tile::{self, SeriesTileReply, TileRanking},
    timeline::Stretch,
};
use dashboard_text::metadata::check_cancel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const SERIES: &str = "autonomy-lines";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutonomyLinesTileFamily {
    pub kind: String,
}

pub fn tile_family_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<AutonomyLinesTileFamily>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TileAsk {
    branch_id: String,
    family: Value,
    bin_hours: i64,
    index: i64,
    #[serde(default)]
    countries: Vec<Country>,
    within: Option<Vec<(Country, Vec<HourSpan>)>>,
    top: Option<u32>,
    hour: Option<f64>,
}

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn merge_spans(mut spans: Vec<Span>) -> Vec<Span> {
    spans.sort_unstable_by_key(|span| (span.first_hour, span.last_hour));
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for span in spans {
        if span.first_hour > span.last_hour {
            continue;
        }
        match out.last_mut() {
            Some(prev) if i64::from(prev.last_hour) + 1 >= i64::from(span.first_hour) => {
                prev.last_hour = prev.last_hour.max(span.last_hour);
            }
            _ => out.push(span),
        }
    }
    out
}

fn intersect_spans(left: &[Span], right: &[Span]) -> Vec<Span> {
    let mut out = Vec::new();
    for a in left {
        for b in right {
            let first_hour = a.first_hour.max(b.first_hour);
            let last_hour = a.last_hour.min(b.last_hour);
            if first_hour <= last_hour {
                out.push(Span { first_hour, last_hour });
            }
        }
    }
    merge_spans(out)
}

fn subject_spans_from_held(
    stretches: &[Stretch],
    held: &HashMap<u32, (Arc<RunSubjects>, Arc<RunLinks>)>,
) -> BTreeMap<u32, Vec<Span>> {
    let mut raw_spans: BTreeMap<u32, Vec<Span>> = BTreeMap::new();
    for stretch in stretches {
        let Some((subjects, links)) = held.get(&stretch.run) else { continue };
        for &(_overlord, subject, ref cells) in &subjects.relations {
            let start = cells.partition_point(|run| run.1 < stretch.first);
            for &(a, b, cell) in cells[start..].iter().take_while(|run| run.0 <= stretch.last) {
                let Some(cell) = cell else { continue };
                let (began, end) = subjects::split(cell);
                let first = a.max(stretch.first);
                let mut last = b.min(stretch.last);
                if end > began {
                    last = last.min(end.saturating_sub(1));
                }
                if first > last {
                    continue;
                }
                for (slot, sa, sb) in links.within(subject, first, last) {
                    if let Some(slot) = slot {
                        raw_spans.entry(slot).or_default().push(Span { first_hour: sa, last_hour: sb });
                    }
                }
            }
        }
    }
    raw_spans.into_iter().map(|(slot, spans)| (slot, merge_spans(spans))).collect()
}

fn active_subjects_and_window(
    subject_spans: &BTreeMap<u32, Vec<Span>>,
    is_named: impl Fn(u32) -> bool,
    timeline_first: i32,
    timeline_last: i32,
    unheld: &[(i32, i32)],
    hour: i32,
    limit: usize,
) -> (Vec<u32>, i32, i32) {
    let mut active = Vec::new();
    let mut from = timeline_first;
    let mut until = timeline_last;
    for &(gap_first, gap_last) in unheld {
        if gap_last < hour {
            from = from.max(gap_last.saturating_add(1));
        } else if gap_first > hour {
            until = until.min(gap_first.saturating_sub(1));
        }
    }
    for (&slot, spans) in subject_spans {
        if !is_named(slot) {
            continue;
        }
        let mut is_active = false;
        for span in spans {
            if span.first_hour <= hour && hour <= span.last_hour {
                is_active = true;
                from = from.max(span.first_hour);
                until = until.min(span.last_hour);
            } else if span.last_hour < hour {
                from = from.max(span.last_hour.saturating_add(1));
            } else if span.first_hour > hour {
                until = until.min(span.first_hour.saturating_sub(1));
            }
        }
        if is_active {
            active.push(slot);
        }
    }
    active.truncate(limit);
    (active, from.min(hour), until.max(hour))
}

fn clone_spans(spans: &[Span]) -> Vec<Span> {
    spans.iter().map(|span| Span { first_hour: span.first_hour, last_hour: span.last_hour }).collect()
}

fn spans_for_country(
    country: &Country,
    subject_spans: &BTreeMap<u32, Vec<Span>>,
    query: &Query<'_>,
) -> Vec<Span> {
    match country {
        Country::Slot(slot) => subject_spans.get(slot).map(|spans| clone_spans(spans)).unwrap_or_default(),
        Country::Tag(tag) => {
            let Some(runs) = query.holders.tags.get(tag) else {
                return Vec::new();
            };
            let mut out = Vec::new();
            for run in runs {
                if let Holder::Found { slot, .. } = run.holder
                    && let Some(spans) = subject_spans.get(&slot)
                {
                    let holder_span = [Span { first_hour: run.first, last_hour: run.last }];
                    out.extend(intersect_spans(spans, &holder_span));
                }
            }
            merge_spans(out)
        }
    }
}

pub fn tile(
    source: &Source,
    query: &Query<'_>,
    params: &Value,
    cancelled: &AtomicBool,
) -> io::Result<SeriesTileReply> {
    let ask: TileAsk = dashboard_query::named::named(params)
        .map_err(|error| invalid(format!("Invalid autonomy tile request: {error}")))?;
    let family: AutonomyLinesTileFamily = serde_json::from_value(ask.family)
        .map_err(|error| invalid(format!("Invalid autonomy tile family: {error}")))?;
    if family.kind != SERIES {
        return Err(invalid(format!("Unknown autonomy tile family {}", family.kind)));
    }

    let series = dashboard_declare::series("autonomy")
        .ok_or_else(|| invalid("Autonomy series is not declared"))?;
    let measure = series
        .measure("progress")
        .ok_or_else(|| invalid("Autonomy progress measure is not declared"))?;
    let line = Line::new(query, series, measure)?;

    let mut held = HashMap::new();
    for stretch in &query.timeline.stretches {
        check_cancel(cancelled)?;
        if let std::collections::hash_map::Entry::Vacant(slot) = held.entry(stretch.run) {
            slot.insert((
                subjects::cached_subjects(source, stretch.run)?,
                dashboard_base::queries::links(source, stretch.run)?,
            ));
        }
    }
    let subject_spans = subject_spans_from_held(&query.timeline.stretches, &held);

    let (asks, within, ranking): (Vec<Ask>, Vec<(Country, Vec<Span>)>, Option<TileRanking>) = if let Some(top) = ask.top {
        if !ask.countries.is_empty() {
            return Err(invalid("A top of autonomy tiles names no country: the ranking names them"));
        }
        let limit = usize::try_from(top)
            .ok()
            .filter(|n| *n >= 1)
            .ok_or_else(|| invalid("A top names one country or more"))?;
        let hour = hour_of(ask.hour, "hour")?;
        if hour < query.timeline.first || hour > query.timeline.last {
            (
                Vec::new(),
                Vec::new(),
                Some(TileRanking {
                    hour,
                    from: hour,
                    until: hour,
                    refused: Some("The requested hour is outside this branch.".to_owned()),
                }),
            )
        } else if let Some(gap) = query.timeline.unheld.iter().find(|gap| gap.first <= hour && hour <= gap.last) {
            (
                Vec::new(),
                Vec::new(),
                Some(TileRanking {
                    hour,
                    from: gap.first,
                    until: gap.last,
                    refused: Some(gap.reason.clone()),
                }),
            )
        } else {
            let unheld: Vec<(i32, i32)> = query.timeline.unheld.iter().map(|gap| (gap.first, gap.last)).collect();
            let (active, from, until) = active_subjects_and_window(
                &subject_spans,
                |slot| query.holders.named(&Country::Slot(slot)),
                query.timeline.first,
                query.timeline.last,
                &unheld,
                hour,
                limit,
            );
            let asks: Vec<Ask> = active.iter().map(|&slot| Ask::whole(&Country::Slot(slot))).collect();
            let within: Vec<(Country, Vec<Span>)> = active
                .iter()
                .map(|&slot| (Country::Slot(slot), subject_spans.get(&slot).map(|spans| clone_spans(spans)).unwrap_or_default()))
                .collect();
            (asks, within, Some(TileRanking { hour, from, until, refused: None }))
        }
    } else {
        let asks = line.asks(&ask.countries, &[])?;
        let within: Vec<(Country, Vec<Span>)> = asks
            .iter()
            .map(|entry| {
                let base = spans_for_country(&entry.country, &subject_spans, query);
                let clipped = match ask.within.as_deref() {
                    Some(extra) => {
                        let extra_spans: Vec<Span> = extra
                            .iter()
                            .filter(|(country, _)| *country == entry.country)
                            .flat_map(|(_, spans)| spans.iter().map(|span| Span { first_hour: span.first_hour, last_hour: span.last_hour }))
                            .collect();
                        if extra_spans.is_empty() { base } else { intersect_spans(&base, &extra_spans) }
                    }
                    None => base,
                };
                (entry.country.clone(), clipped)
            })
            .collect();
        (asks, within, None)
    };

    let mut reply = tile::tile(&line, &ask.branch_id, &asks, Some(&within), ask.bin_hours, ask.index, false)?;
    reply.ranking = ranking;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_query::timeline::Join;

    fn cell(start: i32, end: i32) -> Option<i64> {
        Some(i64::from(start) << 32 | i64::from(end as u32))
    }

    #[test]
    fn autonomy_lines_tiles_clip_to_subject_spans_and_report_exact_ranking_window() {
        let run0 = RunSubjects {
            relations: vec![
                (1, 2, vec![(100, 149, cell(50, 0)), (150, 199, cell(50, 180))]),
                (1, 3, vec![(120, 199, cell(120, 0))]),
            ],
            levels: HashMap::new(),
        };
        let links0 = RunLinks {
            slots: HashMap::from([
                (1, vec![(100, 199, Some(10))]),
                (2, vec![(100, 199, Some(20))]),
                (3, vec![(100, 199, Some(30))]),
            ]),
        };
        let held = HashMap::from([(0, (Arc::new(run0), Arc::new(links0)))]);
        let stretches = [Stretch { run: 0, first: 100, last: 199, join: Join::Start }];

        let spans = subject_spans_from_held(&stretches, &held);
        let pairs = |list: &[Span]| list.iter().map(|s| (s.first_hour, s.last_hour)).collect::<Vec<_>>();
        assert_eq!(pairs(&spans[&20]), vec![(100, 179)]);
        assert_eq!(pairs(&spans[&30]), vec![(120, 199)]);

        assert_eq!(
            active_subjects_and_window(&spans, |_| true, 100, 199, &[], 110, 1500),
            (vec![20], 100, 119),
        );
        assert_eq!(
            active_subjects_and_window(&spans, |_| true, 100, 199, &[], 150, 1500),
            (vec![20, 30], 120, 179),
        );
        assert_eq!(
            active_subjects_and_window(&spans, |_| true, 100, 199, &[], 185, 1500),
            (vec![30], 180, 199),
        );
        assert_eq!(
            active_subjects_and_window(&spans, |_| true, 100, 199, &[(160, 165)], 150, 1500),
            (vec![20, 30], 120, 159),
        );
    }
}
