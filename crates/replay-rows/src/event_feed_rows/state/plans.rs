use super::super::decisions::{
    ALL, CLEAR_ALL_CANDIDATES, CONTAINERS, DecisionChange, DecisionState, IGNORED,
    REMOVE_REQUESTED, TIMED,
};
use super::*;

fn reduce_every(
    state: &mut ScriptEventState,
    row: &EventFeedHistoryRow,
    mut visit: impl FnMut(&DecisionState, &EventFeedEvent) -> io::Result<()>,
) -> io::Result<()> {
    row.validate()?;
    state.touched = Touched::default();
    if state.generation != Some(row.generation) {
        *state = ScriptEventState {
            generation: Some(row.generation),
            clock: state.clock,
            ..ScriptEventState::default()
        };
        state.touched.all = true;
    }
    if state
        .hour
        .is_some_and(|hour| hour.checked_add(1) != Some(row.read_hour))
    {
        state.touched.all = true;
        for owner in state.decisions.values_mut() {
            owner.gap();
        }
        state.pending_ready = false;
        state.fired_ready = false;
        for owner in state.delayed.values_mut() {
            owner.gap();
        }
    }
    if row.coverage != EventFeedCoverage::Complete || row.failure.is_some() {
        state.touched.all = true;
        for owner in state.delayed.values_mut() {
            owner.gap();
        }
    }
    let mut pending_baseline = false;
    let mut fired_baseline = false;
    for event in &row.events {
        if state.sequence.is_some_and(|last| event.sequence <= last) {
            return Err(invalid("script state occurrence order differs"));
        }
        state.sequence = Some(event.sequence);
        let Some(script) = event.script else {
            continue;
        };
        let Some(change) = script.change else {
            continue;
        };
        use EventFeedScriptChange::*;
        match change {
            PendingBaseline | PendingStored { .. } | PendingReady | PendingRemoved => {
                state.touched.pending = true;
            }
            FiredBaseline | FiredMember | FiredRemoved | FiredReady => {
                state.touched.fired = true;
            }
            _ => {}
        }
        match change {
            Decision(change) => {
                let key = owner_key(&state.decisions, event.actor);
                let before = state
                    .decisions
                    .get(&key)
                    .map(|owner| (owner.country, owner.complete()));
                let owner = state.decisions.entry(key).or_default();
                owner.observe(row, event, script, change)?;
                if change.operation != Operation::Rebuild
                    || before != Some((owner.country, owner.complete()))
                {
                    state.touched.decisions.insert(key);
                }
                visit(owner, event)?;
            }
            PendingBaseline => {
                state.pending.clear();
                state.pending_ready = false;
                state.pending_fault = false;
                state.pending_building = true;
            }
            PendingStored { due_hour } => {
                if script.token_unrecorded || script.scope_unrecorded || script.scope_len == 0 {
                    state.pending_fault = true;
                    state.pending_ready = false;
                    continue;
                }
                let start = script.scope_start as usize;
                state.pending.push(PendingEvent {
                    instance: script
                        .instance
                        .ok_or_else(|| invalid("pending event has no instance"))?,
                    definition: identity(row, script)?,
                    originator: event.actor,
                    due_hour,
                    scopes: row.script_scopes[start..start + script.scope_len as usize].to_vec(),
                });
            }
            PendingReady => {
                if !state.pending_building {
                    return Err(invalid("pending registry completion has no baseline"));
                }
                state.pending_building = false;
                state.pending_ready = !state.pending_fault;
                pending_baseline = true;
            }
            PendingRemoved => {
                let instance = script
                    .instance
                    .ok_or_else(|| invalid("pending removal has no instance"))?;
                if let Some(index) = state
                    .pending
                    .iter()
                    .position(|row| row.instance == instance)
                {
                    state.pending.swap_remove(index);
                } else if state.pending_ready {
                    state.pending_fault = true;
                    state.pending_ready = false;
                }
            }
            FiredBaseline => {
                state.fired.clear();
                state.fired_ready = false;
                state.fired_fault = false;
                state.fired_building = true;
            }
            FiredMember => {
                if script.token_unrecorded {
                    state.fired_fault = true;
                    state.fired_ready = false;
                    continue;
                }
                state.fired.push(identity(row, script)?);
            }
            FiredRemoved => {
                if script.token_unrecorded {
                    state.fired_fault = true;
                    state.fired_ready = false;
                    continue;
                }
                let removed = identity(row, script)?;
                if let Some(index) = state.fired.iter().position(|member| *member == removed) {
                    state.fired.remove(index);
                } else if state.fired_ready {
                    state.fired_fault = true;
                    state.fired_ready = false;
                }
            }
            FiredReady => {
                if !state.fired_building {
                    return Err(invalid("fired registry completion has no baseline"));
                }
                state.fired_building = false;
                state.fired_ready = !state.fired_fault;
                fired_baseline = true;
            }
            DelayedBaseline
            | DelayedReady
            | DelayedEmpty
            | DelayedStored { .. }
            | DelayedAdvance { .. }
            | DelayedRemoved { .. }
            | DelayedRefused => {
                let key = owner_key(&state.delayed, event.target);
                state.touched.delayed.insert(key);
                state
                    .delayed
                    .entry(key)
                    .or_default()
                    .observe(row, event, script, change)?;
            }
        }
    }
    if row.coverage == EventFeedCoverage::Unavailable
        || row.failure.is_some()
        || row.script_refusals != 0
        || row.coverage == EventFeedCoverage::Partial
    {
        state.touched.all = true;
    }
    if row.coverage == EventFeedCoverage::Unavailable || row.failure.is_some() {
        for owner in state.decisions.values_mut() {
            owner.gap();
        }
        state.pending_ready = false;
        state.fired_ready = false;
        for owner in state.delayed.values_mut() {
            owner.gap();
        }
    }
    if row.script_refusals != 0 {
        for owner in state.decisions.values_mut() {
            owner.gap();
        }
    }
    if row.coverage == EventFeedCoverage::Partial {
        for owner in state.decisions.values_mut() {
            owner.partial(row.read_hour);
        }
        state.pending_ready &= pending_baseline;
        state.fired_ready &= fired_baseline;
        for owner in state.delayed.values_mut() {
            owner.partial(row.read_hour);
        }
    }
    state.hour = Some(row.read_hour);
    state.stamp();
    Ok(())
}

