
use dashboard_query::{
    Index, Query,
    chart::{ChartBucket, ChartRun, MarkCluster, MarkSummary},
    series::{Ask, Line, MissingSpan, grid},
    tile,
};
use replay::campaign::Replay;
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const WIDTH: u32 = 600;
const ZOOMS: [(&str, i64); 6] = [("campaign", i64::MAX), ("4 years", 4 * 8_760), ("year", 8_760), ("quarter", 2_190), ("month", 730), ("week", 168)];
const SERIES: [(&str, &str); 3] = [("country-measure", "politicalPower"), ("country-stability", "maxCommandPower"), ("army-totals", "ownerStrength")];

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("tile-bench: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| args.iter().position(|arg| arg == name).and_then(|at| args.get(at + 1)).cloned();
    let recording = PathBuf::from(value("--recording").ok_or("--recording")?);
    let out = PathBuf::from(value("--out").ok_or("--out")?);
    let report = match args.first().map(String::as_str) {
        Some("equal") => equal(&recording, value("--views").map_or(Ok(20), |views| views.parse())?, value("--dump").map(PathBuf::from))?,
        Some("time") => time(&recording, value("--group").map_or(Ok(64), |group| group.parse())?, value("--threads").map(|threads| threads.parse()).transpose()?)?,
        Some("hourly") => hourly(&recording, value("--views").map_or(Ok(20), |views| views.parse())?, value("--dump").map(PathBuf::from))?,
        Some("marks") => marks(&recording, value("--views").map_or(Ok(20), |views| views.parse())?)?,
        Some("cells") => cells(&recording, value("--views").map_or(Ok(4), |views| views.parse())?, PathBuf::from(value("--dump").ok_or("--dump")?))?,
        Some("regroup") => regroup(&recording)?,
        _ => return Err("tile-bench equal|time --recording R --out FILE".into()),
    };
    std::fs::write(&out, serde_json::to_string_pretty(&report)?)?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn open(path: &Path) -> Result<Replay> {
    Ok(Replay::open_path(path)?.into_timelines(&Default::default())?)
}

fn query<'a>(replay: &'a Replay, index: &Index) -> Result<Query<'a>> {
    let id = replay
        .timelines()
        .and_then(|timelines| timelines.list.iter().max_by_key(|timeline| timeline.pieces.iter().map(|piece| i64::from(piece.last_hour) - i64::from(piece.first_hour)).sum::<i64>()))
        .ok_or("no timeline")?
        .id;
    Ok(index.query(replay, id)?)
}

fn line<'q, 'a>(query: &'q Query<'a>, kind: &str, measure: &str) -> Result<Line<'q, 'a>> {
    let series = dashboard_declare::series(kind).ok_or("no series")?;
    Ok(Line::new(query, series, series.measure(measure).ok_or("no measure")?)?)
}

fn tags(query: &Query<'_>) -> Vec<String> {
    let mut held: Vec<(i64, &String)> = query
        .holders
        .tags
        .iter()
        .map(|(tag, runs)| (runs.iter().map(|run| i64::from(run.last) - i64::from(run.first) + 1).sum(), tag))
        .collect();
    held.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(b.1)));
    held.into_iter().map(|(_, tag)| tag.clone()).collect()
}

fn unbase64(text: &str) -> Vec<u8> {
    let mut table = [0u8; 256];
    for (at, &byte) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/".iter().enumerate() {
        table[byte as usize] = at as u8;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    for chunk in text.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&byte| byte == b'=').count();
        let word = chunk.iter().take(4 - pad).enumerate().fold(0u32, |word, (at, &byte)| word | u32::from(table[byte as usize]) << (18 - 6 * at));
        out.extend_from_slice(&word.to_be_bytes()[1..4 - pad]);
    }
    out
}

fn clip(spans: impl IntoIterator<Item = MissingSpan>, from: i32, to: i32) -> Vec<MissingSpan> {
    let mut out: Vec<MissingSpan> = Vec::new();
    for span in spans {
        let (first, last) = (span.first_hour.max(from), span.last_hour.min(to));
        if first > last {
            continue;
        }
        match out.last_mut() {
            Some(held) if held.reason == span.reason && held.last_hour + 1 == first => held.last_hour = last,
            _ => out.push(MissingSpan { first_hour: first, last_hour: last, reason: span.reason }),
        }
    }
    out
}

