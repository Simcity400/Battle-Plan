//! The tables a recording's row frames leave to its native capture, read at a query's hour from
//! the recording's runs in place ([`Natives`]), so an hour's reader sees them as its row frames
//! would have held them; and the refusal of what no native reader serves yet ([`unprepared`]).

use crate::Source;
use replay::{
    campaign::Parallel,
    columnar_natives::{Cursor, Natives},
};
use replay_data::{rows::TableId, scope};
use replay_frame::frame::{DecodedFrame, TableUpdate, TableView};
use std::{
    io,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

/// The refusal of `label`, whose data a native recording holds but nothing prepares from it yet.
pub fn unprepared(label: &str) -> io::Error {
    dashboard_core::refused(format!(
        "{label} is not prepared from native recordings yet"
    ))
}

/// Whether `frame` leaves `table` to the native capture: its recording's mode records the
/// table, which its row frame holds as not recorded.
pub fn left_to_native(frame: &DecodedFrame, table: TableId) -> bool {
    matches!(&*frame.tables[table.index()], TableView::NotRecorded { mode } if scope::records(*mode, table))
}

/// One frame of `tables` read from the recording, held between a source's queries so the next
/// hour walks on from the last.
type Held = Arc<Mutex<Cursor>>;

/// The recording a source's queries read in place, and the cursors read through it, one for
/// each set of tables asked.
#[derive(Default)]
struct Queries {
    natives: Option<Arc<Natives>>,
    cursors: Vec<(Vec<TableId>, Held)>,
}

/// The cursor over `tables` of `source`'s recording.
fn cursor(source: &Source, tables: &[TableId]) -> io::Result<Held> {
    let state = source.state::<Queries>();
    let mut queries = state.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((_, cursor)) = queries.cursors.iter().find(|(asked, _)| asked == tables) {
        return Ok(Arc::clone(cursor));
    }
    let replay = source.replay_ref();
    let natives = match &queries.natives {
        Some(natives) => Arc::clone(natives),
        None => {
            let natives = Arc::new(Natives::new(replay)?);
            queries.natives = Some(Arc::clone(&natives));
            natives
        }
    };
    let cursor = Arc::new(Mutex::new(Cursor::new(
        natives,
        tables,
        replay.format,
        replay.capture_mode,
    )?));
    queries.cursors.push((tables.to_vec(), Arc::clone(&cursor)));
    Ok(cursor)
}

/// Why the recording holds no `hour` of `branch`.
fn unheld(branch: u64, hour: i32) -> io::Error {
    dashboard_core::refused(format!(
        "The recording's native capture holds no hour {hour} of branch {branch}"
    ))
}

/// Moves `cursor` to `hour` of `branch` until `cancelled` is set: refused when the recording
/// holds no such hour.
fn reach(
    source: &Source,
    cursor: &mut Cursor,
    branch: u64,
    hour: i32,
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let stop = || cancelled.load(Ordering::Acquire);
    if stop() {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let Some(segment) = source.replay_ref().segment_at(branch, hour) else {
        return Err(unheld(branch, hour));
    };
    if !cursor.move_to(segment, branch, hour, &stop)? {
        return Err(unheld(branch, hour));
    }
    Ok(())
}

/// Fills each of `tables` that `frame` leaves to the native capture with its view at the
/// frame's hour, read from the recording's runs; a frame leaving none is untouched.
pub fn fill(
    source: &Source,
    frame: &mut DecodedFrame,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<()> {
    let mut cut: Vec<TableId> = tables
        .iter()
        .copied()
        .filter(|&table| left_to_native(frame, table))
        .collect();
    if cut.is_empty() {
        return Ok(());
    }
    cut.sort_unstable();
    cut.dedup();
    let cursor = cursor(source, &cut)?;
    let mut cursor = cursor.lock().unwrap_or_else(PoisonError::into_inner);
    reach(source, &mut cursor, frame.branch, frame.hour, cancelled)?;
    let alone = || {};
    let parallel = Parallel {
        workers: std::thread::available_parallelism().map_or(1, usize::from),
        on_worker: &alone,
    };
    let native = cursor.build(Some(&parallel))?;
    for table in cut {
        frame.tables[table.index()] = native.tables[table.index()].clone();
    }
    Ok(())
}

/// [`super::progressive::frame_at`], with the tables its row frame leaves to the native capture
/// [filled](fill) from it.
pub fn frame_at(
    source: &Source,
    branch: u64,
    hour: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<Option<Box<DecodedFrame>>> {
    let Some(mut frame) = super::progressive::frame_at(source, branch, hour, tables, cancelled)?
    else {
        return Ok(None);
    };
    fill(source, &mut frame, tables, cancelled)?;
    Ok(Some(frame))
}

/// Whether `branch`'s row frame at `hour` leaves `table` to the native capture; false at an
/// hour no block records.
fn leaves(
    source: &Source,
    branch: u64,
    hour: i32,
    table: TableId,
    cancelled: &AtomicBool,
) -> io::Result<bool> {
    let Some(block) = source.replay_ref().index.iter().find(|block| {
        block.branch == branch && block.first_hour <= hour && hour <= block.last_hour
    }) else {
        return Ok(false);
    };
    // The row reader's supplying search reads this same block selection.
    if let Some(reads) =
        super::frames::block(source, block, &[TableId::Campaign, table], cancelled)?
    {
        return Ok(reads
            .iter()
            .find(|read| read.tables.contains(&table))
            .and_then(|read| read.frames.iter().find(|frame| frame.hour == hour))
            .is_some_and(|frame| {
                matches!(&frame.tables[table.index()], TableUpdate::NotRecorded { mode } if scope::records(*mode, table))
            }));
    }
    Ok(
        super::progressive::frame_at(source, branch, hour, &[table], cancelled)?
            .is_some_and(|frame| left_to_native(&frame, table)),
    )
}

/// The supplying read of `table` within `cadence` hours of `hour` that
/// [`super::progressive::latest_read`] finds, when `branch`'s row frame at `hour` leaves the
/// table to the native capture: the latest hour whose read slots read it, every hour since
/// recorded and the table complete at each, its status read from the recording's runs (None
/// inside when there is none). None when the row frame holds the table, for the row reader to
/// find.
pub fn read_hour(
    source: &Source,
    branch: u64,
    hour: i32,
    table: TableId,
    cadence: i32,
    cancelled: &AtomicBool,
) -> io::Result<Option<Option<i32>>> {
    if !leaves(source, branch, hour, table, cancelled)? {
        return Ok(None);
    }
    if cadence <= 0 {
        return Err(crate::invalid("A supplying read needs a positive cadence"));
    }
    let held = cursor(source, &[table])?;
    let replay = source.replay_ref();
    let first = hour.saturating_sub(cadence - 1);
    // Row frames keep every table's read slots; a cut table's rows decode as not recorded.
    let frames = super::frames::window(source, branch, first, hour, &[table], cancelled)?;
    // The hours recorded one after another back from `hour`, latest first.
    let mut recorded = Vec::new();
    let mut expected = Some(hour);
    for frame in frames.iter().rev() {
        crate::metadata::check_cancel(cancelled)?;
        if Some(frame.hour) != expected {
            break;
        }
        recorded.push(frame);
        expected = frame.hour.checked_sub(1);
    }
    // The scan below stops at the latest hour whose row frame alone ends it, at the latest.
    let reads = |frame: &DecodedFrame| {
        crate::metadata::slot_read(replay.format, &frame.metrics, table, frame.hour)
    };
    let ends = recorded
        .iter()
        .position(|frame| {
            reads(frame)
                || (!left_to_native(frame, table)
                    && !matches!(&*frame.tables[table.index()], TableView::Complete(_)))
        })
        .map_or(recorded.len(), |at| at + 1);
    // The table's status at the hours before that its row frames leave to the native capture,
    // walked in hour order: whether the recording's runs hold it complete there.
    let mut complete = vec![false; ends];
    {
        let mut cursor = held.lock().unwrap_or_else(PoisonError::into_inner);
        for (at, frame) in recorded[..ends].iter().enumerate().rev() {
            if !left_to_native(frame, table) {
                continue;
            }
            // The scan reads `hour` first: a refusal names it.
            match reach(source, &mut cursor, branch, frame.hour, cancelled) {
                Err(error) if error.kind() != io::ErrorKind::Interrupted => {
                    let held = replay
                        .segment_at(branch, hour)
                        .and_then(|segment| cursor.natives().find(segment, hour));
                    return Err(match held {
                        None => unheld(branch, hour),
                        Some(_) => error,
                    });
                }
                reached => reached?,
            }
            complete[at] = cursor.complete(table)?;
        }
    }
    for (at, frame) in recorded[..ends].iter().enumerate() {
        let complete = if left_to_native(frame, table) {
            complete[at]
        } else {
            matches!(&*frame.tables[table.index()], TableView::Complete(_))
        };
        if !complete {
            return Ok(Some(None));
        }
        if reads(frame) {
            return Ok(Some(Some(frame.hour)));
        }
    }
    Ok(Some(None))
}

/// [`super::progressive::latest_read`], its supplying read found by [`read_hour`] where the
/// row frames leave `table` to the native capture, and that read's frame [filled](fill) from it.
pub fn latest_read(
    source: &Source,
    branch: u64,
    hour: i32,
    table: TableId,
    cadence: i32,
    tables: &[TableId],
    cancelled: &AtomicBool,
) -> io::Result<Option<Box<DecodedFrame>>> {
    let at = match read_hour(source, branch, hour, table, cadence, cancelled)? {
        None => {
            return super::progressive::latest_read(
                source, branch, hour, table, cadence, tables, cancelled,
            );
        }
        Some(None) => return Ok(None),
        Some(Some(at)) => at,
    };
    let mut wanted = tables.to_vec();
    wanted.push(table);
    wanted.sort_unstable();
    wanted.dedup();
    frame_at(source, branch, at, &wanted, cancelled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{CAMPAIGN, file_source, records, source};
    use replay::campaign::Replay;
    use replay_data::scope::CaptureMode;

    #[test]
    fn only_a_table_its_mode_records_held_as_not_recorded_is_left_to_native() {
        let mut frame = source(1, 1, Some(1));
        let mut hold = |table: TableId, mode| {
            *frame.tables[table.index()] = TableView::NotRecorded { mode };
        };
        hold(TableId::CountryExperience, CaptureMode::Full);
        hold(TableId::DivisionModelInputs, CaptureMode::Full);
        hold(TableId::DivisionHistory, CaptureMode::Map);
        assert!(left_to_native(&frame, TableId::CountryExperience));
        // Full records no DivisionModelInputs, and Map no DivisionHistory: they are not recorded.
        assert!(!left_to_native(&frame, TableId::DivisionModelInputs));
        assert!(!left_to_native(&frame, TableId::DivisionHistory));
        assert!(!left_to_native(&frame, TableId::Campaign));
        assert!(!left_to_native(&frame, TableId::Divisions));
    }

    #[test]
    fn a_frame_leaving_nothing_is_untouched_and_one_leaving_tables_is_filled_from_the_recording() {
        let frame = source(1, 1, Some(1));
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame.clone()])]);
        let (source, root) = file_source(replay.unwrap());
        let cancelled = AtomicBool::new(false);
        let mut kept = frame.clone();
        fill(
            &source,
            &mut kept,
            &[TableId::Campaign, TableId::Divisions],
            &cancelled,
        )
        .unwrap();
        assert_eq!(kept, frame);
        let mut cut = frame.clone();
        *cut.tables[TableId::Divisions.index()] = TableView::NotRecorded {
            mode: CaptureMode::Full,
        };
        fill(
            &source,
            &mut cut,
            &[TableId::Campaign, TableId::Divisions],
            &cancelled,
        )
        .unwrap();
        assert_eq!(cut.tables[TableId::Divisions.index()], frame.tables[TableId::Divisions.index()]);
        // An hour the recording does not hold is refused by name.
        let mut unheld = frame;
        unheld.hour = 7;
        *unheld.tables[TableId::Divisions.index()] = TableView::NotRecorded {
            mode: CaptureMode::Full,
        };
        let error = fill(&source, &mut unheld, &[TableId::Divisions], &cancelled)
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "The recording's native capture holds no hour 7 of branch 1"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_supplying_read_the_rows_hold_is_the_row_readers_and_one_left_to_native_reads_its_status() {
        let table = TableId::ProjectPrograms;
        let frames = |view: &dyn Fn() -> TableView| -> Vec<DecodedFrame> {
            (0..30)
                .map(|hour| {
                    let mut frame = source(1, hour, Some(1));
                    *frame.tables[table.index()] = view();
                    frame
                })
                .collect()
        };
        let cancelled = AtomicBool::new(false);
        let held = frames(&|| {
            TableView::Complete(replay_data::rows::TableRows::ProjectPrograms(Vec::new()))
        });
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, held)]).unwrap();
        let (rows, root) = file_source(replay);
        for hour in [0, 3, 4, 29] {
            assert_eq!(
                read_hour(&rows, 1, hour, table, 24, &cancelled).unwrap(),
                None
            );
            assert_eq!(
                latest_read(&rows, 1, hour, table, 24, &[table], &cancelled).unwrap(),
                super::super::progressive::latest_read(
                    &rows,
                    1,
                    hour,
                    table,
                    24,
                    &[table],
                    &cancelled
                )
                .unwrap()
            );
        }
        std::fs::remove_dir_all(root).ok();
        let cut = frames(&|| TableView::NotRecorded {
            mode: CaptureMode::Full,
        });
        let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, cut)]).unwrap();
        let (native, root) = file_source(replay);
        // The recording holds the table not recorded: no complete read supplies it.
        assert_eq!(
            read_hour(&native, 1, 29, table, 24, &cancelled).unwrap(),
            Some(None)
        );
        // An hour no block records is the row reader's, which finds no read.
        assert_eq!(
            read_hour(&native, 1, 30, table, 24, &cancelled).unwrap(),
            None
        );
        std::fs::remove_dir_all(root).ok();
    }
}