const TOKENS: &[u8] = b"alphabravocharliedelta";
const SPANS: [(u32, u32); 4] = [(0, 5), (5, 5), (10, 7), (17, 5)];

fn country(reference: u32, id: u32, instance: u32) -> EventFeedCountry {
    EventFeedCountry {
        reference,
        id,
        instance,
    }
}

struct Op {
    change: EventFeedScriptChange,
    actor: EventFeedCountry,
    target: EventFeedCountry,
    token: Option<usize>,
    unrecorded: bool,
    instance: Option<u32>,
    days: Option<i32>,
    lifecycle: Option<i32>,
    country_target: Option<u32>,
}

impl Op {
    fn decision(actor: EventFeedCountry, change: DecisionChange) -> Self {
        Self {
            change: EventFeedScriptChange::Decision(change),
            actor,
            target: EventFeedCountry::default(),
            token: None,
            unrecorded: false,
            instance: None,
            days: None,
            lifecycle: None,
            country_target: None,
        }
    }
}

fn read(generation: u64, hour: i32, first: u64, ops: Vec<Op>) -> EventFeedHistoryRow {
    let events = ops
        .into_iter()
        .enumerate()
        .map(|(index, op)| {
            let (token_start, token_len) = match (op.unrecorded, op.token) {
                (false, Some(token)) => SPANS[token],
                _ => (0, 0),
            };
            let stored = matches!(op.change, EventFeedScriptChange::DelayedStored { .. });
            EventFeedEvent {
                sequence: first + index as u64,
                generation,
                hour,
                source: EventFeedSource::ScriptState,
                actor: op.actor,
                target: op.target,
                cause: Recorded::Value(Default::default()),
                script: Some(EventFeedScript {
                    token_start,
                    token_len,
                    scope_len: u32::from(stored),
                    native_event: stored.then_some(EventFeedEventIdentity { kind: 2, id: 40 }),
                    instance: op.instance,
                    country_target: op.country_target,
                    token_unrecorded: op.unrecorded,
                    change: Some(op.change),
                    lifecycle: op.lifecycle,
                    days: op.days,
                    ..Default::default()
                }),
            }
        })
        .collect();
    EventFeedHistoryRow {
        generation,
        read_hour: hour,
        coverage: EventFeedCoverage::Complete,
        counts: Some(EventFeedCounts {
            acquired: 100_000,
            settled: 100_000,
            ..Default::default()
        }),
        script_tokens: TOKENS.to_vec(),
        script_scopes: vec![EventFeedScope::default()],
        events,
        ..Default::default()
    }
}

