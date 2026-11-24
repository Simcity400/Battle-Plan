use super::*;
use replay_frame::frame::{FrameEncoder, FrameInput, Metrics};

fn variables_frame(hour: i32, input: TableInput) -> FrameInput {
    let mut frame = FrameInput {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| unavailable("fixture table").into()),
        metrics: Metrics::default(),
    };
    frame.tables[TableId::ScriptVariables.index()] = input;
    frame
}

fn shared_baseline(
    encoder: &FrameEncoder,
    hour: i32,
    force: bool,
) -> Option<(BaselineToken, &[rows::ScriptVariablesRow])> {
    encoder
        .slice_baseline(TableId::ScriptVariables, 1, hour, force)
        .and_then(|(token, rows)| Some((token, rows.rows::<tables::ScriptVariables>()?)))
}

fn complete(capture: &Captured) -> TableInput {
    capture.variables_table(capture.variables.outcome).into()
}

#[test]
fn variable_scope_updates_keep_frame_bytes() {
    const SCOPES: usize = 8;
    for force in [false, true] {
        let mut capacity = Capacities::default();
        capacity.raise_for_script_variables(VariableDemand {
            scopes: SCOPES,
            entries: 2 * SCOPES,
            bytes: 8,
        });
        let mut capture = Captured::with_capacities(&capacity);
        capture.variables.outcome = Outcome::Rows { len: SCOPES };
        for index in 0..SCOPES {
            capture.variables.rows[index] = cvariables::ScopeRow {
                scope: cvariables::Scope::Country(index as u32 + 1),
                entries_start: index * 2,
                entries_len: 2,
                seed: Some(index as u32),
            };
            capture.variable_entries.first[index * 2] = cvariables::Row {
                name_start: 0,
                name_len: 4,
                value_raw: index as i64,
            };
            capture.variable_entries.first[index * 2 + 1] = cvariables::Row {
                name_start: 4,
                name_len: 2,
                value_raw: -(index as i64),
            };
        }
        capture.variable_entries.len = 2 * SCOPES;
        capture.variable_entries.required = 2 * SCOPES;
        capture.variable_names.first[..6].copy_from_slice(b"zetaA\0");
        capture.variable_names.len = 6;
        capture.variable_names.required = 6;
        let mut optimized = FrameEncoder::new();
        let mut reference = FrameEncoder::new();
        let mut scratch = ScriptVariableComparison::default();
        for hour in 0..=10 {
            match hour {
                2 => capture.variables.rows[..SCOPES].reverse(),
                3 => capture.variable_entries.first[0].value_raw = i64::MIN,
                4 => capture.variables.rows[0].seed = None,
                5 => capture.variable_names.first[0] = b'Z',
                6 => capture.variable_entries.first[1] = capture.variable_entries.first[0],
                7 => {
                    capture.variables.outcome = Outcome::Unavailable {
                        reason: "fixture refusal",
                    }
                }
                8 => {
                    capture.variables.outcome = Outcome::Truncated {
                        len: 3,
                        unread_rows: SCOPES as u32 - 3,
                    }
                }
                9 => {
                    capture.variables.outcome = Outcome::Rows { len: SCOPES };
                    capture.variable_names.len = usize::MAX;
                }
                10 => {
                    capture.variables.outcome = Outcome::Rows {
                        len: capture.variables.rows.len() + 1,
                    }
                }
                _ => {}
            }
            let baseline = shared_baseline(&optimized, hour, force);
            let actual = if !force && hour == 2 {
                collector_test_support::with_allocations_denied(|| {
                    capture.variables_input(capture.variables.outcome, baseline, None, &mut scratch)
                })
            } else {
                capture.variables_input(capture.variables.outcome, baseline, None, &mut scratch)
            };
            if !force && matches!(hour, 1 | 2) {
                assert!(matches!(actual, TableInput::ComparedEqual(_)));
            }
            if !force && matches!(hour, 3 | 4) {
                let TableInput::SliceUpdate { update, .. } = &actual else {
                    panic!("one changed scope must not convert all scopes");
                };
                assert_eq!(update.keys.len(), 1);
                assert_eq!(update.rows().len(), 1);
            }
            assert_eq!(
                optimized
                    .encode_bytes(variables_frame(hour, actual), force)
                    .and_then(|frame| frame.into_spool_record())
                    .unwrap(),
                reference
                    .encode_bytes(variables_frame(hour, complete(&capture)), force)
                    .and_then(|frame| frame.into_spool_record())
                    .unwrap(),
                "hour {hour}, force {force}"
            );
        }
    }
}