fn equal(path: &Path, heads: usize, dump: Option<PathBuf>) -> Result<Value> {
    let replay = open(path)?;
    let index = Index::default();
    let query = query(&replay, &index)?;
    let (first, last) = (query.timeline.first, query.timeline.last);
    let all = tags(&query);
    let mut views = Vec::new();
    for at in 0..heads {
        let head = first + ((i64::from(last) - i64::from(first)) * (at as i64 + 1) / heads as i64) as i32;
        let (name, span) = ZOOMS[at % ZOOMS.len()];
        let from = if span == i64::MAX { first } else { (i64::from(head) - span + 1).max(i64::from(first)) as i32 };
        views.push((name, from, head));
    }
    let mut totals = json!({ "views": 0, "tiledViews": 0, "hourlyViews": 0, "bins": 0, "differing": 0, "missingDiffering": 0, "interiorDiffering": 0, "edgeDiffering": 0, "tiles": 0 });
    let mut rows = Vec::new();
    for (kind, measure) in SERIES {
        let line = line(&query, kind, measure)?;
        let held: Vec<String> = all.iter().filter(|tag| !line.parts(*tag, first, last).is_empty()).take(8).cloned().collect();
        for (view, &(name, from, to)) in views.iter().enumerate() {
            totals["views"] = json!(totals["views"].as_u64().unwrap() + 1);
            let hours = i64::from(to) - i64::from(from) + 1;
            if hours <= i64::from(WIDTH) {
                totals["hourlyViews"] = json!(totals["hourlyViews"].as_u64().unwrap() + 1);
                rows.push(json!({ "series": kind, "view": name, "from": from, "to": to, "hourly": true }));
                continue;
            }
            totals["tiledViews"] = json!(totals["tiledViews"].as_u64().unwrap() + 1);
            let (_, bin) = grid(hours, WIDTH);
            let span = tile::BINS as i64 * bin;
            let (wide_from, wide_to) = (i64::from(from).div_euclid(bin) * bin, (i64::from(to).div_euclid(bin) + 1) * bin - 1);
            let bins = ((wide_to - wide_from + 1) / bin) as usize;
            let mut assembled: Vec<Vec<Option<ChartBucket>>> = vec![Vec::with_capacity(bins); held.len()];
            let mut missing: Vec<Vec<MissingSpan>> = vec![Vec::new(); held.len()];
            let mut replies = Vec::new();
            for index in wide_from.div_euclid(span)..=wide_to.div_euclid(span) {
                let reply = tile::tile(&line, "bench", &asks(&held), None, bin, index, false)?;
                let buckets = match reply.hour_bytes {
                    0 => tile::decode_runs(&unbase64(&reply.bins), held.len(), reply.first_hour, line_scale(&line), reply.raw_values)?
                        .into_iter()
                        .flat_map(|runs| {
                            let runs: Vec<ChartRun> = runs.into_iter().map(|(first, last, value, flags)| ChartRun { first_hour: first, last_hour: last, value, break_before: flags & tile::FIRST_BREAK != 0 }).collect();
                            hourly_bins(&runs, reply.first_hour, reply.first_hour + tile::BINS as i32 - 1)
                        })
                        .collect(),
                    _ => tile::decode(&unbase64(&reply.bins), held.len(), reply.first_hour, bin, line_scale(&line), reply.raw_values, reply.hour_bytes)?,
                };
                for (at, tag_bins) in buckets.chunks(tile::BINS).enumerate() {
                    for (bin_at, bucket) in tag_bins.iter().enumerate() {
                        let start = i64::from(reply.first_hour) + bin_at as i64 * bin;
                        if (wide_from..=wide_to).contains(&start) {
                            assembled[at].push(*bucket);
                        }
                    }
                    missing[at].extend(reply.lines[at].missing.iter().cloned());
                }
                totals["tiles"] = json!(totals["tiles"].as_u64().unwrap() + 1);
                if dump.is_some() {
                    replies.push(serde_json::to_value(&reply)?);
                }
            }
            for buckets in &mut assembled {
                if let Some(Some(first)) = buckets.first_mut() {
                    first.first_break = true;
                }
            }
            let wide_hours = wide_to - wide_from + 1;
            let wide_width = if bin == 1 { bins as u32 - 1 } else { bins as u32 };
            assert_eq!(grid(wide_hours, wide_width).1, bin, "{name}: the widened view reads other bins");
            let widened = line.summaries_within(&held.iter().map(|tag| (Ask::whole(tag), vec![(wide_from as i32, wide_to as i32)])).collect::<Vec<_>>(), wide_from as i32, wide_to as i32, wide_width)?;
            let today = line.summaries(&held, from, to, WIDTH)?;
            let (mut differing, mut missing_differing, mut interior, mut edge) = (0, 0, 0, 0);
            for (at, tag) in held.iter().enumerate() {
                let wide: Vec<Option<ChartBucket>> = widened[at].0.buckets.iter().map(|bucket| bucket.map(|bucket| ChartBucket { absolute_total: 0, ..bucket })).collect();
                assert_eq!(wide.len(), assembled[at].len(), "{tag} {name}");
                differing += wide.iter().zip(&assembled[at]).filter(|(a, b)| a != b).count();
                let own: Vec<Option<ChartBucket>> = today[at].0.buckets.iter().map(|bucket| bucket.map(|bucket| ChartBucket { absolute_total: 0, ..bucket })).collect();
                for (slot, (a, b)) in own.iter().zip(&assembled[at]).enumerate() {
                    if a != b {
                        if slot == 0 || slot + 1 == own.len() { edge += 1 } else { interior += 1 }
                    }
                }
                if clip(missing[at].clone(), from, to) != today[at].1 {
                    missing_differing += 1;
                }
            }
            let count = assembled.iter().map(Vec::len).sum::<usize>();
            for (key, add) in [("bins", count), ("differing", differing), ("missingDiffering", missing_differing), ("interiorDiffering", interior), ("edgeDiffering", edge)] {
                totals[key] = json!(totals[key].as_u64().unwrap() + add as u64);
            }
            rows.push(json!({ "series": kind, "view": name, "from": from, "to": to, "binHours": bin, "bins": count, "differing": differing,
                "missingDiffering": missing_differing, "interiorDiffering": interior, "edgeDiffering": edge }));
            if let Some(dir) = &dump {
                std::fs::create_dir_all(dir)?;
                let reference: Vec<Value> = held
                    .iter()
                    .enumerate()
                    .map(|(at, tag)| json!({ "tag": tag, "summary": serde_json::to_value(&widened[at].0).unwrap(), "missing": today[at].1 }))
                    .collect();
                std::fs::write(
                    dir.join(format!("{kind}-{view:02}.json")),
                    serde_json::to_string(&json!({ "from": from, "to": to, "width": WIDTH, "binHours": bin, "tiles": replies, "reference": reference }))?,
                )?;
            }
        }
    }
    Ok(json!({ "recording": path, "timeline": [first, last], "totals": totals, "views": rows }))
}