fn candidate(actor: EventFeedCountry, kind: u8, flags: u8, token: usize, target: u32) -> Op {
    Op {
        token: Some(token),
        target: country(target, target, 0),
        country_target: Some(target),
        ..Op::decision(
            actor,
            DecisionChange {
                flags,
                ordinal: target,
                ..DecisionChange::new(Operation::Candidate, kind)
            },
        )
    }
}

fn is_rebuild(event: &EventFeedEvent) -> bool {
    matches!(
        event.script.and_then(|script| script.change),
        Some(EventFeedScriptChange::Decision(change))
            if change.operation == Operation::Rebuild && change.container == 3
    )
}

fn rostered(owner: &DecisionState) -> serde_json::Value {
    let members: Vec<_> = (0..CONTAINERS)
        .map(|container| owner.members(container as u8).map(<[_]>::to_vec).ok())
        .collect();
    serde_json::json!([
        owner.country,
        owner.complete(),
        members,
        owner.random_counts,
        owner.ignored,
        owner.target_countries,
    ])
}

fn touched(state: &ScriptEventState) -> serde_json::Value {
    let touched = state.touched();
    serde_json::json!([
        touched.all,
        touched.pending,
        touched.fired,
        touched.delayed,
        touched.decisions,
    ])
}

struct Reductions {
    every: ScriptEventState,
    rows: ScriptEventState,
    planned: ScriptEventState,
    walk: Written,
    payloads: Payloads,
    held: HeldPayloads,
    sure: bool,
}

impl Reductions {
    fn new() -> Self {
        Self {
            every: ScriptEventState::default(),
            rows: ScriptEventState::default(),
            planned: ScriptEventState::default(),
            walk: Written::default(),
            payloads: Payloads::default(),
            held: HeldPayloads::default(),
            sure: true,
        }
    }

    fn restart(&mut self) {
        self.walk = Written::default();
        self.payloads = Payloads::default();
        self.sure = true;
    }

