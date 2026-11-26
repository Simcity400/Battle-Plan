use super::*;
use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
use replay_data::rows::*;
use std::{cell::RefCell, rc::Rc};

const RAID: u64 = 7 << 32 | 84;
const UNIT: u64 = 8 << 32 | 55;
pub(super) const MIO: u64 = 6 << 32 | 79;

fn fixture() -> DecodedFrame {
    let mut f = frame(&countries(), &[]);
    *f.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"synthetic_project".to_vec(),
            },
            LexicalTokenRow {
                token: 11,
                literal: b"synthetic_mio".to_vec(),
            },
        ]));
    *f.tables[TableId::Projects.index()] =
        TableView::Complete(TableRows::Projects(vec![ProjectRow {
            owner: 101,
            local_id: 9,
            definition_id: 10,
            ..Default::default()
        }]));
    *f.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(vec![
            IndustrialOrganisationRow {
                country: 101,
                owner: 103,
                ordinal: 1,
                token: 11,
                organisation: OTHER,
                ..Default::default()
            },
            IndustrialOrganisationRow {
                country: 101,
                owner: 103,
                ordinal: 0,
                token: 11,
                organisation: MIO,
                ..Default::default()
            },
        ]));
    *f.tables[TableId::Raids.index()] = TableView::Complete(TableRows::Raids(vec![RaidRow {
        id: 7,
        reference_type: 84,
        ..Default::default()
    }]));
    *f.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: UNIT,
            roster_owner: 103,
            logical_owner: 101,
            ..Default::default()
        }]));
    f
}

#[test]
fn named_carriers_use_pool_country_and_roster_order() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = fixture();
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let caller = Scopes::of(Scope::Country(1));
        let enter = |key| {
            evaluator.enter(
                &content_reader_script::scope::parse(key, false, world.recorded()),
                caller,
            )
        };
        assert_eq!(
            enter("sp:synthetic_project"),
            Ok(Some(Scope::Project { owner: 101, id: 9 }))
        );
        assert_eq!(
            enter("mio:SYNTHETIC_MIO"),
            Ok(Some(Scope::IndustrialOrganisation(MIO)))
        );
        for (scope, owner) in [
            (Scope::Project { owner: 101, id: 9 }, 1),
            (Scope::Unit(UNIT), 1),
            (Scope::IndustrialOrganisation(MIO), 3),
        ] {
            assert_eq!(
                evaluator.enter(
                    &content_reader_script::scope::Scope::Keyword(content_reader_script::scope::Keyword::Owner),
                    Scopes::of(scope)
                ),
                Ok(Some(Scope::Country(owner)))
            );
        }
    });
}

#[test]
fn the_scripted_text_selection_holds_every_carrier_input() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = fixture();
    for table in TableId::ALL {
        if !crate::scripted_text::TABLES.contains(&table) {
            *f.tables[table.index()] = TableView::Unavailable {
                reason: "not decoded for scripted text".into(),
            };
        }
    }
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        assert_eq!(
            evaluator.enter(
                &content_reader_script::scope::parse("mio:synthetic_mio", false, world.recorded()),
                Scopes::of(Scope::Country(1))
            ),
            Ok(Some(Scope::IndustrialOrganisation(MIO)))
        );
        for (scope, owner) in [(Scope::Unit(UNIT), 1), (Scope::IndustrialOrganisation(MIO), 3)] {
            assert_eq!(
                evaluator.enter(
                    &content_reader_script::scope::Scope::Keyword(content_reader_script::scope::Keyword::Owner),
                    Scopes::of(scope)
                ),
                Ok(Some(Scope::Country(owner)))
            );
        }
    });
}

