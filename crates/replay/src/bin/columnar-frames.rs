//! Reads a recording and its `p3-convert` columnar file side by side and checks that every
//! hour's table views agree: the row decoder's frames of every table against
//! [`replay::columnar_frames::Views`]' of the columnar walk. Reports the first differences
//! and the CPU each path took per recorded hour. Both keep the hour before's frame, so a
//! changed table is copied, not changed in place, on both paths.
//! `columnar-frames <recording.battleplan> <file.h4col> [--time]`; `--time` instead times
//! the walk alone, the walk with every table's views, and the row decoder of every table,
//! each keeping no earlier frame; `--time-tables` times each table's views alone; `--facts`
//! compares each hour's change facts, the row recording's updates against
//! [`Views::advance_recorded`]'s: their kinds, changed keys and masks, and deleted keys.

use replay::{
    columnar_frames::{Limits, Recording, Views, schema},
    container::Replay,
};
use replay_columnar::walk::State;
use replay_data::rows::DeltaRow;
use replay_data::rows::{TABLE_COUNT, TableId};
use replay_frame::frame::{DecodedFrame, SharedView, TableUpdate, TableView};
use serde_json::json;
use std::{io, path::PathBuf};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

/// This thread's CPU time in nanoseconds, at the scheduler's resolution.
#[cfg(windows)]
fn thread_ns() -> u64 {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentThread, GetThreadTimes},
    };
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: the pseudo-handle of the current thread and four owned FILETIMEs.
    unsafe {
        GetThreadTimes(
            GetCurrentThread(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    let ticks =
        |time: FILETIME| u64::from(time.dwHighDateTime) << 32 | u64::from(time.dwLowDateTime);
    (ticks(kernel) + ticks(user)) * 100
}

#[cfg(not(windows))]
fn thread_ns() -> u64 {
    0
}

/// A view's kind, sample hour and row count, for reporting.
fn describe(view: &SharedView) -> serde_json::Value {
    let (kind, rows) = match &**view {
        TableView::Complete(rows) => ("complete", Some(rows.len())),
        TableView::Truncated { rows, .. } => ("truncated", Some(rows.len())),
        TableView::Unavailable { .. } => ("unavailable", None),
        TableView::NotRecorded { .. } => ("not recorded", None),
    };
    json!({"kind": kind, "sampled": view.sampled_at(), "rows": rows})
}

/// The first row two views of one table hold differently, each as its debug text.
fn first_row_difference(expected: &SharedView, actual: &SharedView) -> serde_json::Value {
    fn rows(view: &SharedView) -> Option<&replay_data::rows::TableRows> {
        match &**view {
            TableView::Complete(rows) | TableView::Truncated { rows, .. } => Some(rows),
            _ => None,
        }
    }
    let (Some(want), Some(got)) = (rows(expected), rows(actual)) else {
        return json!(null);
    };
    let clip = |text: String| text.chars().take(600).collect::<String>();
    let mut wanted = want.iter();
    let mut gotten = got.iter();
    loop {
        match (wanted.next(), gotten.next()) {
            (None, None) => return json!(null),
            (a, b) if a == b => continue,
            (a, b) => {
                return json!({
                    "expected": a.map(|row| clip(format!("{row:?}"))),
                    "actual": b.map(|row| clip(format!("{row:?}"))),
                });
            }
        }
    }
}

/// CPU per recorded hour of the walk alone, the walk with views of every table, and the row
/// decoder of every table, none keeping an earlier hour's frame.
fn time(replay: &Replay, file: &Recording) -> io::Result<serde_json::Value> {
    let mut state = State::new(schema()?)?;
    let (mut hours, started) = (0_u64, thread_ns());
    file.walk(&mut state, |_, _, _| {
        hours += 1;
        Ok(true)
    })?;
    let walk = thread_ns() - started;
    let mut state = State::new(schema()?)?;
    let mut views = Views::new(&TableId::ALL, replay.format, replay.capture_mode)?;
    let started = thread_ns();
    file.walk(&mut state, |state, manifest, first| {
        drop(views.advance(state, manifest, first)?);
        Ok(true)
    })?;
    let viewed = thread_ns() - started;
    let started = thread_ns();
    let mut decoded = 0_u64;
    for index in replay.index.iter() {
        let mut decoder = replay_frame::frame::FrameDecoder::for_tables(&TableId::ALL);
        for recorded in replay.read_block_recorded_selected(index, &TableId::ALL)? {
            decoder.advance(&recorded)?;
            decoded += 1;
        }
    }
    let rows = thread_ns() - started;
    let per_hour = |ns: u64, hours: u64| ns as f64 / 1000.0 / hours.max(1) as f64;
    Ok(json!({
        "hours": hours,
        "walk_us_per_hour": per_hour(walk, hours),
        "walk_and_views_us_per_hour": per_hour(viewed, hours),
        "row_decoder_us_per_hour": per_hour(rows, decoded),
    }))
}

/// CPU per recorded hour of each table's views over one walk, most costly first: each table
/// advanced by views of it alone, less the cost of a frame of no table.
fn time_tables(replay: &Replay, file: &Recording) -> io::Result<serde_json::Value> {
    let mut state = State::new(schema()?)?;
    let mut alone = TableId::ALL
        .iter()
        .map(|table| Views::new(&[*table], replay.format, replay.capture_mode))
        .collect::<io::Result<Vec<_>>>()?;
    let mut spent = vec![0_u64; TABLE_COUNT];
    let mut hours = 0_u64;
    file.walk(&mut state, |state, manifest, first| {
        for (index, views) in alone.iter_mut().enumerate() {
            let started = thread_ns();
            drop(views.advance(state, manifest, first)?);
            spent[index] += thread_ns() - started;
        }
        hours += 1;
        Ok(true)
    })?;
    let floor = spent[TableId::Campaign.index()];
    let mut tables: Vec<_> = TableId::ALL
        .iter()
        .zip(&spent)
        .map(|(table, ns)| {
            let us = ns.saturating_sub(floor) as f64 / 1000.0 / hours.max(1) as f64;
            (format!("{table:?}"), us)
        })
        .collect();
    tables.sort_by(|a, b| b.1.total_cmp(&a.1));
    let total: f64 = tables.iter().map(|(_, us)| us).sum();
    tables.truncate(40);
    Ok(json!({"hours": hours, "views_us_per_hour": total, "tables": tables}))
}

/// A fact's kind, for reporting.
fn kind(update: &TableUpdate) -> &'static str {
    match update {
        TableUpdate::Snapshot(_) => "snapshot",
        TableUpdate::Delta { .. } => "delta",
        TableUpdate::Unchanged { .. } => "unchanged",
        TableUpdate::Truncated { .. } => "truncated",
        TableUpdate::Unavailable { .. } => "unavailable",
        TableUpdate::NotRecorded { .. } => "not recorded",
        TableUpdate::Incompatible { .. } => "incompatible",
    }
}

/// How one table's facts of one hour compare; `keyframe` is whether the row recording's
/// hour is a keyframe, where a snapshot stands for any continuing complete view.
fn compare_facts(row: &TableUpdate, columnar: &TableUpdate, keyframe: bool) -> &'static str {
    match (row, columnar) {
        (
            TableUpdate::Delta {
                changed: want,
                deleted: want_deleted,
                ..
            },
            TableUpdate::Delta {
                changed: have,
                deleted: have_deleted,
                ..
            },
        ) => {
            let keys = |rows: &[DeltaRow]| rows.iter().map(|row| row.key).collect::<Vec<_>>();
            let mut want_sorted = want_deleted.clone();
            want_sorted.sort_unstable();
            if keys(want) != keys(have) {
                "delta keys differ"
            } else if &want_sorted != have_deleted {
                "deleted keys differ"
            } else if want.iter().zip(have).any(|(a, b)| a.mask != b.mask) {
                "masks differ"
            } else {
                "same"
            }
        }
        (
            TableUpdate::Delta {
                changed, deleted, ..
            },
            TableUpdate::Unchanged { .. },
        ) if changed.is_empty() && deleted.is_empty() => "empty row delta",
        (TableUpdate::Delta { .. }, TableUpdate::Unchanged { .. }) => {
            "row delta, columnar unchanged"
        }
        (TableUpdate::Snapshot(_), TableUpdate::Delta { .. } | TableUpdate::Unchanged { .. })
            if keyframe =>
        {
            "row keyframe"
        }
        _ if kind(row) == kind(columnar) => "same",
        _ => "kinds differ",
    }
}