fn line_scale(line: &Line<'_, '_>) -> f64 {
    10_f64.powi(i32::from(line.decimals()))
}

fn asks(tags: &[String]) -> Vec<Ask> {
    tags.iter().map(|tag| Ask::whole(tag)).collect()
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((sorted.len() - 1) as f64 * fraction).round() as usize]
}

type Runs = Vec<Vec<(i32, i32, f64, u8)>>;

fn view_runs(line: &Line<'_, '_>, tags: &[String], from: i32, to: i32) -> Result<(Vec<Vec<ChartRun>>, Vec<Vec<tile::TilePart>>, Vec<Vec<MissingSpan>>, Vec<Value>)> {
    let span = tile::BINS as i64;
    let mut joined: Vec<Vec<ChartRun>> = vec![Vec::new(); tags.len()];
    let mut parts: Vec<Vec<tile::TilePart>> = vec![Vec::new(); tags.len()];
    let mut missing: Vec<Vec<MissingSpan>> = vec![Vec::new(); tags.len()];
    let mut replies = Vec::new();
    for index in i64::from(from).div_euclid(span)..=i64::from(to).div_euclid(span) {
        let reply = tile::tile(line, "bench", &asks(tags), None, 1, index, false)?;
        assert_eq!(reply.hour_bytes, 0);
        let runs: Runs = tile::decode_runs(&unbase64(&reply.bins), tags.len(), reply.first_hour, line_scale(line), reply.raw_values)?;
        for (at, held) in runs.into_iter().enumerate() {
            for (first, last, value, flags) in held {
                match joined[at].last_mut() {
                    Some(run) if flags & tile::SEAM != 0 && run.last_hour + 1 == first => run.last_hour = last,
                    _ => joined[at].push(ChartRun { first_hour: first, last_hour: last, value, break_before: flags & tile::FIRST_BREAK != 0 }),
                }
            }
            parts[at].extend(reply.lines[at].parts.iter().cloned());
            missing[at].extend(reply.lines[at].missing.iter().cloned());
        }
        replies.push(serde_json::to_value(&reply)?);
    }
    let cut = joined
        .into_iter()
        .zip(&parts)
        .map(|(runs, parts)| {
            let segment = parts.iter().find(|part| part.first_hour <= from && from <= part.last_hour).map(|part| part.segment_first_hour);
            runs.into_iter()
                .filter(|run| run.last_hour >= from && run.first_hour <= to)
                .map(|run| match run.first_hour <= from {
                    true => ChartRun { first_hour: from, last_hour: run.last_hour.min(to), break_before: segment == Some(from), ..run },
                    false => ChartRun { last_hour: run.last_hour.min(to), ..run },
                })
                .collect()
        })
        .collect();
    Ok((cut, parts, missing, replies))
}

