use super::{
    Fold, MilitaryAirFamily, MilitaryAirMetric, MilitaryAirParams,
    at::{self, LINKS_NOT_RECORDED, LOSSES_NOT_RECORDED, MilitaryAirAtParams, UNATTRIBUTED},
    countries, openings, scan, selected,
};
use dashboard_base::{
    Source,
    products::{Country, LoadContext},
    span::HourSpan,
};
use dashboard_query::{
    Query,
    identity::Holder,
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
pub const SUBTOTAL_UNREAD: &str = "Some aircraft losses since the totals' start were not recorded: the line is the recorded subtotal";

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
        None if !ask.countries.is_empty() || ask.hour.is_none() => (ask.countries, None),
        top => {
            if !ask.countries.is_empty() {
                return Err(invalid("A top of air loss tiles names no country: the ranking names them"));
            }
            if top == Some(0) { return Err(invalid("A top names one country or more")); }
            let hour = dashboard_query::series::hour_of(ask.hour, "hour")?;
            let at_params = MilitaryAirAtParams {
                branch_id: branch_id.clone(),
                family: MilitaryAirFamily::AirLosses,
                regions: family.regions.clone(),
                causes: family.causes.clone(),
                from_hour: family.from_hour,
                hour,
                metric: family.rank_metric.unwrap_or(MilitaryAirMetric::Taken),
                top,
                countries: None,
                group: None,
            };
            let win = at::window(source, &at_params, query.timeline.id, load, cancelled)?;
            let at_reply = at::answer_held(&win, &at_params, None)?;
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
        let count_from = match family.from_hour {
            Some(from) => Some(from),
            None => super::first_observed(source, query, MilitaryAirFamily::AirLosses, (query.timeline.first, query.timeline.last))?,
        };
        let mut partial: Vec<(i32, i32, &'static str)> = Vec::new();
        if let Some(from) = count_from {
            let mut unread_from: Option<i32> = None;
            let mut note = |hour: i32| unread_from = Some(unread_from.map_or(hour, |at| at.min(hour)));
            if from < scan_first {
                let family = MilitaryAirFamily::AirLosses;
                if super::observed_since(source, query, family, scan_first, from)? > from || super::latest_in_part(source, query, family, (from, scan_first - 1))?.is_some() {
                    note(scan_first);
                }
            }
            for gap in missing_all.iter().filter(|gap| gap.last_hour >= from) {
                note(gap.first_hour.max(from));
            }
            for span in fold.incomplete.iter().filter(|span| span.last_hour >= from) {
                note(span.first_hour.max(from));
            }
            for pair in query.timeline.stretches.windows(2) {
                let (gap_first, gap_last) = (pair[0].last.saturating_add(1), pair[1].first.saturating_sub(1));
                if gap_first <= gap_last && gap_last >= from && gap_first <= clip {
                    note(gap_first.max(from));
                }
            }
            let mut unplaced: [i128; 2] = [0, 0];
            for ((identity, dealt, ..), total) in &opened {
                if identity.is_none() {
                    unplaced[usize::from(*dealt)] += *total;
                }
            }
            let mut changes: BTreeMap<i32, [i128; 2]> = BTreeMap::new();
            for (leaf, points) in &leaves {
                if leaf.country.is_some() {
                    continue;
                }
                let mut before = 0_i128;
                for &(_, read_hour, total) in points {
                    changes.entry(read_hour).or_default()[usize::from(leaf.dealt)] += total - before;
                    before = total;
                }
            }
            let end = unread_from.map_or(clip, |at| at.saturating_sub(1)).min(clip);
            let mut open: Option<i32> = (unplaced != [0, 0]).then_some(scan_first);
            for (&hour, delta) in changes.range(..=end) {
                unplaced = [unplaced[0] + delta[0], unplaced[1] + delta[1]];
                if hour < scan_first {
                    open = (unplaced != [0, 0]).then_some(scan_first);
                    continue;
                }
                match (open, unplaced != [0, 0]) {
                    (None, true) => open = Some(hour),
                    (Some(first), false) => {
                        partial.push((first, hour - 1, UNATTRIBUTED));
                        open = None;
                    }
                    _ => {}
                }
            }
            if let Some(first) = open
                && first <= end
            {
                partial.push((first, end, UNATTRIBUTED));
            }
            if let Some(at) = unread_from.filter(|&at| at <= clip) {
                partial.push((at, clip, SUBTOTAL_UNREAD));
            }
        }

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
            let before = scan_first - 1;
            let held_before = scan_first > query.timeline.first
                && family.from_hour.is_none_or(|from| from <= before)
                && within.is_none_or(|spans| spans.iter().any(|span| span.first_hour <= before && before <= span.last_hour))
                && matches!((query.holders.at(key, before), query.holders.at(key, scan_first)), (Some(held), Some(now)) if held.holder == now.holder && matches!(now.holder, Holder::Found { .. }))
                && super::observed_at(source, query, MilitaryAirFamily::AirLosses, before)?;
            for at in 0..pieces.len() {
                let joined = match at {
                    0 => pieces[0].first == scan_first && held_before,
                    _ => {
                        let (end, start) = (pieces[at - 1].last, pieces[at].first);
                        end.saturating_add(1) == start
                            && matches!((query.holders.at(key, end), query.holders.at(key, start)), (Some(held), Some(now)) if held.holder == now.holder && matches!(now.holder, Holder::Found { .. }))
                    }
                };
                if joined {
                    pieces[at].break_before = false;
                }
            }
            let missing = clip_missing(&missing_all, family.from_hour, within);
            let mut partial_spans = Vec::new();
            if !partial.is_empty() {
                for piece in &pieces {
                    for &(lo, hi, reason) in &partial {
                        let (a, b) = (piece.first.max(lo), piece.last.min(hi));
                        if a <= b {
                            push_missing(&mut partial_spans, a, b, reason);
                        }
                    }
                }
                partial_spans.sort_by_key(|gap| (gap.first_hour, gap.last_hour));
            }
            drawn_lines.push(pieces);
            tile_lines.push(TileLine { country: key.clone(), part: None, missing, partial: partial_spans, parts: Vec::new(), marks: None });
        }
    } else {
        for key in &keys {
            drawn_lines.push(Vec::new());
            tile_lines.push(TileLine { country: key.clone(), part: None, missing: Vec::new(), partial: Vec::new(), parts: Vec::new(), marks: None });
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
    fn a_loss_with_no_recorded_country_leaves_every_later_total_a_recorded_subtotal() {
        let frames: Vec<DecodedFrame> = (0..=40)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                set::<tables::CountryLinks>(&mut frame, vec![CountryLinkRow { id: 9001, canonical_slot: 7 }]);
                let events = match hour {
                    5 => vec![loss(1, 5, 9001, false, 400_000, None)],
                    15 => vec![loss(2, 15, 9050, false, 100_000, None)],
                    _ => Vec::new(),
                };
                set::<tables::AirLossEvents>(&mut frame, events);
                frame
            })
            .collect();
        let rep = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)]).unwrap().into_timelines(&Default::default()).unwrap();
        let cancelled = AtomicBool::new(false);
        let src = Source::from_replay(SourceKey { source_id: "air-unattributed".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(rep), &cancelled).unwrap();
        let q = query(&src, 1).unwrap();
        let params = serde_json::json!({ "branchId": "1", "family": { "kind": "air-losses", "side": "taken" }, "binHours": 1, "index": 0, "countries": [7] });
        let reply = tile(&src, &q, &params, &LoadContext { content: None }, &cancelled).unwrap();
        let bytes = unbase64(&reply.bins);
        let missing_at = |hour: i32| reply.lines[0].missing.iter().filter(|gap| gap.first_hour <= hour && hour <= gap.last_hour).map(|gap| gap.reason.as_str()).collect::<Vec<_>>();
        let partial_at = |hour: i32| reply.lines[0].partial.iter().filter(|gap| gap.first_hour <= hour && hour <= gap.last_hour).map(|gap| gap.reason.as_str()).collect::<Vec<_>>();
        assert_eq!((bytes[10] & tile::PRESENT, missing_at(10), partial_at(10)), (tile::PRESENT, vec![], vec![]), "before the unplaced loss the total is whole");
        assert_eq!((bytes[18] & tile::PRESENT, missing_at(18), partial_at(18)), (tile::PRESENT, vec![], vec![UNATTRIBUTED]), "after it the line keeps its recorded subtotal as partial rather than masking it as missing");
        let dealt = tile(&src, &q, &serde_json::json!({ "branchId": "1", "family": { "kind": "air-losses", "side": "dealt" }, "binHours": 1, "index": 0, "countries": [7] }), &LoadContext { content: None }, &cancelled).unwrap();
        assert!(dealt.lines[0].missing.iter().all(|gap| !(gap.first_hour <= 18 && 18 <= gap.last_hour)), "partial subtotals stay out of missing so the line still draws");
        assert!(dealt.lines[0].partial.iter().any(|gap| gap.first_hour <= 18 && 18 <= gap.last_hour && gap.reason == UNATTRIBUTED), "an unplaced loss on one side leaves both of a country's totals open, as the head says: {:?}", dealt.lines[0].partial);

        let gapped: Vec<DecodedFrame> = (0..=30)
            .filter(|hour| !(10..=19).contains(hour))
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                set::<tables::CountryLinks>(&mut frame, vec![CountryLinkRow { id: 9001, canonical_slot: 7 }]);
                set::<tables::AirLossEvents>(&mut frame, if hour == 5 { vec![loss(1, 5, 9001, false, 400_000, None)] } else { Vec::new() });
                frame
            })
            .collect();
        let rep = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, gapped)]).unwrap().into_timelines(&Default::default()).unwrap();
        let gap_src = Source::from_replay(SourceKey { source_id: "air-recording-gap".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(rep), &cancelled).unwrap();
        let gap_query = query(&gap_src, 1).unwrap();
        let after = tile(&gap_src, &gap_query, &params, &LoadContext { content: None }, &cancelled).unwrap();
        let gap_bytes = unbase64(&after.bins);
        assert_eq!(gap_bytes[15] & tile::PRESENT, 0, "the recording gap itself has no drawn value");
        assert_eq!(gap_bytes[20] & tile::PRESENT, tile::PRESENT, "hours after the gap keep their witnessed subtotal");
        assert!(after.lines[0].missing.iter().all(|gap| !(gap.first_hour <= 20 && 20 <= gap.last_hour)), "hours after the gap are not masked as missing");
        assert!(after.lines[0].partial.iter().any(|gap| gap.first_hour <= 20 && 20 <= gap.last_hour && gap.reason == SUBTOTAL_UNREAD), "hours unrecorded since the totals' start leave the later total a partial subtotal: {:?}", after.lines[0].partial);
    }

    #[test]
    fn a_line_held_across_a_tile_edge_continues_its_stroke_there() {
        let frames: Vec<DecodedFrame> = (0..=300)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                set::<tables::CountryLinks>(&mut frame, vec![CountryLinkRow { id: 9001, canonical_slot: 7 }]);
                let events = if hour == 5 { vec![loss(1, 5, 9001, true, 250_000, None)] } else { Vec::new() };
                set::<tables::AirLossEvents>(&mut frame, events);
                frame
            })
            .collect();
        let rep = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, frames)])
            .unwrap()
            .into_timelines(&Default::default())
            .unwrap();
        let cancelled = AtomicBool::new(false);
        let src = Source::from_replay(SourceKey { source_id: "air-tile-edge".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(rep), &cancelled).unwrap();
        let q = query(&src, 1).unwrap();
        let load = LoadContext { content: None };
        let params = serde_json::json!({ "branchId": "1", "family": { "kind": "air-losses", "side": "dealt" }, "binHours": 1, "index": 1, "countries": [7] });
        let reply = tile(&src, &q, &params, &load, &cancelled).unwrap();
        let bytes = unbase64(&reply.bins);
        assert_eq!(bytes[0] & tile::FIRST_BREAK, 0, "hour 256 continues the stroke held through 255: no break at the tile edge");

        let chosen_later = serde_json::json!({ "branchId": "1", "family": { "kind": "air-losses", "side": "dealt" }, "binHours": 1, "index": 1, "countries": [7], "within": [[7, [{ "firstHour": 256, "lastHour": 300 }]]] });
        let bytes = unbase64(&tile(&src, &q, &chosen_later, &load, &cancelled).unwrap().bins);
        assert_eq!(bytes[0] & (tile::PRESENT | tile::FIRST_BREAK), tile::PRESENT | tile::FIRST_BREAK, "a country chosen from 256 starts its stroke there");

        let unread = (0..=300)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                set::<tables::CountryLinks>(&mut frame, vec![CountryLinkRow { id: 9001, canonical_slot: 7 }]);
                if hour == 255 {
                    *frame.tables[TableId::AirLossEvents.index()] = TableView::Unavailable { reason: "losses unread".into() };
                } else {
                    set::<tables::AirLossEvents>(&mut frame, if hour == 5 { vec![loss(1, 5, 9001, true, 250_000, None)] } else { Vec::new() });
                }
                frame
            })
            .collect::<Vec<DecodedFrame>>();
        let rep = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, unread)]).unwrap().into_timelines(&Default::default()).unwrap();
        let gap = Source::from_replay(SourceKey { source_id: "air-tile-edge-unread".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(rep), &cancelled).unwrap();
        let gap_query = query(&gap, 1).unwrap();
        let bytes = unbase64(&tile(&gap, &gap_query, &params, &load, &cancelled).unwrap().bins);
        assert_eq!(bytes[0] & (tile::PRESENT | tile::FIRST_BREAK), tile::PRESENT | tile::FIRST_BREAK, "losses unread at 255 leave a break before 256");

        let passed = (0..=300)
            .map(|hour| {
                let mut frame = source(1, hour, Some(1));
                let TableView::Complete(TableRows::Countries(countries)) = &*frame.tables[TableId::Countries.index()] else { unreachable!("fixture countries") };
                let (first_tag, second_tag): (&[u8], &[u8]) = if hour >= 250 { (b"DEU", b"GER") } else { (b"GER", b"ITA") };
                let rows = vec![
                    CountryRow { id: 9001, canonical_slot: 7, tag: first_tag.to_vec(), ..countries[0].clone() },
                    CountryRow { id: 9002, canonical_slot: 8, tag: second_tag.to_vec(), ..countries[0].clone() },
                ];
                set::<tables::Countries>(&mut frame, rows);
                set::<tables::CountryLinks>(&mut frame, vec![CountryLinkRow { id: 9001, canonical_slot: 7 }, CountryLinkRow { id: 9002, canonical_slot: 8 }]);
                let events = match hour {
                    5 => vec![loss(1, 5, 9001, true, 250_000, None)],
                    6 => vec![loss(2, 6, 9002, true, 700_000, None)],
                    _ => Vec::new(),
                };
                set::<tables::AirLossEvents>(&mut frame, events);
                frame
            })
            .collect::<Vec<DecodedFrame>>();
        let rep = Replay::from_records(recorder_derive::writers(), CAMPAIGN.into(), &[records(1, passed)]).unwrap().into_timelines(&Default::default()).unwrap();
        let moved = Source::from_replay(SourceKey { source_id: "air-tag-passed".into(), revision: "1".into() }, "synthetic.battleplan".into(), Arc::new(rep), &cancelled).unwrap();
        let moved_query = query(&moved, 1).unwrap();
        let by_tag = serde_json::json!({ "branchId": "1", "family": { "kind": "air-losses", "side": "dealt" }, "binHours": 1, "index": 0, "countries": ["GER"] });
        let bytes = unbase64(&tile(&moved, &moved_query, &by_tag, &load, &cancelled).unwrap().bins);
        assert_eq!(bytes[250] & (tile::PRESENT | tile::FIRST_BREAK), tile::PRESENT | tile::FIRST_BREAK, "GER passing to another country at 250 breaks its stroke there");
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
            vec![MissingSpan { first_hour: 15, last_hour: 17, reason: LINKS_NOT_RECORDED.to_owned() }],
        );
        assert_eq!(
            top_reply.lines[0].partial,
            vec![MissingSpan { first_hour: 18, last_hour: 20, reason: SUBTOTAL_UNREAD.to_owned() }],
            "after hours whose losses could not be credited, the line is the recorded subtotal, never an exact total"
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
