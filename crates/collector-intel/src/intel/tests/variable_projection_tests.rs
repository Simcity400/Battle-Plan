use super::*;
use replay_frame::frame::{FrameDecoder, FrameEncoder, FrameInput, Metrics};
use std::collections::BTreeMap;

fn variables_frame(hour: i32, inputs: [TableInput; 3]) -> FrameInput {
    let [scopes, names, values] = inputs;
    let mut frame = FrameInput {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| unavailable("fixture table").into()),
        metrics: Metrics::default(),
    };
    frame.tables[TableId::ScriptVariables.index()] = scopes;
    frame.tables[TableId::ScriptVariableNames.index()] = names;
    frame.tables[TableId::ScriptVariableValues.index()] = values;
    frame
}

fn shared_baselines(encoder: &FrameEncoder, hour: i32, force: bool) -> VariableBaselines<'_> {
    VariableBaselines {
        scopes: encoder
            .slice_baseline(TableId::ScriptVariables, 1, hour, force)
            .and_then(|(token, rows)| Some((token, rows.rows::<tables::ScriptVariables>()?))),
        names: encoder
            .slice_baseline(TableId::ScriptVariableNames, 1, hour, force)
            .and_then(|(token, rows)| Some((token, rows.rows::<tables::ScriptVariableNames>()?))),
        values: encoder
            .slice_baseline(TableId::ScriptVariableValues, 1, hour, force)
            .and_then(|(token, rows)| Some((token, rows.rows::<tables::ScriptVariableValues>()?))),
    }
}

fn complete(capture: &Captured) -> [TableInput; 3] {
    capture
        .complete_variable_views(capture.variables.outcome)
        .map(TableInput::Sampled)
}

type DecodedVariables = Result<
    BTreeMap<(u8, u32), (rows::Recorded<Option<u32>>, BTreeMap<Vec<u8>, i64>)>,
    String,
>;

