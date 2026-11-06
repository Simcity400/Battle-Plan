//! The output capture observes building spend and the railway's first progress
//! call. Its separate factory totals already exclude CIC-bank draws, railway
//! carries and instant progress; never price completions.
use super::{
    ConstructionProject, EndKind, ProjectKind, ProjectSource, ScannedLog, UnknownEnd, ended,
    observed::Output,
};
use replay_data::rows::{ConstructionLineOutputRow, Recorded};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord, TS)]
#[serde(rename_all = "camelCase")]
#[derive(replay_spool::Spooled)]
pub enum SpendingGap {
    NotRecorded,
    ReadFailed,
    LinkedOutput,
    InexactOutput,
    ObservationRestart,
    FinalOutputMissing,
    OwnerUnknown,
    /// The output needs earlier observations than the prepared prefix it was
    /// answered from reaches; preparation supplies it later.
    Preparing,
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
    census: BTreeSet<u64>,
    active: BTreeSet<u64>,
    indexed: usize,
    previous: BTreeMap<u64, i32>,
    unattributed: BTreeMap<u64, usize>,
    unowned: BTreeSet<u64>,
    restarted: BTreeSet<u64>,
    unavailable: Option<usize>,
    baseline: BTreeSet<u64>,
}

impl OutputScan {
    pub(super) fn listed(&mut self, ids: impl Iterator<Item = u64>) {
        self.census.extend(ids);
    }

