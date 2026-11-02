//! The tables a recording's row frames leave to its native capture, read at a query's hour from
//! the change-run index its preparation keeps of that capture ([`index_path`]), so an hour's
//! reader sees them as its row frames would have held them; and the refusal of what no native
//! reader serves yet ([`unprepared`]).

use crate::Source;
use engine_allocation::credit::MemoryBudget;
use replay::{
    columnar_runs::{IndexFile, Snapshots},
    container::Parallel,
};
use replay_data::{rows::TableId, scope};
use replay_frame::frame::{DecodedFrame, TableUpdate, TableView};
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::SystemTime,
};

/// The refusal of `label`, whose data a native recording holds but nothing prepares from it yet.
pub fn unprepared(label: &str) -> io::Error {
    dashboard_core::refused(format!(
        "{label} is not prepared from native recordings yet"
    ))
}

/// The folder of `source`'s prepared store holding its change-run index.
pub fn index_folder(source: &Source) -> io::Result<PathBuf> {
    Ok(crate::prepared::recording_folder(source)?.join("change-runs"))
}

/// Where `source`'s change-run index is kept ([`index_folder`]).
pub fn index_path(source: &Source) -> io::Result<PathBuf> {
    Ok(index_folder(source)?.join("index.h4runs"))
}

/// Whether `frame` leaves `table` to the native capture: its recording's mode records the
/// table, which its row frame holds as not recorded.
pub fn left_to_native(frame: &DecodedFrame, table: TableId) -> bool {
    matches!(&*frame.tables[table.index()], TableView::NotRecorded { mode } if scope::records(*mode, table))
}

/// One frame of `tables`' change runs, held between a source's queries so the next hour moves on
/// from the last.
type Cursor = Arc<Mutex<Snapshots>>;

/// The change-run index a source's queries read, by the file's length and last write, and the
/// cursors read through it, one for each set of tables asked.
#[derive(Default)]
struct Held {
    file: Option<((u64, Option<SystemTime>), Arc<IndexFile>)>,
    cursors: Vec<(Vec<TableId>, Cursor)>,
}

/// What one cursor's tables may hold. Each has its own budget: a move short of it waits only on
/// what that cursor releases, never on another's idle tables.
const CURSOR_MEMORY: usize = 1 << 30;

/// The index serving `source` in its `held` state, opened anew when the file changed; refused
/// unless it indexes the recording and holds `tables`.
fn open(source: &Source, held: &mut Held, tables: &[TableId]) -> io::Result<Arc<IndexFile>> {
    let not_indexed = || {
        dashboard_core::refused(format!(
            "The recording holds {tables:?} in its native capture, whose change-run index is not prepared yet"
        ))
    };
    let path = index_path(source)?;
    let metadata = match std::fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Err(not_indexed()),
        Err(error) => return Err(error),
    };
    let identity = (metadata.len(), metadata.modified().ok());
    if held.file.as_ref().is_none_or(|(at, _)| *at != identity) {
        let file = Arc::new(IndexFile::open(&path)?);
        *held = Held {
            file: Some((identity, file)),
            cursors: Vec::new(),
        };
    }
    let file = Arc::clone(&held.file.as_ref().expect("opened above").1);
    if !file.records(source.replay_ref()) || !file.holds(tables) {
        return Err(not_indexed());
    }
    Ok(file)
}

/// The cursor over `tables` of the index serving `source` ([`open`]).
fn cursor(source: &Source, tables: &[TableId]) -> io::Result<Cursor> {
    let state = source.state::<Held>();
    let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
    let file = open(source, &mut held, tables)?;
    if let Some((_, cursor)) = held.cursors.iter().find(|(asked, _)| asked == tables) {
        return Ok(Arc::clone(cursor));
    }
    let replay = source.replay_ref();
    let workers = std::thread::available_parallelism().map_or(1, usize::from);
    let snapshots = Snapshots::new(
        file,
        tables,
        replay.format,
        replay.capture_mode,
        workers,
        MemoryBudget::new(CURSOR_MEMORY),
    )?;
    let cursor = Arc::new(Mutex::new(snapshots));
    held.cursors.push((tables.to_vec(), Arc::clone(&cursor)));
    Ok(cursor)
}

