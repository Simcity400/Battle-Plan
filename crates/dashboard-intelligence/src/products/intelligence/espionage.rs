use super::{Country, Tags, missing};
use dashboard_query::log::Cell;
use game_model::calendar::HOUR_EPOCH;
use replay_data::rows::{OperationCompletionRow, OperationRow, TableId, tables};
#[cfg(test)]
use replay_data::rows::{OperationAssignmentRow, OperativeMembershipRow};
use replay_frame::frame::DecodedFrame;
use serde::Serialize;
use std::{collections::HashMap, io};
#[cfg(any(test, feature = "export-types"))]
use ts_rs::TS;

pub const LOG: &str = "operation-completions";

const DESCRIPTION: usize = 7;
const DESCRIPTION_ERROR: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperationState {
    pub start_hour: Option<i32>,
    pub duration_days: i32,
    pub phase_count: u32,
    pub current_phase: Option<u32>,
    pub elapsed_days: Option<i32>,
    pub preparation_ratio: Option<i32>,
    pub assigned_slots: u32,
    pub auto_repeat: bool,
    pub auto_commence: bool,
    pub terminal: bool,
}

impl OperationState {
    pub fn of(row: &OperationRow, hour: i32) -> Self {
        Self {
            start_hour: (row.duration_days != 0).then_some(row.start_hour),
            duration_days: row.duration_days,
            phase_count: row.phase_count,
            current_phase: current_phase(row, hour),
            elapsed_days: elapsed_days(row, hour),
            preparation_ratio: preparation_ratio(row),
            assigned_slots: row.assigned_slots,
            auto_repeat: row.auto_repeat != 0,
            auto_commence: row.flag_40 != 0,
            terminal: row.terminal != 0,
        }
    }
}

pub fn preparation_ratio(row: &OperationRow) -> Option<i32> {
    let current = row.preparation_current;
    let ratio = if row.preparation_required == 0 {
        0
    } else if [i64::MIN, i64::MIN + 1, i64::MAX].contains(&current) {
        100_000
    } else {
        current.wrapping_mul(100_000).checked_div(row.preparation_required)?.clamp(0, 100_000)
    };
    Some(ratio as i32)
}

pub fn elapsed_days(row: &OperationRow, hour: i32) -> Option<i32> {
    if row.duration_days == 0 || row.terminal != 0 {
        return None;
    }
    let day = |hour: i32| hour.wrapping_sub(HOUR_EPOCH) / 24;
    let elapsed = day(hour).wrapping_sub(day(row.start_hour));
    (elapsed != -1 && elapsed != row.duration_days).then_some(elapsed)
}