fn hourly_bins(runs: &[ChartRun], from: i32, to: i32) -> Vec<Option<ChartBucket>> {
    let mut out = vec![None; (to - from + 1) as usize];
    for run in runs {
        for hour in run.first_hour.max(from)..=run.last_hour.min(to) {
            out[(hour - from) as usize] = Some(ChartBucket {
                absolute_total: 0,
                first_hour: hour,
                last_hour: hour,
                first_value: run.value,
                last_value: run.value,
                last_value_hour: hour,
                min_hour: hour,
                min_value: run.value,
                max_hour: hour,
                max_value: run.value,
                first_break: hour == run.first_hour.max(from) && run.break_before,
                has_break: false,
            });
        }
    }
    out
}

fn hourly(path: &Path, heads: usize, dump: Option<PathBuf>) -> Result<Value> {
    let replay = open(path)?;
    let index = Index::default();
    let query = query(&replay, &index)?;
    let (first, last) = (query.timeline.first, query.timeline.last);
    let all = tags(&query);
    let spans = [168, 120, 72, 48, 24];
    let mut totals = json!({ "views": 0, "runs": 0, "bins": 0, "runsDiffering": 0, "binsDiffering": 0, "missingDiffering": 0, "tiles": 0 });
    let mut rows = Vec::new();
    for (kind, measure) in SERIES {
        let line = line(&query, kind, measure)?;
        let held: Vec<String> = all.iter().filter(|tag| !line.parts(*tag, first, last).is_empty()).take(8).cloned().collect();
        for at in 0..heads {
            let to = first + ((i64::from(last) - i64::from(first)) * (at as i64 + 1) / (heads as i64 + 1)) as i32 + 13 * at as i32;
            let to = to.min(last);
            let from = (to - spans[at % spans.len()] + 1).max(first);
            let today = line.summaries(&held, from, to, WIDTH)?;
            let (runs, _, missing, replies) = view_runs(&line, &held, from, to)?;
            let (mut runs_differing, mut bins_differing, mut missing_differing, mut run_count, mut bin_count) = (0, 0, 0, 0, 0);
            for (at, tag) in held.iter().enumerate() {
                let reply = &today[at].0;
                let wanted = reply.runs.clone().expect("an hourly reply carries its runs");
                run_count += wanted.len();
                if wanted != runs[at] {
                    runs_differing += 1;
                    if runs_differing <= 3 {
                        eprintln!("{kind} {tag} {from}..{to}: runs differ\n  today {wanted:?}\n  tiles {:?}", runs[at]);
                    }
                }
                let bins = hourly_bins(&runs[at], from, to);
                let own: Vec<Option<ChartBucket>> = reply.buckets.iter().map(|bucket| bucket.map(|bucket| ChartBucket { absolute_total: 0, ..bucket })).collect();
                bin_count += own.len();
                bins_differing += own.iter().zip(&bins).filter(|(a, b)| a != b).count() + own.len().abs_diff(bins.len());
                if clip(missing[at].clone(), from, to) != today[at].1 {
                    missing_differing += 1;
                }
            }
            for (key, add) in [("views", 1), ("runs", run_count), ("bins", bin_count), ("runsDiffering", runs_differing), ("binsDiffering", bins_differing), ("missingDiffering", missing_differing), ("tiles", replies.len())] {
                totals[key] = json!(totals[key].as_u64().unwrap() + add as u64);
            }
            rows.push(json!({ "series": kind, "from": from, "to": to, "runs": run_count, "runsDiffering": runs_differing, "binsDiffering": bins_differing, "missingDiffering": missing_differing }));
            if let Some(dir) = &dump {
                std::fs::create_dir_all(dir)?;
                let reference: Vec<Value> = held.iter().enumerate().map(|(at, tag)| json!({ "tag": tag, "summary": serde_json::to_value(&today[at].0).unwrap(), "missing": today[at].1 })).collect();
                std::fs::write(dir.join(format!("hourly-{kind}-{at:02}.json")), serde_json::to_string(&json!({ "from": from, "to": to, "width": WIDTH, "binHours": 1, "tiles": replies, "reference": reference }))?)?;
            }
        }
    }
    Ok(json!({ "recording": path, "timeline": [first, last], "totals": totals, "views": rows }))
}

