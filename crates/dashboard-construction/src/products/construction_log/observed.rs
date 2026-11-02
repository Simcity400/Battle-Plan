//! What the scan reads of an hour ([`super::Scan::observe_changed`]): each building table's
//! levels, the queues and the output at their reads with every line's owner resolved, and the
//! spellings of the building types the lines name. The columnar operator ([`super::facts`])
//! builds it from a segment's walk; the row oracle builds it from decoded frames.

use super::SpendingGap;
use replay_data::rows::{
    BuildingConstructionRow, ConstructionLineOutputRow, RailwayConstructionRow,
};
use std::{mem::size_of, sync::Arc};

/// A building's levels as its table holds it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Building {
    pub location: u32,
    pub kind: u32,
    pub level: i16,
    pub healthy: i16,
}

/// A building table this hour.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum Buildings {
    /// It does not hold every row.
    #[default]
    Incomplete,
    /// It holds every row, and no building's levels changed since the hour before.
    Unchanged,
    /// It holds every row: each building it holds.
    Whole(Vec<Building>),
    /// It holds every row: each building whose levels changed since the hour before, at its
    /// levels now (none, once the table no longer holds it).
    Changed(Vec<Building>),
}

impl Buildings {
    fn heap(&self) -> usize {
        match self {
            Self::Whole(rows) | Self::Changed(rows) => rows.capacity() * size_of::<Building>(),
            Self::Incomplete | Self::Unchanged => 0,
        }
    }
}

/// A building line, its owner's tag resolved where the links resolve its country.
#[derive(Clone, Debug)]
pub struct BuildingLine {
    pub owner: Option<Arc<str>>,
    pub row: BuildingConstructionRow,
}

/// A railway line, its owner resolved, and the states of its route's first and last provinces.
#[derive(Clone, Debug)]
pub struct RailwayLine {
    pub owner: Option<Arc<str>>,
    pub from: Option<u32>,
    pub to: Option<u32>,
    pub row: RailwayConstructionRow,
}

#[derive(Clone, Debug)]
pub struct OutputLine {
    pub row: ConstructionLineOutputRow,
    pub owner: Option<Arc<str>>,
    pub from: Option<u32>,
    pub to: Option<u32>,
}

/// A queue at its slot's read.
#[derive(Debug, Default)]
pub enum Queue<T> {
    /// Not read this hour.
    #[default]
    NotRead,
    /// Read, but the table did not hold every row.
    Failed,
    /// Every line, by id.
    Read(Vec<T>),
}

impl<T> Queue<T> {
    pub fn read(&self) -> bool {
        !matches!(self, Self::NotRead)
    }

    fn lines(&self) -> &[T] {
        match self {
            Self::Read(lines) => lines,
            Self::NotRead | Self::Failed => &[],
        }
    }
}

/// The output table at its read.
#[derive(Debug, Default)]
pub enum Output {
    #[default]
    NotRead,
    /// Read while not complete, and why.
    Unavailable(SpendingGap),
    /// Every row, by line; a row's completion is read only where it ended.
    Read(Vec<OutputLine>),
}

impl Output {
    pub fn read(&self) -> bool {
        !matches!(self, Self::NotRead)
    }
}

/// One hour of a branch.
#[derive(Debug, Default)]
pub struct Hour {
    pub branch: u64,
    pub hour: i32,
    /// StateBuildings and ProvinceBuildings.
    pub buildings: [Buildings; 2],
    pub queue: Queue<BuildingLine>,
    pub railways: Queue<RailwayLine>,
    pub output: Output,
    /// Each building type a line names, where it is first named in its segment or a later
    /// complete dictionary first spells it: its spelling, none where no complete dictionary does.
    pub types: Vec<(u32, Option<String>)>,
}

impl replay::segments::Item for Hour {
    fn heap(&self) -> usize {
        let tag = |owner: &Option<Arc<str>>| owner.as_ref().map_or(0, |tag| tag.len());
        self.buildings.iter().map(Buildings::heap).sum::<usize>()
            + self.queue.lines().len() * size_of::<BuildingLine>()
            + self
                .queue
                .lines()
                .iter()
                .map(|line| tag(&line.owner))
                .sum::<usize>()
            + self
                .railways
                .lines()
                .iter()
                .map(|line| {
                    size_of::<RailwayLine>() + tag(&line.owner) + line.row.route.capacity() * 4
                })
                .sum::<usize>()
            + match &self.output {
                Output::Read(rows) => {
                    rows.capacity() * size_of::<OutputLine>()
                        + rows.iter().map(|line| tag(&line.owner)).sum::<usize>()
                }
                Output::NotRead | Output::Unavailable(_) => 0,
            }
            + self
                .types
                .iter()
                .map(|(_, token)| {
                    size_of::<(u32, Option<String>)>() + token.as_ref().map_or(0, String::capacity)
                })
                .sum::<usize>()
    }
}
