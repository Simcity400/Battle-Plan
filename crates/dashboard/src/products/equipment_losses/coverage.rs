//! What the equipment-loss writer recorded of its own coverage (`EquipmentLossCoverage`): the
//! run's row (the drain's reason each hour, the records not read, the first complete drain, the
//! hour the drain was withheld or became a lower bound, unresolved references) and the identity's
//! row (Combat Log land coverage, fallen convoy counts, ended lend-lease exchanges), read as their
//! history over a window and folded into the reply's spans.

use replay::{
    columnar_frames::Recording,
    lanes::{Cell, Keys, history},
};
use replay_data::rows::{EQUIPMENT_DESTRUCTION_PARTIAL_REASON, EquipmentLossCoverageRow as Row, TableId};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};
use ts_rs::TS;

/// Why a cause's amounts are unknown over a span of hours.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash, TS)]
#[serde(rename_all = "camelCase")]
pub enum UnknownReason {
    /// The cause's record was not read completely at these hours.
    Unread,
    /// A lend-lease exchange ended after its last read: the total is a lower bound from here on.
    EndedAfterLastRead,
    DestructionWithheld,
    DestructionGap,
    DestructionUnresolved,
    DestructionDomain,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub struct UnknownSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: UnknownReason,
    pub detail: Option<String>,
}

/// Combat Log land coverage at an hour.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub enum Coverage {
    Logged,
    NotLogged,
    Partial { ungrouped: u32 },
    Unknown,
    PruneUnobserved,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct CoverageSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub coverage: Coverage,
}

/// The drain at a span of hours: null for a complete drain, else the writer's reason.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, TS)]
#[serde(rename_all = "camelCase")]
pub struct DestructionCoverageSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: Option<String>,
}

/// The run row's cells over a span of hours.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunCells {
    /// Empty for a complete drain; none when the cell is not recorded.
    pub drain: Option<String>,
    pub unread: Option<u8>,
    pub counted_from: Option<i32>,
    pub withheld_from: Option<i32>,
    pub incomplete_from: Option<i32>,
    pub unresolved: Option<u32>,
    pub unresolved_all: Option<u32>,
}

/// The identity's row's cells over a span of hours.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryCells {
    pub land: Option<u8>,
    pub ungrouped: Option<u32>,
    pub convoy_drops: Option<u32>,
    pub exchanges_ended: Option<u32>,
    pub ended_from: Option<i32>,
}

/// Runs of a row: first hour, last hour, its cells; none where the row was not shown (its table
/// unavailable or the row not live).
pub type Runs<T> = Vec<(i32, i32, Option<T>)>;

const NOT_AVAILABLE: &str = "The equipment-loss tables were not available at these hours";
const DRAIN_TEXT_UNREAD: &str = "The drain's reason was not read at these hours";
const COLUMNS: [usize; 15] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14];

/// The coverage of a window, stretch after stretch.
#[derive(Clone, Debug, Default)]
pub struct Held {
    pub run: Runs<RunCells>,
    pub country: Runs<CountryCells>,
}