fn tile_clusters(line: &Line<'_, '_>, tag: &String, bin: i64, from: i32, to: i32) -> Result<Vec<MarkCluster>> {
    let span = tile::BINS as i64 * bin;
    let mut out = Vec::new();
    for index in i64::from(from).div_euclid(span)..=i64::from(to).div_euclid(span) {
        let reply = tile::tile(line, "bench", &[Ask::whole(tag)], None, bin, index, true)?;
        let marks = reply.lines[0].marks.clone().ok_or("no marks")?;
        let mut ordinal: u64 = marks.before.parse()?;
        for (at, count) in marks.bins {
            let start = i64::from(reply.first_hour) + i64::from(at) * bin;
            let (first, last) = (start.max(i64::from(from)), (start + bin - 1).min(i64::from(to)));
            if first <= last {
                out.push(MarkCluster { first_hour: first as i32, last_hour: last as i32, first_ordinal: ordinal.to_string(), count: count.to_string() });
            }
            ordinal += count;
        }
    }
    Ok(out)
}

fn marks(path: &Path, heads: usize) -> Result<Value> {
    let replay = open(path)?;
    let index = Index::default();
    let query = query(&replay, &index)?;
    let (first, last) = (query.timeline.first, query.timeline.last);
    let line = line(&query, "country-measure", "politicalPower")?;
    let held: Vec<String> = tags(&query).into_iter().filter(|tag| !line.parts(tag, first, last).is_empty()).take(8).collect();
    let mut totals = json!({ "views": 0, "clusters": 0, "differing": 0 });
    let mut equal_rows = Vec::new();
    for (at, &bin) in [6i64, 96, 1_536, 6, 96].iter().cycle().take(heads).enumerate() {
        let head = first + ((i64::from(last) - i64::from(first)) * (at as i64 + 1) / (heads as i64 + 1)) as i32;
        let bins = 120i64;
        let to = ((i64::from(head).div_euclid(bin) + 1) * bin - 1).min(i64::from(last)) as i32;
        let from = ((i64::from(to) + 1 - bins * bin).max(i64::from(first))).div_euclid(bin) * bin;
        let from = from.max(i64::from(first).div_euclid(bin) * bin + bin) as i32;
        let hours = i64::from(to) - i64::from(from) + 1;
        if hours < 4 * bin || hours % bin != 0 {
            continue;
        }
        let width = u32::try_from(hours * 18 / bin).expect("a width");
        assert_eq!((hours * 18 + i64::from(width) - 1) / i64::from(width), bin);
        let today = line.marks_of(&held, from, to, width)?;
        let mut differing = 0;
        let mut clusters = 0;
        for (tag, summary) in held.iter().zip(&today) {
            let ours = tile_clusters(&line, tag, bin, from, to)?;
            clusters += summary.clusters.len();
            if ours != summary.clusters {
                differing += 1;
                eprintln!("marks {tag} {from}..{to} bins of {bin}:\n  today {:?}\n  tiles {ours:?}", summary.clusters);
            }
        }
        for (key, add) in [("views", 1), ("clusters", clusters), ("differing", differing)] {
            totals[key] = json!(totals[key].as_u64().unwrap() + add as u64);
        }
        equal_rows.push(json!({ "from": from, "to": to, "binHours": bin, "width": width, "clusters": clusters, "tagsDiffering": differing }));
    }
    let mut pictures = Vec::new();
    for (name, span) in [("campaign", i64::from(last) - i64::from(first) + 1), ("year", 8_760), ("quarter", 2_190)] {
        let to = first + ((i64::from(last) - i64::from(first)) * 2 / 3) as i32;
        let from = (i64::from(to) - span + 1).max(i64::from(first)) as i32;
        let (_, bin) = grid(i64::from(to) - i64::from(from) + 1, WIDTH);
        let today = line.marks_of(&held, from, to, WIDTH)?;
        let mut rows = Vec::new();
        for (tag, summary) in held.iter().zip(&today) {
            let ours = tile_clusters(&line, tag, bin, from, to)?;
            let count = |clusters: &[MarkCluster]| clusters.iter().map(|cluster| cluster.count.parse::<u64>().unwrap()).sum::<u64>();
            rows.push(json!({ "tag": tag, "todayClusters": summary.clusters.len(), "tileClusters": ours.len(),
                "todayDrops": count(&summary.clusters), "tileDrops": count(&ours),
                "todayFirst": summary.clusters.first().map(|cluster| [cluster.first_hour, cluster.last_hour]),
                "tileFirst": ours.first().map(|cluster| [cluster.first_hour, cluster.last_hour]),
                "sameFirstOrdinal": summary.clusters.first().map(|cluster| &cluster.first_ordinal) == ours.first().map(|cluster| &cluster.first_ordinal) }));
        }
        pictures.push(json!({ "view": name, "from": from, "to": to, "width": WIDTH, "binHours": bin, "tags": rows }));
    }
    Ok(json!({ "recording": path, "totals": totals, "equal": equal_rows, "pictures": pictures }))
}