fn facts(replay: &Replay, file: &Recording) -> io::Result<serde_json::Value> {
    let mut state = State::new(schema()?)?;
    let mut views = Views::new(&TableId::ALL, replay.format, replay.capture_mode)?;
    let mut blocks = replay.index.iter();
    let mut pending = std::collections::VecDeque::new();
    let names: Vec<String> = TableId::ALL
        .iter()
        .map(|table| format!("{table:?}"))
        .collect();
    let mut counts: std::collections::BTreeMap<(usize, &'static str), u64> =
        std::collections::BTreeMap::new();
    let mut examples = Vec::new();
    let mut hours = 0_u64;
    file.walk(&mut state, |state, manifest, first| {
        let (_, columnar) = views.advance_recorded(state, manifest, first)?;
        if pending.is_empty() {
            let index = blocks
                .next()
                .ok_or_else(|| invalid("The columnar file holds more hours than the recording"))?;
            pending.extend(replay.read_block_recorded_selected(index, &TableId::ALL)?);
        }
        let row = pending.pop_front().expect("a recorded hour");
        if (row.branch, row.hour) != (columnar.branch, columnar.hour) {
            return Err(invalid(format!(
                "The recording's hour {} of branch {} meets the columnar file's hour {} of branch {}",
                row.hour, row.branch, columnar.hour, columnar.branch
            )));
        }
        for index in 0..TableId::ALL.len() {
            let how = compare_facts(&row.tables[index], &columnar.tables[index], row.keyframe);
            let count = counts.entry((index, how)).or_default();
            *count += 1;
            if how != "same" && *count <= 1 && examples.len() < 80 {
                examples.push(json!({
                    "hour": row.hour,
                    "table": names[index],
                    "how": how,
                    "row": kind(&row.tables[index]),
                    "columnar": kind(&columnar.tables[index]),
                }));
            }
        }
        hours += 1;
        Ok(true)
    })?;
    if !pending.is_empty() || blocks.next().is_some() || hours != file.hours() {
        return Err(invalid(format!(
            "The columnar file's {hours} hours of {} do not cover the recording",
            file.hours()
        )));
    }
    let mut by_kind: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for ((_, how), count) in &counts {
        *by_kind.entry(how).or_default() += count;
    }
    let differing: Vec<_> = counts
        .iter()
        .filter(|((_, how), _)| *how != "same")
        .map(|((table, how), count)| json!([names[*table], how, count]))
        .collect();
    Ok(json!({
        "hours": hours,
        "table_hours_by_comparison": by_kind,
        "differing": differing,
        "examples": examples,
    }))
}

fn main() -> io::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let usage = || invalid("columnar-frames <recording.battleplan> <file.h4col> [--time]");
    let recording = PathBuf::from(args.next().ok_or_else(usage)?);
    let columnar = PathBuf::from(args.next().ok_or_else(usage)?);
    let replay = Replay::open_path(&recording)?;
    let file = Recording::open_path(&columnar, Limits::default())?;
    match args.next() {
        Some(flag) if flag == "--time" => {
            println!("{}", serde_json::to_string_pretty(&time(&replay, &file)?)?);
            return Ok(());
        }
        Some(flag) if flag == "--facts" => {
            println!("{}", serde_json::to_string_pretty(&facts(&replay, &file)?)?);
            return Ok(());
        }
        Some(flag) if flag == "--time-tables" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&time_tables(&replay, &file)?)?
            );
            return Ok(());
        }
        _ => {}
    }
    let mut state = State::new(schema()?)?;
    let mut views = Views::new(&TableId::ALL, replay.format, replay.capture_mode)?;
    let mut previous_expected: Option<DecodedFrame> = None;
    let mut previous_actual: Option<DecodedFrame> = None;
    let mut differing_now = vec![0_u8; TABLE_COUNT];
    let (mut row_ns, mut walk_ns, mut views_ns) = (0_u64, 0_u64, 0_u64);
    let mut hours = 0_usize;
    let mut details = Vec::new();
    let mut differing_tables = vec![0_u64; TABLE_COUNT];
    let mut blocks = replay.index.iter();
    let mut pending = std::collections::VecDeque::new();
    let mut decoder = None;
    let mut last = thread_ns();
    file.walk(&mut state, |state, manifest, first| {
        let walked = thread_ns();
        walk_ns += walked - last;
        let frame = views.advance(state, manifest, first)?;
        let advanced = thread_ns();
        views_ns += advanced - walked;
        // The row decoder's frame of the same hour, its block's updates read as needed.
        if pending.is_empty() {
            let index = blocks
                .next()
                .ok_or_else(|| invalid("The columnar file holds more hours than the recording"))?;
            pending.extend(replay.read_block_recorded_selected(index, &TableId::ALL)?);
            decoder = Some(replay_frame::frame::FrameDecoder::for_tables(&TableId::ALL));
        }
        let recorded = pending.pop_front().expect("a recorded hour");
        let expected = decoder
            .as_mut()
            .expect("a decoder")
            .advance(&recorded)?
            .clone();
        row_ns += thread_ns() - advanced;
        if expected.hour != frame.hour
            || expected.metrics.total_ns != frame.metrics.total_ns
            || expected.metrics.pool_bytes != frame.metrics.pool_bytes
        {
            details.push(json!({"hour": frame.hour, "recording_hour": expected.hour, "what": "hour or metrics"}));
        }
        // Held, the hour before's frames keep their allocations from being reused, so a
        // table both sides kept is one compared already.
        for index in 0..TABLE_COUNT {
            let (want, have) = (&expected.tables[index], &frame.tables[index]);
            let kept = match (&previous_expected, &previous_actual) {
                (Some(before_want), Some(before_have)) => {
                    SharedView::ptr_eq(want, &before_want.tables[index])
                        && SharedView::ptr_eq(have, &before_have.tables[index])
                        && differing_now[index] == 0
                }
                _ => false,
            };
            let same = want.sampled_at() == have.sampled_at() && (kept || want == have);
            differing_now[index] = u8::from(!same);
            if same {
                continue;
            }
            differing_tables[index] += 1;
            // The first differences of each table, a few tables at most.
            if differing_tables[index] <= 2 && details.len() < 60 {
                let (want, have) = (&expected.tables[index], &frame.tables[index]);
                details.push(json!({
                    "hour": frame.hour,
                    "table": format!("{:?}", TableId::ALL[index]),
                    "expected": describe(want),
                    "actual": describe(have),
                    "first_row_difference": first_row_difference(want, have),
                }));
            }
        }
        previous_expected = Some(expected);
        previous_actual = Some(frame);
        hours += 1;
        last = thread_ns();
        Ok(true)
    })?;
    if !pending.is_empty() || blocks.next().is_some() || hours != file.hours() as usize {
        return Err(invalid(format!(
            "The columnar file's {hours} hours of {} do not cover the recording",
            file.hours()
        )));
    }
    let per_hour = |ns: u64| ns as f64 / 1000.0 / hours.max(1) as f64;
    let mut tables: Vec<_> = TableId::ALL
        .iter()
        .zip(&differing_tables)
        .filter(|(_, count)| **count > 0)
        .map(|(table, count)| (format!("{table:?}"), *count))
        .collect();
    tables.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "hours": hours,
            "row_decoder_us_per_hour": per_hour(row_ns),
            "columnar_walk_us_per_hour": per_hour(walk_ns),
            "columnar_views_us_per_hour": per_hour(views_ns),
            "tables_differing": tables,
            "details": details,
        }))?
    );
    Ok(())
}
