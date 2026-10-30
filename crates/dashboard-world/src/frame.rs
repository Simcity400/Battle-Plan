//! The walk's one frame of the tables its lanes read as rows: each hour's edits noted as the
//! walker applies them, the views built only at the first ask of an hour ([`Views::build`]).
//! Tables held whole are built together. A table held by some fields is held apart, even when
//! another lane holds it whole: it is noted only at the hours that touch those fields, so its
//! other fields stay as last built and its view changes only with them.

use super::{Facts, invalid};
use replay::{
    columnar_frames::Views,
    segments::{Manifest, State},
};
use replay_data::{
    format::FormatVersion,
    rows::{TABLE_COUNT, TableId},
    scope::CaptureMode,
};
use replay_frame::frame::{DecodedFrame, Metrics, not_selected};
use std::io;

/// How the frame holds a table's rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hold {
    /// Every field current.
    Whole,
    /// Every row, current in these fields; its others as the row was last built.
    Fields(u64),
}

impl Hold {
    /// Whether rows held as `self` give every field `need` reads current.
    pub fn covers(self, need: Self) -> bool {
        match (self, need) {
            (Self::Whole, _) => true,
            (Self::Fields(held), Self::Fields(read)) => read & !held == 0,
            (Self::Fields(_), Self::Whole) => false,
        }
    }

    /// `self` of `table`, whole when its fields are all of the table's.
    pub fn of(self, table: TableId) -> Self {
        let all = table.full_mask();
        match self {
            Self::Fields(fields) if fields & all == all => Self::Whole,
            hold => hold,
        }
    }

    /// The fields a view held as `self` is noted for; every field when whole.
    fn fields(self) -> u64 {
        match self {
            Self::Whole => u64::MAX,
            Self::Fields(fields) => fields,
        }
    }
}

/// Tables noted and built together: Campaign alone, those held whole, or one held by fields,
/// noted at the hours that touch them.
struct Group {
    watch: Option<(TableId, u64)>,
    views: Views,
    /// This hour's build, once asked for.
    built: Option<DecodedFrame>,
}

pub(crate) struct Frame {
    format: FormatVersion,
    mode: CaptureMode,
    groups: Vec<Group>,
    /// Each table's groups, and the hold each keeps it in.
    held: Vec<Vec<(usize, Hold)>>,
}

impl Frame {
    /// The frame of `tables`, each once per hold as [`super::Needs::union`] leaves them.
    pub(crate) fn new(
        tables: &[(TableId, Hold)],
        format: FormatVersion,
        mode: CaptureMode,
    ) -> io::Result<Self> {
        let mut frame = Self {
            format,
            mode,
            groups: Vec::new(),
            held: engine_allocation::collect_vec((0..TABLE_COUNT).map(|_| Vec::new()))?,
        };
        // Campaign alone, which every frame holds: no ask of another table builds the others.
        frame.group(&[], None)?;
        frame.held[TableId::Campaign.index()].push((0, Hold::Whole));
        let whole = engine_allocation::collect_vec(
            tables
                .iter()
                .filter(|&&(table, hold)| hold == Hold::Whole && table != TableId::Campaign)
                .map(|&(table, _)| table),
        )?;
        if !whole.is_empty() {
            frame.group(&whole, None)?;
        }
        for &(table, hold) in tables {
            if let Hold::Fields(fields) = hold
                && table != TableId::Campaign
            {
                frame.group(&[table], Some((table, fields)))?;
            }
        }
        Ok(frame)
    }

    /// A group of `tables`, noted at the hours that touch `watch`, every hour when None.
    fn group(&mut self, tables: &[TableId], watch: Option<(TableId, u64)>) -> io::Result<()> {
        let at = self.groups.len();
        let hold = watch.map_or(Hold::Whole, |(_, fields)| Hold::Fields(fields));
        for &table in tables {
            engine_allocation::reserve(&mut self.held[table.index()], 1)?;
            self.held[table.index()].push((at, hold));
        }
        engine_allocation::reserve(&mut self.groups, 1)?;
        self.groups.push(Group {
            watch,
            views: Views::new(tables, self.format, self.mode)?,
            built: None,
        });
        Ok(())
    }

    /// The group holding `table` in the fewest fields that still covers `need`.
    fn holding(&self, table: TableId, need: Hold) -> Option<usize> {
        self.held[table.index()]
            .iter()
            .filter(|(_, hold)| hold.covers(need))
            .min_by_key(|(_, hold)| hold.fields().count_ones())
            .map(|&(at, _)| at)
    }

    /// Notes the hour's edits: a walk's first hour builds every view anew.
    pub(crate) fn note(
        &mut self,
        state: &State,
        manifest: &Manifest<'_>,
        facts: &Facts<'_>,
    ) -> io::Result<()> {
        for group in &mut self.groups {
            group.built = None;
            if let Some((table, fields)) = group.watch
                && !facts.touched(table, fields)
            {
                continue;
            }
            group.views.note(state, manifest, facts.first())?;
        }
        Ok(())
    }

    /// See [`super::World::frame_held`].
    pub(crate) fn frame(
        &mut self,
        state: &State,
        asks: &[(TableId, Hold)],
        (branch, hour): (u64, i32),
        metrics: &Metrics,
    ) -> io::Result<DecodedFrame> {
        // A table held in fewer fields than asked is held whole from now on.
        let mut missing = engine_allocation::collect_vec(
            asks.iter()
                .filter(|&&(table, need)| self.holding(table, need).is_none())
                .map(|&(table, _)| table),
        )?;
        if !missing.is_empty() {
            missing.sort_unstable();
            missing.dedup();
            self.group(&missing, None)?;
            // Built anew at this hour, from the rows the walker holds.
            self.groups
                .last_mut()
                .ok_or_else(|| invalid("A frame group was not made"))?
                .views
                .reset();
        }
        let mut frame = DecodedFrame {
            branch,
            hour,
            tables: std::array::from_fn(|_| not_selected()),
            metrics: metrics.clone(),
        };
        for &(table, need) in asks.iter().chain(&[(TableId::Campaign, Hold::Whole)]) {
            let at = self
                .holding(table, need)
                .ok_or_else(|| invalid("A frame table has no group"))?;
            let group = &mut self.groups[at];
            let built = match &mut group.built {
                Some(built) => built,
                built => built.insert(group.views.build(
                    state,
                    (branch, hour),
                    metrics.clone(),
                    None,
                )?),
            };
            frame.tables[table.index()] = built.tables[table.index()].clone();
        }
        Ok(frame)
    }
}