fn droppers(line: &Line<'_, '_>, query: &Query<'_>, n: usize) -> Result<Vec<String>> {
    let (first, last) = (query.timeline.first, query.timeline.last);
    let all: Vec<String> = tags(query).into_iter().filter(|tag| !line.parts(tag, first, last).is_empty()).collect();
    let mut counted: Vec<(u64, String)> = line
        .marks_of(&all, first, last, 1)?
        .into_iter()
        .zip(all)
        .map(|(summary, tag)| (summary.clusters.iter().map(|cluster| cluster.count.parse::<u64>().unwrap_or(0)).sum(), tag))
        .collect();
    counted.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    Ok(counted.into_iter().take(n).map(|(_, tag)| tag).collect())
}

const CELLS: [(i64, i64, i64); 7] = [(1, 2, 40), (1, 3, 40), (1, 6, 40), (1, 12, 40), (1, 18, 40), (48, 18, 40), (96, 32, 30)];

fn cells(path: &Path, heads: usize, dump: PathBuf) -> Result<Value> {
    let replay = open(path)?;
    let index = Index::default();
    let query = query(&replay, &index)?;
    let (first, last) = (query.timeline.first, query.timeline.last);
    let line = line(&query, "country-measure", "politicalPower")?;
    let held = droppers(&line, &query, 8)?;
    std::fs::create_dir_all(&dump)?;
    let mut rows = Vec::new();
    for (b, k, n) in CELLS {
        let span = b * k;
        let (hours, width) = (n * span, (18 * n) as u32);
        if hours + 2 * span > i64::from(last) - i64::from(first) {
            continue;
        }
        let (_, bin) = grid(hours, width);
        assert_eq!(bin, b, "a view of {hours} hours at {width} pixels reads bins of {b}");
        for at in 0..heads {
            let head = i64::from(first) + hours + span + (i64::from(last) - i64::from(first) - hours - 2 * span) * at as i64 / heads.max(1) as i64;
            for shift in [0, span / 3] {
                let to = (head + 1).div_euclid(span) * span - 1 + shift;
                let from = to + 1 - hours;
                let today = line.marks_of(&held, from as i32, to as i32, width)?;
                let tile_span = tile::BINS as i64 * b;
                let mut tiles = Vec::new();
                for index in from.div_euclid(tile_span)..=to.div_euclid(tile_span) {
                    tiles.push(serde_json::to_value(tile::tile(&line, "bench", &asks(&held), None, b, index, true)?)?);
                }
                let name = format!("cells-{b}x{k}-{at:02}-{}", if shift == 0 { "grid" } else { "off" });
                std::fs::write(
                    dump.join(format!("{name}.json")),
                    serde_json::to_string(&json!({ "from": from, "to": to, "width": width, "binHours": b, "k": k, "onGrid": shift == 0, "tiles": tiles,
                        "today": held.iter().zip(&today).map(|(tag, summary)| json!({ "tag": tag, "clusters": summary.clusters })).collect::<Vec<_>>() }))?,
                )?;
                rows.push(json!({ "view": name, "from": from, "to": to, "width": width, "binHours": b, "k": k,
                    "clusters": today.iter().map(|summary| summary.clusters.len()).sum::<usize>() }));
            }
        }
    }
    Ok(json!({ "recording": path, "views": rows }))
}

