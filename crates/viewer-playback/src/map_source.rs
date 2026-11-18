//! The map's hours from a replay's recording: frames of the map tables moved to and built at
//! any hour, read from the recording's runs in place.

use crate::playback::Current;
use replay::{
    campaign::{Parallel, Replay},
    columnar_frames::Pool,
    columnar_natives::{Cursor, Natives},
};
use replay_data::{BranchId, Hour, rows::TableId, rows::tables};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    io,
    sync::{Arc, OnceLock},
};

/// The threads the map's background reads work on, below normal priority from their start, so
/// none competes with a frame the user waits for.
static BACKGROUND: OnceLock<Arc<Pool>> = OnceLock::new();

fn background() -> io::Result<Arc<Pool>> {
    if let Some(pool) = BACKGROUND.get() {
        return Ok(Arc::clone(pool));
    }
    let threads = std::thread::available_parallelism().map_or(1, usize::from);
    let pool = Arc::new(Pool::new(
        threads,
        "map background",
        viewer_foundation::worker_priority::background_priority,
    )?);
    Ok(Arc::clone(BACKGROUND.get_or_init(|| pool)))
}

/// A replay's recording, its runs placed for reading in place, shared by every playback serving
/// the replay's map.
pub struct Columnar {
    natives: Arc<Natives>,
}

impl Columnar {
    pub fn new(natives: Arc<Natives>) -> Self {
        Self { natives }
    }

    /// A playback's source of `replay`'s map, its views carrying change facts, building views
    /// on `workers` threads.
    pub fn open(&self, replay: Arc<Replay>, workers: usize) -> io::Result<MapSource> {
        let cursor = Cursor::new(
            Arc::clone(&self.natives),
            dashboard_base::map_tables::selected_tables(),
            replay.format,
            replay.capture_mode,
        )?
        .with_change_facts();
        Ok(MapSource {
            cursor,
            replay,
            workers: workers.max(1),
        })
    }
}

/// One playback's map hours from a [`Columnar`] source.
pub struct MapSource {
    cursor: Cursor,
    replay: Arc<Replay>,
    workers: usize,
}

impl MapSource {
    pub fn roster_source(&self) -> Arc<Natives> {
        Arc::clone(self.cursor.natives())
    }

    pub fn set_workers(&mut self, workers: usize) {
        self.workers = workers.max(1);
    }

    /// The hour a read of `hour` of `branch` starts from: the hour itself, which no walk of the
    /// caller's precedes; none outside every run.
    pub fn walk_start(&self, branch: BranchId, hour: Hour) -> Option<i64> {
        let segment = self.replay.segment_at(branch, hour)?;
        self.cursor
            .natives()
            .find(segment, hour)
            .map(|_| i64::from(hour))
    }

    /// `hour` of `branch`: with `materialize`, its frame; without, the move taken there and no
    /// frame built. It builds no other hour: the hours before it that a window lacks are read
    /// as hours of their own, after it, and the cursor walks on from whichever it read last.
    /// An hour no run holds is the replay's gap there; any other failure is the hour's, refused
    /// by name. Cancellation leaves the source where it stood, to be asked again.
    pub fn read(
        &mut self,
        branch: BranchId,
        hour: Hour,
        materialize: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<Current> {
        let priority = viewer_foundation::worker_priority::current_priority();
        let behind = priority.is_some_and(|priority| priority < 0);
        self.cursor.set_pool(if behind { Some(background()?) } else { None });
        let on_worker = || {};
        let parallel = Parallel {
            workers: self.workers,
            on_worker: &on_worker,
        };
        let gap = |reason: String| Current::Gap {
            branch,
            hour,
            reason,
        };
        let Some(segment) = self
            .replay
            .segment_at(branch, hour)
            .filter(|segment| self.cursor.natives().find(*segment, hour).is_some())
        else {
            return Ok(gap(crate::playback::gap_reason(&self.replay, branch, hour)));
        };
        let read = (|| -> io::Result<Option<DecodedFrame>> {
            self.cursor.move_to(segment, branch, hour, cancelled)?;
            materialize
                .then(|| self.cursor.build(Some(&parallel)))
                .transpose()
        })();
        let refused = |reason: String| Current::Refused {
            branch,
            hour,
            reason,
        };
        match read {
            Ok(Some(frame)) if campaign_holds(&frame, &self.replay.campaign_id) => {
                Ok(Current::Frame(Arc::new(frame)))
            }
            Ok(Some(_)) => Ok(refused(
                "frame campaign identity differs from replay header".into(),
            )),
            Ok(None) => Ok(gap("hour has not been decoded".into())),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => Err(error),
            Err(error) => Ok(refused(format!("the map's hour cannot be read: {error}"))),
        }
    }
}

#[cfg(test)]
mod tests;

/// Whether `frame`'s Campaign, when complete, is the replay's one campaign `campaign`.
fn campaign_holds(frame: &DecodedFrame, campaign: &[u8]) -> bool {
    match &*frame.tables[TableId::Campaign.index()] {
        TableView::Complete(rows) => rows
            .rows::<tables::Campaign>()
            .is_some_and(|rows| rows.len() == 1 && rows[0].id == campaign),
        _ => true,
    }
}