fn number(cell: Option<&Cell>) -> Option<i64> {
    match cell {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

fn hour(cell: Option<&Cell>) -> Option<i32> {
    number(cell).map(|value| value as i32)
}

impl Held {
    /// Both rows of `table` over `first..=last` of run `run`: their cells at each hour one changes.
    pub fn read(&mut self, recording: &Recording, run: u32, country: u128, whole: u128, first: i32, last: i32) -> io::Result<()> {
        let keys = [country.min(whole), country.max(whole)];
        let read = dashboard_query::cost::storage(|| history(recording, run, TableId::EquipmentLossCoverage, &COLUMNS, Keys::These(&keys), first, last))?;
        let at = |column: usize| read.columns.binary_search(&column).ok();
        for (key, steps) in &read.keys {
            for (index, step) in steps.iter().enumerate() {
                let end = steps.get(index + 1).map_or(last, |next| next.hour - 1);
                let cell = |column: usize| at(column).and_then(|at| step.cells.as_ref().and_then(|cells| cells.get(at)));
                if *key == whole {
                    let cells = step.cells.as_ref().map(|_| RunCells {
                        drain: match cell(4) {
                            Some(&Cell::Text(index)) => read.text(Cell::Text(index)).map(|bytes| String::from_utf8_lossy(bytes).into_owned()),
                            Some(Cell::Unrecorded | Cell::Missing) | None => None,
                            Some(_) => Some(DRAIN_TEXT_UNREAD.to_owned()),
                        },
                        unread: number(cell(5)).map(|value| value as u8),
                        counted_from: hour(cell(6)),
                        withheld_from: hour(cell(7)),
                        incomplete_from: hour(cell(8)),
                        unresolved: number(cell(13)).map(|value| value as u32),
                        unresolved_all: number(cell(14)).map(|value| value as u32),
                    });
                    self.run.push((step.hour, end, cells));
                } else if *key == country {
                    let cells = step.cells.as_ref().map(|_| CountryCells {
                        land: number(cell(0)).map(|value| value as u8),
                        ungrouped: number(cell(1)).map(|value| value as u32),
                        convoy_drops: number(cell(3)).map(|value| value as u32),
                        exchanges_ended: number(cell(11)).map(|value| value as u32),
                        ended_from: hour(cell(12)),
                    });
                    self.country.push((step.hour, end, cells));
                }
            }
        }
        Ok(())
    }

    fn last_run(&self) -> Option<&RunCells> {
        self.run.iter().rev().find_map(|(_, _, cells)| cells.as_ref())
    }

    /// The first complete drain the window's runs counted from.
    pub fn counted_from(&self) -> Option<i32> {
        self.run.iter().filter_map(|(_, _, cells)| cells.as_ref()?.counted_from).min()
    }

    /// The first hour a counted drain was lost or partly unknown, at the window's end.
    pub fn incomplete_from(&self) -> Option<i32> {
        self.last_run()?.incomplete_from
    }

    /// The drain at each recorded hour from the first complete drain: null when complete.
    pub fn destruction_coverage(&self) -> Vec<DestructionCoverageSpan> {
        let counted = self.counted_from();
        let mut out: Vec<DestructionCoverageSpan> = Vec::new();
        for (first, last, cells) in &self.run {
            let Some(drain) = cells.as_ref().and_then(|cells| cells.drain.clone()) else { continue };
            let first = match counted {
                Some(counted) if *last < counted => continue,
                Some(counted) => (*first).max(counted),
                None => continue,
            };
            let reason = (!drain.is_empty()).then_some(drain);
            match out.last_mut() {
                Some(held) if held.reason == reason && held.last_hour + 1 == first => held.last_hour = *last,
                _ => out.push(DestructionCoverageSpan { first_hour: first, last_hour: *last, reason }),
            }
        }
        out
    }

    /// One span per distinct drain reason and detail from its first hour after counting starts
    /// to `last_hour`: missing drains never come back.
    pub fn destruction_unknown(&self, last_hour: i32) -> Vec<UnknownSpan> {
        let Some(counted) = self.counted_from() else { return Vec::new() };
        let mut first: BTreeMap<(UnknownReason, String), i32> = BTreeMap::new();
        let mut note = |reason: UnknownReason, detail: &str, hour: i32| {
            let at = first.entry((reason, detail.to_owned())).or_insert(hour);
            *at = (*at).min(hour);
        };
        for (start, end, cells) in &self.run {
            if *end < counted {
                continue;
            }
            let start = (*start).max(counted);
            match cells {
                None => note(UnknownReason::DestructionGap, NOT_AVAILABLE, start),
                Some(cells) => {
                    for part in cells.drain.as_deref().unwrap_or_default().split("; ").filter(|part| !part.is_empty()) {
                        note(classify(part), part, start);
                    }
                    if let Some(withheld) = cells.withheld_from {
                        note(UnknownReason::DestructionWithheld, Row::DRAIN_TRUNCATED, withheld.max(counted));
                    }
                }
            }
        }
        let mut out: Vec<UnknownSpan> = first
            .into_iter()
            .map(|((reason, detail), first_hour)| UnknownSpan { first_hour, last_hour: last_hour.max(first_hour), reason, detail: Some(detail) })
            .collect();
        out.sort_unstable();
        out
    }

    /// The hours a record (its `UNREAD_*` bit) was not read completely, or the tables were not
    /// available.
    pub fn unread(&self, bit: u8, last_hour: i32) -> Vec<UnknownSpan> {
        let mut out: Vec<UnknownSpan> = Vec::new();
        for (first, last, cells) in &self.run {
            let detail = match cells {
                None => Some(NOT_AVAILABLE.to_owned()),
                Some(RunCells { unread: Some(mask), .. }) if mask & bit != 0 => None,
                Some(RunCells { unread: None, .. }) => Some(NOT_AVAILABLE.to_owned()),
                Some(_) => continue,
            };
            let last = (*last).min(last_hour);
            match out.last_mut() {
                Some(held) if held.detail == detail && held.last_hour + 1 == *first => held.last_hour = last,
                _ => out.push(UnknownSpan { first_hour: *first, last_hour: last, reason: UnknownReason::Unread, detail }),
            }
        }
        out
    }

    /// One span per lend-lease exchange that ended after its last read, from the hour its batch is
    /// lost from.
    pub fn ended(&self, last_hour: i32) -> Vec<UnknownSpan> {
        let mut out = Vec::new();
        let mut before: Option<u32> = None;
        for (first, _, cells) in &self.country {
            let Some(cells) = cells else { continue };
            let Some(now) = cells.exchanges_ended else { continue };
            let rise = match before {
                Some(before) if now >= before => now - before,
                _ => now,
            };
            let from = cells.ended_from.unwrap_or(*first);
            for _ in 0..rise {
                out.push(UnknownSpan { first_hour: from, last_hour: last_hour.max(from), reason: UnknownReason::EndedAfterLastRead, detail: None });
            }
            before = Some(now);
        }
        out
    }

    /// Combat Log land coverage by hour; an hour the identity's row is not shown is unknown.
    pub fn land(&self) -> Vec<CoverageSpan> {
        let mut out: Vec<CoverageSpan> = Vec::new();
        for (first, last, cells) in &self.country {
            let coverage = match cells.as_ref().map(|cells| (cells.land, cells.ungrouped)) {
                Some((Some(Row::LOGGED), _)) => Coverage::Logged,
                Some((Some(Row::NOT_LOGGED), _)) => Coverage::NotLogged,
                Some((Some(Row::PARTIAL), Some(ungrouped))) if ungrouped > 0 => Coverage::Partial { ungrouped },
                Some((Some(Row::PRUNE_UNOBSERVED), _)) => Coverage::PruneUnobserved,
                _ => Coverage::Unknown,
            };
            match out.last_mut() {
                Some(held) if held.coverage == coverage && held.last_hour + 1 == *first => held.last_hour = *last,
                _ => out.push(CoverageSpan { first_hour: *first, last_hour: *last, coverage }),
            }
        }
        out
    }

    /// The warnings main's reply carried: fallen convoy counts, land not kept or unknown, and loss
    /// rows naming countries no link resolves.
    pub fn warnings(&self, combat_log: bool) -> Vec<String> {
        let mut out = Vec::new();
        let drops = self.country.iter().rev().find_map(|(_, _, cells)| cells.as_ref()?.convoy_drops).unwrap_or_default();
        if drops != 0 {
            out.push(format!("{drops} recorded sunk convoys counts fell in value; each was counted as a new entry."));
        }
        if combat_log {
            let land = self.land();
            if land.iter().any(|span| span.coverage == Coverage::NotLogged) {
                out.push("The game did not keep this country's land losses at some hours (the country or feature gate was closed, or its divisions had no orders groups); those losses are unknown, not zero.".to_owned());
            }
            if land.iter().any(|span| span.coverage == Coverage::Unknown) {
                out.push("Some hours have recorded gaps, failed blocks/frames, unread division census, unresolved owners or missing writer inputs; land-loss coverage is unknown there.".to_owned());
            }
        }
        let unresolved = self.last_run().and_then(|cells| if combat_log { cells.unresolved_all } else { cells.unresolved }).unwrap_or_default();
        if unresolved != 0 {
            out.push(format!("Recorded loss data name {unresolved} country references the recorded links do not resolve; they are left out."));
        }
        out
    }
}

/// The kind of unknown a drain reason names.
pub fn classify(reason: &str) -> UnknownReason {
    if reason == Row::DRAIN_NOT_READ || reason == Row::DRAIN_LOST || reason == EQUIPMENT_DESTRUCTION_PARTIAL_REASON {
        UnknownReason::DestructionGap
    } else if reason == Row::DRAIN_OWNER_UNKNOWN {
        UnknownReason::DestructionUnresolved
    } else {
        UnknownReason::DestructionWithheld
    }
}
