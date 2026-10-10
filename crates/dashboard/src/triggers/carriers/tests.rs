use super::*;
use crate::scripted_text::{EventObject, EventScope, EventScopes, Link};
use replay::rows::*;
use std::{cell::RefCell, rc::Rc};

const RAID: u64 = 7 << 32 | 84;
const UNIT: u64 = 8 << 32 | 55;
const MIO: u64 = 6 << 32 | 79;

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
                organisation: MIO + (1 << 32),
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = fixture();
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let caller = Scopes::of(Scope::Country(1));
        let enter = |key| {
            evaluator.enter(
                &installed_content::scope::parse(key, false, world.recorded()),
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
                    &installed_content::scope::Scope::Keyword(installed_content::scope::Keyword::Owner),
                    Scopes::of(scope)
                ),
                Ok(Some(Scope::Country(owner)))
            );
        }
    });
}

#[test]
fn named_carriers_resolve_the_last_of_five_thousand_countries() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let tags = (0..5_000)
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
        rows[0].owner = 14_999;
    }
    if let TableView::Complete(TableRows::IndustrialOrganisations(rows)) =
        &mut *f.tables[TableId::IndustrialOrganisations.index()]
    {
        for row in rows {
            row.country = 14_999;
        }
    }
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let caller = Scopes::of(Scope::Country(5_000));
        assert_eq!(
            evaluator.enter(
                &installed_content::scope::parse("sp:synthetic_project", false, world.recorded()),
                caller
            ),
            Ok(Some(Scope::Project {
                owner: 14_999,
                id: 9
            }))
        );
        assert_eq!(
            evaluator.enter(
                &installed_content::scope::parse("mio:synthetic_mio", false, world.recorded()),
                caller
            ),
            Ok(Some(Scope::IndustrialOrganisation(MIO)))
        );
    });
}

#[test]
fn prefix_misses_clear_but_countryless_prefixes_keep_caller_and_faults_refuse() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = fixture();
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        for key in ["sp:missing", "mio:missing"] {
            let parsed = installed_content::scope::parse(key, false, world.recorded());
            assert_eq!(
                evaluator.enter(&parsed, Scopes::of(Scope::Country(1))),
                Ok(Some(Scope::State(NULL_STATE)))
            );
            assert_eq!(
                evaluator.enter(&parsed, Scopes::of(Scope::Unit(UNIT))),
                Ok(Some(Scope::Unit(UNIT)))
            );
        }
        // Scope keys accept no raid:/unit: namespace. Carriers come from
        // carried THIS/ROOT/FROM/PREV, saved targets, and tagged raid variables.
        for key in ["unit:8", "raid:7"] {
            assert_eq!(
                evaluator.enter(
                    &installed_content::scope::parse(key, false, world.recorded()),
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
        let parsed = installed_content::scope::parse("sp:synthetic_project", false, world.recorded());
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
                    &installed_content::scope::parse("event_target:chosen", false, world.recorded()),
                    entered,
                )
                .unwrap();
            if matches!(scope, Scope::Project { .. }) {
                assert!(selected.is_none()); // Saved targets omit PROJECT.
            } else {
                let selected = selected.unwrap();
                assert_eq!(selected.this, scope);
                assert_eq!(
                    evaluator.enter(
                        &installed_content::scope::Scope::Keyword(installed_content::scope::Keyword::Prev),
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
                &installed_content::scope::parse("event_target:raid", false, world.recorded()),
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
                &installed_content::scope::parse("event_target:unit", false, world.recorded()),
                caller
            ),
            Ok(Some(Scope::Unit(UNIT)))
        );
        // Storage resolution skips UNIT; no invented tagged UNIT value.
        assert_eq!(evaluator.scoped_value(b"event_target:unit", caller), Ok(0));
    });
}

#[test]
fn tagged_variables_decode_recorded_carriers_without_reading_object_stores() {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let mut f = fixture();
    *f.tables[TableId::ScriptVariables.index()] =
        TableView::Complete(TableRows::ScriptVariables(vec![ScriptVariablesRow {
            seed: replay::rows::Recorded::Value(None),
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
        for (name, scope, raw) in [
            (
                b"p".as_slice(),
                Scope::Project { owner: 101, id: 9 },
                0x1000_0009,
            ),
            (
                b"m".as_slice(),
                Scope::IndustrialOrganisation(MIO),
                0x6000_0006,
            ),
            (
                b"r".as_slice(),
                Scope::Raid(RAID),
                (0xffff_ffff_e000_0007_u64) as i64,
            ),
        ] {
            let name = [b"var:".as_slice(), name].concat();
            assert_eq!(evaluator.scoped_scope(&name, caller), Ok(Some(scope)));
            assert_eq!(evaluator.scoped_value(b"THIS", Scopes::of(scope)), Ok(raw));
            assert!(
                evaluator
                    .scoped_value(b"THIS.progress", Scopes::of(scope))
                    .unwrap_err()
                    .contains("variable store")
            );
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let f = fixture();
    with_world(&f, "", Err("none"), |world| {
        let evaluator = Evaluator::new(world, None);
        let list = installed_content::scripted_localisation::RandomList {
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
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
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
            let parsed =
                installed_content::scope::parse(&format!("event_target:{name}"), false, world.recorded());
            assert!(
                evaluator
                    .enter(&parsed, caller)
                    .unwrap_err()
                    .contains(&format!("full {reference} reference"))
            );
        }
        let raid = installed_content::scope::parse("event_target:raid_instance", false, world.recorded());
        assert_eq!(evaluator.enter(&raid, caller), Ok(Some(Scope::Country(1))));
        assert!(
            evaluator
                .scoped_scope(b"event_target:raid_instance", caller)
                .unwrap_err()
                .contains("full raid reference")
        );
    });
}
