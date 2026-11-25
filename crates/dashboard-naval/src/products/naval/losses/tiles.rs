use super::{
    Counts, NavalAttribution, NavalKind, NavalLossesAtParams, NavalPeriod,
    month_start, part_runs, reply_at,
};
use dashboard_base::{
    Source,
    products::{Country, LoadContext, validate_country},
    span::HourSpan,
};
use dashboard_land::products::forces::ForceRun;
use dashboard_query::{
    Query,
    chart::{ChartBucket, ChartRun, merge},
    series::{Line, MissingSpan, Part},
    tile::{self, BINS, SeriesTileReply, TileLine, TileRanking},
};
use dashboard_text::metadata::check_cancel;
use dashboard_world::products::group_members::GroupsAt;
use replay_data::rows::NavalLossTotalRow as Row;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const SERIES: &str = "naval-losses-lines";
const DECIMALS: u8 = 0;
const SCALE: f64 = 1.0;
const MISSING_BASELINE: &str = "Naval losses are not known at this hour: their tables were not read whole after the counting began.";

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NavalLossesTileFamily {
    pub kind: String,
    pub count_kind: NavalKind,
    pub attribution: NavalAttribution,
    pub period: NavalPeriod,
}

pub fn tile_family_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<NavalLossesTileFamily>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TileAsk {
    #[expect(dead_code, reason = "names the branch this request is routed by")]
    branch_id: String,
    #[expect(dead_code, reason = "read as the naval losses tile family")]
    family: Value,
    bin_hours: i64,
    index: i64,
    #[serde(default)]
    countries: Vec<Country>,
    within: Option<Vec<(Country, Vec<HourSpan>)>>,
    top: Option<u32>,
    hour: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
struct DrawnPiece {
    first: i32,
    last: i32,
    value: f64,
    break_before: bool,
}

fn push_missing(out: &mut Vec<MissingSpan>, first_hour: i32, last_hour: i32, reason: &str) {
    if first_hour > last_hour {
        return;
    }
    if let Some(prev) = out.last_mut()
        && prev.reason == reason
        && prev.last_hour.saturating_add(1) >= first_hour
    {
        prev.last_hour = prev.last_hour.max(last_hour);
        return;
    }
    out.push(MissingSpan { first_hour, last_hour, reason: reason.to_owned() });
}

fn clip_pieces(pieces: Vec<DrawnPiece>, within: Option<&[HourSpan]>) -> Vec<DrawnPiece> {
    let Some(spans) = within else { return pieces };
    let mut out = Vec::new();
    for piece in pieces {
        for span in spans {
            let first = piece.first.max(span.first_hour);
            let last = piece.last.min(span.last_hour);
            if first <= last {
                out.push(DrawnPiece {
                    first,
                    last,
                    value: piece.value,
                    break_before: piece.break_before || first > piece.first,
                });
            }
        }
    }
    out
}

fn clip_missing(spans: &[MissingSpan], within: Option<&[HourSpan]>) -> Vec<MissingSpan> {
    let Some(within) = within else {
        return spans.to_vec();
    };
    let mut out = Vec::new();
    for span in spans {
        for bound in within {
            let first = span.first_hour.max(bound.first_hour);
            let last = span.last_hour.min(bound.last_hour);
            if first <= last {
                push_missing(&mut out, first, last, &span.reason);
            }
        }
    }
    out
}

fn bucketed(lines: &[DrawnPiece], start: i32, clip: i32, bin_hours: i64) -> Vec<Option<ChartBucket>> {
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; BINS];
    if clip < start {
        return buckets;
    }
    let start_bin = i64::from(start);
    let end_bin = start_bin + BINS as i64 * bin_hours - 1;
    for line in lines {
        let lo = i64::from(line.first).max(start_bin);
        let hi = i64::from(line.last).min(i64::from(clip)).min(end_bin);
        if lo > hi {
            continue;
        }
        let mut at = lo;
        while at <= hi {
            let slot = ((at - start_bin) / bin_hours) as usize;
            let bin_end = (start_bin + (slot as i64 + 1) * bin_hours - 1).min(hi);
            let from = at as i32;
            let to = bin_end as i32;
            let piece = ChartBucket {
                absolute_total: (line.value.max(0.0) as u128) * (bin_end - at + 1) as u128,
                first_hour: from,
                last_hour: to,
                first_value: line.value,
                last_value: line.value,
                last_value_hour: from,
                min_hour: from,
                min_value: line.value,
                max_hour: from,
                max_value: line.value,
                first_break: line.break_before && from == line.first,
                has_break: false,
            };
            buckets[slot] = match buckets[slot].take() {
                None => Some(piece),
                Some(held) => merge(Some(held), Some(piece)),
            };
            at = bin_end + 1;
        }
    }
    for (slot, bucket) in buckets.iter_mut().enumerate() {
        if let Some(bucket) = bucket {
            let (bin_first, bin_last) = (start_bin + slot as i64 * bin_hours, start_bin + (slot as i64 + 1) * bin_hours - 1);
            bucket.has_break |= i64::from(bucket.first_hour) > bin_first || i64::from(bucket.last_hour) < bin_last.min(i64::from(clip));
        }
    }
    buckets
}