pub fn current_phase(row: &OperationRow, hour: i32) -> Option<u32> {
    let elapsed = elapsed_days(row, hour)?;
    let phase = elapsed.wrapping_mul(row.phase_count as i32).checked_div(row.duration_days)?;
    u32::try_from(phase).ok().filter(|phase| *phase < row.phase_count)
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperativeName {
    pub id: String,
    pub name: Option<String>,
    pub unavailable: Option<String>,
}

#[cfg(test)]
pub fn operatives(row: &OperationRow, slots: Option<&[OperationAssignmentRow]>, members: Option<&[OperativeMembershipRow]>) -> Option<Vec<OperativeName>> {
    let mut slots: Vec<&OperationAssignmentRow> = slots?.iter().filter(|slot| slot.operation == row.id).collect();
    slots.sort_unstable_by_key(|slot| slot.ordinal);
    if slots.len() != row.assigned_slots as usize || slots.iter().enumerate().any(|(at, slot)| slot.ordinal as usize != at) {
        return None;
    }
    let mut named: HashMap<u64, Option<&OperativeMembershipRow>> = HashMap::new();
    for member in members.unwrap_or_default() {
        named
            .entry(member.id)
            .and_modify(|held| {
                if held.is_some_and(|held| held.name != member.name) {
                    *held = None;
                }
            })
            .or_insert(Some(member));
    }
    Some(
        slots
            .iter()
            .filter(|slot| slot.operative != 0)
            .map(|slot| {
                let name = match members {
                    None => Err("The census of operatives was not recorded at this read".to_owned()),
                    Some(_) => named
                        .get(&slot.operative)
                        .copied()
                        .flatten()
                        .ok_or_else(|| "Assigned operative name missing or contradictory at this read".to_owned())
                        .and_then(|member| member.name.require("name of operative memberships").cloned())
                        .and_then(|bytes| {
                            std::str::from_utf8(content_reader_parser::parser::c_string(&bytes))
                                .map(str::to_owned)
                                .map_err(|_| "Assigned operative name has non-text bytes".to_owned())
                        }),
                };
                match name {
                    Ok(name) => OperativeName { id: slot.operative.to_string(), name: Some(name), unavailable: None },
                    Err(why) => OperativeName { id: slot.operative.to_string(), name: None, unavailable: Some(why) },
                }
            })
            .collect(),
    )
}

fn literal(bytes: &[u8]) -> Option<String> {
    std::str::from_utf8(bytes).ok().filter(|text| !text.is_empty()).map(str::to_owned)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(any(test, feature = "export-types"), derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct OperationUnderWay {
    pub id: String,
    pub owner: Country,
    pub target: Country,
    pub operation: Option<String>,
    pub state: OperationState,
}

pub fn under_way(frame: &DecodedFrame, keep: impl Fn(&Tags, &OperationRow) -> bool) -> io::Result<(Vec<OperationUnderWay>, Vec<String>)> {
    let tags = Tags::read(frame)?;
    let rows = frame.complete::<tables::Operations>()?;
    let tokens: HashMap<u32, String> = frame
        .complete::<tables::LexicalTokens>()
        .map(|rows| rows.iter().filter_map(|row| Some((row.token, literal(&row.literal)?))).collect())
        .unwrap_or_default();
    let mut warnings = Vec::new();
    warnings.extend(missing(frame, TableId::LexicalTokens));
    let unnamed = rows.iter().filter(|row| !tokens.contains_key(&row.definition_token)).count();
    if unnamed != 0 && frame.complete::<tables::LexicalTokens>().is_ok() {
        warnings.push(format!("{unnamed} operations name a definition token with no recorded text"));
    }
    let operations = rows
        .iter()
        .filter(|row| row.terminal == 0 && keep(&tags, row))
        .map(|row| OperationUnderWay {
            id: row.id.to_string(),
            owner: Country::of(&tags, row.owner),
            target: Country::of(&tags, row.target),
            operation: tokens.get(&row.definition_token).cloned(),
            state: OperationState::of(row, frame.hour),
        })
        .collect();
    Ok((operations, warnings))
}

fn next_read(reads: &[(i32, i32)], last: i32) -> Option<i32> {
    let at = reads.partition_point(|run| run.1 <= last);
    reads.get(at).map(|run| run.0.max(last + 1))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Ending {
    pub key: u128,
    pub first: i32,
    pub last: i32,
    pub by: i32,
}

pub fn endings(lives: &[(u128, i64, i64)], stretch: (i32, i32), reads: &[(i32, i32)], from: i32, to: i32) -> Vec<Ending> {
    lives
        .iter()
        .filter_map(|&(key, first, last)| {
            let (first, last) = (i32::try_from(first).ok()?.max(stretch.0), i32::try_from(last).ok()?.min(stretch.1));
            if first > last {
                return None;
            }
            let by = next_read(reads, last)?;
            (from <= by && by <= to.min(stretch.1)).then_some(Ending { key, first, last, by })
        })
        .collect()
}

fn number(cells: &[Cell], column: usize) -> Option<i64> {
    match cells.get(column) {
        Some(Cell::Number(value)) => Some(*value),
        _ => None,
    }
}

pub fn description(cells: &[Cell]) -> Result<String, String> {
    match (number(cells, DESCRIPTION_ERROR), cells.get(DESCRIPTION)) {
        (Some(0), Some(Cell::Text(text))) if text.contains('\u{FFFD}') => Err("Stored operation description refused: non-UTF-8 recorded bytes".into()),
        (Some(0), Some(Cell::Text(text))) => Ok(text.clone()),
        (Some(error @ 1..=2), _) => Err(OperationCompletionRow { description_error: error as u8, ..OperationCompletionRow::default() }
            .description_refusal()
            .unwrap_or("Stored operation description refused")
            .to_owned()),
        _ => Err("Stored operation description not recorded".into()),
    }
}