    fn reduce(&mut self, row: &EventFeedHistoryRow) -> Option<usize> {
        let mut visits: [Vec<(u64, serde_json::Value)>; 3] = Default::default();
        let [every, rows, planned] = &mut visits;
        let expected = reduce_every(&mut self.every, row, |owner, event| {
            if is_rebuild(event) {
                every.push((event.sequence, rostered(owner)));
            }
            Ok(())
        });
        let by_rows = self.rows.observe_decisions(row, |owner, change| {
            if is_rebuild(change.event) {
                rows.push((change.sequence, rostered(owner)));
            }
            Ok(())
        });
        let prepared = PreparedHistory::following(row, &mut self.walk, &mut self.payloads).unwrap();
        self.held.admit(&prepared).unwrap();
        let by_plan =
            self.planned
                .observe_prepared(&prepared, &self.held, self.sure, |owner, change| {
                    if is_rebuild(change.event) {
                        planned.push((change.sequence, rostered(owner)));
                    }
                    Ok(())
                });
        let results =
            [expected, by_rows, by_plan].map(|result| result.map_err(|error| error.to_string()));
        if results[2].is_err() {
            self.sure = false;
        }
        assert_eq!(results[1], results[0], "row path at {}", row.read_hour);
        assert_eq!(results[2], results[0], "plan at {}", row.read_hour);
        assert_eq!(visits[1], visits[0]);
        assert_eq!(visits[2], visits[0]);
        let state = serde_json::to_value(&self.every).unwrap();
        assert_eq!(serde_json::to_value(&self.rows).unwrap(), state);
        assert_eq!(
            serde_json::to_value(&self.planned).unwrap(),
            state,
            "plan state at {}",
            row.read_hour
        );
        if results[0].is_ok() {
            assert_eq!(touched(&self.rows), touched(&self.every));
            assert_eq!(touched(&self.planned), touched(&self.every));
        }
        results[0].as_ref().err().map(|_| {
            let stopped = self.every.sequence;
            row.events
                .iter()
                .position(|event| Some(event.sequence) == stopped)
                .unwrap_or(0)
        })
    }

    fn skip(&mut self, row: &EventFeedHistoryRow) {
        let prepared = PreparedHistory::following(row, &mut self.walk, &mut self.payloads).unwrap();
        self.held.admit(&prepared).unwrap();
        self.sure = false;
    }
}

#[test]
fn a_read_leaves_out_candidates_its_clear_cancels_unless_it_stops_first() {
    let owner = country(5, 5, 1);
    let refill = |stop: bool| {
        let mut ops = vec![
            candidate(owner, 0, 0, 0, 30),
            candidate(owner, 0, 0, 1, 30),
            candidate(owner, 2, 0, 2, 30),
            candidate(owner, 1, REMOVE_REQUESTED, 3, 31),
            Op::decision(owner, DecisionChange::new(Operation::Rebuild, 3)),
        ];
        if stop {
            ops.push(Op::decision(
                owner,
                DecisionChange::new(Operation::Ready, 0),
            ));
        }
        ops.push(Op::decision(
            owner,
            DecisionChange {
                flags: ALL | CLEAR_ALL_CANDIDATES,
                ..DecisionChange::new(Operation::Candidate, 0)
            },
        ));
        ops
    };
    let planned = read(1, 10, 1, refill(false));
    let plan = Plan::of(&planned, None);
    let flags: Vec<_> = plan.steps.iter().map(|step| step.flags).collect();
    assert_eq!(
        flags,
        [
            TARGETED | CANCELLED,
            TARGETED | REWRITE | CANCELLED | QUIET,
            TARGETED | REWRITE | CANCELLED | QUIET,
            TARGETED | CANCELLED | QUIET,
            0,
            QUIET,
        ]
    );
    assert!(plan.steps[..4].iter().all(|step| step.cancelled == 5));
    let mut reductions = Reductions::new();
    assert_eq!(reductions.reduce(&planned), None);
    assert_eq!(reductions.reduce(&read(1, 11, 10, refill(true))), Some(5));
    let owner = reductions.planned.decisions.values().next().unwrap();
    assert_eq!(owner.candidates.len(), 3);
}

#[test]
fn a_walk_holds_each_payload_once() {
    let owner = country(5, 5, 1);
    let ops = || vec![candidate(owner, 0, 0, 0, 30), candidate(owner, 1, 0, 1, 31)];
    let (mut walk, mut payloads) = (Written::default(), Payloads::default());
    let first =
        PreparedHistory::following(&read(1, 10, 1, ops()), &mut walk, &mut payloads).unwrap();
    let again = read(1, 11, 3, ops());
    let repeated = PreparedHistory::following(&again, &mut walk, &mut payloads).unwrap();
    assert!(first.added > 0 && first.introduced.len() == 2);
    assert!(repeated.added == 0 && repeated.introduced.is_empty());
    assert_eq!((repeated.first, repeated.steps[1].2), (2, first.steps[1].2));
    assert_eq!(repeated.steps[0].1, again.events[0].sequence);
    let mut reductions = Reductions::new();
    assert_eq!(reductions.reduce(&read(1, 10, 1, ops())), None);
    assert_eq!(reductions.reduce(&again), None);
}