/// Why `file` holds no `hour` of `branch`: its session still being recorded, or none it holds.
fn unheld(source: &Source, file: &IndexFile, branch: u64, hour: i32) -> io::Error {
    let session = file
        .pending(source.replay_ref())
        .into_iter()
        .find_map(|(held, session)| (held == branch).then_some(session));
    dashboard_core::refused(match session {
        Some(session) => format!(
            "session {session} is still being recorded: its hours are read once its native recording lands"
        ),
        None => format!("The native capture's index holds no hour {hour} of branch {branch}"),
    })
}

/// Fills each of `tables` that `frame` leaves to the native capture with its view at the
/// frame's hour, read from the capture's change-run index; a frame leaving none is untouched.
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
    let mut snapshots = cursor.lock().unwrap_or_else(PoisonError::into_inner);
    let (branch, hour) = (frame.branch, frame.hour);
    let alone = || {};
    let stop = || cancelled.load(Ordering::Acquire);
    if !snapshots.move_to(branch, hour, &alone, &stop)? {
        return Err(unheld(source, snapshots.file(), branch, hour));
    }
    let parallel = Parallel {
        workers: std::thread::available_parallelism().map_or(1, usize::from),
        on_worker: &alone,
    };
    let native = snapshots.build(Some(&parallel))?;
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
/// recorded and the table complete at each, its status read from the capture's change-run index
/// (None inside when there is none). None when the row frame holds the table, for the row
/// reader to find.
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
    let file = {
        let state = source.state::<Held>();
        let mut held = state.lock().unwrap_or_else(PoisonError::into_inner);
        open(source, &mut held, &[table])?
    };
    let replay = source.replay_ref();
    let first = hour.saturating_sub(cadence - 1);
    // Row frames keep every table's read slots; a cut table's rows decode as not recorded.
    let frames = super::frames::window(source, branch, first, hour, &[table], cancelled)?;
    let mut expected = hour;
    for frame in frames.iter().rev() {
        crate::metadata::check_cancel(cancelled)?;
        if frame.hour != expected {
            return Ok(Some(None));
        }
        let complete = if left_to_native(frame, table) {
            // No keys: the table's status alone, read without its rows.
            let (view, _) = file
                .rows_at(
                    branch,
                    frame.hour,
                    table,
                    &[],
                    replay.format,
                    replay.capture_mode,
                )?
                .ok_or_else(|| unheld(source, &file, branch, frame.hour))?;
            matches!(view, TableView::Complete(_))
        } else {
            matches!(&*frame.tables[table.index()], TableView::Complete(_))
        };
        if !complete {
            return Ok(Some(None));
        }
        if crate::metadata::slot_read(replay.format, &frame.metrics, table, frame.hour) {
            return Ok(Some(Some(frame.hour)));
        }
        let Some(before) = expected.checked_sub(1) else {
            break;
        };
        expected = before;
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
    use replay::container::Replay;
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
    fn a_frame_leaving_nothing_is_untouched_and_one_leaving_tables_unindexed_is_refused() {
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
        let mut cut = frame;
        *cut.tables[TableId::Divisions.index()] = TableView::NotRecorded {
            mode: CaptureMode::Full,
        };
        let error = fill(
            &source,
            &mut cut,
            &[TableId::Campaign, TableId::Divisions],
            &cancelled,
        )
        .unwrap_err()
        .to_string();
        assert_eq!(
            error,
            "The recording holds [Divisions] in its native capture, whose change-run index is not prepared yet"
        );
        std::fs::remove_dir_all(root).ok();
    }

    #[test]
    fn a_supplying_read_the_rows_hold_is_the_row_readers_and_one_left_to_native_needs_the_index() {
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
        assert_eq!(
            read_hour(&native, 1, 29, table, 24, &cancelled)
                .unwrap_err()
                .to_string(),
            "The recording holds [ProjectPrograms] in its native capture, whose change-run index is not prepared yet"
        );
        // An hour no block records is the row reader's, which finds no read.
        assert_eq!(
            read_hour(&native, 1, 30, table, 24, &cancelled).unwrap(),
            None
        );
        std::fs::remove_dir_all(root).ok();
    }
}