fn runs(lines: &[DrawnPiece], first: i32, clip: i32) -> Vec<ChartRun> {
    lines
        .iter()
        .filter(|line| line.last >= first && line.first <= clip)
        .map(|line| ChartRun {
            first_hour: line.first.max(first),
            last_hour: line.last.min(clip),
            value: line.value,
            break_before: line.break_before && line.first >= first,
        })
        .collect()
}

pub fn tile(source: &Source, query: &Query<'_>, params: &Value, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<SeriesTileReply> {
    let family: NavalLossesTileFamily = dashboard_query::named::named(params.get("family").unwrap_or(&Value::Null))
        .map_err(|error| invalid(format!("Invalid naval losses tile family: {error}")))?;
    if family.kind != SERIES {
        return Err(invalid(format!("Unknown naval losses tile family {}", family.kind)));
    }
    let ask: TileAsk = dashboard_query::named::named(params)
        .map_err(|error| invalid(format!("Invalid naval losses tile request: {error}")))?;
    tile::level_of(ask.bin_hours)?;
    let (first, last) = tile::hours(ask.bin_hours, ask.index)?;
    let branch_id = query.timeline.id.to_string();

    let (keys, ranking) = match ask.top {
        None => {
            ask.countries.iter().try_for_each(validate_country).map_err(invalid)?;
            let mut seen = BTreeSet::new();
            let deduped = ask.countries.into_iter().filter(|country| seen.insert(country.clone())).collect();
            (deduped, None)
        }
        Some(n) => {
            if !ask.countries.is_empty() {
                return Err(invalid("A top of naval loss tiles names no country: the ranking names them"));
            }
            let n = u32::try_from(n).ok().filter(|n| *n >= 1).ok_or_else(|| invalid("A top names one country or more"))?;
            let hour = dashboard_query::series::hour_of(ask.hour, "hour")?;
            let at_params = NavalLossesAtParams {
                branch_id: branch_id.clone(),
                hour,
                countries: Vec::new(),
                top: Some(n),
                group: None,
                attribution: family.attribution,
                kind: family.count_kind,
                period: family.period,
            };
            let groups = GroupsAt::load(source, context, &branch_id, None)?;
            let at_reply = reply_at(query, &at_params, &groups, cancelled)?;
            let from = at_reply.first_hour.min(hour);
            let until = at_reply.last_hour.max(hour);
            if let Some(reason) = at_reply.missing {
                (Vec::new(), Some(TileRanking { hour, from, until, refused: Some(reason) }))
            } else {
                let ranked_keys: Vec<Country> = at_reply
                    .rows
                    .iter()
                    .map(|row| row.slot.map(Country::Slot).unwrap_or_else(|| row.country.clone()))
                    .collect();
                (ranked_keys, Some(TileRanking { hour, from, until, refused: None }))
            }
        }
    };

    let within_by_country: BTreeMap<Country, Vec<HourSpan>> = ask.within.unwrap_or_default().into_iter().collect();
    let scan_first = first.max(query.timeline.first);
    let clip = last.min(query.timeline.last);

    let mut drawn_lines: Vec<Vec<DrawnPiece>> = Vec::with_capacity(keys.len());
    let mut tile_lines: Vec<TileLine> = Vec::with_capacity(keys.len());

    if scan_first <= clip && !keys.is_empty() {
        let stretches: Vec<_> = query.timeline.within(scan_first, clip).collect();
        let series = dashboard_declare::series("naval-losses").ok_or_else(|| invalid("naval-losses is not declared"))?;
        let line = Line::new(query, series, series.measure("total").ok_or_else(|| invalid("naval-losses has no total"))?)?;
        let mut held: Vec<(Country, Vec<Part>)> = Vec::with_capacity(keys.len());
        for country in &keys {
            check_cancel(cancelled)?;
            held.push((country.clone(), line.parts(country, scan_first, clip)));
        }
        let kinds = family.count_kind.kinds();
        let attribution = family.attribution.code();
        let recording = query.replay.recording();
        let mut runs_by_index: Vec<Vec<ForceRun>> = vec![Vec::new(); held.len()];
        for stretch in &stretches {
            check_cancel(cancelled)?;
            let mut holder_keys: Vec<u128> = held
                .iter()
                .flat_map(|(_, parts)| parts.iter())
                .filter(|part| part.run == stretch.run)
                .flat_map(|part| {
                    let owner = dashboard_declare::Key::Holder.of(part.slot, part.id);
                    kinds.iter().map(move |&kind| Row::holder_key(owner, attribution, kind))
                })
                .collect();
            holder_keys.sort_unstable();
            holder_keys.dedup();
            let full_first = query.timeline.at(stretch.first).map_or(stretch.first, |h| h.first);
            let from = match family.period {
                NavalPeriod::Running => stretch.first,
                NavalPeriod::Month => (month_start(stretch.first) - 1).clamp(full_first, stretch.first),
            };
            let counts = Counts::read(recording, stretch.run, kinds, &holder_keys, from, stretch.last)?;
            for ((_, parts), out) in held.iter().zip(runs_by_index.iter_mut()) {
                check_cancel(cancelled)?;
                for part in parts.iter().filter(|part| part.run == stretch.run) {
                    let (lo, hi) = (stretch.first.max(part.first), stretch.last.min(part.last));
                    if lo <= hi {
                        part_runs(part, lo, hi, &counts, family.attribution, family.count_kind, family.period, out);
                    }
                }
            }
        }
        for ((country, _), force_runs) in held.into_iter().zip(runs_by_index) {
            let within = within_by_country.get(&country).map(Vec::as_slice);
            let mut pieces = Vec::new();
            let mut missing = Vec::new();
            for run in force_runs {
                match run.total {
                    Some(total) => pieces.push(DrawnPiece {
                        first: run.first_hour,
                        last: run.last_hour,
                        value: total as f64,
                        break_before: run.starts_segment,
                    }),
                    None => push_missing(&mut missing, run.first_hour, run.last_hour, MISSING_BASELINE),
                }
            }
            let mut pieces = clip_pieces(pieces, within);
            pieces.sort_by_key(|piece| piece.first);
            let missing = clip_missing(&missing, within);
            drawn_lines.push(pieces);
            tile_lines.push(TileLine { country, part: None, missing, parts: Vec::new(), marks: None });
        }
    } else {
        for country in keys {
            drawn_lines.push(Vec::new());
            tile_lines.push(TileLine { country, part: None, missing: Vec::new(), parts: Vec::new(), marks: None });
        }
    }

    let (bins, raw_values, hour_bytes) = match ask.bin_hours {
        1 => {
            let run_lines: Vec<Vec<ChartRun>> = drawn_lines.iter().map(|lines| runs(lines, first, clip)).collect();
            let (bytes, raw) = tile::encode_runs(&run_lines, first, SCALE);
            (bytes, raw, 0)
        }
        bin_hours => {
            let buckets: Vec<Option<ChartBucket>> = drawn_lines.iter().flat_map(|lines| bucketed(lines, first, clip, bin_hours)).collect();
            tile::encode(&buckets, first, bin_hours, SCALE)
        }
    };
    let (immutable, filled) = tile::closure(first, last, query.timeline.last);
    Ok(SeriesTileReply {
        branch_id,
        measure: "total".to_owned(),
        decimals: DECIMALS,
        bin_hours: ask.bin_hours,
        index: ask.index,
        first_hour: first,
        immutable,
        filled,
        raw_values,
        hour_bytes,
        lines: tile_lines,
        bins: tile::base64(&bins),
        ranking,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dashboard_base::test_fixtures::{CAMPAIGN, records, source};
    use replay::campaign::Replay;
    use replay_data::rows::{CountryLinkRow, CountryRow, NavalLossTotalRow, Recorded, TableId, TableRows};
    use replay_frame::frame::TableView;
    use std::sync::Arc;

    fn naval_source() -> Source {
        const FIRST: i32 = 1_000;
        const LAST: i32 = 1_040;
        let frames = (FIRST..=LAST)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let TableView::Complete(TableRows::Countries(countries)) = &*frame.tables[TableId::Countries.index()] else {
                    unreachable!("fixture countries")
                };
                let mut countries = countries.clone();
                countries.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"ENG".to_vec(), ..countries[0].clone() });
                *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(countries));
                *frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(vec![
                    CountryLinkRow { id: 11, canonical_slot: 7 },
                    CountryLinkRow { id: 9001, canonical_slot: 7 },
                    CountryLinkRow { id: 9002, canonical_slot: 8 },
                ]));
                let known = hour >= 1_010;
                let mut rows = vec![NavalLossTotalRow {
                    slot: NavalLossTotalRow::RUN,
                    country: 0,
                    attribution: 0,
                    kind: NavalLossTotalRow::SHIPS,
                    total: Recorded::Value(0),
                    unknown: Recorded::Value(0),
                    counted_from: Recorded::Value(1_010),
                    known: Recorded::Value(known),
                    lost_from: Recorded::Value(None),
                }];
                if hour >= 1_020 {
                    rows.push(NavalLossTotalRow {
                        slot: 7,
                        country: 9001,
                        attribution: NavalLossTotalRow::LOST,
                        kind: NavalLossTotalRow::SHIPS,
                        total: Recorded::Value(if hour >= 1_030 { 5 } else { 2 }),
                        unknown: Recorded::Value(0),
                        counted_from: Recorded::Value(1_010),
                        known: Recorded::Value(true),
                        lost_from: Recorded::Value(None),
                    });
                }
                *frame.tables[TableId::NavalLossTotals.index()] = TableView::Complete(TableRows::NavalLossTotals(rows));
                frame
            })
            .collect();
        let replay = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        Source::from_replay(
            dashboard_base::SourceKey { source_id: "naval-tiles".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(replay),
            &AtomicBool::new(false),
        )
        .unwrap()
    }

    #[test]
    fn naval_losses_tile_drops_zeros_in_top_and_marks_unknown_baseline_missing() {
        let src = naval_source();
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        let load = LoadContext { content: None };
        let cancel = AtomicBool::new(false);
        let family = serde_json::json!({
            "kind": SERIES,
            "countKind": "ships",
            "attribution": "lost",
            "period": "running",
        });

        let top_reply = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": family,
                "binHours": 1,
                "index": 1_000 / BINS as i64,
                "top": 5,
                "hour": 1_035,
            }),
            &load,
            &cancel,
        )
        .unwrap();
        assert_eq!(top_reply.lines.len(), 1);
        assert_eq!(top_reply.lines[0].country, Country::Slot(7));
        assert!(top_reply.ranking.as_ref().is_some_and(|rank| rank.refused.is_none() && rank.from <= 1_035 && rank.until >= 1_035));
        assert_eq!(top_reply.lines[0].missing.len(), 1);
        assert_eq!(top_reply.lines[0].missing[0].first_hour, 1_000);
        assert_eq!(top_reply.lines[0].missing[0].last_hour, 1_009);
        assert!(!top_reply.bins.is_empty());

        let clipped = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": family,
                "binHours": 1,
                "index": 1_000 / BINS as i64,
                "countries": [7],
                "within": [[7, [{ "firstHour": 1_005, "lastHour": 1_025 }]]],
            }),
            &load,
            &cancel,
        )
        .unwrap();
        assert_eq!(clipped.lines.len(), 1);
        assert_eq!(clipped.lines[0].missing[0].first_hour, 1_005);
        assert_eq!(clipped.lines[0].missing[0].last_hour, 1_009);
    }
}