#[test]
fn a_long_token_is_held_once_and_reduces_as_its_row() {
    let owner = country(5, 5, 1);
    let long = |hour: i32, first: u64| {
        let mut row = read(1, hour, first, vec![candidate(owner, 0, 0, 0, 30)]);
        let token = b"a_decision_token_longer_than_any_payload_holds_in_place_of_its_own";
        row.script_tokens = token.to_vec();
        let script = row.events[0].script.as_mut().unwrap();
        (script.token_start, script.token_len) = (0, token.len() as u32);
        row
    };
    let (mut walk, mut payloads) = (Written::default(), Payloads::default());
    let first = PreparedHistory::following(&long(10, 1), &mut walk, &mut payloads).unwrap();
    let again = PreparedHistory::following(&long(11, 2), &mut walk, &mut payloads).unwrap();
    assert!(matches!(first.introduced[0].token, Token::Long(_)));
    assert!(first.added >= size_of::<Payload>() + 66);
    assert!(again.added == 0 && again.introduced.is_empty());
    assert_eq!(first.steps[0].2, again.steps[0].2);
    let mut reductions = Reductions::new();
    assert_eq!(reductions.reduce(&long(10, 1)), None);
    assert_eq!(reductions.reduce(&long(11, 2)), None);
    let owner = reductions.planned.decisions.values().next().unwrap();
    assert_eq!(owner.candidates.iter().next().unwrap().1.len(), 66);
}

#[test]
fn a_reduction_holds_its_walk_payloads_in_order() {
    let owner = country(5, 5, 1);
    let ops = |target| vec![candidate(owner, 0, 0, 0, target)];
    let (mut walk, mut payloads) = (Written::default(), Payloads::default());
    let first =
        PreparedHistory::following(&read(1, 10, 1, ops(30)), &mut walk, &mut payloads).unwrap();
    let second =
        PreparedHistory::following(&read(1, 11, 2, ops(31)), &mut walk, &mut payloads).unwrap();
    let (mut held, mut state) = (HeldPayloads::default(), ScriptEventState::default());
    assert!(held.admit(&second).is_err());
    held.admit(&first).unwrap();
    assert!(
        state
            .observe_prepared(&second, &held, true, |_, _| Ok(()))
            .is_err()
    );
    held.admit(&second).unwrap();
    state
        .observe_prepared(&second, &held, true, |_, _| Ok(()))
        .unwrap();
    let (mut walk, mut payloads) = (Written::default(), Payloads::default());
    let other =
        PreparedHistory::following(&read(1, 12, 3, ops(32)), &mut walk, &mut payloads).unwrap();
    held.admit(&other).unwrap();
    assert_eq!(held.0.len(), 1);
}

