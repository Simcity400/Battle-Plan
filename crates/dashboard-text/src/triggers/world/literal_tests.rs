use super::super::{inputs, refusal};
use super::*;
use dashboard_core::test_fixtures::game;
use literals::Lookup;
use replay_data::rows::{LexicalTokenRow, TableId, TableRows};

struct Reference<'a> {
    frame: &'a DecodedFrame,
    literals: Index<BTreeMap<u32, &'a [u8]>>,
}

impl<'a> Reference<'a> {
    fn token(&self, token: u32) -> Result<Option<&'a [u8]>, Failure> {
        inputs::query_resolved(
            || inputs::Key::World(Query::Token(token)),
            || {
                let literals = get(&self.literals, || {
                    let tokens = self.frame.complete::<tables::LexicalTokens>().map_err(|error| {
                        let failure =
                            Failure::input(TableId::LexicalTokens, "", None, error.to_string());
                        failure.resolution_failed();
                        failure
                    })?;
                    Ok(tokens
                        .iter()
                        .map(|row| (row.token, row.literal.as_slice()))
                        .collect())
                })?;
                Ok(refusal::reference(
                    literals.get(&token).copied(),
                    token != 0,
                    || refusal::missing_reference(tables::LexicalTokens::ID, None),
                ))
            },
        )
    }
}

fn rows(tokens: &[u32]) -> Vec<LexicalTokenRow> {
    tokens
        .iter()
        .enumerate()
        .map(|(index, &token)| LexicalTokenRow {
            token,
            literal: vec![token as u8, 0, index as u8, 255],
        })
        .collect()
}

fn frame(hour: replay_data::Hour, view: TableView) -> DecodedFrame {
    let mut frame = DecodedFrame {
        branch: 7,
        hour,
        tables: std::array::from_fn(|index| {
            TableView::Complete(TableRows::empty(TableId::ALL[index])).into()
        }),
        metrics: Default::default(),
    };
    *frame.tables[tables::LexicalTokens::ID.index()] = view;
    frame
}

fn with_world<T>(frame: &DecodedFrame, run: impl FnOnce(World<'_>) -> T) -> T {
    let dlcs = BTreeSet::new();
    let triggers = TriggerNames::default();
    let aliases = Arc::default();
    run(World::read(
        frame,
        ContentFacts::content_only(&dlcs, &triggers, &aliases),
    ))
}

type Value = Result<Option<Vec<u8>>, Failure>;
type Observed = (Vec<(Value, BTreeSet<Failure>)>, Vec<u8>, BTreeSet<Failure>);

fn observe<'a>(
    frame: &DecodedFrame,
    tokens: &[u32],
    lookup: impl Fn(u32) -> Result<Option<&'a [u8]>, Failure>,
) -> Observed {
    let ((values, trace), refusals) = refusal::capture(|| {
        inputs::capture(frame, || {
            tokens
                .iter()
                .map(|&token| {
                    refusal::capture(|| lookup(token).map(|value| value.map(<[u8]>::to_vec)))
                })
                .collect()
        })
    });
    (values, format!("{trace:?}").into_bytes(), refusals)
}

fn check(frame: &DecodedFrame) {
    with_world(frame, |actual| {
        let reference = Reference {
            frame,
            literals: OnceLock::new(),
        };
        assert!(actual.literals.get().is_none());
        assert!(reference.literals.get().is_none());
        for tokens in [
            &[7, 0, 2, 99, 1, 7, u32::MAX, 3][..],
            &[99, 3, u32::MAX, 7, 0, 1, 2, 7][..],
            &[7, 0, 2, 99, 1, 7, u32::MAX, 3][..],
        ] {
            assert_eq!(
                observe(frame, tokens, |token| actual.token(token)),
                observe(frame, tokens, |token| reference.token(token)),
            );
        }
    });
}

