//! The map's hours from a recording's change-run index: frames of the map tables moved to and
//! built at any hour.

use crate::playback::Current;
use engine_allocation::credit::MemoryBudget;
use replay::{
    columnar_runs::{IndexFile, Snapshots, beyond_memory},
    container::{Parallel, Replay},
};
use replay_data::{BranchId, Hour, rows::TableId, rows::tables};
use replay_frame::frame::{DecodedFrame, FrameDecoder, TableView};
use std::{io, sync::Arc};

/// A change-run index of a replay's native columnar recordings, holding exactly the hours the
/// replay records but those of its sessions still being recorded, and every map table, shared
/// by every playback serving the replay's map.
pub struct Columnar {
    file: Arc<IndexFile>,
}

impl Columnar {
    /// `file` as `replay`'s map source; none when it is of another campaign, does not hold
    /// exactly the hours the replay records of the sessions it does not hold pending, or lacks
    /// a map table.
    pub fn verify(replay: &Replay, file: Arc<IndexFile>) -> Option<Arc<Self>> {
        (file.records(replay) && file.holds(dashboard_base::map_tables::selected_tables()))
            .then(|| Arc::new(Self { file }))
    }

    /// A playback's source of `replay`'s map, its views' change facts charged to `changes`,
    /// moving and building views on `workers` threads.
    pub fn open(
        &self,
        replay: Arc<Replay>,
        changes: MemoryBudget,
        workers: usize,
    ) -> io::Result<MapSource> {
        let snapshots = Snapshots::new(
            Arc::clone(&self.file),
            dashboard_base::map_tables::selected_tables(),
            replay.format,
            replay.capture_mode,
            workers,
            tables_memory().clone(),
        )?
        .with_change_facts(changes);
        Ok(MapSource {
            pending: self.file.pending(&replay),
            snapshots,
            replay,
            workers: workers.max(1),
        })
    }
}

/// What every map source's tables may hold at once: half the viewer's 4 GB working set.
fn tables_memory() -> &'static MemoryBudget {
    static MEMORY: std::sync::OnceLock<MemoryBudget> = std::sync::OnceLock::new();
    MEMORY.get_or_init(|| MemoryBudget::new(2 << 30))
}

/// One playback's map hours from a [`Columnar`] index.
pub struct MapSource {
    snapshots: Snapshots,
    replay: Arc<Replay>,
    workers: usize,
    /// The replay's branches whose session the index holds pending, each with its session.
    pending: Vec<(BranchId, u64)>,
}

impl MapSource {
    pub fn roster_source(&self) -> (Arc<IndexFile>, MemoryBudget) {
        (Arc::clone(self.snapshots.file()), tables_memory().clone())
    }

    pub fn set_workers(&mut self, workers: usize) {
        self.workers = workers.max(1);
        self.snapshots.set_workers(self.workers);
    }

    /// The hour a read of `hour` of `branch` starts from: the hour itself, which no walk
    /// precedes; none outside every run.
    pub fn walk_start(&self, branch: BranchId, hour: Hour) -> Option<i64> {
        self.snapshots
            .file()
            .index
            .find(branch, hour)
            .map(|_| i64::from(hour))
    }

    /// `hour` of `branch`: with `materialize`, its frame; without, the move taken there and no
    /// frame built. It builds no other hour: the hours before it that a window lacks are read
    /// as hours of their own, after it, and the cursor moves on from whichever it read last.
    /// An hour no run holds is the replay's gap there. A move short of the memory the map's
    /// tables share waits for it until `cancelled`; one needing more than they may ever hold is
    /// refused, never a gap. Resource pressure and cancellation leave the source to be asked
    /// again; damage is the hour's gap.
    pub fn read(
        &mut self,
        branch: BranchId,
        hour: Hour,
        materialize: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<Current> {
        let priority = viewer_foundation::worker_priority::current_priority();
        let on_worker = move || {
            if priority.is_some_and(|priority| priority < 0) {
                viewer_foundation::worker_priority::background_priority();
            }
        };
        let parallel = Parallel {
            workers: self.workers,
            on_worker: &on_worker,
        };
        let gap = |reason: String| Current::Gap {
            branch,
            hour,
            reason,
        };
        if self.snapshots.file().index.find(branch, hour).is_none() {
            let recorded = self.replay.index.iter().any(|block| {
                block.branch == branch && block.first_hour <= hour && hour <= block.last_hour
            });
            let pending = self
                .pending
                .iter()
                .find(|&&(held, _)| held == branch)
                .filter(|_| recorded);
            return Ok(match pending {
                Some(&(_, session)) => Current::Pending {
                    branch,
                    hour,
                    reason: format!(
                        "session {session} is still being recorded: the map holds its hours \
                         once its native recording lands"
                    ),
                },
                None => gap(crate::playback::gap_reason(&self.replay, branch, hour)),
            });
        }
        let read = (|| -> io::Result<Option<DecodedFrame>> {
            if cancelled() {
                return Err(io::ErrorKind::Interrupted.into());
            }
            self.snapshots
                .move_to(branch, hour, &on_worker, cancelled)?;
            materialize
                .then(|| self.snapshots.build(Some(&parallel)))
                .transpose()
        })();
        match read {
            Ok(Some(frame)) if campaign_holds(&frame, &self.replay.campaign_id) => {
                Ok(Current::Frame(Arc::new(frame)))
            }
            Ok(Some(_)) => Ok(gap(
                "frame campaign identity differs from replay header".into()
            )),
            Ok(None) => Ok(gap("hour has not been decoded".into())),
            Err(error) if beyond_memory(&error) => Ok(Current::Refused {
                branch,
                hour,
                reason: error.to_string(),
            }),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) || FrameDecoder::change_fact_admission_refused(&error) =>
            {
                Err(error)
            }
            Err(error) => Ok(gap(format!("unreadable change-run hour: {error}"))),
        }
    }
}

/// Whether `frame`'s Campaign, when complete, is the replay's one campaign `campaign`.
fn campaign_holds(frame: &DecodedFrame, campaign: &[u8]) -> bool {
    match &*frame.tables[TableId::Campaign.index()] {
        TableView::Complete(rows) => rows
            .rows::<tables::Campaign>()
            .is_some_and(|rows| rows.len() == 1 && rows[0].id == campaign),
        _ => true,
    }
}
