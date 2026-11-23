
use super::{Natives, invalid};
use crate::columnar_frames::{Frames, Pool, Recording};
use replay_frame::frame::DecodedFrame;
use replay_data::rows::TableId;
use replay_columnar::manifest::Status;
use std::{io, ops::Deref, sync::Arc};

struct Held(Arc<Natives>);

impl Deref for Held {
    type Target = Recording;

    fn deref(&self) -> &Recording {
        self.0.recording()
    }
}

pub struct Cursor {
    natives: Arc<Natives>,
    tables: Vec<TableId>,
    frames: Frames<Held>,
    moved: Option<(u64, i32)>,
    pool: Option<Arc<Pool>>,
    changes: bool,
}

impl Cursor {
    pub fn new(natives: Arc<Natives>, tables: &[TableId]) -> io::Result<Self> {
        let frames = Frames::new(Held(Arc::clone(&natives)), tables)?;
        let tables = frames.selected().collect();
        Ok(Self { natives, tables, frames, moved: None, pool: None, changes: false })
    }

    pub fn with_change_facts(mut self) -> Self {
        self.changes = true;
        self.frames = self.frames.with_change_facts();
        self
    }

    pub fn set_pool(&mut self, pool: Option<Arc<Pool>>) {
        self.pool = pool;
    }

    pub fn natives(&self) -> &Arc<Natives> {
        &self.natives
    }

    pub fn tables(&self) -> &[TableId] {
        &self.tables
    }

    fn reset(&mut self) -> io::Result<()> {
        self.moved = None;
        let frames = Frames::new(Held(Arc::clone(&self.natives)), &self.tables)?;
        self.frames = if self.changes { frames.with_change_facts() } else { frames };
        Ok(())
    }

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

    pub fn build(&mut self) -> io::Result<DecodedFrame> {
        let (branch, _) = self.moved.ok_or_else(|| invalid("A frame is built before any move"))?;
        let frames = &mut self.frames;
        let built = Pool::on(self.pool.as_deref(), || frames.frame(branch));
        if built.is_err() {
            self.reset()?;
        }
        built
    }

    pub fn frame(&mut self, segment: u64, branch: u64, hour: i32) -> io::Result<Option<DecodedFrame>> {
        if !self.move_to(segment, branch, hour, &|| false)? {
            return Ok(None);
        }
        self.build().map(Some)
    }

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
