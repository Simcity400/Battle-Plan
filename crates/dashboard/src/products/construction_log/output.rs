//! The output capture observes building spend and the railway's first progress
//! call. Its separate factory totals already exclude CIC-bank draws, railway
//! carries and instant progress; never price completions.
use super::{ConstructionProject, EndKind, ProjectKind, ScannedLog, UnknownEnd, ended};
use replay::{
    frame::{DecodedFrame, TableView},
    rows::{ConstructionLineOutputRow, Recorded, TableId, tables},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
pub enum SpendingGap {
    NotRecorded,
    ReadFailed,
    LinkedOutput,
    InexactOutput,
    ObservationRestart,
    FinalOutputMissing,
    OwnerUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConstructionOutputRead {
    pub known_at: i32,
    pub since_hour: Option<i32>,
    /// Fixed5 factory output since `since_hour`; null when attribution is unknown.
    pub construction_raw: Option<String>,
    pub repair_raw: Option<String>,
    pub reason: Option<SpendingGap>,
    pub observed_since_creation: bool,
    /// Includes zero-factory natural repair advances; it is not an IC amount.
    pub repair_ticks: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct OutputUnknown {
    pub known_at: i32,
    pub reason: SpendingGap,
    pub line: Option<u64>,
    pub until: Option<i32>,
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct OutputScan {
    projects: BTreeMap<u64, Option<usize>>,
    active: BTreeSet<u64>,
    indexed: usize,
    previous: BTreeMap<u64, i32>,
    unattributed: BTreeMap<u64, usize>,
    unowned: BTreeSet<u64>,
    restarted: BTreeSet<u64>,
    unavailable: Option<usize>,
}

impl OutputScan {
    pub(super) fn indexed(&self) -> usize {
        self.indexed
    }
    pub(super) fn retained(&self) -> impl Iterator<Item = usize> + '_ {
        self.active.iter().filter_map(|id| self.projects[id])
    }

    pub(super) fn compact(&mut self, kept: &BTreeMap<usize, usize>) {
        for index in self.projects.values_mut() {
            *index = index.and_then(|index| kept.get(&index).copied());
        }
        self.indexed = kept.len();
    }

    pub(super) fn gap(&mut self, log: &mut ScannedLog, hour: i32) {
        // A line's final publication can have been wholly inside the lost frames.
        log.output_unknown.push(OutputUnknown {
            known_at: hour,
            reason: SpendingGap::NotRecorded,
            line: None,
            until: None,
        });
    }

    pub(super) fn observe(&mut self, log: &mut ScannedLog, frame: &DecodedFrame) {
        for (index, project) in log.projects.iter().enumerate().skip(self.indexed) {
            let id = project.id.parse().expect("native construction handle");
            self.projects.insert(id, Some(index));
            self.active.insert(id);
        }
        self.indexed = log.projects.len();
        let rows = frame.complete::<tables::ConstructionLineOutput>().ok();
        let unavailable_reason = match &*frame.tables[TableId::ConstructionLineOutput.index()] {
            TableView::Unavailable { reason } if reason.contains("not recorded") => {
                SpendingGap::NotRecorded
            }
            _ => SpendingGap::ReadFailed,
        };
        log.output_reads
            .push((frame.hour, rows.is_none().then_some(unavailable_reason)));
        match (rows, self.unavailable) {
            (None, None) => {
                self.unavailable = Some(log.output_unknown.len());
                log.output_unknown.push(OutputUnknown {
                    known_at: frame.hour,
                    reason: unavailable_reason,
                    line: None,
                    until: None,
                });
            }
            (Some(_), Some(index)) => {
                log.output_unknown[index].until = Some(frame.hour);
                self.unavailable = None;
            }
            _ => {}
        }
        let present: BTreeMap<_, _> = rows
            .unwrap_or_default()
            .iter()
            .map(|row| (row.line, row))
            .collect();
        for &id in &self.active {
            let index = self.projects[&id].expect("active construction output project");
            let project = &mut log.projects[index];
            let row = present.get(&id).copied();
            if row.is_some_and(|row| {
                row.railway != (project.kind == ProjectKind::Railway)
                    || self
                        .previous
                        .get(&id)
                        .is_some_and(|was| *was != row.since_hour)
            }) {
                self.restarted.insert(id);
            }
            let reason = match row {
                _ if self.restarted.contains(&id) => Some(SpendingGap::ObservationRestart),
                Some(row) if row.linked => Some(SpendingGap::LinkedOutput),
                Some(row) if !row.exact => Some(SpendingGap::InexactOutput),
                Some(_) => None,
                None if rows.is_none() => Some(unavailable_reason),
                None => Some(SpendingGap::FinalOutputMissing),
            };
            if project.end.is_some() {
                apply_ending(project, row, frame.hour, self.restarted.contains(&id));
            }
            if let Some(reason) = reason {
                log.output_reasons.insert((reason, project.id.clone()));
            }
            let repair_ticks = row.map_or(0, |row| row.repair_ticks);
            if repair_ticks > 0 && project.first_repair_tick.is_none() {
                project.first_repair_tick = Some(frame.hour);
            }
            log.outputs[index].push(ConstructionOutputRead {
                known_at: frame.hour,
                since_hour: row.map(|row| row.since_hour),
                construction_raw: row
                    .filter(|_| reason.is_none())
                    .map(|row| row.construction_raw.to_string()),
                repair_raw: row
                    .filter(|_| reason.is_none())
                    .map(|row| row.repair_raw.to_string()),
                reason,
                observed_since_creation: row.is_some_and(|row| row.observed_since_creation),
                repair_ticks,
            });
        }
        if let Some(rows) = rows {
            for row in rows {
                if !self.projects.contains_key(&row.line) {
                    self.unowned.insert(row.line);
                    log.unowned_lines = self.unowned.len();
                    // An exact unlinked zero has no unknown factory amount to
                    // assign, though its unidentified project is still counted.
                    if row.exact && !row.linked && row.construction_raw == 0 && row.repair_raw == 0
                    {
                        continue;
                    }
                    let index = *self.unattributed.entry(row.line).or_insert_with(|| {
                        let index = log.output_unknown.len();
                        log.output_unknown.push(OutputUnknown {
                            known_at: frame.hour,
                            reason: SpendingGap::OwnerUnknown,
                            line: Some(row.line),
                            until: None,
                        });
                        index
                    });
                    if row.ended {
                        log.output_unknown[index].until = frame.hour.checked_add(1);
                    }
                }
                if row.ended {
                    self.active.remove(&row.line);
                    self.previous.remove(&row.line);
                } else {
                    self.previous.insert(row.line, row.since_hour);
                }
            }
        }
    }
}

fn apply_ending(
    project: &mut ConstructionProject,
    row: Option<&ConstructionLineOutputRow>,
    hour: i32,
    restarted: bool,
) {
    let Some(row) =
        row.filter(|row| row.ended && row.railway == (project.kind == ProjectKind::Railway))
    else {
        return;
    };
    match row.completion {
        Recorded::Value(Some(event)) if event.hour <= hour => {
            // Independent of linked/instant IC and target changes: this is the
            // observed daily or linked completion immediately before deletion.
            project.end = Some(ended(
                if event.cancelled {
                    EndKind::Removed
                } else {
                    EndKind::Finished
                },
                Some(event.hour),
                hour,
                None,
            ));
        }
        Recorded::Value(None)
            if !row.linked
                && !restarted
                && project
                    .runs
                    .last()
                    .is_some_and(|last| row.since_hour <= last.first_hour) =>
        {
            // The complete observation covered the last live read and departure,
            // with no completion result: some other path removed the line.
            // A linked receiver can complete inside its primary advance; older
            // pending writers did not observe that path, so absence stays unknown.
            project.end = Some(ended(EndKind::Removed, None, hour, None));
        }
        _ => {
            project.end = Some(ended(
                EndKind::Unknown,
                None,
                hour,
                Some(UnknownEnd::CompletionNotRecorded),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::by_read;
    use super::*;
    use crate::products::construction_log::tests::{
        CIVILIAN, building_at, building_line, construction_frame, railway_line, timed_scan,
    };
    use crate::test_fixtures::{CAMPAIGN, records};
    use replay::{
        container::Replay,
        rows::{ConstructionCompletion, TableRows},
    };
    use std::sync::atomic::AtomicBool;

    #[test]
    fn signed_or_wrapped_progress_does_not_claim_a_coincident_focus_rise() {
        for (before, after, factory_output) in [
            // Both samples stay below cost: the negative advance builds no level.
            (600_000, 500_000, Some(-100_000)),
            // Native qword addition of 100,000 crosses the signed boundary;
            // an older recording need not have any captured output proof.
            (i64::MAX - 49_999, i64::MIN + 50_000, None),
        ] {
            let frames = (100..=124)
                .map(|hour| {
                    let read = hour == 100 || hour == 124;
                    let line = replay::rows::BuildingConstructionRow {
                        cost: 1_000_000,
                        produced: if hour < 124 { before } else { after },
                        speed: factory_output.unwrap_or(100_000),
                        ..building_line(1, 5, 1, 0, 1)
                    };
                    let mut frame = construction_frame(
                        hour,
                        read,
                        vec![line],
                        Vec::new(),
                        Some(vec![building_at(
                            5,
                            CIVILIAN,
                            if hour < 120 { (1, 1) } else { (2, 2) },
                        )]),
                        Vec::new(),
                    );
                    *frame.tables[TableId::ConstructionLineOutput.index()] = match factory_output {
                        Some(amount) => {
                            TableView::Complete(TableRows::ConstructionLineOutput(vec![
                                ConstructionLineOutputRow {
                                    line: 1,
                                    since_hour: 100,
                                    construction_raw: if hour < 124 { 0 } else { amount },
                                    exact: true,
                                    completion: Recorded::Value(None),
                                    ..Default::default()
                                },
                            ]))
                        }
                        None => TableView::Unavailable {
                            reason: "not recorded: older output prefix".into(),
                        },
                    };
                    frame
                })
                .collect();
            let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
            let log = timed_scan(&replay, 1, &AtomicBool::new(false)).unwrap();
            let project = &log.projects[0];
            assert!(project.built.is_empty());
            assert!(project.added.is_empty());
            assert!(project.unaccounted.is_empty());
            assert!(project.end.is_none());
            assert_eq!(project.possible, [by_read(1, 124, false)]);
            let output = log.outputs[0]
                .iter()
                .find(|output| output.known_at == 124)
                .unwrap();
            assert_eq!(
                output.construction_raw,
                factory_output.map(|amount| amount.to_string()),
            );
            assert_eq!(
                output.reason,
                factory_output.is_none().then_some(SpendingGap::NotRecorded),
            );
        }
    }

    #[test]
    #[cfg_attr(
        not(feature = "slow-tests"),
        ignore = "slow: runs in the full suite (--features slow-tests)"
    )]
    fn native_outcomes_decide_ending_independently_of_focus_rises_and_output_exactness() {
        for (railway, linked, completion, since, expected) in [
            (
                false,
                true,
                Recorded::Value(Some(ConstructionCompletion {
                    hour: 120,
                    cancelled: false,
                })),
                100,
                EndKind::Finished,
            ),
            (
                true,
                true,
                Recorded::Value(Some(ConstructionCompletion {
                    hour: 120,
                    cancelled: true,
                })),
                100,
                EndKind::Removed,
            ),
            (false, true, Recorded::Value(None), 100, EndKind::Unknown),
            (false, false, Recorded::Value(None), 100, EndKind::Removed),
            (false, false, Recorded::Value(None), 120, EndKind::Unknown),
        ] {
            let frames = (100..=124)
                .map(|hour| {
                    let lines = if !railway && hour < 124 {
                        vec![building_line(1, 5, 1, 0, 1)]
                    } else {
                        Vec::new()
                    };
                    let rails = if railway && hour < 124 {
                        vec![railway_line(1, &[1001, 1002], 1)]
                    } else {
                        Vec::new()
                    };
                    // A focus raised this building at the daily boundary. It cannot
                    // prove the queue finished, even when it coincides with removal.
                    let mut frame = construction_frame(
                        hour,
                        hour == 100 || hour == 124,
                        lines,
                        rails,
                        Some(vec![building_at(
                            5,
                            CIVILIAN,
                            if hour < 120 { (1, 1) } else { (2, 2) },
                        )]),
                        Vec::new(),
                    );
                    let row = ConstructionLineOutputRow {
                        line: 1,
                        railway,
                        since_hour: if hour < 124 { 100 } else { since },
                        ended: hour == 124,
                        exact: false,
                        linked,
                        completion: if hour == 124 {
                            completion
                        } else {
                            Recorded::Value(None)
                        },
                        ..Default::default()
                    };
                    *frame.tables[TableId::ConstructionLineOutput.index()] =
                        TableView::Complete(TableRows::ConstructionLineOutput(vec![row]));
                    frame
                })
                .collect();
            let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, frames)]).unwrap();
            let log = timed_scan(&replay, 1, &AtomicBool::new(false)).unwrap();
            let project = &log.projects[0];
            let end = project.end.as_ref().unwrap();
            assert_eq!(end.kind, expected);
            assert_eq!(end.known_at, 124);
            assert_eq!(
                end.hour,
                if matches!(completion, Recorded::Value(Some(_))) {
                    Some(120)
                } else {
                    None
                }
            );
            assert!(project.built.is_empty());
            assert_eq!(project.unaccounted[0].count, 1);
            let mut older = project.clone();
            let railway = older.kind == ProjectKind::Railway;
            apply_ending(
                &mut older,
                Some(&ConstructionLineOutputRow {
                    line: 1,
                    railway,
                    ended: true,
                    completion: Recorded::NotRecorded,
                    ..Default::default()
                }),
                124,
                false,
            );
            assert_eq!(older.end.unwrap().kind, EndKind::Unknown);
        }
    }
}