#[test]
fn token_lookup_matches_map_values_refusals_and_demand_across_hours() {
    game::enter();
    let cases: &[&[u32]] = &[
        &[],
        &[0],
        &[1],
        &[0, 1, 2, 3, 7, u32::MAX],
        &[7, 3, 2, 1, 0],
        &[0, 1, 3, 2, 7],
        &[2, 7, 0, 1, 3],
        &[1, 1, 2, 7],
        &[7, 2, 7, 0, 0, 1],
    ];
    for (hour, tokens) in cases.iter().enumerate() {
        let data = rows(tokens);
        let lookup = Lookup::new(replay_frame::frame::Rowset::Decoded(&data));
        assert_eq!(
            matches!(lookup, Lookup::Sorted(_)),
            tokens.windows(2).all(|pair| pair[0] < pair[1])
        );
        let expected: BTreeMap<_, _> = data
            .iter()
            .map(|row| (row.token, row.literal.as_slice()))
            .collect();
        for token in [0, 1, 2, 3, 7, 99, u32::MAX] {
            assert_eq!(lookup.get(token).unwrap(), expected.get(&token).copied());
        }
        check(&frame(
            hour as replay_data::Hour,
            TableView::Complete(TableRows::LexicalTokens(data)),
        ));
    }
}

#[test]
fn token_lookup_keeps_partial_missing_and_wrong_table_failures() {
    game::enter();
    for (hour, view) in [
        TableView::Truncated {
            rows: TableRows::LexicalTokens(rows(&[7, 1, 7])),
            missing: 4,
        },
        TableView::Unavailable {
            reason: "synthetic token read error".into(),
        },
        TableView::Complete(TableRows::Countries(Vec::new())),
        TableView::NotRecorded {
            mode: replay_data::scope::CaptureMode::Full,
        },
    ]
    .into_iter()
    .enumerate()
    {
        check(&frame(hour as replay_data::Hour, view));
    }
}

#[test]
fn public_token_lookup_keeps_null_and_last_duplicate_semantics() {
    game::enter();
    let frame = frame(
        0,
        TableView::Complete(TableRows::LexicalTokens(rows(&[7, 0, 7]))),
    );
    with_world(&frame, |world| {
        assert!(world.literals.get().is_none());
        assert_eq!(world.token(7).unwrap(), Some([7, 0, 2, 255].as_slice()));
        let held = std::ptr::from_ref(world.literals.get().unwrap());
        assert_eq!(world.token(0).unwrap(), Some([0, 0, 1, 255].as_slice()));
        assert_eq!(held, std::ptr::from_ref(world.literals.get().unwrap()));
    });
    let frame = self::frame(1, TableView::Complete(TableRows::LexicalTokens(Vec::new())));
    with_world(&frame, |world| {
        assert_eq!(
            refusal::capture(|| world.token(0)),
            (Ok(None), BTreeSet::new())
        );
        for _ in 0..2 {
            let (value, failures) = refusal::capture(|| world.token(99));
            assert_eq!(value, Ok(None));
            assert_eq!(
                failures.iter().map(ToString::to_string).collect::<Vec<_>>(),
                ["lexical tokens: not available or refused"]
            );
        }
    });
}

#[test]
fn mod_defined_folded_names_remain_independent_of_literal_lookup() {
    game::enter();
    let frame = frame(
        0,
        TableView::Complete(TableRows::LexicalTokens(vec![LexicalTokenRow {
            token: 9001,
            literal: b"Mod_Defined_Key".to_vec(),
        }])),
    );
    with_world(&frame, |world| {
        assert_eq!(world.token_of(b"MOD_DEFINED_KEY").unwrap(), Some(9001));
        assert!(world.literals.get().is_none());
        assert_eq!(
            world.token(9001).unwrap(),
            Some(b"Mod_Defined_Key".as_slice())
        );
        assert_eq!(world.token_of(b"mod_defined_key").unwrap(), Some(9001));
    });
}