fn regroup(path: &Path) -> Result<Value> {
    let replay = open(path)?;
    let index = Index::default();
    let query = query(&replay, &index)?;
    let (first, last) = (query.timeline.first, query.timeline.last);
    let line = line(&query, "country-measure", "politicalPower")?;
    let held = droppers(&line, &query, 8)?;
    let mut frames = Vec::new();
    let start = first + (last - first) / 2;
    let mut previous: Option<Vec<MarkSummary>> = None;
    let (mut changed_total, mut compared_total) = (0, 0);
    for step in 0..30 {
        let to = start + step * 24;
        let from = to - 8_760 + 1;
        let today = line.marks_of(&held, from, to, WIDTH)?;
        if let Some(before) = &previous {
            let (mut changed, mut compared) = (0, 0);
            for (old, new) in before.iter().zip(&today) {
                for cluster in old.clusters.iter().filter(|cluster| cluster.last_hour < to - 24 && cluster.first_hour > from + 2_000) {
                    compared += 1;
                    let again = new.clusters.iter().find(|other| other.first_ordinal == cluster.first_ordinal);
                    if again.is_none_or(|other| other.count != cluster.count || other.first_hour != cluster.first_hour || other.last_hour != cluster.last_hour) {
                        changed += 1;
                    }
                }
            }
            changed_total += changed;
            compared_total += compared;
            frames.push(json!({ "to": to, "compared": compared, "changed": changed }));
        }
        previous = Some(today);
    }
    Ok(json!({ "recording": path, "tags": held, "width": WIDTH, "viewHours": 8_760, "step": 24, "compared": compared_total, "changed": changed_total, "frames": frames }))
}