#[test]
fn named_carriers_resolve_the_last_country() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let tags = (0..24)
        .map(|i| {
            String::from_utf8(vec![
                b'A' + (i / 676) as u8,
                b'A' + (i / 26 % 26) as u8,
                b'A' + (i % 26) as u8,
            ])
            .unwrap()
        })
        .collect::<Vec<_>>();
    let countries = tags
        .iter()
        .enumerate()
        .map(|(i, tag)| {
            (
                10_000 + i as u32,
                i as u32 + 1,
                tag.as_str(),
                0,
                tag.as_str(),
                "",
            )
        })
        .collect::<Vec<_>>();
    let mut f = fixture();
    let scaled = frame(&countries, &[]);
    for table in [
        TableId::Countries,
        TableId::CountryLinks,
        TableId::CountryAppearance,
        TableId::CountryCounterInputs,
    ] {
        f.tables[table.index()] = scaled.tables[table.index()].clone();
    }
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *f.tables[TableId::Projects.index()]
    {
        rows[0].owner = 10_023;
    }
    if let TableView::Complete(TableRows::IndustrialOrganisations(rows)) =
        &mut *f.tables[TableId::IndustrialOrganisations.index()]
    {
        for row in rows {
            row.country = 10_023;
        }
    }
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let caller = Scopes::of(Scope::Country(24));
        assert_eq!(
            evaluator.enter(
                &content_reader_script::scope::parse("sp:synthetic_project", false, world.recorded()),
                caller
            ),
            Ok(Some(Scope::Project {
                owner: 10_023,
                id: 9
            }))
        );
        assert_eq!(
            evaluator.enter(
                &content_reader_script::scope::parse("mio:synthetic_mio", false, world.recorded()),
                caller
            ),
            Ok(Some(Scope::IndustrialOrganisation(MIO)))
        );
    });
}

#[test]
fn prefix_misses_clear_but_countryless_prefixes_keep_caller_and_faults_refuse() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = fixture();
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        for key in ["sp:missing", "mio:missing"] {
            let parsed = content_reader_script::scope::parse(key, false, world.recorded());
            assert_eq!(
                evaluator.enter(&parsed, Scopes::of(Scope::Country(1))),
                Ok(Some(Scope::State(NULL_STATE)))
            );
            assert_eq!(
                evaluator.enter(&parsed, Scopes::of(Scope::Unit(UNIT))),
                Ok(Some(Scope::Unit(UNIT)))
            );
        }
        for key in ["unit:8", "raid:7"] {
            assert_eq!(
                evaluator.enter(
                    &content_reader_script::scope::parse(key, false, world.recorded()),
                    Scopes::of(Scope::Country(1))
                ),
                Ok(None)
            );
        }
    });
    *f.tables[TableId::Projects.index()] = TableView::Unavailable {
        reason: "synthetic project fault".into(),
    };
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let parsed = content_reader_script::scope::parse("sp:synthetic_project", false, world.recorded());
        assert!(
            evaluator
                .enter(&parsed, Scopes::of(Scope::Country(1)))
                .unwrap_err()
                .contains("synthetic project fault")
        );
        assert_eq!(
            evaluator.enter(&parsed, Scopes::of(Scope::Unit(UNIT))),
            Ok(Some(Scope::Unit(UNIT)))
        );
    });
}

#[test]
fn carried_targets_round_trip_links_seeds_and_description_storage() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = fixture();
    with_world(&f, "", Err("none"), |world| {
        for (scope, object) in [
            (
                Scope::Project { owner: 101, id: 9 },
                EventObject::Project { owner: 101, id: 9 },
            ),
            (Scope::Raid(RAID), EventObject::Raid(RAID)),
            (Scope::Unit(UNIT), EventObject::Unit(UNIT)),
        ] {
            let graph = Rc::new(RefCell::new(EventScopes::new(vec![
                EventScope {
                    root: Link::Scope(1),
                    from: Link::Scope(2),
                    random_seed: 19,
                    random_counter: 23,
                    ..EventScope::of(EventObject::Country(1))
                },
                EventScope::of(EventObject::Country(3)),
                EventScope::of(EventObject::Country(4)),
            ])));
            let evaluator = Evaluator::new(world, None).with_event_scopes(graph.clone());
            let caller = evaluator.event_scopes(0).unwrap();
            let entered = evaluator.with_this(caller, scope).unwrap();
            assert_eq!(
                (entered.this, entered.root, entered.from, entered.prev),
                (
                    scope,
                    Scope::Country(3),
                    Some(Scope::Country(4)),
                    Some(Scope::Country(1))
                )
            );
            let at = entered.cursor.unwrap();
            assert_eq!(graph.borrow().scopes[at].this, object);
            assert_eq!(
                (
                    graph.borrow().scopes[at].random_seed,
                    graph.borrow().scopes[at].random_counter
                ),
                (19, 23)
            );
            graph.borrow_mut().scopes[at].from = Link::Scope(1);
            evaluator.save_tooltip_target(b"chosen", entered).unwrap();
            let entered = evaluator.event_scopes(at).unwrap();
            let selected = evaluator
                .enter_scope(
                    &content_reader_script::scope::parse("event_target:chosen", false, world.recorded()),
                    entered,
                )
                .unwrap();
            if matches!(scope, Scope::Project { .. }) {
                assert!(selected.is_none());
            } else {
                let selected = selected.unwrap();
                assert_eq!(selected.this, scope);
                assert_eq!(
                    evaluator.enter(
                        &content_reader_script::scope::Scope::Keyword(
                            content_reader_script::scope::Keyword::Prev
                        ),
                        selected
                    ),
                    Ok(Some(scope))
                );
            }
        }
    });
}