type Scope = (cvariables::Scope, Option<u32>, Vec<(&'static str, i64)>);

fn country(id: u32, seed: u32, entries: &[(&'static str, i64)]) -> Scope {
    (cvariables::Scope::Country(id), Some(seed), entries.to_vec())
}

fn countries() -> Vec<Scope> {
    (1..=6)
        .map(|id| {
            country(
                id,
                id * 10,
                &[
                    ("alpha", i64::from(id)),
                    ("beta", -i64::from(id)),
                    ("gamma", 7),
                ],
            )
        })
        .collect()
}

fn captured(scopes: &[Scope]) -> Captured {
    let entries = scopes.iter().map(|scope| scope.2.len()).sum();
    let bytes = scopes
        .iter()
        .flat_map(|scope| &scope.2)
        .map(|(name, _)| name.len())
        .sum();
    let mut capacity = Capacities::default();
    capacity.raise_for_script_variables(VariableDemand {
        scopes: scopes.len(),
        entries,
        bytes,
    });
    let mut capture = Captured::with_capacities(&capacity);
    let (mut entry, mut byte) = (0, 0);
    for (index, (scope, seed, values)) in scopes.iter().enumerate() {
        capture.variables.rows[index] = cvariables::ScopeRow {
            scope: *scope,
            entries_start: entry,
            entries_len: values.len(),
            seed: *seed,
        };
        for (name, value) in values {
            capture.variable_entries.first[entry] = cvariables::Row {
                name_start: byte,
                name_len: name.len(),
                value_raw: *value,
            };
            capture.variable_names.first[byte..byte + name.len()].copy_from_slice(name.as_bytes());
            entry += 1;
            byte += name.len();
        }
    }
    capture.variables.outcome = Outcome::Rows { len: scopes.len() };
    capture.variable_entries.len = entry;
    capture.variable_entries.required = entry;
    capture.variable_names.len = byte;
    capture.variable_names.required = byte;
    capture
}

fn listed(held: &[Scope], current: &[Scope]) -> (bool, Vec<u32>) {
    let same_scopes = held.len() == current.len()
        && held
            .iter()
            .zip(current)
            .all(|(held, current)| held.0 == current.0);
    let changed = current
        .iter()
        .enumerate()
        .filter(|(_, scope)| !held.contains(scope))
        .map(|(ordinal, _)| ordinal as u32)
        .collect();
    (same_scopes, changed)
}

struct Run {
    encoder: FrameEncoder,
    reference: FrameEncoder,
    scratch: ScriptVariableComparison,
    check: conversion_check::Check,
    hour: i32,
    held: Option<(u64, Vec<Scope>)>,
}

struct Converted {
    input: TableInput,
    work: VariableWork,
}

impl Run {
    fn new() -> Self {
        Self {
            encoder: FrameEncoder::new(),
            reference: FrameEncoder::new(),
            scratch: ScriptVariableComparison::default(),
            check: conversion_check::Check::new(),
            hour: 0,
            held: None,
        }
    }

    fn hour(&mut self, scopes: &[Scope]) -> Converted {
        self.convert(scopes, true)
    }

    fn uncontinued(&mut self, scopes: &[Scope]) -> Converted {
        self.convert(scopes, false)
    }

    fn convert(&mut self, scopes: &[Scope], continued: bool) -> Converted {
        let mut capture = captured(scopes);
        let generation = self.hour as u64 + 1;
        capture.variable_delta.generation = Some(generation);
        if continued && let Some((since, held)) = &self.held {
            let (same_scopes, changed) = listed(held, scopes);
            capture.variable_delta.since = Some(*since);
            capture.variable_delta.same_scopes = same_scopes;
            capture.variable_delta.changed = changed;
        }
        self.held = Some((generation, scopes.to_vec()));
        let outcome = capture.variables.outcome;
        let baseline = shared_baseline(&self.encoder, self.hour, false);
        let input = capture.variables_input(outcome, baseline, None, &mut self.scratch);
        let work = self
            .scratch
            .take_work()
            .expect("every conversion records its work");
        let today = complete(&capture);
        self.check.record(baseline, &today, &input);
        let actual = self
            .encoder
            .encode_bytes(variables_frame(self.hour, input.clone()), false)
            .and_then(|frame| frame.into_spool_record())
            .unwrap();
        let expected = self
            .reference
            .encode_bytes(variables_frame(self.hour, today), false)
            .and_then(|frame| frame.into_spool_record())
            .unwrap();
        assert_eq!(actual, expected, "hour {}", self.hour);
        self.hour += 1;
        Converted { input, work }
    }
}

fn update(input: &TableInput) -> (Vec<(u8, u32)>, &[rows::ScriptVariablesRow]) {
    let TableInput::SliceUpdate { update, .. } = input else {
        panic!("a keyed update: {input:?}");
    };
    let keys = update
        .keys
        .iter()
        .map(|&key| ((key >> 32) as u8, key as u32))
        .collect();
    (
        keys,
        update.rows().rows::<tables::ScriptVariables>().unwrap(),
    )
}

#[test]
fn membership_changes_convert_their_scope_alone() {
    let mut run = Run::new();
    let base = countries();
    let first = run.hour(&base);
    assert!(matches!(first.input, TableInput::SharedComplete(_)));
    assert_eq!(first.work.path, Path::Complete(Complete::NoBaseline));
    let settling = run.hour(&base);
    assert!(matches!(settling.input, TableInput::ComparedEqual(_)));
    assert_eq!(settling.work.path, Path::AllScopes(AllScopes::NoKeptHour));
    assert_eq!(settling.work.scopes_compared, 6);

    let equal = run.hour(&base);
    assert!(matches!(equal.input, TableInput::ComparedEqual(_)));
    assert_eq!(equal.work.path, Path::Changes);
    assert_eq!(
        (
            equal.work.scopes_compared,
            equal.work.entries_compared,
            equal.work.rows_built
        ),
        (0, 0, 0)
    );

    let mut grown = base.clone();
    grown[2].2.push(("delta", 4));
    let entry = run.hour(&grown);
    let (keys, rows) = update(&entry.input);
    assert_eq!(keys, [(COUNTRY_SCOPE, 3)]);
    assert_eq!(rows[0].entries.len(), 4);
    assert_eq!(entry.work.path, Path::Changes);
    assert_eq!(
        (
            entry.work.scopes_compared,
            entry.work.rows_built,
            entry.work.entry_sets_changed
        ),
        (1, 1, 1)
    );

    let mut added = grown.clone();
    added.push(country(7, 70, &[("alpha", 1)]));
    let scope = run.hour(&added);
    let (keys, rows) = update(&scope.input);
    assert_eq!(keys, [(COUNTRY_SCOPE, 7)]);
    assert_eq!((rows[0].scope_kind, rows[0].scope_id), (COUNTRY_SCOPE, 7));
    assert!(scope.work.reordered);
    assert_eq!(
        (scope.work.scopes_compared, scope.work.scopes_added),
        (1, 1)
    );

    let mut removed = added.clone();
    removed.remove(1);
    let gone = run.hour(&removed);
    let (keys, rows) = update(&gone.input);
    assert_eq!(keys, [(COUNTRY_SCOPE, 2)]);
    assert!(rows.is_empty());
    assert_eq!(
        (gone.work.scopes_compared, gone.work.scopes_removed),
        (0, 1)
    );

    let settled = run.hour(&removed);
    assert!(matches!(settled.input, TableInput::ComparedEqual(_)));
    assert_eq!(
        (settled.work.scopes_compared, settled.work.rows_built),
        (0, 0)
    );
    assert_eq!(run.check.differences, 0);
}

#[test]
fn every_hour_holds_todays_rows_and_writes_todays_bytes() {
    let mut run = Run::new();
    let base = countries();
    let mut added = base.clone();
    added.push(country(7, 70, &[("alpha", 1)]));
    let mut removed = base.clone();
    removed.remove(1);
    let mut grown = base.clone();
    grown[2].2.push(("delta", 4));
    let mut reseeded = base.clone();
    reseeded[4].1 = Some(12_345);
    let mut unseeded = base.clone();
    unseeded[4].1 = None;
    let mut renamed = base.clone();
    renamed[0].2[1].0 = "bets";
    let mut reordered = base.clone();
    reordered.reverse();
    let mut moved = base.clone();
    moved[0].2.swap(0, 2);
    let mut swapped = base.clone();
    swapped.swap(1, 4);
    swapped[1].2[0].1 = 41;
    let mut duplicate = base.clone();
    duplicate[3].2[1].0 = "alpha";
    let mut several = grown.clone();
    several[0].2.clear();
    several[5].2[2].1 = 70;
    several.push(country(9, 90, &[]));
    fn whole(input: &TableInput) -> bool {
        matches!(input, TableInput::SharedComplete(_))
    }
    fn keys(count: usize) -> impl Fn(&TableInput) -> bool {
        move |input| matches!(input, TableInput::SliceUpdate { update, .. } if update.keys.len() == count)
    }
    fn equal(input: &TableInput) -> bool {
        matches!(input, TableInput::ComparedEqual(_))
    }
    fn refused(input: &TableInput) -> bool {
        matches!(input, TableInput::Sampled(TableView::Unavailable { .. }))
    }
    type Form = Box<dyn Fn(&TableInput) -> bool>;
    let hours: &[(&[Scope], bool, Form)] = &[
        (&base, true, Box::new(whole)),
        (&base, true, Box::new(equal)),
        (&added, true, Box::new(keys(1))),
        (&added, true, Box::new(equal)),
        (&base, true, Box::new(keys(1))),
        (&removed, true, Box::new(keys(1))),
        (&removed, true, Box::new(equal)),
        (&base, true, Box::new(keys(1))),
        (&grown, true, Box::new(keys(1))),
        (&base, true, Box::new(keys(1))),
        (&reseeded, true, Box::new(keys(1))),
        (&unseeded, true, Box::new(keys(1))),
        (&base, true, Box::new(keys(1))),
        (&renamed, true, Box::new(keys(1))),
        (&base, true, Box::new(keys(1))),
        (&reordered, true, Box::new(equal)),
        (&reordered, true, Box::new(equal)),
        (&base, true, Box::new(equal)),
        (&moved, true, Box::new(equal)),
        (&base, true, Box::new(equal)),
        (&swapped, true, Box::new(keys(1))),
        (&base, false, Box::new(keys(1))),
        (&grown, false, Box::new(keys(1))),
        (&several, true, Box::new(keys(3))),
        (&base, true, Box::new(keys(4))),
        (&duplicate, true, Box::new(refused)),
        (&base, true, Box::new(whole)),
        (&base, true, Box::new(equal)),
    ];
    for (hour, (scopes, continued, form)) in hours.iter().enumerate() {
        let converted = if *continued {
            run.hour(scopes)
        } else {
            run.uncontinued(scopes)
        };
        assert!(form(&converted.input), "hour {hour}: {:?}", converted.input);
    }
    assert_eq!(run.check.hours, hours.len() as u64);
    assert_eq!(run.check.differences, 0);
}

#[test]
fn conversion_check_counts_and_names_an_injected_difference() {
    let mut run = Run::new();
    let scopes = countries();
    for _ in 0..3 {
        run.hour(&scopes);
    }
    let mut one = scopes.clone();
    one[3].2[0].1 = -5;
    let mut capture = captured(&one);
    capture.variable_delta = VariableDelta {
        generation: Some(4),
        since: Some(3),
        same_scopes: true,
        changed: vec![3],
    };
    let outcome = capture.variables.outcome;
    let baseline = shared_baseline(&run.encoder, run.hour, false);
    let (token, _) = baseline.unwrap();
    let input = capture.variables_input(outcome, baseline, None, &mut run.scratch);
    let today = complete(&capture);
    let TableInput::SliceUpdate { update, .. } = &input else {
        panic!("one changed scope: {input:?}");
    };
    let mut injected = (**update).clone();
    let TableRows::ScriptVariables(rows) = &mut injected.rows else {
        panic!("ScriptVariables rows");
    };
    rows[0].entries[0].value_raw += 1;
    let injected = TableInput::SliceUpdate {
        token,
        update: Arc::new(injected),
    };
    let mut check = conversion_check::Check::new();
    check.record(baseline, &today, &input);
    check.record(baseline, &today, &injected);
    check.record(baseline, &today, &TableInput::ComparedEqual(token));
    check.record(baseline, &today, &unavailable("fixture refusal").into());
    assert_eq!(
        (check.hours, check.hours_differing, check.differences),
        (4, 3, 3)
    );
    let named = |scope, column| {
        Some(conversion_check::Difference {
            hour: Some(3),
            scope,
            column,
        })
    };
    assert_eq!(
        check.first[..4],
        [
            named(Some((COUNTRY_SCOPE, 4)), "entries"),
            named(Some((COUNTRY_SCOPE, 4)), "entries"),
            named(None, "outcome"),
            None,
        ]
    );
}