#[test]
fn targets_a_walk_wrote_hold_only_while_it_reduced_every_read() {
    let owner = country(5, 5, 1);
    let first = read(1, 10, 1, vec![candidate(owner, 0, 0, 0, 30)]);
    let failed = read(
        1,
        11,
        2,
        vec![
            Op::decision(owner, DecisionChange::new(Operation::Ready, 0)),
            candidate(owner, 0, 0, 0, 31),
        ],
    );
    let again = read(
        1,
        12,
        4,
        vec![candidate(owner, 0, 0, 0, 30), candidate(owner, 0, 0, 0, 31)],
    );
    let mut reductions = Reductions::new();
    assert_eq!(reductions.reduce(&first), None);
    assert!(reductions.sure);
    let (mut assumed, mut payloads) = (Written::default(), Payloads::default());
    let mut held = HeldPayloads::default();
    for read in [&first, &failed] {
        let prepared = PreparedHistory::following(read, &mut assumed, &mut payloads).unwrap();
        held.admit(&prepared).unwrap();
    }
    let blind = PreparedHistory::following(&again, &mut assumed, &mut payloads).unwrap();
    held.admit(&blind).unwrap();
    assert_eq!(
        blind
            .steps
            .iter()
            .map(|(step, _, _)| step.flags & WRITTEN)
            .collect::<Vec<_>>(),
        [WRITTEN, WRITTEN]
    );
    assert_eq!(reductions.reduce(&failed), Some(0));
    assert!(!reductions.sure);
    let mut trusting = ScriptEventState::default();
    trusting.observe_decisions(&first, |_, _| Ok(())).unwrap();
    let _ = trusting.observe_decisions(&failed, |_, _| Ok(()));
    trusting
        .observe_prepared(&blind, &held, true, |_, _| Ok(()))
        .unwrap();
    assert_eq!(
        trusting
            .decisions
            .values()
            .next()
            .unwrap()
            .target_countries
            .len(),
        1
    );
    assert_eq!(reductions.reduce(&again), None);
    assert_eq!(
        reductions
            .planned
            .decisions
            .values()
            .next()
            .unwrap()
            .target_countries
            .len(),
        2
    );
}