fn time(path: &Path, group: usize, threads: Option<usize>) -> Result<Value> {
    let bins = [1, 5, 6, 90, 96, 1_440, 1_536, 23_040, 24_576, 368_640];
    let (timeline, all) = {
        let replay = open(path)?;
        let index = Index::default();
        let query = query(&replay, &index)?;
        ((query.timeline.first, query.timeline.last), tags(&query))
    };
    let threads = threads.unwrap_or_else(|| (std::thread::available_parallelism().map_or(1, std::num::NonZero::get) / 2).max(2));
    let sets: Vec<(String, Vec<String>)> = [8, 100, all.len()].into_iter().map(|n| (format!("{n}"), all.iter().take(n).cloned().collect())).collect();
    let mut cache = replay_tools::cold_cache::probe_file(
        path,
        replay_tools::cold_cache::ResetScope::FreshRecordingAndIndexPerBatch,
    )?;
    let mut rows = Vec::new();
    let mut opens = Vec::new();
    for (name, held) in &sets {
        let groups: Vec<&[String]> = held.chunks(group.max(1)).collect();
        let (mut every, mut views_all) = (Vec::new(), Vec::new());
        for &bin in &bins {
            let span = tile::BINS as i64 * bin;
            let (first_index, last_index) = (i64::from(timeline.0).div_euclid(span), i64::from(timeline.1).div_euclid(span));
            let (mut times, mut bytes, mut views) = (Vec::new(), Vec::new(), Vec::new());
            let mut index = first_index;
            while index <= last_index {
                let indices: Vec<i64> = (index..=(index + 2).min(last_index)).collect();
                index += 3;
                cache = replay_tools::cold_cache::probe_file(
                    path,
                    replay_tools::cold_cache::ResetScope::FreshRecordingAndIndexPerBatch,
                )?;
                let opened = Instant::now();
                let replay = open(path)?;
                let index_state = Index::default();
                let query = query(&replay, &index_state)?;
                opens.push(opened.elapsed().as_secs_f64() * 1e3);
                let jobs: Vec<(i64, &[String])> = indices.iter().flat_map(|&index| groups.iter().map(move |&tags| (index, tags))).collect();
                let next = std::sync::atomic::AtomicUsize::new(0);
                let started = Instant::now();
                let done: Vec<Vec<(f64, usize)>> = std::thread::scope(|scope| {
                    let workers: Vec<_> = (0..threads.min(jobs.len()))
                        .map(|_| {
                            scope.spawn(|| -> std::result::Result<Vec<(f64, usize)>, String> {
                                let mut out = Vec::new();
                                loop {
                                    let at = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    let Some(&(index, tags)) = jobs.get(at) else { return Ok(out) };
                                    let begun = Instant::now();
                                    let line = line(&query, "country-measure", "politicalPower").map_err(|error| error.to_string())?;
                                    let reply = tile::tile(&line, "bench", &asks(tags), None, bin, index, false).map_err(|error| error.to_string())?;
                                    let text = serde_json::to_string(&reply).map_err(|error| error.to_string())?;
                                    out.push((begun.elapsed().as_secs_f64() * 1e3, text.len()));
                                }
                            })
                        })
                        .collect();
                    workers.into_iter().map(|worker| worker.join().expect("a worker")).collect::<std::result::Result<_, _>>()
                })?;
                views.push(started.elapsed().as_secs_f64() * 1e3);
                for (time, size) in done.into_iter().flatten() {
                    times.push(time);
                    bytes.push(size);
                }
            }
            every.extend(times.iter().copied());
            views_all.extend(views.iter().copied());
            let mut sorted = times.clone();
            sorted.sort_by(f64::total_cmp);
            views.sort_by(f64::total_cmp);
            bytes.sort_unstable();
            rows.push(json!({ "keys": name, "groups": groups.len(), "binHours": bin, "level": tile::level_of(bin)?, "replies": times.len(), "views": views.len(),
                "p50": percentile(&sorted, 0.5), "p99": percentile(&sorted, 0.99), "max": sorted.last(),
                "viewP50": percentile(&views, 0.5), "viewP99": percentile(&views, 0.99), "viewMax": views.last(),
                "bytesP50": bytes[bytes.len() / 2], "bytesMax": bytes.last() }));
            eprintln!("{}", rows.last().unwrap());
        }
        every.sort_by(f64::total_cmp);
        views_all.sort_by(f64::total_cmp);
        rows.push(json!({ "keys": name, "groups": groups.len(), "binHours": "all", "replies": every.len(), "p50": percentile(&every, 0.5), "p99": percentile(&every, 0.99), "max": every.last(),
            "views": views_all.len(), "viewP50": percentile(&views_all, 0.5), "viewP99": percentile(&views_all, 0.99), "viewMax": views_all.last() }));
    }
    opens.sort_by(f64::total_cmp);
    Ok(json!({ "recording": path, "timeline": [timeline.0, timeline.1], "tags": all.len(), "group": group, "threads": threads, "cache": cache, "openP50": percentile(&opens, 0.5), "rows": rows }))
}