#[test]
fn saved_carriers_follow_the_context_and_variable_readers() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = fixture();
    with_world(&f, "", Err("none"), |world| {
        let graph = Rc::new(RefCell::new(EventScopes::new(vec![EventScope::of(
            EventObject::Country(1),
        )])));
        graph.borrow_mut().saved_targets.insert(
            0,
            vec![
                (b"raid".to_vec(), EventObject::Raid(RAID)),
                (b"unit".to_vec(), EventObject::Unit(UNIT)),
            ],
        );
        let evaluator = Evaluator::new(world, None).with_event_scopes(graph);
        let caller = evaluator.event_scopes(0).unwrap();
        assert_eq!(
            evaluator.enter(
                &content_reader_script::scope::parse("event_target:raid", false, world.recorded()),
                caller
            ),
            Ok(Some(Scope::Country(1)))
        );
        assert_eq!(
            evaluator.scoped_scope(b"event_target:raid", caller),
            Ok(Some(Scope::Raid(RAID)))
        );
        assert_eq!(
            evaluator.enter(
                &content_reader_script::scope::parse("event_target:unit", false, world.recorded()),
                caller
            ),
            Ok(Some(Scope::Unit(UNIT)))
        );
        assert_eq!(evaluator.scoped_value(b"event_target:unit", caller), Ok(0));
    });
}

#[test]
fn tagged_variables_decode_recorded_carriers_without_reading_object_stores() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = fixture();
    *f.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            seed: replay_data::rows::Recorded::Value(None),
            scope_kind: 1,
            scope_id: 101,
            entries: [
                (b"p".to_vec(), 0x1000_0009),
                (b"m".to_vec(), 0x6000_0006),
                (b"r".to_vec(), (0xffff_ffff_e000_0007_u64) as i64),
            ]
            .into_iter()
            .map(|(name, value_raw)| ScriptVariableEntry { name, value_raw })
            .collect(),
        }]));
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let caller = Scopes::of(Scope::Country(1));
        for (name, scope, raw, details) in [
            (
                b"p".as_slice(),
                Scope::Project { owner: 101, id: 9 },
                0x1000_0009,
                false,
            ),
            (
                b"m".as_slice(),
                Scope::IndustrialOrganisation(MIO),
                0x6000_0006,
                true,
            ),
            (
                b"r".as_slice(),
                Scope::Raid(RAID),
                (0xffff_ffff_e000_0007_u64) as i64,
                false,
            ),
        ] {
            let name = [b"var:".as_slice(), name].concat();
            assert_eq!(evaluator.scoped_scope(&name, caller), Ok(Some(scope)));
            assert_eq!(evaluator.scoped_value(b"THIS", Scopes::of(scope)), Ok(raw));
            let refused = evaluator
                .scoped_value(b"THIS.progress", Scopes::of(scope))
                .unwrap_err();
            if details {
                assert!(refused.names("IndustrialOrganisationDetails"), "{refused:?}");
            } else {
                assert!(refused.contains("variable store"), "{refused:?}");
            }
        }
        assert_eq!(
            evaluator.scoped_scope(b"sp:synthetic_project", caller),
            Ok(Some(Scope::Project { owner: 101, id: 9 }))
        );
        assert_eq!(
            evaluator.scoped_scope(b"mio:synthetic_mio", caller),
            Ok(Some(Scope::IndustrialOrganisation(MIO)))
        );
        assert_eq!(evaluator.scoped_scope(b"sp:absent", caller), Ok(None));
    });
    if let TableView::Complete(TableRows::Projects(rows)) =
        &mut *f.tables[TableId::Projects.index()]
    {
        rows.push(ProjectRow {
            owner: 102,
            local_id: 9,
            definition_id: 10,
            ..Default::default()
        });
    }
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        assert!(
            evaluator
                .scoped_scope(b"var:p", Scopes::of(Scope::Country(1)))
                .unwrap_err()
                .contains("ambiguous")
        );
    });
}

