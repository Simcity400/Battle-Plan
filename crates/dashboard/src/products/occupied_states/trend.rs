//! Additional consumer of the occupation scan's already-decoded daily joins.
use super::*;
use crate::metadata::IdentityTrack;
use crate::products::{
    country_groups::HourSpan,
    country_history::{Identity, observe_identity},
    occupation_trend::{
        History, OccupationCollaboration, OccupationCollaborationActor, OccupationTrendRun,
        same_value,
    },
};

#[derive(Default)]
struct Total {
    resistance: i64,
    compliance: i64,
    states: u32,
    held: std::collections::HashSet<u32>,
    occupied: std::collections::BTreeSet<Identity>,
    held_complete: bool,
}

#[derive(Default)]
struct Track {
    segments: Vec<HourSpan>,
    last_identity: IdentityTrack,
    runs: Vec<OccupationTrendRun>,
    last: Option<(Identity, i32)>,
}

#[derive(Default)]
pub(super) struct TrendScan {
    tracks: BTreeMap<String, Track>,
    samples: HashMap<Identity, OccupationTrendRun>,
    totals: HashMap<Identity, Total>,
    collaboration: HashMap<(Identity, Identity), (i64, i32)>,
    read_hour: i32,
    inventory_complete: bool,
    dirty: bool,
    read_names: Arc<CountryIndex>,
    refused: Refusals,
    collaboration_refused: Refusals,
    warnings: Vec<String>,
}

// P: 140FE98F0, ADD at 140FE9A1F/26; IMUL at 140FE9A68/8D;
// IDIV at 140FE9A71/96. Keep native wrapping and truncation. Non-finite
// native sums have no displayable mean and are refused instead of charted.
fn mean(sum: i64, count: u32) -> Option<i64> {
    if count == 0 || !crate::products::finite_fixed64(sum) {
        return None;
    }
    sum.wrapping_mul(100_000)
        .checked_div(i64::from(count) * 100_000)
}

impl TrendScan {
    pub(super) fn gap(&mut self) {
        self.samples.clear();
        self.collaboration.clear();
        self.dirty = true;
    }

    pub(super) fn unavailable(&mut self, reason: String) {
        self.warnings.push(reason);
    }

    pub(super) fn missing_state(&mut self, state: u32) {
        self.refused.note(
            "active resistance names a state absent from States",
            state.to_string(),
        );
    }

    pub(super) fn collaboration(
        &mut self,
        frame: &DecodedFrame,
        hour: i32,
        identities: &HashMap<u32, u32>,
        links: &[CountryLinkRow],
    ) -> io::Result<()> {
        if !has_rows(frame, TableId::CountryCollaboration) {
            self.dirty |= !self.collaboration.is_empty();
            self.collaboration.clear();
        } else if slot_read(&frame.metrics, TableId::CountryCollaboration, hour) {
            self.dirty = true;
            self.collaboration.clear();
            let identity = |raw| {
                let slot = link_slot(links, raw)?;
                Some((slot, *identities.get(&slot)?))
            };
            for row in recorded_rows::<tables::CountryCollaboration>(frame)? {
                if let Some(key) = identity(row.source).zip(identity(row.target)) {
                    self.collaboration.insert(key, (row.progress, hour));
                } else {
                    self.collaboration_refused.note(
                        "CountryCollaboration link is unresolved",
                        format!("{} → {}", row.source, row.target),
                    );
                }
            }
        }
        Ok(())
    }

