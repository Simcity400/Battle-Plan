//! Demand-driven retained script state reduced from the existing event feed.
//! Baselines prove membership at attachment; they never create past fires.
use super::*;
use std::collections::BTreeMap;
mod delayed;
pub use delayed::DelayedEvent;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub struct ScriptIdentity {
    pub native_event: EventFeedEventIdentity,
    pub token: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PendingEvent {
    pub instance: u32,
    pub definition: ScriptIdentity,
    pub originator: EventFeedCountry,
    pub due_hour: i32,
    /// Binding IDs are local to this graph, preserving shared/self links.
    pub scopes: Vec<EventFeedScope>,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct ScriptEventState {
    #[serde(with = "owner_pairs")]
    decisions: BTreeMap<(u32, u32), super::decisions::DecisionState>,
    #[serde(with = "owner_pairs")]
    delayed: BTreeMap<(u32, u32), delayed::Owner>,
    generation: Option<u64>,
    hour: Option<i32>,
    sequence: Option<u64>,
    pending: Vec<PendingEvent>,
    fired: Vec<ScriptIdentity>,
    pending_ready: bool,
    fired_ready: bool,
    pending_fault: bool,
    fired_fault: bool,
    pending_building: bool,
    fired_building: bool,
}
impl ScriptEventState {
    pub fn decisions(
        &self,
        country: EventFeedCountry,
    ) -> Result<&super::decisions::DecisionState, &'static str> {
        self.decisions
            .get(&(country.id, country.instance))
            .filter(|state| state.complete())
            .ok_or("decision owner baseline/writer certificate unavailable")
    }
    pub fn decision_owners(&self) -> impl Iterator<Item = &super::decisions::DecisionState> {
        self.decisions.values()
    }

    pub fn delayed(&self, country: EventFeedCountry) -> Result<&[DelayedEvent], &'static str> {
        self.delayed
            .get(&(country.id, country.instance))
            .ok_or("country delayed-event baseline was not observed")?
            .rows()
    }
    pub fn delayed_owners(
        &self,
    ) -> impl Iterator<Item = (EventFeedCountry, Result<&[DelayedEvent], &'static str>)> {
        self.delayed
            .values()
            .map(|owner| (owner.country, owner.rows()))
    }
    pub fn pending(&self) -> Result<&[PendingEvent], &'static str> {
        self.pending_ready
            .then_some(self.pending.as_slice())
            .ok_or("pending event registry has no complete baseline and writer coverage")
    }
    /// Native membership retains duplicates; callers may sort for set display
    /// without inventing an occurrence time for pre-attachment members.
    pub fn fired(&self) -> Result<&[ScriptIdentity], &'static str> {
        self.fired_ready
            .then_some(self.fired.as_slice())
            .ok_or("fired_event_names has no complete baseline and writer coverage")
    }
    pub fn hour(&self) -> Option<i32> {
        self.hour
    }
    pub fn observe(&mut self, row: &EventFeedHistoryRow) -> io::Result<()> {
        self.observe_decisions(row, |_, _| Ok(()))
    }
    /// Visit each decision boundary after reducing its preceding changes.
    pub fn observe_decisions(
        &mut self,
        row: &EventFeedHistoryRow,
        mut visit: impl FnMut(&super::decisions::DecisionState, &EventFeedEvent) -> io::Result<()>,
    ) -> io::Result<()> {
        row.validate()?;
        if self.generation != Some(row.generation) {
            *self = Self {
                generation: Some(row.generation),
                ..Self::default()
            };
        }
        if self
            .hour
            .is_some_and(|hour| hour.checked_add(1) != Some(row.read_hour))
        {
            for owner in self.decisions.values_mut() {
                owner.gap();
            }
            self.pending_ready = false;
            self.fired_ready = false;
            for owner in self.delayed.values_mut() {
                owner.gap();
            }
        }
        let mut pending_baseline = false;
        let mut fired_baseline = false;
        for event in &row.events {
            if self.sequence.is_some_and(|last| event.sequence <= last) {
                return Err(invalid("script state occurrence order differs"));
            }
            self.sequence = Some(event.sequence);
            let Some(script) = event.script else {
                continue;
            };
            let Some(change) = script.change else {
                continue;
            };
            use EventFeedScriptChange::*;
            match change {
                Decision(change) => {
                    let owner = self
                        .decisions
                        .entry((event.actor.id, event.actor.instance))
                        .or_default();
                    owner.observe(row, event, script, change)?;
                    visit(owner, event)?;
                }
                PendingBaseline => {
                    self.pending.clear();
                    self.pending_ready = false;
                    self.pending_fault = false;
                    self.pending_building = true;
                }
                PendingStored { due_hour } => {
                    if script.token_unrecorded || script.scope_unrecorded || script.scope_len == 0 {
                        self.pending_fault = true;
                        self.pending_ready = false;
                        continue;
                    }
                    let start = script.scope_start as usize;
                    let scopes = &row.script_scopes[start..start + script.scope_len as usize];
                    allocation::reserve_exact(&mut self.pending, 1)?;
                    self.pending.push(PendingEvent {
                        instance: script
                            .instance
                            .ok_or_else(|| invalid("pending event has no instance"))?,
                        definition: identity(row, script)?,
                        originator: event.actor,
                        due_hour,
                        scopes: allocation::copy_vec(scopes)?,
                    });
                }
                PendingReady => {
                    if !self.pending_building {
                        return Err(invalid("pending registry completion has no baseline"));
                    }
                    self.pending_building = false;
                    self.pending_ready = !self.pending_fault;
                    pending_baseline = true;
                }
                PendingRemoved => {
                    let instance = script
                        .instance
                        .ok_or_else(|| invalid("pending removal has no instance"))?;
                    if let Some(index) =
                        self.pending.iter().position(|row| row.instance == instance)
                    {
                        // Both native removal writers replace with the last entry.
                        self.pending.swap_remove(index);
                    } else if self.pending_ready {
                        self.pending_fault = true;
                        self.pending_ready = false;
                    }
                }
                FiredBaseline => {
                    self.fired.clear();
                    self.fired_ready = false;
                    self.fired_fault = false;
                    self.fired_building = true;
                }
                FiredMember => {
                    if script.token_unrecorded {
                        self.fired_fault = true;
                        self.fired_ready = false;
                        continue;
                    }
                    allocation::reserve_exact(&mut self.fired, 1)?;
                    self.fired.push(identity(row, script)?);
                }
                FiredRemoved => {
                    if script.token_unrecorded {
                        self.fired_fault = true;
                        self.fired_ready = false;
                        continue;
                    }
                    let removed = identity(row, script)?;
                    if let Some(index) = self.fired.iter().position(|member| *member == removed) {
                        // Native replacement erases one matched definition, not every
                        // equal identity. Initial duplicates retain multiplicity.
                        self.fired.remove(index);
                    } else if self.fired_ready {
                        self.fired_fault = true;
                        self.fired_ready = false;
                    }
                }
                FiredReady => {
                    if !self.fired_building {
                        return Err(invalid("fired registry completion has no baseline"));
                    }
                    self.fired_building = false;
                    self.fired_ready = !self.fired_fault;
                    fired_baseline = true;
                }
                DelayedBaseline
                | DelayedReady
                | DelayedEmpty
                | DelayedStored { .. }
                | DelayedAdvance { .. }
                | DelayedRemoved { .. } => {
                    self.delayed
                        .entry((event.target.id, event.target.instance))
                        .or_default()
                        .observe(row, event, script, change)?;
                }
            }
        }
        if row.coverage == EventFeedCoverage::Unavailable || row.failure.is_some() {
            for owner in self.decisions.values_mut() {
                owner.gap();
            }
            self.pending_ready = false;
            self.fired_ready = false;
            for owner in self.delayed.values_mut() {
                owner.gap();
            }
        }
        if row.script_refusals != 0 {
            for owner in self.decisions.values_mut() {
                owner.gap();
            }
        }
        if row.coverage == EventFeedCoverage::Partial {
            for owner in self.decisions.values_mut() {
                owner.partial(row.read_hour);
            }
            self.pending_ready &= pending_baseline;
            self.fired_ready &= fired_baseline;
            for owner in self.delayed.values_mut() {
                owner.partial(row.read_hour);
            }
        }
        self.hour = Some(row.read_hour);
        Ok(())
    }
}
fn identity(row: &EventFeedHistoryRow, script: EventFeedScript) -> io::Result<ScriptIdentity> {
    let start = script.token_start as usize;
    let end = start + script.token_len as usize;
    Ok(ScriptIdentity {
        native_event: script
            .native_event
            .ok_or_else(|| invalid("script registry event identity absent"))?,
        token: allocation::copy_vec(&row.script_tokens[start..end])?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use EventFeedScriptChange::*;
    fn row(
        hour: i32,
        first: u64,
        changes: &[(EventFeedScriptChange, Option<u32>, Option<u32>)],
    ) -> EventFeedHistoryRow {
        EventFeedHistoryRow {
            generation: 7,
            read_hour: hour,
            coverage: EventFeedCoverage::Complete,
            counts: Some(EventFeedCounts {
                acquired: 1000,
                settled: 1000,
                ..Default::default()
            }),
            script_tokens: b"synthetic.1".to_vec(),
            script_scopes: vec![EventFeedScope::default()],
            events: changes
                .iter()
                .enumerate()
                .map(|(n, (change, instance, id))| EventFeedEvent {
                    sequence: first + n as u64,
                    generation: 7,
                    hour,
                    source: EventFeedSource::ScriptState,
                    actor: if matches!(change, PendingStored { .. } | DelayedStored { .. }) {
                        EventFeedCountry {
                            reference: 3,
                            id: 2,
                            instance: 9,
                        }
                    } else {
                        Default::default()
                    },
                    target: if matches!(
                        change,
                        DelayedBaseline
                            | DelayedReady
                            | DelayedEmpty
                            | DelayedStored { .. }
                            | DelayedAdvance { .. }
                            | DelayedRemoved { .. }
                    ) {
                        EventFeedCountry {
                            reference: 6,
                            id: 5,
                            instance: 7,
                        }
                    } else {
                        Default::default()
                    },
                    cause: Recorded::Value(Default::default()),
                    script: Some(EventFeedScript {
                        token_len: if id.is_some() { 11 } else { 0 },
                        scope_len: u32::from(matches!(
                            change,
                            PendingStored { .. } | DelayedStored { .. }
                        )),
                        native_event: id.map(|id| EventFeedEventIdentity { kind: 99, id }),
                        instance: *instance,
                        change: Some(*change),
                        ..Default::default()
                    }),
                })
                .collect(),
            ..Default::default()
        }
    }
    #[test]
    fn refused_decision_families_do_not_poison_independent_registry_certificates() {
        let mut state = ScriptEventState::default();
        let mut baseline = row(
            10,
            0,
            &[
                (PendingBaseline, None, None),
                (PendingReady, None, None),
                (FiredBaseline, None, None),
                (FiredReady, None, None),
            ],
        );
        baseline.script_refusals = DECISION_FAMILY_REFUSALS;
        state.observe(&baseline).unwrap();
        assert!(state.pending().unwrap().is_empty());
        assert!(state.fired().unwrap().is_empty());
        let mut next = row(11, 4, &[]);
        next.script_refusals = DECISION_FAMILY_REFUSALS;
        state.observe(&next).unwrap();
        assert_eq!(next.refused_script_families().count(), 5);
        assert!(state.pending().unwrap().is_empty());
        assert!(state.fired().unwrap().is_empty());
    }
    #[test]
    fn sparse_pass_cancellation_behind_its_cursor_closes_without_decrementing_a_swapped_tail() {
        let owner = EventFeedCountry {
            reference: 6,
            id: 5,
            instance: 7,
        };
        let mut state = ScriptEventState::default();
        state
            .observe(&row(
                1,
                0,
                &[
                    (DelayedBaseline, None, None),
                    (DelayedStored { remaining: 9 }, Some(1), Some(91)),
                    (DelayedStored { remaining: 9 }, Some(2), Some(92)),
                    (DelayedStored { remaining: 9 }, Some(3), Some(93)),
                    (DelayedReady, None, None),
                ],
            ))
            .unwrap();
        state
            .observe(&row(
                2,
                5,
                &[
                    (
                        DelayedAdvance {
                            count: 2,
                            finished: false,
                        },
                        None,
                        None,
                    ),
                    (
                        DelayedRemoved {
                            iteration: false,
                            visited: false,
                        },
                        Some(1),
                        None,
                    ),
                    (
                        DelayedRemoved {
                            iteration: false,
                            visited: false,
                        },
                        Some(2),
                        None,
                    ),
                    (
                        DelayedAdvance {
                            count: 0,
                            finished: true,
                        },
                        None,
                        None,
                    ),
                ],
            ))
            .unwrap();
        let timers = state.delayed(owner).unwrap();
        assert_eq!(timers.len(), 1);
        assert_eq!((timers[0].instance, timers[0].remaining), (3, 9));
    }
    #[test]
    fn fired_replacement_erases_one_member_without_reset_and_survives_codec_round_trip() {
        let mut state = ScriptEventState::default();
        state
            .observe(&row(
                10,
                0,
                &[
                    (FiredBaseline, None, None),
                    (FiredMember, None, Some(91)),
                    (FiredMember, None, Some(91)),
                    (FiredMember, None, Some(92)),
                    (FiredReady, None, None),
                ],
            ))
            .unwrap();
        let delta = row(
            11,
            5,
            &[
                (FiredRemoved, None, Some(91)),
                (FiredMember, None, Some(93)),
            ],
        );
        let mut encoded = Vec::new();
        delta.encode(&mut encoded).unwrap();
        state
            .observe(&EventFeedHistoryRow::decode(&mut Decoder::new(&encoded)).unwrap())
            .unwrap();
        assert_eq!(
            state
                .fired()
                .unwrap()
                .iter()
                .map(|member| member.native_event.id)
                .collect::<Vec<_>>(),
            [91, 92, 93]
        );
        state
            .observe(&row(12, 7, &[(FiredRemoved, None, Some(91))]))
            .unwrap();
        assert_eq!(
            state
                .fired()
                .unwrap()
                .iter()
                .map(|member| member.native_event.id)
                .collect::<Vec<_>>(),
            [92, 93]
        );
        state
            .observe(&row(13, 8, &[(FiredRemoved, None, Some(99))]))
            .unwrap();
        assert!(
            state.fired().is_err(),
            "an erased unknown member invalidates coverage"
        );
    }
    #[test]
    fn registry_baseline_and_deltas_retain_duplicate_instances_and_fire_once_members_without_inventing_fires()
     {
        let mut state = ScriptEventState::default();
        let mut baseline = row(
            10,
            0,
            &[
                (PendingBaseline, None, None),
                (PendingStored { due_hour: -25 }, Some(3), Some(90)),
                (PendingStored { due_hour: 50 }, Some(4), Some(91)),
                (PendingStored { due_hour: 60 }, Some(3), Some(92)),
                (PendingReady, None, None),
                (FiredBaseline, None, None),
                (FiredMember, None, Some(90)),
                (FiredMember, None, Some(90)),
                (FiredReady, None, None),
            ],
        );
        baseline.coverage = EventFeedCoverage::Partial;
        let mut bytes = Vec::new();
        baseline.encode(&mut bytes).unwrap();
        let decoded = EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap();
        assert_eq!(decoded, baseline);
        state.observe(&decoded).unwrap();
        assert_eq!(
            state
                .pending()
                .unwrap()
                .iter()
                .map(|row| row.definition.native_event.id)
                .collect::<Vec<_>>(),
            [90, 91, 92]
        );
        assert_eq!(state.pending().unwrap()[0].originator.instance, 9);
        assert_eq!(state.pending().unwrap()[0].due_hour, -25);
        assert_eq!(state.fired().unwrap().len(), 2);
        let delta = row(
            11,
            9,
            &[
                (PendingRemoved, Some(3), None),
                (FiredMember, None, Some(91)),
            ],
        );
        state.observe(&delta).unwrap();
        assert_eq!(
            state
                .pending()
                .unwrap()
                .iter()
                .map(|row| row.definition.native_event.id)
                .collect::<Vec<_>>(),
            [92, 91]
        );
        assert_eq!(state.fired().unwrap().len(), 3);
        state.observe(&row(12, 12, &[])).unwrap();
        assert_eq!(state.pending().unwrap().len(), 2);
        for end in 0..bytes.len() {
            assert!(EventFeedHistoryRow::decode(&mut Decoder::new(&bytes[..end])).is_err());
        }
    }
    #[test]
    fn registry_gaps_unknown_operands_and_world_replacement_require_a_new_baseline() {
        let mut state = ScriptEventState::default();
        state
            .observe(&row(
                1,
                0,
                &[
                    (PendingBaseline, None, None),
                    (PendingReady, None, None),
                    (FiredBaseline, None, None),
                    (FiredReady, None, None),
                ],
            ))
            .unwrap();
        assert!(state.pending().unwrap().is_empty());
        assert!(state.fired().unwrap().is_empty());
        state.observe(&row(3, 4, &[])).unwrap();
        assert!(state.pending().is_err());
        assert!(state.fired().is_err());
        let mut partial = row(
            4,
            4,
            &[
                (PendingBaseline, None, None),
                (PendingStored { due_hour: 5 }, Some(1), Some(90)),
                (PendingReady, None, None),
            ],
        );
        let script = partial.events[1].script.as_mut().unwrap();
        script.scope_len = 0;
        script.scope_unrecorded = true;
        state.observe(&partial).unwrap();
        assert!(state.pending().is_err());
        let mut next = row(
            5,
            0,
            &[(PendingBaseline, None, None), (PendingReady, None, None)],
        );
        next.generation = 8;
        for event in &mut next.events {
            event.generation = 8;
        }
        state.observe(&next).unwrap();
        assert!(state.pending().unwrap().is_empty());
        assert!(state.fired().is_err());
        assert!(
            state
                .observe(&row(6, 6, &[(PendingReady, None, None)]))
                .is_err()
        );
    }
    #[test]
    fn registry_delayed_prefixes_replay_in_pass_schedule_cancel_swap_and_signed_countdowns() {
        let owner = EventFeedCountry {
            reference: 6,
            id: 5,
            instance: 7,
        };
        let mut state = ScriptEventState::default();
        state
            .observe(&row(
                1,
                0,
                &[
                    (DelayedBaseline, None, None),
                    (DelayedStored { remaining: 1 }, Some(1), Some(90)),
                    (DelayedStored { remaining: 5 }, Some(2), Some(91)),
                    (DelayedReady, None, None),
                ],
            ))
            .unwrap();
        // Fire the first timer; its immediate effects append another timer.
        // Swap-removal visits the newly appended timer in this same pass.
        state
            .observe(&row(
                2,
                4,
                &[
                    (
                        DelayedAdvance {
                            count: 1,
                            finished: false,
                        },
                        None,
                        None,
                    ),
                    (DelayedStored { remaining: 9 }, Some(3), Some(92)),
                    (
                        DelayedRemoved {
                            iteration: true,
                            visited: true,
                        },
                        Some(1),
                        None,
                    ),
                    (
                        DelayedAdvance {
                            count: 2,
                            finished: true,
                        },
                        None,
                        None,
                    ),
                ],
            ))
            .unwrap();
        let remaining = |state: &ScriptEventState| {
            state
                .delayed(owner)
                .unwrap()
                .iter()
                .map(|timer| (timer.instance, timer.remaining))
                .collect::<Vec<_>>()
        };
        assert_eq!(remaining(&state), [(3, 8), (2, 4)]);
        state
            .observe(&row(
                3,
                8,
                &[(
                    DelayedAdvance {
                        count: 2,
                        finished: true,
                    },
                    None,
                    None,
                )],
            ))
            .unwrap();
        assert_eq!(remaining(&state), [(3, 7), (2, 3)]);
        state
            .observe(&row(
                4,
                9,
                &[
                    (
                        DelayedAdvance {
                            count: 1,
                            finished: false,
                        },
                        None,
                        None,
                    ),
                    (
                        DelayedRemoved {
                            iteration: false,
                            visited: false,
                        },
                        Some(2),
                        None,
                    ),
                    (DelayedStored { remaining: 7 }, Some(4), Some(93)),
                    (
                        DelayedAdvance {
                            count: 1,
                            finished: true,
                        },
                        None,
                        None,
                    ),
                ],
            ))
            .unwrap();
        assert_eq!(remaining(&state), [(3, 6), (4, 6)]);
        state
            .observe(&row(
                5,
                13,
                &[
                    (
                        DelayedAdvance {
                            count: 1,
                            finished: false,
                        },
                        None,
                        None,
                    ),
                    (
                        DelayedRemoved {
                            iteration: true,
                            visited: false,
                        },
                        Some(4),
                        None,
                    ),
                    (
                        DelayedAdvance {
                            count: 0,
                            finished: true,
                        },
                        None,
                        None,
                    ),
                ],
            ))
            .unwrap();
        assert_eq!(remaining(&state), [(3, 5)]);
        let mut wrapped = row(
            6,
            16,
            &[
                (DelayedEmpty, None, None),
                (
                    DelayedStored {
                        remaining: i32::MIN,
                    },
                    Some(5),
                    Some(94),
                ),
                (
                    DelayedAdvance {
                        count: 1,
                        finished: true,
                    },
                    None,
                    None,
                ),
            ],
        );
        let mut bytes = Vec::new();
        wrapped.encode(&mut bytes).unwrap();
        wrapped = EventFeedHistoryRow::decode(&mut Decoder::new(&bytes)).unwrap();
        state.observe(&wrapped).unwrap();
        assert_eq!(remaining(&state), [(5, i32::MAX)]);
        state
            .observe(&row(
                7,
                19,
                &[(
                    DelayedAdvance {
                        count: 1,
                        finished: false,
                    },
                    None,
                    None,
                )],
            ))
            .unwrap();
        assert!(
            state.delayed(owner).is_err(),
            "an unclosed native pass cannot certify the boundary"
        );
    }
    #[test]
    fn registry_quiet_delayed_pass_encodes_one_compact_count_for_five_thousand_timers() {
        let owner = EventFeedCountry {
            reference: 6,
            id: 5,
            instance: 7,
        };
        let mut changes = vec![(DelayedBaseline, None, None)];
        for id in 1..=5000 {
            changes.push((DelayedStored { remaining: 20 }, Some(id), Some(id)));
        }
        changes.push((DelayedReady, None, None));
        let mut baseline = row(1, 0, &changes);
        baseline.counts.as_mut().unwrap().acquired = 10000;
        baseline.counts.as_mut().unwrap().settled = 10000;
        let mut state = ScriptEventState::default();
        state.observe(&baseline).unwrap();
        let mut quiet = row(
            2,
            5002,
            &[(
                DelayedAdvance {
                    count: 5000,
                    finished: true,
                },
                None,
                None,
            )],
        );
        quiet.counts.as_mut().unwrap().acquired = 10000;
        quiet.counts.as_mut().unwrap().settled = 10000;
        assert_eq!(quiet.events.len(), 1);
        assert!(quiet.events[0].encoded_bytes().unwrap() <= 20);
        state.observe(&quiet).unwrap();
        assert_eq!(state.delayed(owner).unwrap().len(), 5000);
        assert!(
            state
                .delayed(owner)
                .unwrap()
                .iter()
                .all(|timer| timer.remaining == 19)
        );
    }
}

// JSON continuations encode tuple-keyed owner maps as ordered pairs.
mod owner_pairs {
    use super::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    pub fn serialize<S: Serializer, T: Serialize>(
        map: &BTreeMap<(u32, u32), T>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(map.iter())
    }
    pub fn deserialize<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<(u32, u32), T>, D::Error> {
        Ok(Vec::<((u32, u32), T)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}