#[test]
fn carrier_random_list_identity_uses_object_hash_arms() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = fixture();
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let list = content_reader_scripted_localisation::scripted_localisation::RandomList {
            seed: items("seed=1"),
            ..Default::default()
        };
        for (scope, identity) in [
            (Scope::Unit(UNIT), 8 * 3),
            (Scope::Raid(RAID), 7 * 0x79d),
            (Scope::Project { owner: 101, id: 9 }, 0),
        ] {
            assert_eq!(
                evaluator.localisation_seed(&list, Scopes::of(scope)),
                Ok(Some(100_000 + identity))
            );
        }
    });
}

#[test]
fn persisted_carrier_targets_name_the_missing_reference_and_keep_raid_context() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = fixture();
    *f.tables[TableId::GlobalSavedEventTargets.index()] =
        TableView::Complete(TableRows::GlobalSavedEventTargets(
            [
                GlobalSavedEventTargetRow::PROJECT,
                GlobalSavedEventTargetRow::UNIT,
                GlobalSavedEventTargetRow::RAID_INSTANCE,
                GlobalSavedEventTargetRow::INDUSTRIAL_ORGANISATION,
            ]
            .into_iter()
            .enumerate()
            .map(|(ordinal, live)| GlobalSavedEventTargetRow {
                ordinal: ordinal as u32,
                name: GlobalSavedEventTargetRow::kind_name(live)
                    .as_bytes()
                    .to_vec(),
                live,
                ..Default::default()
            })
            .collect(),
        ));
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let caller = Scopes::of(Scope::Country(1));
        for (name, reference) in [
            ("project", "project"),
            ("unit", "unit"),
            ("industrial_organisation", "organisation"),
        ] {
            let parsed = content_reader_script::scope::parse(
                &format!("event_target:{name}"),
                false,
                world.recorded(),
            );
            assert!(
                evaluator
                    .enter(&parsed, caller)
                    .unwrap_err()
                    .contains(&format!("full {reference} reference"))
            );
        }
        let raid =
            content_reader_script::scope::parse("event_target:raid_instance", false, world.recorded());
        assert_eq!(evaluator.enter(&raid, caller), Ok(Some(Scope::Country(1))));
        assert!(
            evaluator
                .scoped_scope(b"event_target:raid_instance", caller)
                .unwrap_err()
                .contains("full raid reference")
        );
    });
}

pub(super) fn sized(hour: i32, sizes: Option<&[(u64, i32)]>) -> DecodedFrame {
    let mut f = fixture();
    f.hour = hour;
    let TableView::Complete(TableRows::IndustrialOrganisations(organisations)) =
        &mut *f.tables[TableId::IndustrialOrganisations.index()]
    else {
        unreachable!("the fixture holds its organisations");
    };
    organisations.push(IndustrialOrganisationRow {
        country: 102,
        owner: 104,
        ordinal: 0,
        token: 11,
        organisation: HELD,
        ..Default::default()
    });
    *f.tables[TableId::IndustrialOrganisationDetails.index()] = match sizes {
        Some(sizes) => TableView::Complete(TableRows::IndustrialOrganisationDetails(
            sizes
                .iter()
                .map(|&(organisation, size)| {
                    let mut row = IndustrialOrganisationDetailsRow {
                        organisation,
                        country: 101,
                        ..Default::default()
                    };
                    row.state.size = size;
                    row
                })
                .collect(),
        )),
        None => TableView::Unavailable {
            reason: "not recorded".into(),
        },
    };
    f
}