    pub(super) fn has_census(&self, id: u64) -> bool {
        self.census.contains(&id)
    }
    pub(super) fn knows(&self, id: u64) -> bool {
        self.projects.contains_key(&id)
    }
    pub(super) fn project(&self, id: u64) -> Option<usize> {
        self.projects.get(&id).copied().flatten()
    }
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
        self.baseline
            .retain(|id| self.projects.get(id).is_none_or(Option::is_some));
    }

    pub(super) fn claim(&mut self, id: u64, hour: i32, unknown: &mut [OutputUnknown]) -> usize {
        let mut baseline = self.unowned.remove(&id);
        if let Some(gap) = self.unattributed.remove(&id) {
            unknown[gap].until.get_or_insert(hour);
            baseline = true;
        }
        if baseline {
            self.baseline.insert(id);
        }
        self.unowned.len()
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

    /// The output table's read at `hour`.
    pub(super) fn observe(&mut self, log: &mut ScannedLog, hour: i32, output: &Output) {
        for (index, project) in log.projects.iter().enumerate().skip(self.indexed) {
            let id = project.id.parse().expect("native construction handle");
            self.projects.insert(id, Some(index));
            self.active.insert(id);
            self.claim(id, hour, &mut log.output_unknown);
        }
        self.indexed = log.projects.len();
        log.unowned_lines = self.unowned.len();
        let (rows, unavailable_reason) = match output {
            Output::Read(rows) => (Some(rows.as_slice()), SpendingGap::ReadFailed),
            Output::Unavailable(reason) => (None, *reason),
            Output::NotRead => return,
        };
        log.output_reads
            .push((hour, rows.is_none().then_some(unavailable_reason)));
        match (rows, self.unavailable) {
            (None, None) => {
                self.unavailable = Some(log.output_unknown.len());
                log.output_unknown.push(OutputUnknown {
                    known_at: hour,
                    reason: unavailable_reason,
                    line: None,
                    until: None,
                });
            }
            (Some(_), Some(index)) => {
                log.output_unknown[index].until = Some(hour);
                self.unavailable = None;
            }
            _ => {}
        }
        let present: BTreeMap<_, _> = rows
            .unwrap_or_default()
            .iter()
            .map(|line| (line.row.line, &line.row))
            .collect();
        for &id in &self.active {
            let index = self.projects[&id].expect("active construction output project");
            let project = &mut log.projects[index];
            let row = present.get(&id).copied();
            if !self.census.contains(&id)
                && row.is_some_and(|row| !matches!(row.country, Recorded::Value(Some(_))))
            {
                self.unowned.insert(id);
            }
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
                _ if self.unowned.contains(&id) => Some(SpendingGap::OwnerUnknown),
                _ if self.restarted.contains(&id) => Some(SpendingGap::ObservationRestart),
                Some(row) if row.linked => Some(SpendingGap::LinkedOutput),
                Some(row) if !row.exact => Some(SpendingGap::InexactOutput),
                Some(_) => None,
                None if rows.is_none() => Some(unavailable_reason),
                None => Some(SpendingGap::FinalOutputMissing),
            };
            if project.end.is_some()
                || (project.source == ProjectSource::OutputAlone
                    && row.is_some_and(|row| row.ended))
            {
                apply_ending(project, row, hour, self.restarted.contains(&id));
            }
            if let Some(reason) = reason {
                log.output_reasons.insert((reason, project.id.clone()));
            }
            if reason.is_none() && self.baseline.remove(&id) {
                log.output_baselines.entry(id).or_default().insert(hour);
            }
            let repair_ticks = row.map_or(0, |row| row.repair_ticks);
            if repair_ticks > 0 && project.first_repair_tick.is_none() {
                project.first_repair_tick = Some(hour);
            }
            log.outputs[index].push(ConstructionOutputRead {
                known_at: hour,
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
            for line in rows {
                let row = &line.row;
                if !self.projects.contains_key(&row.line) || self.unowned.contains(&row.line) {
                    self.unowned.insert(row.line);
                    log.unowned_lines = self.unowned.len();
                    // An exact unlinked zero has no unknown factory amount to
                    // assign, though its unidentified project is still counted.
                    if !self.projects.contains_key(&row.line)
                        && row.exact
                        && !row.linked
                        && row.construction_raw == 0
                        && row.repair_raw == 0
                    {
                        continue;
                    }
                    let index = *self.unattributed.entry(row.line).or_insert_with(|| {
                        let index = log.output_unknown.len();
                        log.output_unknown.push(OutputUnknown {
                            known_at: hour,
                            reason: SpendingGap::OwnerUnknown,
                            line: Some(row.line),
                            until: None,
                        });
                        index
                    });
                    if row.ended {
                        log.output_unknown[index].until = hour.checked_add(1);
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
        log.unowned_lines = self.unowned.len();
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
                && (project
                    .runs
                    .last()
                    .is_some_and(|last| row.since_hour <= last.first_hour)
                    || (project.source == ProjectSource::OutputAlone
                        && row.since_hour <= project.first_read)) =>
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
    use dashboard_core::test_fixtures::{CAMPAIGN, records};
    use replay::container::Replay;
    use replay_data::{rows::ConstructionCompletion, rows::TableId, rows::TableRows};
    use replay_frame::frame::TableView;
    use std::sync::atomic::AtomicBool;

    fn owned_output(railway: bool) -> ConstructionLineOutputRow {
        ConstructionLineOutputRow {
            line: 59,
            railway,
            since_hour: 100,
            ended: true,
            exact: true,
            construction_raw: 370_000,
            observed_since_creation: true,
            country: Recorded::Value(Some(11)),
            building_type: Recorded::Value((!railway).then_some(CIVILIAN)),
            target_flags: Recorded::Value((!railway).then_some(2)),
            target_location: Recorded::Value((!railway).then_some(5)),
            target_state: Recorded::Value((!railway).then_some(5)),
            target_country: Recorded::Value((!railway).then_some(0)),
            route_first: Recorded::Value(railway.then_some(1001)),
            route_last: Recorded::Value(railway.then_some(1003)),
            completion: Recorded::Value(Some(ConstructionCompletion {
                hour: 120,
                cancelled: false,
            })),
            ..Default::default()
        }
    }

    #[test]
    fn a_line_created_and_ended_between_censuses_is_attributed_from_its_output() {
        for railway in [false, true] {
            let mut frame = construction_frame(
                124,
                true,
                Vec::new(),
                Vec::new(),
                Some(Vec::new()),
                Vec::new(),
            );
            *frame.tables[TableId::ConstructionLineOutput.index()] = TableView::Complete(
                TableRows::ConstructionLineOutput(vec![owned_output(railway)]),
            );
            let replay = Replay::from_records(CAMPAIGN.into(), &[records(1, vec![frame])]).unwrap();
            let log = timed_scan(&replay, 1, &AtomicBool::new(false)).unwrap();
            assert_eq!(log.projects.len(), 1);
            let project = &log.projects[0];
            assert_eq!(project.source, ProjectSource::OutputAlone);
            assert_eq!(project.tag, "GER");
            assert_eq!(project.first_read, 124);
            assert_eq!(
                (project.created_hour, project.census_read, project.repair),
                (None, None, None)
            );
            assert_eq!(project.cost_raw, None);
            assert!(project.runs.is_empty() && project.built.is_empty());
            assert_eq!(project.state, Some(if railway { 40 } else { 5 }));
            assert_eq!(project.to_state, railway.then_some(41));
            assert_eq!(project.end.as_ref().unwrap().kind, EndKind::Finished);
            assert_eq!(log.unowned_lines, 0);
            assert!(
                !log.output_unknown
                    .iter()
                    .any(|gap| gap.reason == SpendingGap::OwnerUnknown)
            );
            let (leaves, _) = crate::products::construction_spending::leaves(&log, "GER");
            assert_eq!(
                leaves
                    .iter()
                    .filter_map(|leaf| leaf.spent.last())
                    .map(|step| step.total_raw.parse::<i128>().unwrap())
                    .sum::<i128>(),
                370_000
            );
        }
    }

    #[test]
    fn a_recorded_empty_route_keeps_its_owner_without_inventing_endpoints() {
        let mut frame = construction_frame(
            124,
            true,
            Vec::new(),
            Vec::new(),
            Some(Vec::new()),
            Vec::new(),
        );
        let mut output = owned_output(true);
        output.route_first = Recorded::Value(None);
        output.route_last = Recorded::Value(None);
        *frame.tables[TableId::ConstructionLineOutput.index()] =
            TableView::Complete(TableRows::ConstructionLineOutput(vec![output]));
        let observed = super::super::row::Rows::default().hour(&frame, [true; 3]);
        let mut scan = super::super::Scan::default();
        scan.observe_changed(&observed);
        assert_eq!(scan.log.projects.len(), 1);
        let project = &scan.log.projects[0];
        assert_eq!(project.source, ProjectSource::OutputAlone);
        assert_eq!(project.tag, "GER");
        assert_eq!((project.state, project.to_state), (None, None));
        assert_eq!(scan.log.unowned_lines, 0);
    }

    #[test]
    fn older_output_metadata_and_unresolved_owners_stay_unknown() {
        for historical in [false, true] {
            let mut row = owned_output(false);
            if historical {
                use replay_data::{
                    codec::Decoder,
                    rows::{Fields, RowCodec},
                };
                let mut bytes = Vec::new();
                row.encode_fields(&mut bytes).unwrap();
                let mut input = Decoder::new(&bytes);
                for field in &ConstructionLineOutputRow::SCHEMA[..15] {
                    field.decode(&mut input).unwrap();
                }
                let length = bytes.len() - input.remaining();
                let mut input = Decoder::new(&bytes[..length]);
                input.format =
                    replay_data::rows::compatibility::CONSTRUCTION_OUTPUT_OWNER_FORMAT - 1;
                input.recorded_fields = 15;
                row =
                    ConstructionLineOutputRow::decode_fields(row.line as u128, &mut input).unwrap();
                input.finish().unwrap();
                assert_eq!(row.country, Recorded::NotRecorded);
            } else {
                row.country = Recorded::Value(Some(9999));
            }
            let mut frame = construction_frame(
                124,
                true,
                Vec::new(),
                Vec::new(),
                Some(Vec::new()),
                Vec::new(),
            );
            *frame.tables[TableId::ConstructionLineOutput.index()] =
                TableView::Complete(TableRows::ConstructionLineOutput(vec![row]));
            let mut rows = super::super::row::Rows::default();
            let observed = rows.hour(&frame, [true; 3]);
            let mut scan = super::super::Scan::default();
            scan.observe_changed(&observed);
            assert!(scan.log.projects.is_empty());
            assert_eq!(scan.log.unowned_lines, 1);
            assert!(
                scan.log
                    .output_unknown
                    .iter()
                    .any(|gap| gap.reason == SpendingGap::OwnerUnknown)
            );
        }
    }

    #[test]
    fn a_later_census_enriches_one_project_without_backfilling_its_first_evidence() {
        let mut rows = super::super::row::Rows::default();
        let mut scan = super::super::Scan::default();
        let mut output = owned_output(false);
        output.ended = false;
        output.completion = Recorded::Value(None);
        for hour in 100..=148 {
            let census = hour == 100 || hour == 148;
            let lines = if hour == 148 {
                vec![building_line(59, 5, 1, 0, 7)]
            } else {
                Vec::new()
            };
            let mut frame = construction_frame(
                hour,
                census,
                lines,
                Vec::new(),
                Some(Vec::new()),
                Vec::new(),
            );
            *frame.tables[TableId::ConstructionLineOutput.index()] =
                TableView::Complete(TableRows::ConstructionLineOutput(if hour >= 124 {
                    vec![output.clone()]
                } else {
                    Vec::new()
                }));
            let observed = rows.hour(
                &frame,
                [census, census, hour == 100 || hour == 124 || hour == 148],
            );
            scan.observe_changed(&observed);
            if hour == 124 {
                assert_eq!(scan.log.projects[0].source, ProjectSource::OutputAlone);
                assert_eq!(scan.log.projects[0].created_hour, None);
                assert!(scan.log.projects[0].runs.is_empty());
            }
        }
        assert_eq!(scan.log.projects.len(), 1);
        let project = &scan.log.projects[0];
        assert_eq!(project.source, ProjectSource::Census);
        assert_eq!(
            (
                project.first_read,
                project.census_read,
                project.created_hour
            ),
            (124, Some(148), Some(90))
        );
        assert_eq!(project.runs[0].first_hour, 148);
        assert!(project.output_origin.is_some());
        assert_eq!(scan.log.outputs[0].first().unwrap().known_at, 124);
        assert_eq!(
            scan.log.outputs[0]
                .first()
                .unwrap()
                .construction_raw
                .as_deref(),
            Some("370000")
        );
        assert_eq!(scan.log.unowned_lines, 0);
    }

    fn spending(log: &ScannedLog, tag: &str, repair: bool) -> i128 {
        crate::products::construction_spending::leaves(log, tag)
            .0
            .iter()
            .filter(|leaf| leaf.repair == repair)
            .filter_map(|leaf| leaf.spent.last())
            .map(|step| step.total_raw.parse::<i128>().unwrap())
            .sum()
    }

    #[test]
    fn cleared_and_initially_unknown_owners_resume_only_after_the_census_baseline() {
        for (initially_known, census_first) in [(true, false), (false, false), (true, true)] {
            let mut rows = super::super::row::Rows::default();
            let mut scan = super::super::Scan::default();
            for hour in 100..=196 {
                let read = [100, 124, 148, 172, 196].contains(&hour);
                let census = hour == 100 || hour == 172;
                let listed = census && (hour == 172 || census_first);
                let mut line = building_line(59, 5, 1, 0, 7);
                line.country = if census_first { 11 } else { 12 };
                let mut frame = construction_frame(
                    hour,
                    census,
                    if listed { vec![line] } else { Vec::new() },
                    Vec::new(),
                    Some(Vec::new()),
                    Vec::new(),
                );
                *frame.tables[TableId::Countries.index()] =
                    TableView::Complete(TableRows::Countries(
                        [(11, 1, b"GER"), (12, 2, b"ITA")]
                            .map(|(id, canonical_slot, tag)| replay_data::rows::CountryRow {
                                id,
                                canonical_slot,
                                tag: tag.to_vec(),
                                ..Default::default()
                            })
                            .to_vec(),
                    ));
                *frame.tables[TableId::CountryLinks.index()] =
                    TableView::Complete(TableRows::CountryLinks(
                        [(11, 1), (12, 2)]
                            .map(|(id, canonical_slot)| replay_data::rows::CountryLinkRow {
                                id,
                                canonical_slot,
                            })
                            .to_vec(),
                    ));
                let mut output = owned_output(false);
                output.ended = false;
                output.completion = Recorded::Value(None);
                let total = match hour {
                    ..124 => 100,
                    ..148 => 200,
                    ..172 => 300,
                    ..196 => 400,
                    _ => 450,
                };
                output.construction_raw = total;
                output.repair_raw = total / 5;
                output.repair_ticks = ((hour - 100) / 24 + 1) as i32;
                if hour != 100 || !initially_known {
                    output.country = Recorded::Value(None);
                    output.building_type = Recorded::Value(None);
                    output.target_flags = Recorded::Value(None);
                    output.target_location = Recorded::Value(None);
                    output.target_state = Recorded::Value(None);
                    output.target_country = Recorded::Value(None);
                }
                *frame.tables[TableId::ConstructionLineOutput.index()] =
                    TableView::Complete(TableRows::ConstructionLineOutput(vec![output]));
                scan.observe_changed(&rows.hour(&frame, [census, census, read]));
                if [124, 148].contains(&hour) {
                    assert_eq!(scan.log.unowned_lines, usize::from(!census_first));
                    assert_eq!(
                        spending(&scan.log, "GER", false),
                        if initially_known {
                            if census_first { total as i128 } else { 100 }
                        } else {
                            0
                        }
                    );
                    if !census_first {
                        let mut warnings = Vec::new();
                        super::super::scan_warnings(&scan.log, &mut warnings);
                        assert!(
                            warnings.iter().any(|warning| warning
                                .starts_with("1 construction output lines have no recorded owner"))
                        );
                    }
                }
                if hour == 172 && !census_first {
                    assert_eq!(spending(&scan.log, "ITA", false), 0);
                    assert_eq!(scan.log.unowned_lines, 0);
                }
            }
            assert_eq!(scan.log.projects.len(), 1);
            assert_eq!(scan.log.projects[0].source, ProjectSource::Census);
            if census_first {
                assert_eq!(spending(&scan.log, "GER", false), 450);
                assert_eq!(spending(&scan.log, "GER", true), 90);
                assert!(
                    !scan
                        .log
                        .output_unknown
                        .iter()
                        .any(|gap| gap.reason == SpendingGap::OwnerUnknown)
                );
            } else {
                assert_eq!(
                    spending(&scan.log, "GER", false),
                    if initially_known { 100 } else { 0 }
                );
                assert_eq!(
                    spending(&scan.log, "GER", true),
                    if initially_known { 20 } else { 0 }
                );
                assert_eq!(spending(&scan.log, "ITA", false), 50);
                assert_eq!(spending(&scan.log, "ITA", true), 10);
                let gaps: Vec<_> = scan
                    .log
                    .output_unknown
                    .iter()
                    .filter(|gap| gap.reason == SpendingGap::OwnerUnknown)
                    .collect();
                assert_eq!(gaps.len(), 1);
                assert_eq!(
                    (gaps[0].known_at, gaps[0].until, gaps[0].line),
                    (if initially_known { 124 } else { 100 }, Some(172), Some(59))
                );
                assert!(
                    scan.log.outputs[0]
                        .iter()
                        .all(|read| read.since_hour == Some(100))
                );
                let baseline = scan.log.outputs[0]
                    .iter()
                    .find(|read| read.known_at == 172)
                    .unwrap();
                assert_eq!(baseline.construction_raw.as_deref(), Some("400"));
                assert_eq!(baseline.repair_raw.as_deref(), Some("80"));
                assert_eq!(
                    scan.log.outputs[0]
                        .last()
                        .unwrap()
                        .construction_raw
                        .as_deref(),
                    Some("450")
                );
                if initially_known {
                    assert_eq!(
                        scan.log.outputs[0][0].construction_raw.as_deref(),
                        Some("100")
                    );
                    assert!(
                        scan.log.outputs[0][1..3]
                            .iter()
                            .all(|read| read.reason == Some(SpendingGap::OwnerUnknown)
                                && read.construction_raw.is_none())
                    );
                }
            }
        }
    }

    #[test]
    fn a_census_listing_an_unresolved_owner_keeps_the_owner_unknown() {
        let mut rows = super::super::row::Rows::default();
        let mut scan = super::super::Scan::default();
        for hour in 100..=196 {
            let read = [100, 124, 148, 172, 196].contains(&hour);
            let census = [100, 148, 172].contains(&hour);
            let mut line = building_line(59, 5, 1, 0, 7);
            // 13 is in no roster: the census at 148 lists the line without an owner.
            line.country = if hour == 148 { 13 } else { 12 };
            let mut frame = construction_frame(
                hour,
                census,
                if hour >= 148 && census { vec![line] } else { Vec::new() },
                Vec::new(),
                Some(Vec::new()),
                Vec::new(),
            );
            *frame.tables[TableId::Countries.index()] =
                TableView::Complete(TableRows::Countries(
                    [(11, 1, b"GER"), (12, 2, b"ITA")]
                        .map(|(id, canonical_slot, tag)| replay_data::rows::CountryRow {
                            id,
                            canonical_slot,
                            tag: tag.to_vec(),
                            ..Default::default()
                        })
                        .to_vec(),
                ));
            *frame.tables[TableId::CountryLinks.index()] =
                TableView::Complete(TableRows::CountryLinks(
                    [(11, 1), (12, 2)]
                        .map(|(id, canonical_slot)| replay_data::rows::CountryLinkRow {
                            id,
                            canonical_slot,
                        })
                        .to_vec(),
                ));
            let mut output = owned_output(false);
            output.ended = false;
            output.completion = Recorded::Value(None);
            output.construction_raw = match hour {
                ..124 => 100,
                ..148 => 200,
                ..172 => 300,
                ..196 => 400,
                _ => 450,
            };
            output.repair_raw = 0;
            output.repair_ticks = 0;
            // The identity clears at the census read that lists the line unresolved.
            if hour >= 148 {
                output.country = Recorded::Value(None);
                output.building_type = Recorded::Value(None);
                output.target_flags = Recorded::Value(None);
                output.target_location = Recorded::Value(None);
                output.target_state = Recorded::Value(None);
                output.target_country = Recorded::Value(None);
            }
            *frame.tables[TableId::ConstructionLineOutput.index()] =
                TableView::Complete(TableRows::ConstructionLineOutput(vec![output]));
            scan.observe_changed(&rows.hour(&frame, [census, census, read]));
            if hour == 148 {
                assert_eq!(scan.log.unowned_lines, 1);
                assert_eq!(spending(&scan.log, "GER", false), 200);
            }
        }
        assert_eq!(spending(&scan.log, "GER", false), 200);
        assert_eq!(spending(&scan.log, "ITA", false), 50);
        let gaps: Vec<_> = scan
            .log
            .output_unknown
            .iter()
            .filter(|gap| gap.reason == SpendingGap::OwnerUnknown)
            .collect();
        assert_eq!(gaps.len(), 1);
        assert_eq!((gaps[0].known_at, gaps[0].until), (148, Some(172)));
    }

    #[test]
    fn a_census_at_the_same_read_supplies_the_owner_and_target() {
        let mut frame = construction_frame(
            124,
            true,
            vec![building_line(59, 5, 1, 0, 7)],
            Vec::new(),
            Some(Vec::new()),
            Vec::new(),
        );
        let mut output = owned_output(false);
        output.country = Recorded::Value(Some(12));
        output.target_state = Recorded::Value(Some(999));
        output.ended = false;
        output.completion = Recorded::Value(None);
        *frame.tables[TableId::ConstructionLineOutput.index()] =
            TableView::Complete(TableRows::ConstructionLineOutput(vec![output]));
        let observed = super::super::row::Rows::default().hour(&frame, [true; 3]);
        let mut scan = super::super::Scan::default();
        scan.observe_changed(&observed);
        assert_eq!(scan.log.projects.len(), 1);
        let project = &scan.log.projects[0];
        assert_eq!(project.source, ProjectSource::Census);
        assert_eq!(project.tag, "GER");
        assert_eq!(project.state, Some(5));
        assert_eq!(project.created_hour, Some(90));
        assert_eq!(
            scan.log.outputs[0][0].construction_raw.as_deref(),
            Some("370000")
        );
    }

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
                    let line = replay_data::rows::BuildingConstructionRow {
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
