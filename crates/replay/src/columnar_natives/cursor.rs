//! The map's hour pipeline: frames of some tables at any hour, read through [`Frames`], the
//! next hour of a run a step, a later one an advance from where each table stands unless a
//! restore of it lies between, any other a seek.

use super::{Natives, invalid};
use crate::{
    campaign::Parallel,
    columnar_frames::{Frames, Pool, Recording},
    format::FormatVersion,
    frame::DecodedFrame,
    rows::TableId,
    scope::CaptureMode,
};
use replay_columnar::manifest::Status;
use std::{io, ops::Deref, sync::Arc};

/// The recording [`Natives`] hold, for a cursor's frames to read.
struct Held(Arc<Natives>);

impl Deref for Held {
    type Target = Recording;

    fn deref(&self) -> &Recording {
        self.0.recording()
    }
}

/// Frames of some tables at any hour [`Natives`] hold.
pub struct Cursor {
    natives: Arc<Natives>,
    tables: Vec<TableId>,
    frames: Frames<Held>,
    /// The hour moved to last, which a build builds; none after a move or build failed.
    moved: Option<(u64, i32)>,
    pool: Option<Arc<Pool>>,
    /// Its frames' complete views carry change facts and field stamps.
    changes: bool,
}

impl Cursor {
    /// Frames of `tables`; the recording's own format and capture mode frame them, `format` and
    /// `mode` being the replay's same.
    pub fn new(natives: Arc<Natives>, tables: &[TableId], format: FormatVersion, mode: CaptureMode) -> io::Result<Self> {
        let _ = (format, mode);
        let frames = Frames::new(Held(Arc::clone(&natives)), tables)?;
        let tables = frames.selected().collect();
        Ok(Self { natives, tables, frames, moved: None, pool: None, changes: false })
    }

    /// Frames whose complete views carry change facts and field stamps: an hour stepped to from
    /// the one built last names what it changed ([`Frames::with_change_facts`]).
    pub fn with_change_facts(mut self) -> Self {
        self.changes = true;
        self.frames = self.frames.with_change_facts();
        self
    }

    /// Moves and builds work on `pool`'s threads from now on, or on rayon's.
    pub fn set_pool(&mut self, pool: Option<Arc<Pool>>) {
        self.pool = pool;
    }

    pub fn natives(&self) -> &Arc<Natives> {
        &self.natives
    }

    /// The tables its frames hold, in table order.
    pub fn tables(&self) -> &[TableId] {
        &self.tables
    }

    /// Readers left part way by a failed move or build are let go: the next move seeks anew.
    fn reset(&mut self) -> io::Result<()> {
        self.moved = None;
        let frames = Frames::new(Held(Arc::clone(&self.natives)), &self.tables)?;
        self.frames = if self.changes { frames.with_change_facts() } else { frames };
        Ok(())
    }

    /// Moves to `hour` of `branch`, read from the runs of recorded segment `segment`, building
    /// no frame: false when the recording holds no such hour. `cancelled` is asked before the
    /// move; a move it stops is interrupted and leaves the cursor where it was.
    pub fn move_to(&mut self, segment: u64, branch: u64, hour: i32, cancelled: &dyn Fn() -> bool) -> io::Result<bool> {
        let Some(place) = self.natives.find(segment, hour) else {
            return Ok(false);
        };
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let frames = &mut self.frames;
        let moved = Pool::on(self.pool.as_deref(), || frames.go_to(place.run, place.ordinal));
        if let Err(error) = moved {
            self.reset()?;
            return Err(error);
        }
        self.moved = Some((branch, hour));
        Ok(true)
    }

    /// The frame of the hour moved to last. Views are built on rayon's threads, or the pool's;
    /// `parallel` is not used.
    pub fn build(&mut self, parallel: Option<&Parallel<'_>>) -> io::Result<DecodedFrame> {
        let _ = parallel;
        let (branch, _) = self.moved.ok_or_else(|| invalid("A frame is built before any move"))?;
        let frames = &mut self.frames;
        let built = Pool::on(self.pool.as_deref(), || frames.frame(branch));
        if built.is_err() {
            self.reset()?;
        }
        built
    }

    /// The tables' frame at `hour` of `branch`, read from recorded segment `segment`; none when
    /// the recording holds no such hour.
    pub fn frame(&mut self, segment: u64, branch: u64, hour: i32, parallel: Option<&Parallel<'_>>) -> io::Result<Option<DecodedFrame>> {
        if !self.move_to(segment, branch, hour, &|| false)? {
            return Ok(None);
        }
        self.build(parallel).map(Some)
    }

    /// Whether `table`, one of its tables, is complete at the hour moved to last, as its view
    /// there would be: read whole, and neither withheld nor damaged.
    pub fn complete(&self, table: TableId) -> io::Result<bool> {
        if self.moved.is_none() {
            return Err(invalid("A table's status is read before any move"));
        }
        if !self.tables.contains(&table) {
            return Err(invalid(format!("{table:?} is not read by this cursor")));
        }
        Ok(self.frames.reader(table).is_some_and(|reader| reader.status() == Status::Complete))
    }
}
