use replay::campaign::Replay;
use replay_recording::{columnar_frames::Pool, columnar_natives::{Cursor, Natives}};
use replay_data::{BranchId, Hour, rows::TableId, rows::tables};
use replay_frame::frame::{DecodedFrame, TableView};
use std::{
    io,
    sync::{Arc, OnceLock},
};

static BACKGROUND: OnceLock<Arc<Pool>> = OnceLock::new();

fn background() -> io::Result<Arc<Pool>> {
    if let Some(pool) = BACKGROUND.get() {
        return Ok(Arc::clone(pool));
    }
    let pool = Arc::new(Pool::new(
        viewer_runtime::retirement::builder_count(),
        "map background",
        viewer_runtime::worker_priority::background_priority,
    )?);
    Ok(Arc::clone(BACKGROUND.get_or_init(|| pool)))
}

pub struct Columnar {
    natives: Arc<Natives>,
}

impl Columnar {
    pub fn new(natives: Arc<Natives>) -> Self {
        Self { natives }
    }

    pub fn open(&self, replay: Arc<Replay>) -> io::Result<MapSource> {
        let cursor = Cursor::new(
            Arc::clone(&self.natives),
            dashboard_base::map_tables::selected_tables(),
        )?
        .with_change_facts();
        Ok(MapSource { cursor, replay })
    }
}

pub struct MapSource {
    cursor: Cursor,
    replay: Arc<Replay>,
}

impl MapSource {
    pub fn roster_source(&self) -> Arc<Natives> {
        Arc::clone(self.cursor.natives())
    }

    pub fn read(
        &mut self,
        branch: BranchId,
        hour: Hour,
        materialize: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<Current> {
        let priority = viewer_runtime::worker_priority::current_priority();
        let behind = priority.is_some_and(|priority| priority < 0);
        self.cursor.set_pool(if behind { Some(background()?) } else { None });
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
            return Ok(gap(gap_reason(&self.replay, branch, hour)));
        };
        let read = (|| -> io::Result<Option<DecodedFrame>> {
            self.cursor.move_to(segment, branch, hour, cancelled)?;
            materialize.then(|| self.cursor.build()).transpose()
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

fn campaign_holds(frame: &DecodedFrame, campaign: &[u8]) -> bool {
    match &*frame.tables[TableId::Campaign.index()] {
        TableView::Complete(rows) => rows
            .rows::<tables::Campaign>()
            .is_some_and(|rows| rows.len() == 1 && rows[0].id == campaign),
        _ => true,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Current {
    Frame(Arc<DecodedFrame>),
    Gap {
        branch: BranchId,
        hour: Hour,
        reason: String,
    },
    Refused {
        branch: BranchId,
        hour: Hour,
        reason: String,
    },
}

pub fn gap_reason(replay: &Replay, branch: BranchId, hour: Hour) -> String {
    let reason = replay
        .gaps
        .iter()
        .filter(|gap| gap.branch == branch && gap.first_hour <= hour && hour <= gap.last_hour)
        .map(|gap| gap.reason.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    if reason.is_empty() {
        "hour was not recorded".into()
    } else {
        reason
    }
}
