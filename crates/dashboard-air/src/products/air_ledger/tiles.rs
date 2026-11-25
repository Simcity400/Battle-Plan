use super::{
    Fold, MilitaryAirFamily, MilitaryAirMetric, MilitaryAirParams,
    at::{self, LINKS_NOT_RECORDED, LOSSES_NOT_RECORDED, MilitaryAirAtParams},
    countries, openings, scan, selected,
};
use dashboard_base::{
    Source,
    products::{Country, LoadContext},
    span::HourSpan,
};
use dashboard_query::{
    Query,
    chart::{ChartBucket, ChartRun, merge},
    series::MissingSpan,
    tile::{self, BINS, SeriesTileReply, TileLine, TileRanking},
};
use dashboard_text::metadata::check_cancel;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{Arc, PoisonError, atomic::AtomicBool},
};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const SERIES: &str = "air-losses";
const DECIMALS: u8 = 5;
const SCALE: f64 = 100_000.0;
const KEPT_FOLDS: usize = 8;

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, what.into())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum AirLossSide {
    Dealt,
    Taken,
}

impl AirLossSide {
    fn is_dealt(self) -> bool {
        matches!(self, Self::Dealt)
    }

    fn name(self) -> &'static str {
        match self {
            Self::Dealt => "dealt",
            Self::Taken => "taken",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AirLossesTileFamily {
    pub kind: String,
    pub side: AirLossSide,
    #[serde(default)]
    pub regions: Vec<u32>,
    #[serde(default)]
    pub causes: Vec<u32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub from_hour: Option<i32>,
    #[serde(default)]
    #[cfg_attr(any(test, feature = "export-types"), ts(optional))]
    pub rank_metric: Option<MilitaryAirMetric>,
}

pub fn tile_family_walk() -> dashboard_query::fields::Walk {
    dashboard_query::fields::walk::<AirLossesTileFamily>()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TileAsk {
    #[expect(dead_code, reason = "names the branch this request is routed by")]
    branch_id: String,
    #[expect(dead_code, reason = "read as the air losses tile family")]
    family: Value,
    bin_hours: i64,
    index: i64,
    #[serde(default)]
    countries: Vec<Country>,
    within: Option<Vec<(Country, Vec<HourSpan>)>>,
    top: Option<u32>,
    hour: Option<f64>,
}

type FoldKey = (u64, i32, i32);

#[derive(Default)]
struct TileFolds(VecDeque<(FoldKey, Arc<Fold>)>);

pub(super) fn forget(source: &Source) {
    source.state::<TileFolds>().lock().unwrap_or_else(PoisonError::into_inner).0.clear();
}

fn cached_scan(source: &Source, query: &Query<'_>, (first, last): (i32, i32), cancelled: &AtomicBool) -> io::Result<Arc<Fold>> {
    let key = (query.timeline.id, first, last);
    let state = source.state::<TileFolds>();
    if let Some((_, found)) = state.lock().unwrap_or_else(PoisonError::into_inner).0.iter().find(|(held, _)| *held == key) {
        return Ok(Arc::clone(found));
    }
    let made = Arc::new(scan(source, query, MilitaryAirFamily::AirLosses, (first, last), cancelled)?);
    let mut kept = state.lock().unwrap_or_else(PoisonError::into_inner);
    kept.0.retain(|(held, _)| *held != key);
    kept.0.push_back((key, Arc::clone(&made)));
    while kept.0.len() > KEPT_FOLDS {
        kept.0.pop_front();
    }
    Ok(made)
}

#[derive(Clone, Copy, Debug)]
struct DrawnPiece {
    first: i32,
    last: i32,
    value: f64,
    absolute: u128,
    break_before: bool,
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
                    absolute: piece.absolute,
                    break_before: piece.break_before || first > piece.first,
                });
            }
        }
    }
    out
}