pub(super) const OTHER: u64 = MIO - (1 << 32);
pub(super) const HELD: u64 = MIO + (3 << 32);
pub(super) const SIZES: &[(u64, i32)] = &[(OTHER, 5), (MIO, 6), (HELD, 8)];

pub(super) fn in_organisation(world: &World, organisation: u64, trigger: &str) -> Eval {
    world.evaluate(
        &items(trigger),
        Scopes::of(Scope::IndustrialOrganisation(organisation)),
    )
}

#[test]
fn organisation_size_compares_as_an_integer_trigger() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = sized(1, Some(SIZES));
    with_world(&f, "", Err("none"), |world| {
        for (operator, below, equal, above) in [
            (">", Eval::True, Eval::False, Eval::False),
            ("<", Eval::False, Eval::False, Eval::True),
            ("=", Eval::False, Eval::True, Eval::False),
        ] {
            for (operand, expected) in [(5, &below), (6, &equal), (7, &above)] {
                let trigger = format!("has_mio_size {operator} {operand}");
                assert_eq!(
                    &in_organisation(world, MIO, &trigger),
                    expected,
                    "{trigger}"
                );
            }
        }
        assert_eq!(
            in_organisation(world, OTHER, "has_mio_size > 5"),
            Eval::False
        );
        assert_eq!(
            in_organisation(world, OTHER, "has_mio_size = 5"),
            Eval::True
        );
        assert_eq!(
            in_organisation(world, MIO, "NOT = { has_mio_size > 5 }"),
            Eval::False
        );
        assert!(matches!(
            in_organisation(world, MIO + (2 << 32), "has_mio_size > 5"),
            Eval::Unknown(_)
        ));
        assert_eq!(
            in_organisation(world, OTHER, "has_mio_size > 5 has_mio_size > unknown_size"),
            Eval::False
        );
    });
    with_world(&sized(1, None), "", Err("none"), |world| {
        assert!(matches!(
            in_organisation(world, MIO, "has_mio_size > 5"),
            Eval::Unknown(_)
        ));
    });
}

#[test]
fn organisation_size_operands_are_whole_units() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = sized(1, Some(SIZES));
    with_world(&f, "", Err("none"), |world| {
        for (trigger, expected) in [
            ("has_mio_size > 5.9", Eval::True),
            ("has_mio_size > 6.9", Eval::False),
            ("has_mio_size = 6.9", Eval::True),
            ("has_mio_size > \"5\"", Eval::True),
            ("has_mio_size = \"6.7\"", Eval::True),
            ("has_mio_size < \"6.7\"", Eval::False),
            ("has_mio_size > -1", Eval::True),
            ("has_mio_size < -1", Eval::False),
            ("has_mio_size = -6", Eval::False),
        ] {
            assert_eq!(in_organisation(world, MIO, trigger), expected, "{trigger}");
        }
        let evaluator = Evaluator::new(world, None);
        let scope = Scopes::of(Scope::IndustrialOrganisation(MIO));
        for (raw, trigger, expected) in [
            (550_000, "has_mio_size > size_limit", Eval::True),
            (550_000, "has_mio_size = size_limit", Eval::False),
            (670_000, "has_mio_size = size_limit", Eval::True),
            (670_000, "has_mio_size < size_limit", Eval::False),
            (700_000, "has_mio_size < size_limit", Eval::True),
        ] {
            assert_eq!(
                evaluator.evaluate_seeded(
                    &items(trigger),
                    scope,
                    &[(b"size_limit".as_slice(), raw)]
                ),
                expected,
                "{trigger} with {raw}"
            );
        }
        assert_eq!(
            in_organisation(world, MIO, "has_mio_size > unknown_size"),
            Eval::True
        );
        assert!(matches!(
            in_organisation(world, MIO, "has_mio_size > size"),
            Eval::Unknown(_)
        ));
    });
}