#[test]
fn plans_reduce_as_every_change_does() {
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut draw = move |bound: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % u64::from(bound)) as u32
    };
    for restarts in [1, 5, 40, 100_000] {
        let mut reductions = Reductions::new();
        let (mut sequence, mut hour, mut generation) = (1_u64, 100, 3_u64);
        let (mut cancelled, mut written, mut restored) = (0, 0, 0);
        for index in 0..900 {
            if index % restarts == 0 {
                reductions.restart();
            }
            hour += if draw(30) == 0 { 2 } else { 1 };
            if draw(150) == 0 {
                generation += 1;
            }
            let mut ops = Vec::new();
            for _ in 0..draw(4) {
                let id = 5 + draw(2);
                let actor = country(id, id, 1 + u32::from(draw(6) == 0));
                match draw(9) {
                    0..=3 => {
                        for _ in 0..draw(9) {
                            let kind = draw(4) as u8;
                            let flags = if draw(6) == 0 { REMOVE_REQUESTED } else { 0 };
                            let token = draw(4) as usize;
                            let target = 30 + draw(3);
                            let mut op = candidate(actor, kind, flags, token, target);
                            op.unrecorded = draw(25) == 0;
                            if draw(8) == 0 {
                                op.target.id += 100;
                            }
                            if draw(10) == 0 {
                                op.actor.instance += 1;
                            }
                            ops.push(op);
                        }
                        if draw(2) == 0 {
                            ops.push(Op::decision(
                                actor,
                                DecisionChange::new(Operation::Rebuild, 3),
                            ));
                        }
                        if draw(5) == 0 {
                            ops.push(Op::decision(
                                actor,
                                DecisionChange::new(Operation::Ready, 0),
                            ));
                        }
                        let clear = match draw(4) {
                            0 => Some(DecisionChange {
                                flags: ALL | CLEAR_ALL_CANDIDATES,
                                ..DecisionChange::new(Operation::Candidate, 0)
                            }),
                            1 => Some(DecisionChange {
                                flags: ALL,
                                ..DecisionChange::new(Operation::Candidate, draw(4) as u8)
                            }),
                            2 => Some(DecisionChange::new(Operation::Baseline, 0)),
                            _ => None,
                        };
                        if let Some(clear) = clear {
                            let mut op = Op::decision(actor, clear);
                            op.unrecorded = draw(10) == 0;
                            ops.push(op);
                        }
                    }
                    4 => {
                        ops.push(Op::decision(
                            actor,
                            DecisionChange::new(Operation::Baseline, 0),
                        ));
                        for _ in 0..draw(4) {
                            ops.push(Op {
                                token: Some(draw(4) as usize),
                                instance: Some(1 + draw(6)),
                                days: Some(draw(20) as i32),
                                lifecycle: Some(draw(3) as i32),
                                ..Op::decision(actor, DecisionChange::new(Operation::Store, TIMED))
                            });
                        }
                        ops.push(Op::decision(
                            actor,
                            DecisionChange::new(Operation::Ready, 0),
                        ));
                    }
                    5 => {
                        let begin = DecisionChange {
                            ordinal: draw(4),
                            ..DecisionChange::new(Operation::PassBegin, TIMED)
                        };
                        ops.push(Op::decision(actor, begin));
                        ops.push(Op::decision(
                            actor,
                            DecisionChange::new(Operation::PassEnd, TIMED),
                        ));
                    }
                    6 => {
                        let operation = if draw(2) == 0 {
                            Operation::RandomIncrement
                        } else {
                            Operation::Ignore
                        };
                        let flags = if draw(2) == 0 { IGNORED } else { 0 };
                        let target = 30 + draw(3);
                        ops.push(Op {
                            token: Some(draw(4) as usize),
                            target: country(target, 0, 0),
                            ..Op::decision(
                                actor,
                                DecisionChange {
                                    flags,
                                    ..DecisionChange::new(operation, 0)
                                },
                            )
                        });
                    }
                    7 => {
                        let timers = country(40 + draw(2), 40, 0);
                        let mut op =
                            Op::decision(actor, DecisionChange::new(Operation::Rebuild, 0));
                        op.target = timers;
                        op.change = match draw(4) {
                            0 => EventFeedScriptChange::DelayedBaseline,
                            1 => {
                                op.token = Some(0);
                                op.instance = Some(1 + draw(5));
                                EventFeedScriptChange::DelayedStored {
                                    remaining: draw(9) as i32,
                                }
                            }
                            2 => EventFeedScriptChange::DelayedReady,
                            _ => EventFeedScriptChange::DelayedAdvance {
                                count: draw(3),
                                finished: true,
                            },
                        };
                        ops.push(op);
                    }
                    _ => {
                        let mut op =
                            Op::decision(actor, DecisionChange::new(Operation::Update, TIMED));
                        op.instance = Some(1 + draw(6));
                        op.lifecycle = Some(draw(3) as i32);
                        op.days = Some(draw(9) as i32);
                        ops.push(op);
                    }
                }
            }
            let count = ops.len() as u64;
            let mut row = read(generation, hour, sequence, ops);
            sequence += count;
            match draw(40) {
                0 => row.coverage = EventFeedCoverage::Partial,
                1 => {
                    row.coverage = EventFeedCoverage::Unavailable;
                    row.failure = Some(b"synthetic loss".to_vec());
                }
                _ => {}
            }
            if draw(80) == 0 {
                reductions.skip(&row);
                continue;
            }
            let plan = Plan::of(&row, None);
            cancelled += plan
                .steps
                .iter()
                .filter(|step| step.flags & (QUIET | CANCELLED) == QUIET | CANCELLED)
                .count();
            let walked = reductions.sure;
            let mut probe = reductions.walk.clone();
            let known = Plan::of(&row, Some(&mut probe));
            if walked {
                written += known
                    .steps
                    .iter()
                    .filter(|step| step.flags & (WRITTEN | REWRITE) == WRITTEN)
                    .count();
            }
            if let Some(at) = reductions.reduce(&row) {
                restored += usize::from(plan.steps.iter().any(|step| {
                    step.flags & CANCELLED != 0
                        && (step.event as usize) < at
                        && step.cancelled as usize > at
                }));
            }
        }
        assert!(cancelled > 200, "{cancelled} candidate changes left out");
        if (2..=1_000).contains(&restarts) {
            assert!(written > 30, "{written} target writes the walk had made");
        }
        assert!(
            restored > 3,
            "{restored} failed reads restoring cancelled candidates"
        );
    }
}
