use super::{
    Content, EquipmentProducedRaceParams, MAX_EXACT, ProducedCount, ProducedMeasure, ProducedUnknown,
    Typed, asked_parts, back_of, decimals, invalid, produced_measure, race_reply, run_types,
    unknown_of,
};
use dashboard_base::{
    Source,
    products::{Country, LoadContext, validate_country},
    span::HourSpan,
};
use dashboard_query::{
    Query,
    chart::{ChartBucket, ChartRun, merge},
    series::{Line, MissingSpan, totals::Gap},
    tile::{self, BINS, SeriesTileReply, TileLine, TileRanking},
};
use dashboard_text::metadata::check_cancel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    sync::atomic::AtomicBool,
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const SERIES: &str = "equipment-produced-lines";

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EquipmentProducedTileFamily {
    pub kind: String,
    pub count: ProducedCount,
    pub measure: ProducedMeasure,
    #[serde(default)]
    pub types: Vec<String>,
    pub refits: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from_hour: Option<i32>,
}

pub fn tile_family_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<EquipmentProducedTileFamily>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TileAsk {
    #[expect(dead_code, reason = "names the branch this request is routed by")]
    branch_id: String,
    #[expect(dead_code, reason = "read as the equipment produced tile family")]
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

fn unknown_reason(gap: Gap) -> &'static str {
    let reason = match gap {
        Gap::NotRecorded => ProducedUnknown::NotRecorded,
        Gap::Cell { reason, .. } => unknown_of(reason),
        Gap::WindowIncomplete => ProducedUnknown::WindowIncomplete,
        Gap::Restarted => ProducedUnknown::ObservationRestarted,
        Gap::OutOfRange => ProducedUnknown::OutOfRange,
    };
    match reason {
        ProducedUnknown::NotRecorded => "Production was not recorded for this country then.",
        ProducedUnknown::UnitsInexact => "A production line left a delivery out of its count.",
        ProducedUnknown::SpendingUnavailable => "The factory IC a line applied was not recorded.",
        ProducedUnknown::LostOutput => "A production line went without its final count.",
        ProducedUnknown::MissedPublication => "Production was not read at some hours.",
        ProducedUnknown::TypeNotRecorded => "Designs whose type was not recorded hold output that could be of this type.",
        ProducedUnknown::WindowIncomplete => "The window reaches before this country's recorded production.",
        ProducedUnknown::ObservationRestarted => "Production counting began again inside the window.",
        ProducedUnknown::RocketTableUnavailable => "Rocket site output was not recorded then.",
        ProducedUnknown::RocketInputInexact => "Rocket site output was not counted exactly.",
        ProducedUnknown::RocketOwnerUnreadable => "Rocket site output of an unknown country was recorded.",
        ProducedUnknown::OutOfRange => "The amount is too large to show exactly.",
    }
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

pub(super) fn range_baseline(line: &Line<'_, '_>, query: &Query<'_>, measure: ProducedMeasure, from_hour: Option<i32>) -> io::Result<Option<i32>> {
    if measure != ProducedMeasure::Total {
        return Ok(None);
    }
    let Some(start) = from_hour else {
        return Ok(None);
    };
    let Some(stretch) = query.timeline.at(start).copied() else {
        return Ok(None);
    };
    if start <= stretch.first {
        return Ok(None);
    }
    let clock = line.clock(stretch.first, start)?;
    Ok(super::measure::read_at(&clock.reads, start))
}

pub fn tile(source: &Source, query: &Query<'_>, params: &Value, context: &LoadContext, cancelled: &AtomicBool) -> io::Result<SeriesTileReply> {
    let family: EquipmentProducedTileFamily = dashboard_query::named::named(params.get("family").unwrap_or(&Value::Null))
        .map_err(|error| invalid(format!("Invalid equipment produced tile family: {error}")))?;
    if family.kind != SERIES {
        return Err(invalid(format!("Unknown equipment produced tile family {}", family.kind)));
    }
    let ask: TileAsk = dashboard_query::named::named(params)
        .map_err(|error| invalid(format!("Invalid equipment produced tile request: {error}")))?;
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
                return Err(invalid("A top of equipment produced tiles names no country: the ranking names them"));
            }
            let n = u32::try_from(n).ok().filter(|n| *n >= 1).ok_or_else(|| invalid("A top names one country or more"))?;
            let hour = dashboard_query::series::hour_of(ask.hour, "hour")?;
            let at_params = EquipmentProducedRaceParams {
                branch_id: branch_id.clone(),
                hour,
                countries: Vec::new(),
                group: None,
                top: Some(n),
                count: family.count,
                measure: family.measure,
                types: family.types.clone(),
                refits: family.refits,
                from_hour: family.from_hour,
            };
            let at_reply = race_reply(source, query, &at_params, context, cancelled)?;
            let from = at_reply.first_hour.min(hour);
            let until = at_reply.last_hour.max(hour);
            if let Some(reason) = at_reply.missing {
                (Vec::new(), Some(TileRanking { hour, from, until, refused: Some(reason) }))
            } else {
                let ranked_keys: Vec<Country> = at_reply.rows.into_iter().map(|row| row.country).collect();
                (ranked_keys, Some(TileRanking { hour, from, until, refused: None }))
            }
        }
    };

    let within_by_country: BTreeMap<Country, Vec<HourSpan>> = ask.within.unwrap_or_default().into_iter().collect();
    let scan_first = first.max(query.timeline.first);
    let clip = last.min(query.timeline.last);
    let dec = decimals(family.count);
    let scale = 10_f64.powi(i32::from(dec));

    let mut drawn_lines: Vec<Vec<DrawnPiece>> = Vec::with_capacity(keys.len());
    let mut tile_lines: Vec<TileLine> = Vec::with_capacity(keys.len());

    if scan_first <= clip && !keys.is_empty() {
        let series = dashboard_declare::series("produced-values").ok_or_else(|| invalid("Equipment produced's series is not declared"))?;
        let measure_decl = series.measure(produced_measure(family.count, family.refits)).ok_or_else(|| invalid("Equipment produced's measure is not declared"))?;
        let line = Line::new(query, series, measure_decl)?;
        let baseline = range_baseline(&line, query, family.measure, family.from_hour)?;
        let fixed_back = back_of(family.measure);
        let range_fn = |base: i32| move |read: i32| (read >= base).then_some(base);
        let range_closure = baseline.map(range_fn);
        let back: Option<&dyn Fn(i32) -> Option<i32>> = match (&range_closure, &fixed_back) {
            (Some(func), _) => Some(func as &dyn Fn(i32) -> Option<i32>),
            (None, Some(func)) => Some(func as &dyn Fn(i32) -> Option<i32>),
            (None, None) => None,
        };
        let met: BTreeSet<u32> = query.timeline.within(scan_first, clip).map(|stretch| stretch.run).collect();
        let mut warnings = Vec::new();
        let mut notes = BTreeSet::new();
        let types: BTreeMap<u32, BTreeMap<u32, Typed>> = match family.types.is_empty() {
            true => met.iter().map(|&run| (run, BTreeMap::new())).collect(),
            false => {
                let content = Content::read(source, context, &mut warnings);
                run_types(source, query, &content, &met, &mut notes)?
            }
        };
        let parts = asked_parts(&types, &family.types);
        let parts_of = |run: u32| parts.get(&run).cloned().unwrap_or_default();
        check_cancel(cancelled)?;
        let total_runs = line.totals(&keys, &parts_of, back, scan_first, clip)?;
        for (country, runs_for_country) in keys.into_iter().zip(total_runs) {
            let within = within_by_country.get(&country).map(Vec::as_slice);
            let mut pieces = Vec::new();
            let mut missing = Vec::new();
            for run in runs_for_country {
                match run.value {
                    Ok(raw) if i128::from(raw).abs() <= MAX_EXACT => pieces.push(DrawnPiece {
                        first: run.first,
                        last: run.last,
                        value: raw as f64 / scale,
                        break_before: run.opens,
                    }),
                    Ok(_) => push_missing(&mut missing, run.first, run.last, unknown_reason(Gap::OutOfRange)),
                    Err(gap) => push_missing(&mut missing, run.first, run.last, unknown_reason(gap)),
                }
            }
            let pieces = clip_pieces(pieces, within);
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
            let (bytes, raw) = tile::encode_runs(&run_lines, first, scale);
            (bytes, raw, 0)
        }
        bin_hours => {
            let buckets: Vec<Option<ChartBucket>> = drawn_lines.iter().flat_map(|lines| bucketed(lines, first, clip, bin_hours)).collect();
            tile::encode(&buckets, first, bin_hours, scale)
        }
    };
    let (immutable, filled) = tile::closure(first, last, query.timeline.last);
    Ok(SeriesTileReply {
        branch_id,
        measure: produced_measure(family.count, family.refits).to_owned(),
        decimals: dec,
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
    use crate::products::equipment_produced::tests::two_producers;

    #[test]
    fn equipment_produced_tiles_cover_total_day_and_range_from_hour() {
        const FIRST: i32 = 1_000;
        const LAST: i32 = 1_300;
        let src = two_producers("produced-tiles", FIRST, LAST);
        let query = dashboard_base::queries::query(&src, 1).unwrap();
        let load = LoadContext { content: None };
        let cancelled = AtomicBool::new(false);

        let top_reply = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "count": "units",
                    "measure": "total",
                    "types": [],
                    "refits": false
                },
                "binHours": 1,
                "index": i64::from(LAST).div_euclid(BINS as i64),
                "top": 5,
                "hour": LAST
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(top_reply.ranking.as_ref().and_then(|r| r.refused.as_deref()), None);
        assert_eq!(top_reply.lines.len(), 2);
        assert!(!top_reply.bins.is_empty());

        let range_start = FIRST + 120;
        let full_at = race_reply(
            &src,
            &query,
            &EquipmentProducedRaceParams {
                branch_id: "1".into(),
                hour: LAST,
                countries: vec![Country::Tag("GER".into())],
                group: None,
                top: None,
                count: ProducedCount::Units,
                measure: ProducedMeasure::Total,
                types: Vec::new(),
                refits: false,
                from_hour: None,
            },
            &load,
            &cancelled,
        )
        .unwrap();
        let range_at = race_reply(
            &src,
            &query,
            &EquipmentProducedRaceParams {
                branch_id: "1".into(),
                hour: LAST,
                countries: vec![Country::Tag("GER".into())],
                group: None,
                top: None,
                count: ProducedCount::Units,
                measure: ProducedMeasure::Total,
                types: Vec::new(),
                refits: false,
                from_hour: Some(range_start),
            },
            &load,
            &cancelled,
        )
        .unwrap();
        let full_ger = full_at.rows[0].value.unwrap();
        let range_ger = range_at.rows[0].value.unwrap();
        assert!(range_ger > 0 && range_ger < full_ger, "range total {range_ger} must exclude pre-range production from full total {full_ger}");

        let range_tile = tile(
            &src,
            &query,
            &serde_json::json!({
                "branchId": "1",
                "family": {
                    "kind": SERIES,
                    "count": "units",
                    "measure": "total",
                    "types": [],
                    "refits": false,
                    "fromHour": range_start
                },
                "binHours": 1,
                "index": i64::from(LAST).div_euclid(BINS as i64),
                "countries": [7]
            }),
            &load,
            &cancelled,
        )
        .unwrap();
        assert_eq!(range_tile.lines.len(), 1);
        assert!(!range_tile.bins.is_empty());
    }
}
