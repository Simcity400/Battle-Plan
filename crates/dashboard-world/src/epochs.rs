//! What a reader shared by the row path and a walk holds in place of a view's identity: each
//! table's epoch ([`Epochs`]), moved on at every hour that may have changed it in the fields the
//! lane reads of it, so an equal epoch tells the table unchanged since, as a kept copy's view
//! told it; and one hour's tables with how they tell a table unchanged ([`Viewed`]).

use super::{Hold, Lexicon, World};
use replay_data::rows::{TABLE_COUNT, TableId};
use replay_frame::frame::{DecodedFrame, HourFrame, SharedView, ViewIdentity};
use std::io;

pub struct Epochs {
    /// Each table followed, with the fields a change to which moves it on.
    fields: Vec<(TableId, u64)>,
    /// The tables followed that only [`Self::moved`] moves on, past the first hour.
    manual: Vec<TableId>,
    epochs: Vec<u64>,
    next: u64,
    /// Before the first hour, or after a restart: every table moves on at the next.
    cold: bool,
}

impl Epochs {
    /// The epochs of `asks` and Campaign, each table once, in the fields its holds name.
    pub fn new(asks: &[(TableId, Hold)]) -> io::Result<Self> {
        let mut fields: Vec<(TableId, u64)> = Vec::new();
        for &(table, hold) in asks.iter().chain(&[(TableId::Campaign, Hold::Whole)]) {
            let read = match hold.of(table) {
                Hold::Whole => u64::MAX,
                Hold::Fields(read) => read,
            };
            match fields.iter_mut().find(|(held, _)| *held == table) {
                Some((_, held)) => *held |= read,
                None => {
                    engine_allocation::reserve(&mut fields, 1)?;
                    fields.push((table, read));
                }
            }
        }
        Ok(Self {
            fields,
            manual: Vec::new(),
            epochs: engine_allocation::filled(TABLE_COUNT, 0)?,
            next: 1,
            cold: true,
        })
    }

    /// Also follows `tables`, moved on only by [`Self::moved`] past the first hour: tables a
    /// reader holds its own rows of, which it tells changed itself.
    pub fn moved_only(mut self, tables: &[TableId]) -> io::Result<Self> {
        engine_allocation::reserve(&mut self.manual, tables.len())?;
        self.manual.extend_from_slice(tables);
        Ok(self)
    }

    /// Moves on each table `world`'s hour may have changed. Ask it every hour of the walk, or an
    /// epoch misses the hours between.
    pub fn hour(&mut self, world: &World) {
        let cold = std::mem::take(&mut self.cold);
        for &(table, fields) in &self.fields {
            if cold || world.touched(table, fields) {
                self.epochs[table.index()] = self.next;
                self.next += 1;
            }
        }
        if cold {
            for &table in &self.manual {
                self.epochs[table.index()] = self.next;
                self.next += 1;
            }
        }
    }

    fn follows(&self, table: TableId) -> bool {
        self.manual.contains(&table) || self.fields.iter().any(|(held, _)| *held == table)
    }

    /// Moves `table` on, if followed: a reader changed which of its rows it holds.
    pub fn moved(&mut self, table: TableId) {
        if self.follows(table) {
            self.epochs[table.index()] = self.next;
            self.next += 1;
        }
    }

    /// Every table moves on at the next hour: what was held before is not this walk's.
    pub fn restart(&mut self) {
        self.cold = true;
    }

    /// `table`'s epoch; none for a table not followed, which no epoch tells unchanged.
    pub fn get(&self, table: TableId) -> Option<u64> {
        self.follows(table).then(|| self.epochs[table.index()])
    }

    /// Whether `table` is followed and `epoch` is its epoch now: unchanged since it was read.
    pub fn same(&self, table: TableId, epoch: Option<u64>) -> bool {
        epoch.is_some() && self.get(table) == epoch
    }
}

/// A table as an hour held it: a decoded view's identity, or a walked table's epoch.
#[derive(Clone)]
#[derive(replay_spool::Spooled)]
pub enum Mark {
    View(Option<ViewIdentity>),
    Epoch(Option<u64>),
}

/// One hour's tables, and how they tell a table unchanged since an hour that held it: a decoded
/// frame by its views, a walk by its tables' epochs.
#[derive(Clone, Copy)]
pub struct Viewed<'f> {
    pub frame: HourFrame<'f>,
    pub epochs: Option<&'f Epochs>,
    /// The walk's lexer folds ([`World::lexicon`]); none on a decoded frame.
    pub lexicon: Option<&'f Lexicon>,
}

impl<'f> From<&'f DecodedFrame> for Viewed<'f> {
    fn from(frame: &'f DecodedFrame) -> Self {
        Self::decoded(frame)
    }
}

impl<'f> Viewed<'f> {
    pub fn decoded(frame: &'f DecodedFrame) -> Self {
        Self {
            frame: HourFrame::from(frame),
            epochs: None,
            lexicon: None,
        }
    }

    /// A walk's hour, its tables told unchanged by `epochs`.
    pub fn walked(world: &'f World, state: &'f replay::segments::State, epochs: &'f Epochs) -> Self {
        Self {
            frame: HourFrame::from(world.columns(state)),
            epochs: Some(epochs),
            lexicon: world.lexicon(),
        }
    }

    pub fn hour(self) -> i32 {
        self.frame.hour()
    }

    /// A decoded frame's view of `table`.
    pub fn view(self, table: TableId) -> Option<&'f SharedView> {
        self.frame
            .decoded()
            .map(|frame| &frame.decoded_frame().tables[table.index()])
    }

    pub fn mark(self, table: TableId) -> Mark {
        match self.epochs {
            Some(epochs) => Mark::Epoch(epochs.get(table)),
            None => Mark::View(self.frame.identity(table)),
        }
    }

    /// Whether `table` is the one `mark` held.
    pub fn is(self, table: TableId, mark: &Mark) -> bool {
        match (mark, self.epochs) {
            (Mark::Epoch(epoch), Some(epochs)) => epochs.same(table, *epoch),
            (Mark::View(Some(identity)), None) => {
                self.view(table).is_some_and(|view| identity.is(view))
            }
            _ => false,
        }
    }
}
