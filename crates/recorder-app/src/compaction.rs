//! After the game closes, with File size set to Small, the recorder compacts the
//! recordings its sessions finished. Compaction streams, verifies every frame
//! and publishes only a strictly smaller file; manual Compact stays in the
//! replay app.
use replay::container::{CompactionOutcome, CompactionStage, compact_with};
use std::{cell::Cell, io, path::Path};
use windows_sys::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN, THREAD_MODE_BACKGROUND_END,
};

/// Windows background mode for this thread's CPU and I/O while compacting.
pub(crate) struct BackgroundPriority;

impl BackgroundPriority {
    pub(crate) fn enter() -> io::Result<Self> {
        if unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self)
    }
}

impl Drop for BackgroundPriority {
    fn drop(&mut self) {
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_END);
        }
    }
}

pub fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Compaction cancelled")
}

/// Compact one finished recording. `cancel` is asked at every streaming step;
/// `progress` hears the share of blocks written.
pub fn compact(
    path: &Path,
    cancel: &dyn Fn() -> bool,
    progress: &dyn Fn(f64),
) -> io::Result<CompactionOutcome> {
    let _priority = BackgroundPriority::enter()?;
    compact_reporting(path, cancel, progress)
}

fn compact_reporting(
    path: &Path,
    cancel: &dyn Fn() -> bool,
    progress: &dyn Fn(f64),
) -> io::Result<CompactionOutcome> {
    let blocks = Cell::new(0_usize);
    compact_with(path, |stage, _| {
        if cancel() {
            return Err(cancelled());
        }
        match stage {
            CompactionStage::Started { blocks: total } => blocks.set(total),
            CompactionStage::BlockWritten(position) if blocks.get() > 0 => {
                progress((position + 1) as f64 / blocks.get() as f64)
            }
            _ => {}
        }
        Ok(())
    })
    .map(|result| result.outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay::{
        branches::{BranchReason, BranchStart, Parentage},
        container::Replay,
        frame::{Frame, Metrics, TableUpdate},
        identity::{Identity, SampleBoundary, SessionStart, Verification},
        rows::{CampaignRow, ClockRow, TableId, TableRows},
        spool::Record,
    };
    use std::{cell::RefCell, fs};

    fn synthetic_recording(root: &Path) -> std::path::PathBuf {
        let path = root.join("synthetic.battleplan");
        let session = SessionStart {
            id: 1,
            process_id: 1,
            process_start_time: 1,
            qpc_frequency: 1,
            sample_boundary: SampleBoundary::EndOfHour,
            identity: Identity {
                game_version: "synthetic".into(),
                exe_hash: [1; 32],
                exe_size: 1,
                vanilla_hash: [2; 32],
                dlc: vec![],
                mods: vec![],
                coverage_policy: 1,
                language: "synthetic".into(),
                recorder_version: "synthetic".into(),
                collector_hash: [3; 32],
                verification: Verification::Unchecked,
            },
        };
        let mut records = vec![
            Record::SessionStart(session),
            Record::BranchStart(BranchStart {
                id: 1,
                session: 1,
                loaded_hour: Some(0),
                first_hour: 1,
                parent: Parentage::Unknown,
                reason: BranchReason::SessionStart,
            }),
        ];
        for hour in 1..=3 {
            // Only the campaign and clock are recorded; tables that must hold one
            // row when complete stay unavailable.
            let mut tables = std::array::from_fn(|_| TableUpdate::Unavailable {
                reason: "synthetic unavailable table".into(),
            });
            tables[TableId::Campaign.index()] =
                TableUpdate::Snapshot(TableRows::Campaign(vec![CampaignRow {
                    id: b"synthetic".to_vec(),
                }]));
            tables[TableId::Clock.index()] =
                TableUpdate::Snapshot(TableRows::Clock(vec![ClockRow {
                    hour,
                    ..Default::default()
                }]));
            records.push(Record::Frame(Frame {
                branch: 1,
                hour,
                keyframe: hour == 1,
                metrics: Metrics::default(),
                tables,
            }));
        }
        Replay::from_records(b"synthetic".to_vec(), &[records])
            .unwrap()
            .write_atomic(&path)
            .unwrap();
        path
    }

    fn scratch() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "battle-plan-recorder-compaction-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_game_starting_cancels_compaction_and_keeps_the_recording_as_it_was() {
        let root = scratch();
        let path = synthetic_recording(&root);
        let before = fs::read(&path).unwrap();
        let steps = Cell::new(0);
        let error = compact_reporting(
            &path,
            &|| {
                steps.set(steps.get() + 1);
                steps.get() > 3
            },
            &|_| {},
        )
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read(&path).unwrap(), before);
        // Only the recording is left behind: no candidate or backup file.
        assert_eq!(fs::read_dir(&root).unwrap().count(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn compaction_reports_block_progress_up_to_complete() {
        let root = scratch();
        let path = synthetic_recording(&root);
        let reported = RefCell::new(Vec::new());
        let outcome =
            compact_reporting(&path, &|| false, &|share| reported.borrow_mut().push(share))
                .unwrap();
        assert!(matches!(
            outcome,
            CompactionOutcome::Compacted | CompactionOutcome::AlreadySmall
        ));
        let reported = reported.into_inner();
        assert_eq!(reported.last(), Some(&1.0));
        assert!(reported.windows(2).all(|pair| pair[0] <= pair[1]));
        fs::remove_dir_all(root).unwrap();
    }
}
