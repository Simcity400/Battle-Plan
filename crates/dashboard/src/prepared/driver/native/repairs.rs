use super::backend::{Native, Projected};
use crate::prepared::driver::publication::Stage;
use dashboard_base::{
    prepared::{
        roots::Published,
        series::{compress, decompress},
    },
    products::dockyard_repairs::{
        Publisher,
        facts::{self, Hour, Projector},
    },
};
use dashboard_world::Needs;
use replay::{columnar_frames::Segment, container::Replay};
use replay_data::rows::TableId;
use std::{io, sync::atomic::AtomicBool};

impl Projected for Hour {
    fn hour(&self) -> i32 {
        self.hour
    }
}

pub(crate) struct Dockyards<'p> {
    pub(crate) replay: &'p Replay,
}
impl Native for Dockyards<'_> {
    type Operator = Projector;
    type Branch = Publisher;
    fn tables(&self) -> &'static [TableId] {
        facts::TABLES
    }
    fn needs(&self) -> io::Result<Needs> {
        Ok(facts::needs())
    }
    fn operator(&self, _: &Segment) -> io::Result<Projector> {
        Projector::new(self.replay.format)
    }
    fn branch(&self, _: u64, _: i32) -> io::Result<Publisher> {
        Ok(Publisher::default())
    }
    fn observe(
        &self,
        state: &mut Publisher,
        _: &mut Stage<'_, '_>,
        branch: u64,
        mut hour: Hour,
    ) -> io::Result<()> {
        state.observe(self.replay, branch, (&mut hour).into())
    }
    fn flush(
        &self,
        state: &mut Publisher,
        stage: &mut Stage<'_, '_>,
        branch: u64,
        _: Option<i32>,
        cancelled: &AtomicBool,
    ) -> io::Result<()> {
        state.flush(stage.pages()?, branch, cancelled)
    }
    fn save(&self, state: &mut Publisher) -> io::Result<Option<Vec<u8>>> {
        compress(&serde_json::to_vec(state)?, 3).map(Some)
    }
    fn resume(
        &self,
        _: &mut Stage<'_, '_>,
        _: u64,
        _: &Published,
        saved: &[u8],
    ) -> io::Result<Publisher> {
        Ok(serde_json::from_slice(&decompress(saved)?)?)
    }
}