#[test]
fn named_organisations_read_their_own_size() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = sized(1, Some(SIZES));
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let named = |slot: u32| {
            evaluator
                .enter(
                    &content_reader_script::scope::parse("mio:SYNTHETIC_MIO", false, world.recorded()),
                    Scopes::of(Scope::Country(slot)),
                )
                .unwrap()
                .unwrap()
        };
        assert_eq!(named(1), Scope::IndustrialOrganisation(MIO));
        assert_eq!(named(2), Scope::IndustrialOrganisation(HELD));
        let owner = |scope: Scope| {
            evaluator.enter(
                &content_reader_script::scope::Scope::Keyword(content_reader_script::scope::Keyword::Owner),
                Scopes::of(scope),
            )
        };
        assert_eq!(owner(named(1)), Ok(Some(Scope::Country(3))));
        assert_eq!(owner(named(2)), Ok(Some(Scope::Country(4))));
        for (slot, size) in [(1, 6), (2, 8)] {
            let scope = Scopes::of(named(slot));
            assert_eq!(
                world.evaluate(&items(&format!("has_mio_size = {size}")), scope),
                Eval::True
            );
            assert_eq!(
                world.evaluate(&items(&format!("has_mio_size > {size}")), scope),
                Eval::False
            );
        }
    });
}

struct DetailsChanged<'a>(crate::triggers::inputs::TableChange<'a>);

impl crate::triggers::inputs::TableChanges for DetailsChanged<'_> {
    fn table(&self, table: TableId) -> crate::triggers::inputs::TableChange<'_> {
        if table == TableId::IndustrialOrganisationDetails {
            self.0
        } else {
            crate::triggers::inputs::TableChange::Same
        }
    }
}

#[test]
fn organisation_size_inputs_follow_that_organisation_s_size() {
    use crate::triggers::inputs::{Key, Since, TableChange, TableRead, capture};
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let size = (0..64)
        .find(|&field| {
            TableId::IndustrialOrganisationDetails.field_name(field) == Some("state.size")
        })
        .expect("the details table names its size field");
    let first = sized(1, Some(SIZES));
    let (value, inputs) = with_world(&first, "", Err("none"), |world| {
        capture(&first, || in_organisation(world, MIO, "has_mio_size > 5"))
    });
    assert_eq!(value, Eval::True);
    let reads: Vec<TableRead> = inputs
        .keys()
        .filter_map(|key| key.dependence())
        .flatten()
        .filter(|read| read.table == TableId::IndustrialOrganisationDetails)
        .collect();
    assert_eq!(
        reads,
        [
            TableRead::fields(TableId::IndustrialOrganisationDetails, &["state.size"])
                .row(u128::from(MIO))
        ]
    );
    assert_eq!(reads[0].fields, 1 << size);
    fn confirm(
        inputs: &crate::triggers::inputs::Inputs,
        frame: &DecodedFrame,
        change: TableChange<'_>,
    ) -> (bool, bool, usize) {
        let changes = DetailsChanged(change);
        let since = Since::hour_before(frame, &changes).unwrap();
        let unchanged = inputs.unchanged(&since);
        let mut rereads = 0;
        let confirmed = crate::triggers::inputs::reusing(|| {
            with_world(frame, "", Err("none"), |world| {
                inputs
                    .clone()
                    .confirm_since(frame, Some(&since), |key| match key {
                        Key::World(query) => {
                            if matches!(
                                query,
                                crate::triggers::world::Query::IndustrialOrganisationSize(_)
                            ) {
                                rereads += 1;
                            }
                            Some(world.reread(query))
                        }
                        _ => None,
                    })
            })
        });
        (unchanged, confirmed, rereads)
    }
    let changed = |key: u64| DeltaRow {
        format: 0,
        key: u128::from(key),
        mask: 1 << size,
        recorded_fields: TableId::IndustrialOrganisationDetails.schema().len(),
        values: Vec::new(),
    };
    let other = sized(2, Some(&[(OTHER, 7), (MIO, 6), (HELD, 8)]));
    let rows = [changed(OTHER)];
    assert_eq!(
        confirm(&inputs, &other, TableChange::Rows(&rows, &[])),
        (true, true, 0)
    );
    let mut funded = sized(2, Some(SIZES));
    if let TableView::Complete(TableRows::IndustrialOrganisationDetails(rows)) = &mut *funded.tables[TableId::IndustrialOrganisationDetails.index()] {
        rows.iter_mut().filter(|row| row.organisation == MIO).for_each(|row| row.state.funds_raw += 1);
    }
    let funds = (0..64)
        .find(|&field| TableId::IndustrialOrganisationDetails.field_name(field) == Some("state.funds_raw"))
        .expect("the details table names its funds field");
    let rows = [DeltaRow { mask: 1 << funds, ..changed(MIO) }];
    assert_eq!(
        confirm(&inputs, &funded, TableChange::Rows(&rows, &[])),
        (true, true, 0),
        "a change to the organisation's funds alone leaves its size, and the evaluation, as they were"
    );
    let shrunk = sized(2, Some(&[(OTHER, 5), (MIO, 5), (HELD, 8)]));
    let rows = [changed(MIO)];
    assert_eq!(
        confirm(&inputs, &shrunk, TableChange::Rows(&rows, &[])),
        (false, false, 1)
    );
    with_world(&shrunk, "", Err("none"), |world| {
        assert_eq!(in_organisation(world, MIO, "has_mio_size > 5"), Eval::False);
    });
    let gone = sized(2, Some(&[(OTHER, 5), (HELD, 8)]));
    let deleted = [u128::from(MIO)];
    assert_eq!(
        confirm(&inputs, &gone, TableChange::Rows(&[], &deleted)),
        (false, false, 1)
    );
    let unavailable = sized(2, None);
    assert_eq!(
        confirm(&inputs, &unavailable, TableChange::Any),
        (false, false, 1)
    );
}

