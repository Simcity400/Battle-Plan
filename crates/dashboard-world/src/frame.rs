//! The walk's one frame of the tables its lanes read as rows: each hour's edits noted as the
//! walker applies them, each table's view built only at its first ask of an hour, the tables an
//! ask builds of a group built in one pass ([`Views::update_tables`]). The tables held whole are
//! one group, Campaign among them, and those held by some fields another. A table held by some
//! fields is held apart, even when another lane holds it whole: it is noted only at the hours
//! that touch those fields, so its other fields stay as last built and its view changes only
//! with them.

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
use replay_frame::frame::DecodedFrame;
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

/// Tables noted and built together: each noted every hour when held whole, else at the hours
/// that touch its fields, and built at its first ask of an hour.
struct Group {
    views: Views,
    /// Each table's fields as the group holds it, by table: every field when whole.
    fields: Vec<u64>,
    /// Whether each table's view is built for this hour, by table.
    built: Vec<bool>,
    /// The tables an ask builds, kept from ask to ask.
    wanted: Vec<TableId>,
}

pub(crate) struct Frame {
    format: FormatVersion,
    mode: CaptureMode,
    groups: Vec<Group>,
    /// Each table's groups, and the hold each keeps it in.
    held: Vec<Vec<(usize, Hold)>>,
    /// Each asked table's group, kept from ask to ask.
    placed: Vec<(TableId, usize)>,
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
            placed: Vec::new(),
        };
        // Campaign, which every frame holds, among the tables held whole.
        let mut whole = engine_allocation::collect_vec(
            tables
                .iter()
                .filter(|&&(table, hold)| hold == Hold::Whole && table != TableId::Campaign)
                .copied(),
        )?;
        engine_allocation::reserve(&mut whole, 1)?;
        whole.push((TableId::Campaign, Hold::Whole));
        frame.group(&whole)?;
        let fields = engine_allocation::collect_vec(
            tables
                .iter()
                .filter(|&&(table, hold)| {
                    matches!(hold, Hold::Fields(_)) && table != TableId::Campaign
                })
                .copied(),
        )?;
        if !fields.is_empty() {
            frame.group(&fields)?;
        }
        Ok(frame)
    }

    /// A group of `tables`, each in its hold.
    fn group(&mut self, tables: &[(TableId, Hold)]) -> io::Result<()> {
        let at = self.groups.len();
        let mut fields = engine_allocation::filled(TABLE_COUNT, 0)?;
        for &(table, hold) in tables {
            fields[table.index()] = hold.fields();
            engine_allocation::reserve(&mut self.held[table.index()], 1)?;
            self.held[table.index()].push((at, hold));
        }
        let selected = engine_allocation::collect_vec(tables.iter().map(|&(table, _)| table))?;
        engine_allocation::reserve(&mut self.groups, 1)?;
        self.groups.push(Group {
            views: Views::new(&selected, self.format, self.mode)?,
            fields,
            built: engine_allocation::filled(TABLE_COUNT, false)?,
            wanted: Vec::new(),
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
        for Group {
            views,
            fields,
            built,
            ..
        } in &mut self.groups
        {
            built.fill(false);
            views.note_tables(state, manifest, facts.first(), &|table| {
                let fields = fields[table.index()];
                fields == u64::MAX || facts.touched(table, fields)
            })?;
        }
        Ok(())
    }

    /// See [`super::World::frame_held_into`]: `frame` already dated `hour`.
    pub(crate) fn frame(
        &mut self,
        state: &State,
        asks: &[(TableId, Hold)],
        hour: i32,
        frame: &mut DecodedFrame,
    ) -> io::Result<()> {
        // A table held in fewer fields than asked is held whole from now on.
        let mut missing = engine_allocation::collect_vec(
            asks.iter()
                .filter(|&&(table, need)| self.holding(table, need).is_none())
                .map(|&(table, _)| (table, Hold::Whole)),
        )?;
        if !missing.is_empty() {
            missing.sort_unstable_by_key(|&(table, _)| table);
            missing.dedup();
            self.group(&missing)?;
            // Built anew at this hour, from the rows the walker holds.
            self.groups
                .last_mut()
                .ok_or_else(|| invalid("A frame group was not made"))?
                .views
                .reset();
        }
        self.placed.clear();
        for group in &mut self.groups {
            group.wanted.clear();
        }
        engine_allocation::reserve(&mut self.placed, asks.len() + 1)?;
        for &(table, need) in asks.iter().chain(&[(TableId::Campaign, Hold::Whole)]) {
            let at = self
                .holding(table, need)
                .ok_or_else(|| invalid("A frame table has no group"))?;
            let group = &mut self.groups[at];
            if !group.built[table.index()] {
                engine_allocation::reserve(&mut group.wanted, 1)?;
                group.wanted.push(table);
            }
            self.placed.push((table, at));
        }
        for group in &mut self.groups {
            if group.wanted.is_empty() {
                continue;
            }
            group
                .views
                .update_tables(state, hour, &group.wanted, None)?;
            for &table in &group.wanted {
                group.built[table.index()] = true;
            }
        }
        for &(table, at) in &self.placed {
            frame.tables[table.index()] = self.groups[at]
                .views
                .view(table)
                .ok_or_else(|| invalid("A frame table was not built"))?
                .clone();
        }
        Ok(())
    }
}