fn clip_missing(spans: &[MissingSpan], from_hour: Option<i32>, within: Option<&[HourSpan]>) -> Vec<MissingSpan> {
    let mut out: Vec<MissingSpan> = Vec::new();
    for span in spans {
        let first = from_hour.map_or(span.first_hour, |from| span.first_hour.max(from));
        let last = span.last_hour;
        if first > last {
            continue;
        }
        match within {
            None => push_missing(&mut out, first, last, &span.reason),
            Some(allowed) => {
                for range in allowed {
                    let lo = first.max(range.first_hour);
                    let hi = last.min(range.last_hour);
                    if lo <= hi {
                        push_missing(&mut out, lo, hi, &span.reason);
                    }
                }
            }
        }
    }
    out
}

fn push_missing(out: &mut Vec<MissingSpan>, first: i32, last: i32, reason: &str) {
    match out.last_mut() {
        Some(prev) if prev.reason == reason && prev.last_hour.saturating_add(1) >= first => {
            prev.last_hour = prev.last_hour.max(last);
        }
        _ => out.push(MissingSpan { first_hour: first, last_hour: last, reason: reason.to_owned() }),
    }
}

fn base_missing(query: &Query<'_>, fold: &Fold, first: i32, clip: i32) -> Vec<MissingSpan> {
    let mut out = Vec::new();
    for stretch in query.timeline.within(first, clip) {
        let lo = stretch.first.max(first);
        let hi = stretch.last.min(clip);
        let mut hour = lo;
        while hour <= hi {
            let in_observed = fold.observed.iter().find(|s| s.first_hour <= hour && hour <= s.last_hour);
            if let Some(span) = in_observed {
                hour = match span.last_hour.checked_add(1) {
                    Some(next) => next,
                    None => break,
                };
                continue;
            }
            let in_unjoined = fold.unjoined.iter().find(|s| s.first_hour <= hour && hour <= s.last_hour);
            if let Some(span) = in_unjoined {
                let end = span.last_hour.min(hi);
                push_missing(&mut out, hour, end, LINKS_NOT_RECORDED);
                hour = match end.checked_add(1) {
                    Some(next) => next,
                    None => break,
                };
                continue;
            }
            let next_obs = fold.observed.iter().find(|s| s.first_hour > hour).map(|s| s.first_hour - 1);
            let next_unj = fold.unjoined.iter().find(|s| s.first_hour > hour).map(|s| s.first_hour - 1);
            let end = [Some(hi), next_obs, next_unj].into_iter().flatten().min().unwrap_or(hi);
            push_missing(&mut out, hour, end, LOSSES_NOT_RECORDED);
            hour = match end.checked_add(1) {
                Some(next) => next,
                None => break,
            };
        }
    }
    out
}