#[test]
fn project_built_in_variables_are_refused() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = fixture();
    with_world(&f, "", Err("none"), |world| {
        let scope = Scopes::of(Scope::Project { owner: 101, id: 9 });
        for name in content_reader_script::variable_builtins::PROJECT.iter() {
            let value = world.evaluate(&items(&format!("check_variable = {{ {name} > 0 }}")), scope);
            assert!(
                matches!(&value, Eval::Unknown(_)) && format!("{value:?}").contains("built-in"),
                "{name}: {value:?}"
            );
        }
    });
}

#[test]
fn a_completed_project_is_known_only_from_the_countrys_accepted_project_rows() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = fixture();
    let TableView::Complete(TableRows::Projects(rows)) = &mut *f.tables[TableId::Projects.index()] else { unreachable!() };
    rows[0].completed = 1;
    let completed = rows[0].clone();
    let reward = "if = { limit = { is_special_project_completed = sp:synthetic_project } add_political_power = 5 } else = { add_political_power = 7 }";
    let limit = |world: &World| world.evaluate(&items("is_special_project_completed = sp:synthetic_project"), Scopes::of(Scope::Country(1)));
    let accepted = |rows: Vec<ProjectRow>, slots: &[u32]| AcceptedProjects {
        owners: rows.iter().map(|row| row.owner).collect(),
        rows: rows.into_iter().map(|row| ((row.owner, row.local_id), row)).collect(),
        slots: slots.iter().copied().collect(),
    };
    with_world_accepted(&f, accepted(vec![completed], &[1]), |world| {
        assert_eq!(limit(world), Eval::True, "the accepted completed row proves the if branch");
    });
    with_world_accepted(&f, accepted(Vec::new(), &[1]), |world| {
        assert_eq!(limit(world), Eval::False, "every owner read and none completed proves the else branch");
        assert!(
            matches!(world.evaluate(&items("is_special_project_completed = sp:never_spelled"), Scopes::of(Scope::Country(1))), Eval::Unknown(_)),
            "a project with no recorded spelling is unknown, never false"
        );
    });
    with_world_accepted(&f, accepted(Vec::new(), &[]), |world| {
        assert!(
            matches!(limit(world), Eval::Unknown(_)),
            "O-S1: the second owner's completed row was read before it joined the country, so neither branch of `{reward}` is known and the reward is withheld"
        );
    });
}