    pub(super) fn begin<'a>(
        &mut self,
        hour: i32,
        countries: Arc<CountryIndex>,
        links: &[CountryLinkRow],
        garrisons: impl Iterator<Item = &'a replay::rows::StateGarrisonRow>,
        complete: bool,
    ) {
        // P: 140FE9420 adds a state to the group vector and garrison map
        // together; 140FEA0C0 -> 140FE83C0 removes it from both. Only a matching
        // supplying inventory read establishes the complete held denominator.
        self.read_hour = hour;
        self.inventory_complete = complete;
        self.dirty = true;
        self.totals.clear();
        self.samples.clear();
        self.read_names = countries;
        let identities = &self.read_names.identities;
        for row in garrisons {
            let controller = link_slot(links, row.controller)
                .and_then(|slot| identities.get(&slot).map(|&id| (slot, id)));
            if let Some(controller) = controller {
                let total = self.totals.entry(controller).or_insert_with(|| Total {
                    held_complete: true,
                    ..Total::default()
                });
                total.held.insert(row.state);
                if let Some(occupied) = link_slot(links, row.occupied)
                    .and_then(|slot| identities.get(&slot).map(|&id| (slot, id)))
                {
                    total.occupied.insert(occupied);
                } else {
                    self.collaboration_refused.note(
                        "held occupied-country link is unresolved",
                        format!("state {}, raw link {}", row.state, row.occupied),
                    );
                }
            } else {
                self.inventory_complete = false;
                self.refused.note(
                    "held controller link is unresolved",
                    format!("state {}, raw link {}", row.state, row.controller),
                );
            }
        }
    }

    pub(super) fn state(
        &mut self,
        row: &StateResistanceRow,
        state: &StateRow,
        identities: &HashMap<u32, u32>,
        links: &[CountryLinkRow],
    ) {
        let identity = |raw| {
            let slot = link_slot(links, raw)?;
            Some((slot, *identities.get(&slot)?))
        };
        let Some(controller) = identity(state.controller) else {
            self.inventory_complete = false;
            self.refused.note(
                "active resistance controller link is unresolved",
                format!("state {}, raw link {}", state.id, state.controller),
            );
            return;
        };
        let total = self.totals.entry(controller).or_default();
        // A visible active resistance state also proves its own membership. A
        // garrison read missing it cannot establish a complete denominator.
        if total.held.insert(state.id) {
            total.held_complete = false;
        }
        total.resistance = total.resistance.wrapping_add(row.level);
        total.compliance = total.compliance.wrapping_add(row.compliance);
        total.states += 1;
        if let Some(occupied) = identity(row.country) {
            total.occupied.insert(occupied);
        } else {
            self.collaboration_refused.note(
                "active resistance occupied-country link is unresolved",
                format!("state {}, raw link {}", state.id, row.country),
            );
        }
    }

    pub(super) fn finish_read(&mut self) {
        for (&identity, total) in &self.totals {
            if total.states == 0 {
                continue;
            }
            let Some((resistance, compliance)) =
                mean(total.resistance, total.states).zip(mean(total.compliance, total.states))
            else {
                self.refused.note(
                    "native occupation mean is non-finite or not representable",
                    format!("country slot {}, read {}", identity.0, self.read_hour),
                );
                continue;
            };
            self.samples.insert(
                identity,
                OccupationTrendRun {
                    first_hour: self.read_hour,
                    last_hour: self.read_hour,
                    read_hour: self.read_hour,
                    starts_segment: false,
                    resistance_raw: resistance.to_string(),
                    compliance_raw: compliance.to_string(),
                    states: total.states,
                    held_states: total.held.len() as u32,
                    held_complete: self.inventory_complete && total.held_complete,
                    collaboration: Vec::new(),
                },
            );
        }
    }

    pub(super) fn project(
        &mut self,
        hour: i32,
        names: &HashMap<Identity, String>,
        names_changed: bool,
    ) {
        // An old read cannot resume after its occupier disappeared, even if the
        // same native identity later returns without a new resistance read.
        if names_changed {
            self.samples
                .retain(|identity, _| names.contains_key(identity));
        }
        // Allocate strings/vectors only on a supplying read or country-table
        // identity change. Unchanged hourly projections just extend endpoints.
        let changed = self.dirty || names_changed;
        if changed {
            for (identity, sample) in &mut self.samples {
                sample.collaboration = self
                    .totals
                    .get(identity)
                    .into_iter()
                    .flat_map(|total| &total.occupied)
                    .map(|occupied| {
                        let current = names.get(occupied);
                        let tag = current.or_else(|| self.read_names.names.get(occupied));
                        // The held tag cannot name a later holder at hover time.
                        // Current native keys remain proved until names_changed
                        // splits this run; an unnamed actor retains its amount.
                        if current.is_none() {
                            self.collaboration_refused.note(
                                "occupied native country naming identity is unavailable",
                                format!("slot {}, country {}", occupied.0, occupied.1),
                            );
                        }
                        // P: StartResistance 140F85ED0 reads the occupied country's
                        // +0xFD8 relation toward its controller; source is occupied.
                        let read = self.collaboration.get(&(*occupied, *identity));
                        OccupationCollaboration {
                            tag: tag.cloned(),
                            raw: read.map(|(raw, _)| raw.to_string()),
                            read_hour: read.map(|(_, hour)| *hour),
                            actor: Some(OccupationCollaborationActor {
                                slot: occupied.0,
                                country_id: occupied.1,
                                name_hour: current.map(|_| hour),
                            }),
                        }
                    })
                    .collect();
                sample
                    .collaboration
                    .sort_unstable_by(|a, b| a.tag.cmp(&b.tag));
            }
        }
        self.dirty = false;
        for (identity, tag) in names {
            if !self.tracks.contains_key(tag) {
                self.tracks.insert(tag.clone(), Track::default());
            }
            let track = self.tracks.get_mut(tag).expect("inserted above");
            observe_identity(
                &mut track.segments,
                &mut track.last_identity,
                *identity,
                hour,
            );
            let Some(sample) = self.samples.get(identity) else {
                continue;
            };
            let contiguous = track
                .last
                .is_some_and(|(seen, at)| seen == *identity && at.checked_add(1) == Some(hour));
            if contiguous
                && let Some(last) = track.runs.last_mut()
                && (!changed || same_value(last, sample))
            {
                last.last_hour = hour;
                track.last = Some((*identity, hour));
                continue;
            }
            let mut value = sample.clone();
            value.first_hour = hour;
            value.last_hour = hour;
            value.starts_segment = !contiguous;
            track.runs.push(value);
            track.last = Some((*identity, hour));
        }
    }

    pub(super) fn finish(self) -> History {
        let mut history = History {
            warnings: self.warnings,
            ..History::default()
        };
        history
            .warnings
            .extend(self.refused.lines("recorded occupation trend facts"));
        history.collaboration_warnings = self
            .collaboration_refused
            .lines("recorded occupation trend facts");
        for (tag, track) in self.tracks {
            if !track.runs.is_empty() {
                history.series.insert(tag.clone(), track.runs);
                history.segments.insert(tag, track.segments);
            }
        }
        history
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::occupation_trend::{OccupationTrendParams, reply};
    use replay::rows::{CountryCollaborationRow, StateGarrisonRow, TableRows};

    #[test]
    fn occupation_trend_collaboration_only_read_splits_after_unchanged_projection() {
        let countries = [
            CountryRow {
                id: 1,
                canonical_slot: 11,
                tag: b"Test occupier".to_vec(),
                ..Default::default()
            },
            CountryRow {
                id: 2,
                canonical_slot: 12,
                tag: b"Test occupied country".to_vec(),
                ..Default::default()
            },
        ];
        let links = [
            CountryLinkRow {
                id: 1,
                canonical_slot: 11,
            },
            CountryLinkRow {
                id: 2,
                canonical_slot: 12,
            },
        ];
        let countries = Arc::new(country_index(&countries, &links, &mut Refusals::default()));
        let state = StateRow {
            id: 1,
            controller: 1,
            owner: 1,
            ..Default::default()
        };
        let resistance = StateResistanceRow {
            state: 1,
            country: 2,
            active: 1,
            level: 10,
            compliance: 20,
            ..Default::default()
        };
        let garrisons = [StateGarrisonRow {
            controller: 1,
            occupied: 2,
            state: 1,
            ..Default::default()
        }];
        let mut frame = crate::test_fixtures::source(1, 100, None);
        *frame.tables[TableId::CountryCollaboration.index()] =
            TableView::Complete(TableRows::CountryCollaboration(vec![
                CountryCollaborationRow {
                    source: 2,
                    target: 1,
                    progress: 12345,
                },
            ]));
        frame.metrics.table_ns[TableId::CountryCollaboration.index()] = 1;
        let mut scan = TrendScan::default();
        scan.begin(100, countries.clone(), &links, garrisons.iter(), true);
        scan.state(&resistance, &state, &countries.identities, &links);
        scan.finish_read();
        scan.collaboration(&frame, 100, &countries.identities, &links)
            .unwrap();
        scan.project(100, &countries.names, true);
        scan.project(101, &countries.names, false);
        assert!(!scan.dirty);
        assert_eq!(scan.tracks["Test occupier"].runs.len(), 1);
        assert_eq!(scan.tracks["Test occupier"].runs[0].last_hour, 101);

        // Exercise the independent collaboration consumer without begin/state:
        // its dirty flag must split a previously unchanged hourly projection.
        // Recording cadence orchestration remains covered by the shared scan.
        frame.hour = 102;
        if let TableView::Complete(TableRows::CountryCollaboration(rows)) =
            &mut *frame.tables[TableId::CountryCollaboration.index()]
        {
            rows[0].progress = 23456;
        }
        scan.collaboration(&frame, 102, &countries.identities, &links)
            .unwrap();
        assert!(scan.dirty);
        scan.project(102, &countries.names, false);
        scan.project(103, &countries.names, false);
        let history = scan.finish();
        let mut params = OccupationTrendParams {
            branch_id: "1".into(),
            tags: vec![],
            top: None,
            collaboration: true,
        };
        let with = reply("1", &history, &params, vec![], &AtomicBool::new(false)).unwrap();
        let runs = &with.series[0].runs;
        assert_eq!(
            runs.iter()
                .map(|run| (
                    run.first_hour,
                    run.last_hour,
                    run.read_hour,
                    run.collaboration[0].raw.as_deref(),
                    run.collaboration[0].read_hour
                ))
                .collect::<Vec<_>>(),
            [
                (100, 101, 100, Some("12345"), Some(100)),
                (102, 103, 100, Some("23456"), Some(102)),
            ]
        );
        assert!(!runs[1].starts_segment);
        assert!(
            runs.iter()
                .all(|run| run.resistance_raw == "10" && run.compliance_raw == "20")
        );
        params.collaboration = false;
        let without = reply("1", &history, &params, vec![], &AtomicBool::new(false)).unwrap();
        assert_eq!(without.series[0].runs.len(), 1);
        assert_eq!(
            (
                without.series[0].runs[0].first_hour,
                without.series[0].runs[0].last_hour,
                without.series[0].runs[0].read_hour
            ),
            (100, 103, 100)
        );
    }

    #[test]
    fn occupation_trend_mean_is_native_fixed5_wrapping_and_truncation() {
        assert_eq!(mean(3, 2), Some(1));
        assert_eq!(mean(-3, 2), Some(-1));
        assert_eq!(mean(0, 0), None);
        assert_eq!(mean(i64::MAX, 1), None);
        let sum = 9007199254740993i64;
        assert_eq!(mean(sum, 1), Some(-31705341376687));
    }
}
