//! A lane's own decoded copies of a few tables, for a lane that keeps rows past the hour that
//! read them ([`Copies`]): each table built from the walk's views in the fields it holds, and
//! built again only at an hour that may have changed them; otherwise carried, dated as the
//! walker dates it.

use super::{Hold, World};
use replay::segments::State;
use replay_data::rows::TableId;
use replay_frame::frame::{DecodedFrame, Metrics, TableView, not_selected};
use std::io;

pub struct Copies {
    asks: Vec<(TableId, Hold)>,
    frame: DecodedFrame,
    /// Whether each ask was ever copied.
    held: Vec<bool>,
}

impl Copies {
    /// Copies of `asks`, each table once, built at the first hour they are brought to.
    pub fn new(asks: Vec<(TableId, Hold)>) -> io::Result<Self> {
        Ok(Self {
            held: engine_allocation::filled(asks.len(), false)?,
            asks,
            frame: DecodedFrame {
                branch: 0,
                hour: 0,
                tables: std::array::from_fn(|_| not_selected()),
                metrics: Metrics::default(),
            },
        })
    }

    pub fn asks(&self) -> &[(TableId, Hold)] {
        &self.asks
    }

    /// The copies brought to `world`'s hour: each table copied again where the hour may have
    /// changed its held fields. Whether any was.
    pub fn hour(&mut self, world: &World, state: &State) -> io::Result<bool> {
        let (branch, hour) = world.branch_hour();
        self.frame.branch = branch;
        self.frame.hour = hour;
        self.frame.metrics.clone_from(world.metrics());
        let mut copied = false;
        for (&(table, hold), held) in self.asks.iter().zip(&mut self.held) {
            let index = table.index();
            let fields = match hold.of(table) {
                Hold::Whole => u64::MAX,
                Hold::Fields(fields) => fields,
            };
            if !*held || world.touched(table, fields) {
                self.frame.tables[index] = world.copy(state, table, hold)?;
                *held = true;
                copied = true;
            } else if matches!(&*self.frame.tables[index], TableView::Complete(_)) {
                // A carried view is dated by the table's last complete read, as the walk's are.
                self.frame.tables[index] = self.frame.tables[index]
                    .clone()
                    .with_sampled_at(state.sampled_at(index));
            }
        }
        Ok(copied)
    }

    /// The copies, dated the hour they were last brought to.
    pub fn frame(&self) -> &DecodedFrame {
        &self.frame
    }

    /// The copies, for a reader that dates them itself.
    pub fn frame_mut(&mut self) -> &mut DecodedFrame {
        &mut self.frame
    }

    /// Forgets every copy: the next hour copies each again.
    pub fn clear(&mut self) {
        self.held.fill(false);
        for &(table, _) in &self.asks {
            self.frame.tables[table.index()] = not_selected();
        }
    }
}