fn decoded_logical(frame: &replay_frame::frame::DecodedFrame) -> DecodedVariables {
    match (
        &*frame.tables[TableId::ScriptVariables.index()],
        &*frame.tables[TableId::ScriptVariableNames.index()],
        &*frame.tables[TableId::ScriptVariableValues.index()],
    ) {
        (
            TableView::Complete(TableRows::ScriptVariables(scopes)),
            TableView::Complete(TableRows::ScriptVariableNames(names)),
            TableView::Complete(TableRows::ScriptVariableValues(values)),
        ) => {
            let dict: BTreeMap<u32, &[u8]> =
                names.iter().map(|r| (r.id, r.name.as_slice())).collect();
            let mut out: BTreeMap<(u8, u32), (rows::Recorded<Option<u32>>, BTreeMap<Vec<u8>, i64>)> =
                BTreeMap::new();
            for s in scopes {
                assert!(s.entries.is_empty());
                out.insert((s.scope_kind, s.scope_id), (s.seed, BTreeMap::new()));
            }
            for v in values {
                let name = dict.get(&v.var_id).expect("known var_id").to_vec();
                out.get_mut(&(v.scope_kind, v.scope_id))
                    .expect("known scope")
                    .1
                    .insert(name, v.value_raw);
            }
            Ok(out)
        }
        (a, b, c) => Err(format!("{a:?}|{b:?}|{c:?}")),
    }
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
        let mut opt_dec = FrameDecoder::new();
        let mut ref_dec = FrameDecoder::new();
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
            let baseline = shared_baselines(&optimized, hour, force);
            let actual = if !force && hour == 2 {
                collector_test_support::with_allocations_denied(|| {
                    capture.variables_inputs(capture.variables.outcome, baseline, None, &mut scratch)
                })
            } else {
                capture.variables_inputs(capture.variables.outcome, baseline, None, &mut scratch)
            };
            if !force && matches!(hour, 1 | 2) {
                assert!(actual.iter().all(|i| matches!(i, TableInput::ComparedEqual(_))));
            }
            if !force && hour == 3 {
                assert!(matches!(actual[0], TableInput::ComparedEqual(_)));
                assert!(matches!(actual[1], TableInput::ComparedEqual(_)));
                let TableInput::SliceUpdate { update, .. } = &actual[2] else {
                    panic!("one changed value must emit one value update");
                };
                assert_eq!(update.keys.len(), 1);
                assert_eq!(update.rows().len(), 1);
            }
            if !force && hour == 4 {
                let TableInput::SliceUpdate { update, .. } = &actual[0] else {
                    panic!("one changed seed must emit one scope update");
                };
                assert_eq!(update.keys.len(), 1);
                assert_eq!(update.rows().len(), 1);
                assert!(matches!(actual[1], TableInput::ComparedEqual(_)));
                assert!(matches!(actual[2], TableInput::ComparedEqual(_)));
            }
            let opt_wire = optimized
                .encode_bytes(variables_frame(hour, actual), force)
                .unwrap()
                .decode()
                .unwrap();
            let ref_wire = reference
                .encode_bytes(variables_frame(hour, complete(&capture)), force)
                .unwrap()
                .decode()
                .unwrap();
            let opt_frame = opt_dec.apply(&opt_wire).unwrap();
            let ref_frame = ref_dec.apply(&ref_wire).unwrap();
            assert_eq!(
                decoded_logical(&opt_frame),
                decoded_logical(&ref_frame),
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
    opt_dec: FrameDecoder,
    ref_dec: FrameDecoder,
    scratch: ScriptVariableComparison,
    check: conversion_check::Check,
    hour: i32,
    held: Option<(u64, Vec<Scope>)>,
}

struct Converted {
    inputs: [TableInput; 3],
    work: VariableWork,
}

impl Run {
    fn new() -> Self {
        Self {
            encoder: FrameEncoder::new(),
            reference: FrameEncoder::new(),
            opt_dec: FrameDecoder::new(),
            ref_dec: FrameDecoder::new(),
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
        let baselines = shared_baselines(&self.encoder, self.hour, false);
        let inputs = capture.variables_inputs(outcome, baselines, None, &mut self.scratch);
        let work = self
            .scratch
            .take_work()
            .expect("every conversion records its work");
        let today = complete(&capture);
        self.check.record(baselines, &today, &inputs);
        let actual = self
            .encoder
            .encode_bytes(variables_frame(self.hour, inputs.clone()), false)
            .unwrap()
            .decode()
            .unwrap();
        let expected = self
            .reference
            .encode_bytes(variables_frame(self.hour, today), false)
            .unwrap()
            .decode()
            .unwrap();
        let actual_frame = self.opt_dec.apply(&actual).unwrap();
        let expected_frame = self.ref_dec.apply(&expected).unwrap();
        assert_eq!(
            decoded_logical(&actual_frame),
            decoded_logical(&expected_frame),
            "hour {}",
            self.hour
        );
        self.hour += 1;
        Converted { inputs, work }
    }
}

fn scope_update(input: &TableInput) -> (Vec<(u8, u32)>, &[rows::ScriptVariablesRow]) {
    let TableInput::SliceUpdate { update, .. } = input else {
        panic!("a keyed scope update: {input:?}");
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

fn value_update(input: &TableInput) -> &[rows::ScriptVariableValueRow] {
    let TableInput::SliceUpdate { update, .. } = input else {
        panic!("a keyed value update: {input:?}");
    };
    update
        .rows()
        .rows::<tables::ScriptVariableValues>()
        .unwrap()
}

#[test]
fn membership_changes_convert_their_scope_alone() {
    let mut run = Run::new();
    let base = countries();
    let first = run.hour(&base);
    assert!(
        first
            .inputs
            .iter()
            .all(|i| matches!(i, TableInput::SharedComplete(_)))
    );
    assert_eq!(first.work.path, Path::Complete(Complete::NoBaseline));
    let settling = run.hour(&base);
    assert!(
        settling
            .inputs
            .iter()
            .all(|i| matches!(i, TableInput::ComparedEqual(_)))
    );
    assert_eq!(settling.work.path, Path::AllScopes(AllScopes::NoKeptHour));
    assert_eq!(settling.work.scopes_compared, 6);

    let equal = run.hour(&base);
    assert!(
        equal
            .inputs
            .iter()
            .all(|i| matches!(i, TableInput::ComparedEqual(_)))
    );
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
    assert!(matches!(entry.inputs[0], TableInput::ComparedEqual(_)));
    let TableInput::SliceUpdate { update: name_up, .. } = &entry.inputs[1] else {
        panic!("new name delta should append one ScriptVariableNames row");
    };
    assert_eq!(name_up.keys.len(), 1);
    let val_rows = value_update(&entry.inputs[2]);
    assert_eq!(val_rows.len(), 1);
    assert_eq!((val_rows[0].scope_kind, val_rows[0].scope_id, val_rows[0].value_raw), (COUNTRY_SCOPE, 3, 4));
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
    let (keys, rows) = scope_update(&scope.inputs[0]);
    assert_eq!(keys, [(COUNTRY_SCOPE, 7)]);
    assert_eq!((rows[0].scope_kind, rows[0].scope_id), (COUNTRY_SCOPE, 7));
    assert!(matches!(scope.inputs[1], TableInput::ComparedEqual(_)));
    assert_eq!(value_update(&scope.inputs[2]).len(), 1);
    assert!(scope.work.reordered);
    assert_eq!(
        (scope.work.scopes_compared, scope.work.scopes_added),
        (1, 1)
    );

    let mut removed = added.clone();
    removed.remove(1);
    let gone = run.hour(&removed);
    let (keys, rows) = scope_update(&gone.inputs[0]);
    assert_eq!(keys, [(COUNTRY_SCOPE, 2)]);
    assert!(rows.is_empty());
    assert!(matches!(gone.inputs[1], TableInput::ComparedEqual(_)));
    let TableInput::SliceUpdate { update: del_vals, .. } = &gone.inputs[2] else {
        panic!("removed scope must delete its variable values");
    };
    assert_eq!(del_vals.keys.len(), 3);
    assert!(del_vals.rows().is_empty());
    assert_eq!(
        (gone.work.scopes_compared, gone.work.scopes_removed),
        (0, 1)
    );

    let settled = run.hour(&removed);
    assert!(
        settled
            .inputs
            .iter()
            .all(|i| matches!(i, TableInput::ComparedEqual(_)))
    );
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
    fn whole(inputs: &[TableInput; 3]) -> bool {
        inputs
            .iter()
            .all(|i| matches!(i, TableInput::SharedComplete(_)))
    }
    fn equal(inputs: &[TableInput; 3]) -> bool {
        inputs
            .iter()
            .all(|i| matches!(i, TableInput::ComparedEqual(_)))
    }
    fn refused(inputs: &[TableInput; 3]) -> bool {
        inputs
            .iter()
            .all(|i| matches!(i, TableInput::Sampled(TableView::Unavailable { .. })))
    }
    fn delta(inputs: &[TableInput; 3]) -> bool {
        inputs
            .iter()
            .any(|i| matches!(i, TableInput::SliceUpdate { .. }))
            && inputs.iter().all(|i| {
                matches!(
                    i,
                    TableInput::ComparedEqual(_) | TableInput::SliceUpdate { .. }
                )
            })
    }
    type Form = Box<dyn Fn(&[TableInput; 3]) -> bool>;
    let hours: &[(&[Scope], bool, Form)] = &[
        (&base, true, Box::new(whole)),
        (&base, true, Box::new(equal)),
        (&added, true, Box::new(delta)),
        (&added, true, Box::new(equal)),
        (&base, true, Box::new(delta)),
        (&removed, true, Box::new(delta)),
        (&removed, true, Box::new(equal)),
        (&base, true, Box::new(delta)),
        (&grown, true, Box::new(delta)),
        (&base, true, Box::new(delta)),
        (&reseeded, true, Box::new(delta)),
        (&unseeded, true, Box::new(delta)),
        (&base, true, Box::new(delta)),
        (&renamed, true, Box::new(delta)),
        (&base, true, Box::new(delta)),
        (&reordered, true, Box::new(equal)),
        (&reordered, true, Box::new(equal)),
        (&base, true, Box::new(equal)),
        (&moved, true, Box::new(equal)),
        (&base, true, Box::new(equal)),
        (&swapped, true, Box::new(delta)),
        (&base, false, Box::new(delta)),
        (&grown, false, Box::new(delta)),
        (&several, true, Box::new(delta)),
        (&base, true, Box::new(delta)),
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
        assert!(form(&converted.inputs), "hour {hour}: {:?}", converted.inputs);
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
    let baselines = shared_baselines(&run.encoder, run.hour, false);
    let (scopes_token, _) = baselines.scopes.unwrap();
    let (names_token, _) = baselines.names.unwrap();
    let (values_token, _) = baselines.values.unwrap();
    let inputs = capture.variables_inputs(outcome, baselines, None, &mut run.scratch);
    let today = complete(&capture);
    let TableInput::SliceUpdate { update, .. } = &inputs[2] else {
        panic!("one changed value: {:?}", inputs[2]);
    };
    let mut injected_update = (**update).clone();
    let TableRows::ScriptVariableValues(rows) = &mut injected_update.rows else {
        panic!("ScriptVariableValues rows");
    };
    rows[0].value_raw += 1;
    let injected = [
        inputs[0].clone(),
        inputs[1].clone(),
        TableInput::SliceUpdate {
            token: values_token,
            update: Arc::new(injected_update),
        },
    ];
    let unchanged = [
        TableInput::ComparedEqual(scopes_token),
        TableInput::ComparedEqual(names_token),
        TableInput::ComparedEqual(values_token),
    ];
    let failed = [
        unavailable("fixture refusal").into(),
        unavailable("fixture refusal").into(),
        unavailable("fixture refusal").into(),
    ];
    let mut check = conversion_check::Check::new();
    check.record(baselines, &today, &inputs);
    check.record(baselines, &today, &injected);
    check.record(baselines, &today, &unchanged);
    check.record(baselines, &today, &failed);
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

#[test]
fn complete_no_baseline_preserves_interned_variable_ids() {
    let mut encoder = FrameEncoder::new();
    let mut scratch = ScriptVariableComparison::default();
    let initial = countries();
    let cap0 = captured(&initial);
    let in0 = cap0.variables_inputs(
        cap0.variables.outcome,
        VariableBaselines::default(),
        Some((1, 1, 0)),
        &mut scratch,
    );
    let _ = encoder.encode_bytes(variables_frame(0, in0), true).unwrap();

    let mut added = initial;
    added[0].2.push(("aaa_new", 99));
    let cap1 = captured(&added);
    let in1 = cap1.variables_inputs(
        cap1.variables.outcome,
        shared_baselines(&encoder, 1, false),
        Some((1, 1, 1)),
        &mut scratch,
    );
    let _ = encoder.encode_bytes(variables_frame(1, in1), false).unwrap();

    let cap2 = captured(&added);
    let in2 = cap2.variables_inputs(
        cap2.variables.outcome,
        VariableBaselines::default(),
        Some((1, 1, 2)),
        &mut scratch,
    );
    let TableInput::SharedComplete(rows) = &in2[1] else {
        panic!("complete rotation must emit SharedComplete ScriptVariableNames");
    };
    let TableRows::ScriptVariableNames(names) = rows.as_ref() else {
        panic!("expected ScriptVariableNames");
    };
    let pairs: Vec<(u32, &[u8])> = names.iter().map(|r| (r.id, r.name.as_slice())).collect();
    assert_eq!(
        pairs,
        vec![
            (0, b"alpha".as_slice()),
            (1, b"beta".as_slice()),
            (2, b"gamma".as_slice()),
            (3, b"aaa_new".as_slice()),
        ]
    );

    let replaced = vec![(
        cvariables::Scope::Country(1),
        Some(11),
        vec![("beta", 20), ("ephemeral_next", 77)],
    )];
    let cap3 = captured(&replaced);
    let in3 = cap3.variables_inputs(
        cap3.variables.outcome,
        VariableBaselines::default(),
        Some((1, 1, 3)),
        &mut scratch,
    );
    let TableInput::SharedComplete(names3) = &in3[1] else {
        panic!("complete rotation must emit SharedComplete ScriptVariableNames");
    };
    let TableRows::ScriptVariableNames(names3) = names3.as_ref() else {
        panic!("expected ScriptVariableNames");
    };
    let pairs3: Vec<(u32, &[u8])> = names3.iter().map(|r| (r.id, r.name.as_slice())).collect();
    assert_eq!(
        pairs3,
        vec![(0, b"beta".as_slice()), (1, b"ephemeral_next".as_slice())]
    );
    let TableInput::SharedComplete(vals3) = &in3[2] else {
        panic!("complete rotation must emit SharedComplete ScriptVariableValues");
    };
    let TableRows::ScriptVariableValues(vals3) = vals3.as_ref() else {
        panic!("expected ScriptVariableValues");
    };
    let val_pairs3: Vec<(u32, i64)> = vals3.iter().map(|r| (r.var_id, r.value_raw)).collect();
    assert_eq!(val_pairs3, vec![(0, 20), (1, 77)]);
}