fn bucketed(lines: &[DrawnPiece], first: i32, clip: i32, bin_hours: i64) -> Vec<Option<ChartBucket>> {
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; BINS];
    let start_bin = i64::from(first);
    for line in lines {
        let (start, end) = (i64::from(line.first).max(start_bin), i64::from(line.last).min(i64::from(clip)));
        let mut at = start;
        while at <= end {
            let slot = ((at - start_bin) / bin_hours) as usize;
            if slot >= BINS {
                break;
            }
            let bin_end = (start_bin + (slot as i64 + 1) * bin_hours - 1).min(end);
            let (from, to) = (at as i32, bin_end as i32);
            let piece = ChartBucket {
                absolute_total: line.absolute * (bin_end - at + 1) as u128,
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

fn slot_of_identity(identity: &str, fallback_tag: &str) -> Country {
    identity
        .split(':')
        .next()
        .and_then(|part| part.parse::<u32>().ok())
        .map_or_else(|| Country::Tag(fallback_tag.to_owned()), Country::Slot)
}

pub fn tile(source: &Source, query: &Query<'_>, params: &Value, load: &LoadContext, cancelled: &AtomicBool) -> io::Result<SeriesTileReply> {
    let family: AirLossesTileFamily = dashboard_query::named::named(params.get("family").unwrap_or(&Value::Null))
        .map_err(|error| invalid(format!("Invalid air losses tile family: {error}")))?;
    if family.kind != SERIES {
        return Err(invalid(format!("Unknown air losses tile family {}", family.kind)));
    }
    let ask: TileAsk = dashboard_query::named::named(params)
        .map_err(|error| invalid(format!("Invalid air losses tile request: {error}")))?;
    tile::level_of(ask.bin_hours)?;
    let (first, last) = tile::hours(ask.bin_hours, ask.index)?;
    let branch_id = query.timeline.id.to_string();

    let (keys, ranking) = match ask.top {
        None => (ask.countries, None),
        Some(n) => {
            if !ask.countries.is_empty() {
                return Err(invalid("A top of air loss tiles names no country: the ranking names them"));
            }
            let n = u32::try_from(n).ok().filter(|n| *n >= 1).ok_or_else(|| invalid("A top names one country or more"))?;
            let hour = dashboard_query::series::hour_of(ask.hour, "hour")?;
            let at_params = MilitaryAirAtParams {
                branch_id: branch_id.clone(),
                family: MilitaryAirFamily::AirLosses,
                regions: family.regions.clone(),
                causes: family.causes.clone(),
                from_hour: family.from_hour,
                hour,
                metric: family.rank_metric.unwrap_or(MilitaryAirMetric::Taken),
                top: Some(n),
                countries: None,
                group: None,
            };
            let win = at::window(source, &at_params, query.timeline.id, load, cancelled)?;
            let at_reply = at::answer(&win, &at_params, None)?;
            let until = at_reply.valid_until.max(hour);
            if let Some(reason) = at_reply.missing.or(at_reply.rank_missing) {
                (Vec::new(), Some(TileRanking { hour, from: hour, until, refused: Some(reason) }))
            } else {
                let ranked_keys: Vec<Country> = at_reply
                    .rows
                    .iter()
                    .map(|row| slot_of_identity(&row.identity, &row.tag))
                    .collect();
                (ranked_keys, Some(TileRanking { hour, from: hour, until, refused: None }))
            }
        }
    };

    let scan_first = first.max(query.timeline.first);
    let clip = last.min(query.timeline.last);
    let is_dealt = family.side.is_dealt();

    let mut drawn_lines: Vec<Vec<DrawnPiece>> = Vec::with_capacity(keys.len());
    let mut tile_lines: Vec<TileLine> = Vec::with_capacity(keys.len());

    if scan_first <= clip && !keys.is_empty() {
        let fold = cached_scan(source, query, (scan_first, clip), cancelled)?;
        let ledger_params = MilitaryAirParams {
            branch_id: branch_id.clone(),
            family: MilitaryAirFamily::AirLosses,
            regions: family.regions.clone(),
            causes: family.causes.clone(),
            from_hour: family.from_hour,
            window: 0,
            top: None,
        };
        let opened = openings(source, query, &ledger_params, scan_first, true, cancelled)?;
        let leaves = selected(&fold, &ledger_params)?;
        let country_spans = countries(query, &fold.observed);
        let missing_all = base_missing(query, &fold, scan_first, clip);

        let mut opening_by_identity: BTreeMap<&str, i128> = BTreeMap::new();
        for ((identity, dealt, ..), total) in &opened {
            if *dealt == is_dealt {
                if let Some(identity) = identity.as_deref() {
                    *opening_by_identity.entry(identity).or_default() += *total;
                }
            }
        }

        let mut deltas_by_identity: BTreeMap<&str, BTreeMap<i32, i128>> = BTreeMap::new();
        for (leaf, points) in &leaves {
            if leaf.dealt != is_dealt {
                continue;
            }
            let Some((_, identity)) = leaf.country.as_ref() else { continue };
            let entry = deltas_by_identity.entry(identity.as_str()).or_default();
            let mut before = 0_i128;
            for &(_, read_hour, total) in points {
                let delta = total - before;
                before = total;
                *entry.entry(read_hour).or_default() += delta;
            }
        }

        for key in &keys {
            check_cancel(cancelled)?;
            let within = ask
                .within
                .as_deref()
                .and_then(|list| list.iter().find(|(country, _)| country == key).map(|(_, spans)| spans.as_slice()));
            let mut pieces: Vec<DrawnPiece> = Vec::new();
            for ((tag, identity), present) in &country_spans {
                if !at::asks(key, tag, identity) {
                    continue;
                }
                let base_total = opening_by_identity.get(identity.as_str()).copied().unwrap_or(0);
                let deltas = deltas_by_identity.get(identity.as_str());
                let mut prefix: Vec<(i32, i128)> = Vec::new();
                if let Some(deltas) = deltas {
                    let mut running = base_total;
                    for (&read_hour, &delta) in deltas {
                        running += delta;
                        prefix.push((read_hour, running));
                    }
                }
                let total_at = |hour: i32| -> i128 {
                    let idx = prefix.partition_point(|&(h, _)| h <= hour);
                    idx.checked_sub(1).map_or(base_total, |i| prefix[i].1)
                };
                for span in present {
                    let span_first = family.from_hour.map_or(span.first_hour, |from| span.first_hour.max(from));
                    let span_last = span.last_hour;
                    if span_first > span_last {
                        continue;
                    }
                    let mut change_hours = vec![span_first];
                    for &(h, _) in &prefix {
                        if h > span_first && h <= span_last {
                            change_hours.push(h);
                        }
                    }
                    for (idx, &start) in change_hours.iter().enumerate() {
                        let end = change_hours.get(idx + 1).map_or(span_last, |next| next - 1);
                        let raw = total_at(start);
                        let val = raw as f64 / SCALE;
                        let abs = raw.unsigned_abs();
                        match pieces.last_mut() {
                            Some(prev) if idx > 0 && prev.last.saturating_add(1) == start && prev.value.to_bits() == val.to_bits() && prev.absolute == abs => {
                                prev.last = end;
                            }
                            _ => pieces.push(DrawnPiece {
                                first: start,
                                last: end,
                                value: val,
                                absolute: abs,
                                break_before: idx == 0,
                            }),
                        }
                    }
                }
            }
            let mut pieces = clip_pieces(pieces, within);
            pieces.sort_by_key(|piece| piece.first);
            let missing = clip_missing(&missing_all, family.from_hour, within);
            drawn_lines.push(pieces);
            tile_lines.push(TileLine { country: key.clone(), part: None, missing, parts: Vec::new(), marks: None });
        }
    } else {
        for key in &keys {
            drawn_lines.push(Vec::new());
            tile_lines.push(TileLine { country: key.clone(), part: None, missing: Vec::new(), parts: Vec::new(), marks: None });
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
        measure: family.side.name().to_owned(),
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
    use dashboard_base::{
        SourceKey,
        queries::query,
        test_fixtures::{CAMPAIGN, records, source},
    };
    use replay::campaign::Replay;
    use replay_data::rows::{AirLossEventRow, CountryLinkRow, CountryRow, TableId, TableRows, TableSchema, tables};
    use replay_frame::frame::{DecodedFrame, TableView};

    fn unbase64(text: &str) -> Vec<u8> {
        const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let val = |byte: u8| ALPHABET.iter().position(|&c| c == byte).unwrap() as u32;
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
        for chunk in bytes.chunks_exact(4) {
            let a = val(chunk[0]);
            let b = val(chunk[1]);
            out.push(((a << 2) | (b >> 4)) as u8);
            if chunk[2] != b'=' {
                let c = val(chunk[2]);
                out.push((((b & 15) << 4) | (c >> 2)) as u8);
                if chunk[3] != b'=' {
                    let d = val(chunk[3]);
                    out.push((((c & 3) << 6) | d) as u8);
                }
            }
        }
        out
    }

    fn set<T: TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
        *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
        frame.metrics.table_ns[T::ID.index()] = 1;
    }

    fn loss(id: u64, game_hour: i32, country: u32, credited_enemy: bool, amount_raw: i64, opponent: Option<u32>) -> AirLossEventRow {
        AirLossEventRow { id, game_hour, country, region: 1, native_cause: 1, family_base: 0, credited_enemy, amount_raw, opponent, daily_slot: 0 }
    }

    #[test]
    fn air_losses_tile_buckets_cumulative_totals_drops_zeros_in_top_and_marks_unjoined_missing() {
        let frames: Vec<DecodedFrame> = (0..=20)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let TableView::Complete(TableRows::Countries(countries)) = &*frame.tables[TableId::Countries.index()] else { unreachable!("fixture countries") };
                let mut countries = countries.clone();
                countries.push(CountryRow { id: 9002, canonical_slot: 8, tag: b"SOV".to_vec(), ..countries[0].clone() });
                countries.push(CountryRow { id: 9003, canonical_slot: 9, tag: b"FRA".to_vec(), ..countries[0].clone() });
                set::<tables::Countries>(&mut frame, countries);
                if (15..=17).contains(&hour) {
                    *frame.tables[TableId::CountryLinks.index()] = TableView::Unavailable { reason: "not read".into() };
                } else {
                    set::<tables::CountryLinks>(&mut frame, vec![
                        CountryLinkRow { id: 9001, canonical_slot: 7 },
                        CountryLinkRow { id: 9002, canonical_slot: 8 },
                        CountryLinkRow { id: 9003, canonical_slot: 9 },
                    ]);
                }
                let mut events = Vec::new();
                if hour == 5 {
                    events.push(loss(1, 5, 9001, true, 250_000, Some(9002)));
                    events.push(loss(2, 5, 9002, false, 250_000, Some(9001)));
                }
                if hour == 12 {
                    events.push(loss(3, 12, 9001, true, 150_000, Some(9002)));
                }
                set::<tables::AirLossEvents>(&mut frame, events);
                frame
            })
            .collect();
        let rep = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let cancelled = AtomicBool::new(false);
        let src = Source::from_replay(
            SourceKey { source_id: "air-tiles".into(), revision: "1".into() },
            "synthetic.battleplan".into(),
            Arc::new(rep),
            &cancelled,
        )
        .unwrap();
        let q = query(&src, 1).unwrap();
        let load = LoadContext { content: None };

        let top_params = serde_json::json!({
            "branchId": "1",
            "family": { "kind": "air-losses", "side": "dealt", "rankMetric": "dealt" },
            "binHours": 1,
            "index": 0,
            "top": 5,
            "hour": 10
        });
        let top_reply = tile(&src, &q, &top_params, &load, &cancelled).unwrap();
        assert_eq!(top_reply.lines.len(), 1);
        assert_eq!(top_reply.lines[0].country, Country::Slot(7));
        let ranking = top_reply.ranking.as_ref().unwrap();
        assert_eq!(ranking.refused, None);
        assert_eq!(ranking.hour, 10);
        assert_eq!(ranking.until, 11);
        assert_eq!(
            top_reply.lines[0].missing,
            vec![MissingSpan { first_hour: 15, last_hour: 17, reason: LINKS_NOT_RECORDED.to_owned() }]
        );

        let decoded = tile::decode_runs(
            &unbase64(&top_reply.bins),
            top_reply.lines.len(),
            top_reply.first_hour,
            SCALE,
            top_reply.raw_values,
        )
        .unwrap();
        assert_eq!(decoded[0].len(), 4);
        assert_eq!((decoded[0][0].0, decoded[0][0].1, decoded[0][0].2), (0, 4, 0.0));
        assert_eq!((decoded[0][1].0, decoded[0][1].1, decoded[0][1].2), (5, 11, 2.5));
        assert_eq!((decoded[0][2].0, decoded[0][2].1, decoded[0][2].2), (12, 14, 4.0));
        assert_eq!((decoded[0][3].0, decoded[0][3].1, decoded[0][3].2), (18, 20, 4.0));
    }
}
