use crate::span::HourSpan;
use dashboard_query::{
    chart::{ChartBucket, ChartRun, merge},
    series::MissingSpan,
    tile::{self, BINS},
};

pub fn push_span(out: &mut Vec<MissingSpan>, first_hour: i32, last_hour: i32, reason: &str) {
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

pub fn clip_runs(pieces: Vec<ChartRun>, within: Option<&[HourSpan]>) -> Vec<ChartRun> {
    let Some(spans) = within else { return pieces };
    let mut out = Vec::new();
    for piece in pieces {
        for span in spans {
            let first = piece.first_hour.max(span.first_hour);
            let last = piece.last_hour.min(span.last_hour);
            if first <= last {
                out.push(ChartRun {
                    first_hour: first,
                    last_hour: last,
                    value: piece.value,
                    break_before: piece.break_before || first > piece.first_hour,
                });
            }
        }
    }
    out
}

pub fn clip_spans(spans: &[MissingSpan], within: Option<&[HourSpan]>) -> Vec<MissingSpan> {
    let Some(within) = within else {
        return spans.to_vec();
    };
    let mut out = Vec::new();
    for span in spans {
        for bound in within {
            let first = span.first_hour.max(bound.first_hour);
            let last = span.last_hour.min(bound.last_hour);
            if first <= last {
                push_span(&mut out, first, last, &span.reason);
            }
        }
    }
    out
}

fn bucketed(lines: &[ChartRun], start: i32, clip: i32, bin_hours: i64) -> Vec<Option<ChartBucket>> {
    let mut buckets: Vec<Option<ChartBucket>> = vec![None; BINS];
    if clip < start {
        return buckets;
    }
    let start_bin = i64::from(start);
    let end_bin = start_bin + BINS as i64 * bin_hours - 1;
    for line in lines {
        let lo = i64::from(line.first_hour).max(start_bin);
        let hi = i64::from(line.last_hour).min(i64::from(clip)).min(end_bin);
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
                first_break: line.break_before && from == line.first_hour,
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

pub fn encode(mut lines: Vec<Vec<ChartRun>>, first: i32, clip: i32, bin_hours: i64, scale: f64) -> (Vec<u8>, bool, u8) {
    if bin_hours == 1 {
        for runs in &mut lines {
            runs.retain_mut(|run| {
                run.break_before &= run.first_hour >= first;
                run.first_hour = run.first_hour.max(first);
                run.last_hour = run.last_hour.min(clip);
                run.first_hour <= run.last_hour
            });
        }
        let (bytes, raw) = tile::encode_runs(&lines, first, scale);
        return (bytes, raw, 0);
    }
    let buckets: Vec<_> = lines.iter().flat_map(|runs| bucketed(runs, first, clip, bin_hours)).collect();
    tile::encode(&buckets, first, bin_hours, scale)
}
