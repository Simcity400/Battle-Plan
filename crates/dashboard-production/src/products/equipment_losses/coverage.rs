
use replay_recording::columnar_frames::Recording;
use replay_lanes::lanes::{Cell, Keys, history};
use replay_data::rows::{
    EQUIPMENT_DESTRUCTION_LEGACY_PARTIAL_REASON, EQUIPMENT_DESTRUCTION_PARTIAL_REASON, EquipmentLossCoverageRow as Row, TableId, compatibility::EQUIPMENT_DESTRUCTION_STARTUP_FORMAT,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum UnknownReason {
    Unread,
    EndedAfterLastRead,
    DestructionWithheld,
    DestructionGap,
    DestructionUnresolved,
    DestructionDomain,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct UnknownSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: UnknownReason,
    pub detail: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum Coverage {
    Logged,
    NotLogged,
    Partial { ungrouped: u32 },
    Unknown,
    PruneUnobserved,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct CoverageSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub coverage: Coverage,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DestructionCoverageSpan {
    pub first_hour: i32,
    pub last_hour: i32,
    pub reason: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct DestructionRun {
    pub first_hour: i32,
    pub last_hour: i32,
    pub counted_from: Option<i32>,
    pub withheld_from: Option<i32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunCells {
    pub drain: Option<String>,
    pub drain_state: Option<u8>,
    pub unread: Option<u8>,
    pub counted_from: Option<i32>,
    pub withheld_from: Option<i32>,
    pub incomplete_from: Option<i32>,
    pub unresolved: Option<u32>,
    pub unresolved_all: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CountryCells {
    pub land: Option<u8>,
    pub ungrouped: Option<u32>,
    pub convoy_drops: Option<u32>,
    pub exchanges_ended: Option<u32>,
    pub ended_from: Option<i32>,
}

pub type Runs<T> = Vec<(i32, i32, Option<T>)>;

const NOT_AVAILABLE: &str = "The equipment-loss tables were not available at these hours";
const DRAIN_TEXT_UNREAD: &str = "The drain's reason was not read at these hours";
const DRAIN_STATE_UNRECORDED: &str = "The drain's coverage state was not recorded at these hours";
const COLUMNS: [usize; 16] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];

#[derive(Clone, Debug, Default)]
pub struct Held {
    pub run: Runs<RunCells>,
    pub country: Runs<CountryCells>,
    pub legacy_partial_text: bool,
    pub run_starts: Vec<usize>,
    pub(super) last_run_id: Option<u32>,
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
    pub fn read(&mut self, recording: &Recording, run: u32, country: u128, whole: u128, first: i32, last: i32) -> io::Result<()> {
        self.legacy_partial_text |= recording.format() < EQUIPMENT_DESTRUCTION_STARTUP_FORMAT;
        if self.last_run_id != Some(run) {
            if !self.run.is_empty() {
                self.run_starts.push(self.run.len());
            }
            self.last_run_id = Some(run);
        }
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
                        drain_state: number(cell(15)).map(|value| value as u8),
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

    fn run_segments(&self) -> Vec<&[(i32, i32, Option<RunCells>)]> {
        if self.run.is_empty() {
            return Vec::new();
        }
        let mut splits: Vec<usize> = self.run_starts.iter().copied().filter(|&at| at > 0 && at < self.run.len()).collect();
        for index in 1..self.run.len() {
            if self.run[index - 1].1.saturating_add(1) < self.run[index].0 {
                splits.push(index);
            }
        }
        splits.sort_unstable();
        splits.dedup();
        let mut out = Vec::with_capacity(splits.len() + 1);
        let mut start = 0;
        for split in splits {
            out.push(&self.run[start..split]);
            start = split;
        }
        out.push(&self.run[start..]);
        out
    }

    fn last_run(&self) -> Option<&RunCells> {
        self.run.iter().rev().find_map(|(_, _, cells)| cells.as_ref())
    }

    pub fn active_run_start(&self) -> Option<i32> {
        self.run_segments().last()?.first().map(|(first, _, _)| *first)
    }

    #[cfg(test)]
    pub fn counted_from(&self) -> Option<i32> {
        self.run_segments().last()?.iter().filter_map(|(_, _, cells)| cells.as_ref()?.counted_from).min()
    }

    pub fn incomplete_from(&self) -> Option<i32> {
        self.run_segments().last()?.iter().rev().find_map(|(_, _, cells)| cells.as_ref())?.incomplete_from
    }

    pub fn destruction_runs(&self) -> Vec<DestructionRun> {
        let mut out = Vec::new();
        for segment in self.run_segments() {
            let (Some((first_hour, _, _)), Some((_, last_hour, _))) = (segment.first(), segment.last()) else { continue };
            let counted_from = segment.iter().filter_map(|(_, _, cells)| cells.as_ref()?.counted_from).min();
            let withheld_from = segment.iter().filter_map(|(_, _, cells)| cells.as_ref()?.withheld_from).min();
            out.push(DestructionRun { first_hour: *first_hour, last_hour: *last_hour, counted_from, withheld_from });
        }
        out
    }

    pub fn destruction_coverage(&self) -> Vec<DestructionCoverageSpan> {
        let mut out: Vec<DestructionCoverageSpan> = Vec::new();
        for segment in self.run_segments() {
            let seg_counted = segment.iter().filter_map(|(_, _, cells)| cells.as_ref()?.counted_from).min();
            let seg_withheld = segment.iter().filter_map(|(_, _, cells)| cells.as_ref()?.withheld_from).min();
            let Some(floor) = seg_counted.or(seg_withheld) else { continue };
            for (first, last, cells) in segment {
                if *last < floor {
                    continue;
                }
                let Some(cells) = cells.as_ref() else { continue };
                let Some(reason) = drain_coverage_reason(cells, self.legacy_partial_text, seg_counted.is_none()) else { continue };
                let first = (*first).max(floor);
                match out.last_mut() {
                    Some(held) if held.reason == reason && held.last_hour + 1 == first => held.last_hour = *last,
                    _ => out.push(DestructionCoverageSpan { first_hour: first, last_hour: *last, reason }),
                }
            }
        }
        out
    }

    pub fn destruction_unknown(&self, last_hour: i32) -> Vec<UnknownSpan> {
        let mut out: Vec<UnknownSpan> = Vec::new();
        let segments = self.run_segments();
        let last_index = segments.len().saturating_sub(1);
        for (index, segment) in segments.into_iter().enumerate() {
            let seg_counted = segment.iter().filter_map(|(_, _, cells)| cells.as_ref()?.counted_from).min();
            let seg_withheld = segment.iter().filter_map(|(_, _, cells)| cells.as_ref()?.withheld_from).min();
            let Some(floor) = seg_counted.or(seg_withheld) else { continue };
            let seg_end = if index == last_index {
                last_hour
            } else {
                segment.last().map_or(last_hour, |(_, end, _)| (*end).min(last_hour))
            };
            let mut first: BTreeMap<(UnknownReason, String), i32> = BTreeMap::new();
            let mut note = |reason: UnknownReason, detail: &str, hour: i32| {
                let at = first.entry((reason, detail.to_owned())).or_insert(hour);
                *at = (*at).min(hour);
            };
            for (start, end, cells) in segment {
                if *end < floor {
                    continue;
                }
                let start = (*start).max(floor);
                match cells {
                    None => note(UnknownReason::DestructionGap, NOT_AVAILABLE, start),
                    Some(cells) => {
                        let mut saw_withheld = false;
                        classify_drain(cells, self.legacy_partial_text, |reason, detail| {
                            let at = if reason == UnknownReason::DestructionWithheld {
                                saw_withheld = true;
                                cells.withheld_from.map_or(start, |withheld| withheld.min(start).max(floor))
                            } else {
                                start
                            };
                            note(reason, detail, at);
                        });
                        if let Some(withheld) = cells.withheld_from
                            && !saw_withheld
                        {
                            note(UnknownReason::DestructionWithheld, Row::DRAIN_WITHHELD, withheld.max(floor));
                        }
                    }
                }
            }
            out.extend(first.into_iter().map(|((reason, detail), first_hour)| UnknownSpan {
                first_hour,
                last_hour: seg_end.max(first_hour),
                reason,
                detail: Some(detail),
            }));
        }
        out.sort_unstable();
        out
    }

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

pub fn is_complete_drain(cells: &RunCells, legacy_partial_text: bool) -> bool {
    match cells.drain_state {
        Some(Row::DRAIN_STATE_COMPLETE) => cells.drain.as_deref() == Some(""),
        Some(_) => false,
        None if legacy_partial_text => cells.drain.as_deref() == Some(""),
        None => false,
    }
}

fn default_drain_reason(state: u8) -> &'static str {
    if state & Row::DRAIN_STATE_WITHHELD != 0 {
        Row::DRAIN_WITHHELD
    } else if state & Row::DRAIN_STATE_PARTIAL_INTERVAL != 0 {
        EQUIPMENT_DESTRUCTION_PARTIAL_REASON
    } else if state & Row::DRAIN_STATE_NOT_READ != 0 {
        Row::DRAIN_NOT_READ
    } else if state & Row::DRAIN_STATE_LOST != 0 {
        Row::DRAIN_LOST
    } else if state & Row::DRAIN_STATE_OWNER_UNKNOWN != 0 {
        Row::DRAIN_OWNER_UNKNOWN
    } else if state & Row::DRAIN_STATE_SUPPLEMENTAL_UNAVAILABLE != 0 {
        Row::DRAIN_SUPPLEMENTAL_NOT_RECORDED
    } else {
        Row::DRAIN_WITHHELD
    }
}

fn drain_coverage_reason(cells: &RunCells, legacy_partial_text: bool, withheld_only: bool) -> Option<Option<String>> {
    if withheld_only {
        let reason = cells.drain.as_deref().filter(|text| !text.is_empty()).map(str::to_owned).unwrap_or_else(|| Row::DRAIN_WITHHELD.to_owned());
        return Some(Some(reason));
    }
    match cells.drain_state {
        Some(Row::DRAIN_STATE_COMPLETE) => cells.drain.as_ref().map(|drain| (!drain.is_empty()).then(|| drain.clone())),
        Some(state) => {
            let reason = cells.drain.as_deref().filter(|text| !text.is_empty()).map(str::to_owned).unwrap_or_else(|| default_drain_reason(state).to_owned());
            Some(Some(reason))
        }
        None if legacy_partial_text => cells.drain.as_ref().map(|drain| (!drain.is_empty()).then(|| drain.clone())),
        None => {
            let reason = cells.drain.as_deref().filter(|text| !text.is_empty()).map(str::to_owned).unwrap_or_else(|| DRAIN_STATE_UNRECORDED.to_owned());
            Some(Some(reason))
        }
    }
}

fn take_exact<'a>(parts: &mut Vec<&'a str>, target: &str) -> bool {
    if let Some(at) = parts.iter().position(|&part| part == target) {
        parts.remove(at);
        true
    } else {
        false
    }
}

fn classify_drain(cells: &RunCells, legacy_partial_text: bool, mut note: impl FnMut(UnknownReason, &str)) {
    let drain = cells.drain.as_deref().unwrap_or_default();
    if let Some(state) = cells.drain_state {
        if state == Row::DRAIN_STATE_COMPLETE {
            for part in drain.split("; ").filter(|part| !part.is_empty()) {
                note(UnknownReason::DestructionWithheld, part);
            }
            return;
        }
        if state & !Row::DRAIN_STATE_MASK != 0 {
            let detail = if drain.is_empty() { Row::DRAIN_WITHHELD } else { drain };
            note(UnknownReason::DestructionWithheld, detail);
            return;
        }
        let mut parts: Vec<&str> = drain.split("; ").filter(|part| !part.is_empty()).collect();
        if state & Row::DRAIN_STATE_LOST != 0 {
            if state == Row::DRAIN_STATE_LOST {
                let detail = if drain.is_empty() { Row::DRAIN_LOST } else { drain };
                note(UnknownReason::DestructionGap, detail);
                return;
            }
            take_exact(&mut parts, Row::DRAIN_LOST);
            note(UnknownReason::DestructionGap, Row::DRAIN_LOST);
        }
        if state & Row::DRAIN_STATE_NOT_READ != 0 {
            if state == Row::DRAIN_STATE_NOT_READ {
                let detail = if drain.is_empty() { Row::DRAIN_NOT_READ } else { drain };
                note(UnknownReason::DestructionGap, detail);
                return;
            }
            take_exact(&mut parts, Row::DRAIN_NOT_READ);
            note(UnknownReason::DestructionGap, Row::DRAIN_NOT_READ);
        }
        if state & Row::DRAIN_STATE_OWNER_UNKNOWN != 0 {
            if state == Row::DRAIN_STATE_OWNER_UNKNOWN {
                let detail = if drain.is_empty() { Row::DRAIN_OWNER_UNKNOWN } else { drain };
                note(UnknownReason::DestructionUnresolved, detail);
                return;
            }
            take_exact(&mut parts, Row::DRAIN_OWNER_UNKNOWN);
            note(UnknownReason::DestructionUnresolved, Row::DRAIN_OWNER_UNKNOWN);
        }
        let remaining = parts.join("; ");
        if state & Row::DRAIN_STATE_PARTIAL_INTERVAL != 0 {
            let detail = if remaining.is_empty() { EQUIPMENT_DESTRUCTION_PARTIAL_REASON } else { &remaining };
            note(UnknownReason::DestructionGap, detail);
        }
        if state & Row::DRAIN_STATE_SUPPLEMENTAL_UNAVAILABLE != 0 {
            let detail = if remaining.is_empty() { Row::DRAIN_SUPPLEMENTAL_NOT_RECORDED } else { &remaining };
            note(UnknownReason::DestructionWithheld, detail);
        }
        if state & Row::DRAIN_STATE_WITHHELD != 0 {
            let detail = if remaining.is_empty() { Row::DRAIN_WITHHELD } else { &remaining };
            note(UnknownReason::DestructionWithheld, detail);
        }
        return;
    }
    if !legacy_partial_text {
        let detail = if drain.is_empty() { DRAIN_STATE_UNRECORDED } else { drain };
        note(UnknownReason::DestructionGap, detail);
        return;
    }
    for part in drain.split("; ").filter(|part| !part.is_empty()) {
        note(classify_text(part, true), part);
    }
}

#[cfg(test)]
pub fn classify(reason: &str) -> UnknownReason {
    classify_text(reason, false)
}

#[cfg(test)]
pub fn classify_historical(reason: &str) -> UnknownReason {
    classify_text(reason, true)
}

fn classify_text(reason: &str, legacy_partial_text: bool) -> UnknownReason {
    if reason == Row::DRAIN_NOT_READ || reason == Row::DRAIN_LOST || (legacy_partial_text && reason == EQUIPMENT_DESTRUCTION_LEGACY_PARTIAL_REASON) {
        UnknownReason::DestructionGap
    } else if reason == Row::DRAIN_OWNER_UNKNOWN {
        UnknownReason::DestructionUnresolved
    } else {
        UnknownReason::DestructionWithheld
    }
}
