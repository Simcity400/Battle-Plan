use super::*;
use crate::fixtures::{Folder, folder_catalogue};
use crate::effect_description::tests::{Fixture, fresh};
use crate::scripted_text::{EventScope, Link};
use replay_data::rows::*;
use replay_frame::frame::TableView;

const RAID: u64 = 7 << 32 | 84;
const UNIT: u64 = 8 << 32 | 55;
const MIO: u64 = 6 << 32 | 79;
const WING: u64 = 9 << 32 | 77;

fn assert_missing_reference(table: TableId, read: impl FnOnce() -> Result<String, String>) {
    let (result, failures) = crate::triggers::refusal::capture(read);
    assert_eq!(
        result.unwrap_err(),
        format!("{}: the object it refers to is not recorded", table.words())
    );
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(
        failures.first().unwrap().to_string(),
        format!("{}: not available or refused", table.words())
    );
}

fn fixture() -> (Fixture, Folder) {
    let mut f = Fixture::new();
    let (catalogue, folder) = folder_catalogue(&[
        (
            "common/defines/test.lua",
            "NDefines={NGame={MAX_SCRIPTED_LOC_RECURSION=30},NCharacter={OFFICER_CORP_ADVISOR_ENTRIES_IN_MENU={}},NSupply={MAX_RAILWAY_LEVEL=5}}",
        ),
        (
            "common/special_projects/projects/test.txt",
            "synthetic_project={narrative={name=PROJECT_NAME}} synthetic_default={}",
        ),
        (
            "localisation/test_l_english.yml",
            concat!(
                "\u{feff}l_english:\n AAA:0 \"Alpha\"\n BBB:0 \"Beta\"\n",
                " PROJECT_NAME:0 \"base\"\n AAA_PROJECT_NAME:0 \"Project[ROOT.GetTag]/[FROM.GetTag]/[PREV.GetTag]\"\n",
                " synthetic_default:0 \"Default\"\n SYN_MIO:0 \"base MIO\"\n BBB_SYN_MIO:0 \"MIO[FROM.GetTag]\"\n",
                " EFFECT_CONTEXT_SPECIAL_PROJECT:0 \"$SPECIAL_PROJECT$ $FLAG$\"\n",
                " EFFECT_ADD_ARMY_EXPERIENCE:0 \"XP$VAL|0$\"\n",
                " raid_type_synthetic_raid:0 \"Strike\"\n TARGET:0 \"Hit $LOCATION$\"\n",
                " raid_inst_item_desc:0 \"$TYPE$/$TARGET$\"\n RAID_TARGET_NAME:0 \"$WHAT$ in $LOCATION$\"\n",
            ),
        ),
    ]);
    catalogue
        .scripted_content()
        .unwrap()
        .set_synthetic_environment(Default::default());
    f.catalogue = catalogue;
    *f.frame.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 10,
                literal: b"synthetic_project".to_vec(),
            },
            LexicalTokenRow {
                token: 11,
                literal: b"synthetic_default".to_vec(),
            },
            LexicalTokenRow {
                token: 12,
                literal: b"synthetic_raid".to_vec(),
            },
            LexicalTokenRow {
                token: 13,
                literal: b"SYN_MIO".to_vec(),
            },
        ]));
    *f.frame.tables[TableId::Projects.index()] = TableView::Complete(TableRows::Projects(vec![
        ProjectRow {
            owner: 100,
            local_id: 9,
            definition_id: 10,
            ..Default::default()
        },
        ProjectRow {
            owner: 100,
            local_id: 10,
            definition_id: 11,
            ..Default::default()
        },
    ]));
    *f.frame.tables[TableId::Raids.index()] =
        TableView::Complete(TableRows::Raids(vec![RaidRow {
            reference_type: 84,
            id: 7,
            ..Default::default()
        }]));
    *f.frame.tables[TableId::RaidTargets.index()] =
        TableView::Complete(TableRows::RaidTargets(vec![RaidTargetRow {
            reference_type: 84,
            id: 7,
            raid_type: 12,
            scope: RaidScope {
                state: 2,
                ..Default::default()
            },
            ..Default::default()
        }]));
    *f.frame.tables[TableId::Divisions.index()] =
        TableView::Complete(TableRows::Divisions(vec![DivisionRow {
            id: UNIT,
            logical_owner: 100,
            ..Default::default()
        }]));
    *f.frame.tables[TableId::IndustrialOrganisations.index()] =
        TableView::Complete(TableRows::IndustrialOrganisations(vec![
            IndustrialOrganisationRow {
                organisation: MIO,
                country: 100,
                owner: 200,
                token: 13,
                ..Default::default()
            },
        ]));
    let mut raids = content_reader_raids::raids::Catalog::default();
    let mut kind = content_reader_raids::raids::RaidType {
        name: "synthetic_raid".into(),
        ..Default::default()
    };
    kind.tooltip.target_loc_key = "TARGET".into();
    raids.types.push(kind);
    assert!(
        f.catalogue
            .scripted_content()
            .unwrap()
            .effects()
            .unwrap()
            .raids
            .set(Ok(raids))
            .is_ok()
    );
    (f, folder)
}

#[test]
fn project_and_mio_names_use_keys_and_fresh_environments() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (f, _folder) = fixture();
    let mut graph = EventScopes::new(vec![
        EventScope {
            root: Link::Scope(1),
            from: Link::Scope(1),
            prev: Link::Scope(1),
            ..EventScope::of(EventObject::Country(1))
        },
        EventScope::of(EventObject::Country(2)),
    ]);
    f.with_renderer(&mut graph, Content::default(), |renderer| {
        assert_eq!(
            renderer.project_name(100, 9).as_deref(),
            Ok("ProjectAAA/AAA/AAA")
        );
        assert_eq!(renderer.project_name(100, 10).as_deref(), Ok("Default"));
        assert_eq!(renderer.organisation_name(MIO).as_deref(), Ok("MIOBBB"));
        assert_missing_reference(TableId::Projects, || renderer.project_name(100, 999));
        assert_missing_reference(TableId::IndustrialOrganisations, || {
            renderer.organisation_name(999)
        });
    });
    let out = f.render(
        "sp:synthetic_project={army_experience=2}",
        &mut fresh(),
        Content::default(),
    );
    assert_eq!(out.text, "ProjectAAA/AAA/AAA \u{12}AAA XP2\n");
    assert!(out.refusals.is_empty(), "{:?}", out.refusals);
}

#[test]
fn raid_name_joins_instance_target_type_and_content_and_refuses_unknown_identity() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut f, _folder) = fixture();
    f.with_renderer(&mut fresh(), Content::default(), |renderer| {
        assert_eq!(renderer.raid_name(RAID).as_deref(), Ok("Strike/Hit Two"));
        assert_missing_reference(TableId::Raids, || renderer.raid_name(999));
        assert!(
            renderer
                .unit_name(UNIT)
                .unwrap_err()
                .contains("cached army name is not recorded")
        );
        assert!(
            renderer
                .unit_name(999)
                .unwrap_err()
                .contains("Divisions lacks")
        );
    });
    if let TableView::Complete(TableRows::RaidTargets(rows)) =
        &mut *f.frame.tables[TableId::RaidTargets.index()]
    {
        rows[0].raid_type = 0;
    }
    f.with_renderer(&mut fresh(), Content::default(), |renderer| {
        assert_eq!(renderer.raid_name(RAID).as_deref(), Ok("/Two"));
    });
    *f.frame.tables[TableId::RaidTargets.index()] = TableView::Unavailable {
        reason: "synthetic target fault".into(),
    };
    f.with_renderer(&mut fresh(), Content::default(), |renderer| {
        assert!(
            renderer
                .raid_name(RAID)
                .unwrap_err()
                .contains("synthetic target fault")
        );
    });
}

#[test]
fn unit_and_raid_contexts_have_no_invented_header_and_still_walk_children() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (f, _folder) = fixture();
    for object in [EventObject::Raid(RAID), EventObject::Unit(UNIT)] {
        let mut graph = EventScopes::new(vec![EventScope::of(object)]);
        let out = f.render("THIS={army_experience=2}", &mut graph, Content::default());
        assert_eq!(out.text, "XP2\n");
        assert!(out.refusals.is_empty(), "{:?}", out.refusals);
    }
}

#[test]
fn carried_saved_targets_do_not_invent_country_or_state_members() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (f, _folder) = fixture();
    let mut graph = fresh();
    graph.saved_targets.insert(
        0,
        vec![
            (b"raid".to_vec(), EventObject::Raid(RAID)),
            (b"unit".to_vec(), EventObject::Unit(UNIT)),
            (
                b"project".to_vec(),
                EventObject::Project { owner: 100, id: 9 },
            ),
        ],
    );
    f.with_renderer(&mut graph, Content::default(), |renderer| {
        for name in ["raid", "unit", "project"] {
            let operand =
                content_reader_parser::parser::parse(format!("target=event_target:{name}").as_bytes())
                    .unwrap();
            let scopes = renderer.evaluator().unwrap().event_scopes(0).unwrap();
            assert_eq!(renderer.country_target(&operand.items[0], scopes), Ok(None));
            assert_eq!(renderer.state_target(&operand.items[0], scopes), Ok(0));
        }
    });
}

#[test]
fn unit_name_programmatic_overrides_follow_recorded_group_and_language() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let names = content_reader_division_names::division_names::parse(
        br#"G={ordered={2={"Generated %d"}}}"#,
        &Default::default(),
    )
    .unwrap();
    let content = Content {
        division_names: Some(&names),
        ..Default::default()
    };
    let (mut f, _folder) = fixture();
    for (group, programmatic, custom, language, expected) in [
        (
            "",
            1,
            &b"Stored %d [Root.GetTag]"[..],
            "english",
            Ok("Stored %d [Root.GetTag]"),
        ),
        (
            "",
            255,
            &b"Stored"[..],
            "french",
            Err("stored in english without its key"),
        ),
        (
            "",
            0,
            &b"Player literal"[..],
            "french",
            Ok("Player literal"),
        ),
        ("G", 1, &b"Ignored"[..], "english", Ok("Generated 2")),
        ("G", 255, &[0xff][..], "french", Ok("Generated 2")),
        (
            "",
            1,
            &[0xff][..],
            "english",
            Err("programmatic division name is not UTF-8"),
        ),
    ] {
        std::sync::Arc::make_mut(&mut f.catalogue.localisation).language = language.into();
        *f.frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: UNIT,
                name: Recorded::NotRecorded,
                naming: Recorded::Value(Some(NameOperands {
                    group: group.as_bytes().to_vec(),
                    order: 2,
                    ordered: 1,
                    programmatic,
                    custom: custom.to_vec(),
                    ..Default::default()
                })),
                ..Default::default()
            }]));
        f.with_renderer(&mut fresh(), content, |renderer| match expected {
            Ok(text) => assert_eq!(renderer.unit_name(UNIT).as_deref(), Ok(text)),
            Err(reason) => {
                let why = renderer.unit_name(UNIT).unwrap_err();
                assert!(
                    why.contains(reason),
                    "{group}/{programmatic}/{language}: {why}"
                );
            }
        });
    }
}

#[test]
fn unit_carrier_uses_retained_army_names_without_generator_content() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut f, _folder) = fixture();
    for (name, expected) in [
        (
            Recorded::Value(Some(b"Retained legacy name".to_vec())),
            Ok("Retained legacy name"),
        ),
        (Recorded::Value(Some(Vec::new())), Ok("")),
        (
            Recorded::Value(None),
            Err("name of division status unavailable"),
        ),
    ] {
        *f.frame.tables[TableId::DivisionStatus.index()] =
            TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
                division: UNIT,
                name,
                naming: Recorded::Value(None),
                ..Default::default()
            }]));
        f.with_renderer(
            &mut fresh(),
            Content::default(),
            |renderer| match expected {
                Ok(name) => assert_eq!(renderer.unit_name(UNIT).as_deref(), Ok(name)),
                Err(reason) => assert!(renderer.unit_name(UNIT).unwrap_err().contains(reason)),
            },
        );
    }
}

fn wing_fixture(bytes: &[u8]) -> (Fixture, Folder) {
    let (mut f, folder) = fixture();
    *f.frame.tables[TableId::Raids.index()] =
        TableView::Complete(TableRows::Raids(vec![RaidRow {
            reference_type: RAID as u32,
            id: (RAID >> 32) as u32,
            air_wing: WING,
            phase: 4,
            ..Default::default()
        }]));
    *f.frame.tables[TableId::AirWings.index()] =
        TableView::Complete(TableRows::AirWings(vec![AirWingRow {
            id: WING,
            owner: 100,
            name: Recorded::NotRecorded,
            ..Default::default()
        }]));
    set_wing_name(&mut f, bytes);
    (f, folder)
}

fn set_wing_name(f: &mut Fixture, bytes: &[u8]) {
    use engine_native::{
        raid_tooltip_operands::ARMY_RESOLVED, raid_tooltip_operands::NAME,
        raid_tooltip_operands::REFERENCES, raid_tooltip_operands::WING_NAME,
        raid_tooltip_operands::WING_RESOLVED,
    };
    let values = [
        (REFERENCES, 0, 0),
        (REFERENCES, 1, WING as i64),
        (NAME, ARMY_RESOLVED, 0),
        (NAME, WING_RESOLVED, 1),
        (WING_NAME, u32::MAX, bytes.len() as i64),
    ];
    let chunks = bytes.chunks(8).enumerate().map(|(index, chunk)| {
        let mut packed = [0; 8];
        packed[..chunk.len()].copy_from_slice(chunk);
        (WING_NAME, index as u32, i64::from_le_bytes(packed))
    });
    *f.frame.tables[TableId::RaidTooltipOperands.index()] =
        TableView::Complete(TableRows::RaidTooltipOperands(
            values
                .into_iter()
                .chain(chunks)
                .enumerate()
                .map(|(ordinal, (kind, key, raw))| RaidTooltipOperandRow {
                    raid: RAID,
                    ordinal: ordinal as u32,
                    kind,
                    key,
                    raw,
                })
                .collect(),
        ));
}

#[test]
fn stored_air_wing_names_cover_non_raids_and_do_not_mask_capture_failures() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut f, _folder) = wing_fixture(b"Older raid fallback");
    for (name, language, expected) in [
        (
            Recorded::Value(Some(b"Wing %d [Root.GetTag]".to_vec())),
            "english",
            Ok("Wing %d [Root.GetTag]"),
        ),
        (
            Recorded::Value(Some(b"Wing".to_vec())),
            "french",
            Err("stored in english without its key"),
        ),
        (Recorded::Value(Some(Vec::new())), "french", Ok("")),
        (
            Recorded::Value(Some(vec![0xff])),
            "english",
            Err("air wing name is not UTF-8"),
        ),
        (
            Recorded::Value(None),
            "english",
            Err("guarded read or name arena refusal"),
        ),
    ] {
        if let TableView::Complete(TableRows::AirWings(rows)) =
            &mut *f.frame.tables[TableId::AirWings.index()]
        {
            rows[0].name = name;
        }
        for table in [
            TableId::Countries,
            TableId::CountryLinks,
            TableId::Provinces,
            TableId::States,
            TableId::Divisions,
            TableId::Raids,
            TableId::RaidTooltipOperands,
        ] {
            *f.frame.tables[table.index()] = TableView::Unavailable {
                reason: "not needed for stored wing names".into(),
            };
        }
        std::sync::Arc::make_mut(&mut f.catalogue.localisation).language = language.into();
        f.with_renderer(
            &mut fresh(),
            Content::default(),
            |renderer| match expected {
                Ok(text) => assert_eq!(renderer.unit_name(WING).as_deref(), Ok(text)),
                Err(reason) => assert!(renderer.unit_name(WING).unwrap_err().contains(reason)),
            },
        );
    }
}

#[test]
fn old_raid_wing_names_do_not_need_the_division_census() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut f, _folder) = wing_fixture(b"Older raid fallback");
    let frame = f.frame.clone();
    for missing_wing_table in [false, true] {
        f.frame = frame.clone();
        *f.frame.tables[TableId::Divisions.index()] = TableView::Unavailable {
            reason: "division census failed".into(),
        };
        if missing_wing_table {
            *f.frame.tables[TableId::AirWings.index()] = TableView::Unavailable {
                reason: "wing census failed".into(),
            };
        }
        f.with_renderer(&mut fresh(), Content::default(), |renderer| {
            assert_eq!(
                renderer.unit_name(WING).as_deref(),
                Ok("Older raid fallback")
            );
        });
    }
}

#[test]
fn unit_name_dispatches_raid_divisions_and_selected_wings() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut f, _folder) = wing_fixture(b"Wing %d [Root.GetTag]");
    *f.frame.tables[TableId::DivisionStatus.index()] =
        TableView::Complete(TableRows::DivisionStatus(vec![DivisionStatusRow {
            division: UNIT,
            name: Recorded::NotRecorded,
            naming: Recorded::Value(Some(NameOperands {
                custom: b"Raid division".to_vec(),
                ..Default::default()
            })),
            ..Default::default()
        }]));
    let mut graph = EventScopes::new(vec![EventScope::of(EventObject::Raid(RAID))]);
    f.with_renderer(&mut graph, Content::default(), |renderer| {
        let raid = renderer.context.world().unwrap().raid(RAID).unwrap();
        assert_eq!(
            renderer.unit_name(raid.air_wing).as_deref(),
            Ok("Wing %d [Root.GetTag]")
        );
    });
    if let TableView::Complete(TableRows::Raids(rows)) =
        &mut *f.frame.tables[TableId::Raids.index()]
    {
        rows[0].unit = UNIT;
        rows[0].air_wing = 0;
    }
    f.with_renderer(&mut graph, Content::default(), |renderer| {
        let raid = renderer.context.world().unwrap().raid(RAID).unwrap();
        assert_eq!(
            renderer.unit_name(raid.unit).as_deref(),
            Ok("Raid division")
        );
    });
}

#[test]
fn active_raid_wing_names_share_stored_text_language_and_utf8_rules() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut f, _folder) = wing_fixture(b"");
    for (bytes, language, expected) in [
        (&b"Stored wing"[..], "english", Ok("Stored wing")),
        (
            &b"Stored wing"[..],
            "french",
            Err("stored in english without its key"),
        ),
        (&b""[..], "french", Ok("")),
        (&[0xff][..], "english", Err("raid wing name is not UTF-8")),
    ] {
        set_wing_name(&mut f, bytes);
        std::sync::Arc::make_mut(&mut f.catalogue.localisation).language = language.into();
        f.with_renderer(
            &mut fresh(),
            Content::default(),
            |renderer| match expected {
                Ok(text) => assert_eq!(renderer.unit_name(WING).as_deref(), Ok(text)),
                Err(reason) => assert!(renderer.unit_name(WING).unwrap_err().contains(reason)),
            },
        );
    }
}

#[test]
fn active_raid_wing_name_table_refuses_capture_and_join_faults() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    use engine_native::{
        raid_tooltip_operands::ARMY_RESOLVED, raid_tooltip_operands::NAME,
        raid_tooltip_operands::REFERENCES, raid_tooltip_operands::UNREADABLE,
        raid_tooltip_operands::WING_NAME, raid_tooltip_operands::WING_RESOLVED,
    };
    let (mut f, _folder) = wing_fixture(b"");
    let frame = f.frame.clone();
    for (kind, key, raw, reason) in [
        (REFERENCES, 0, 55, "naming reference differs"),
        (
            REFERENCES,
            1,
            (WING + (1 << 32)) as i64,
            "naming reference differs",
        ),
        (NAME, ARMY_RESOLVED, 1, "Army-before-wing"),
        (NAME, WING_RESOLVED, 0, "did not resolve"),
        (UNREADABLE, 0, 0, "unreadable when recorded"),
        (WING_NAME, u32::MAX, -1, "length exceeds capacity"),
        (WING_NAME, u32::MAX, -2, "data pointer is null"),
        (WING_NAME, u32::MAX, -3, "terminator is invalid"),
        (WING_NAME, u32::MAX, -4, "chunk index overflows"),
        (WING_NAME, u32::MAX, -5, "header or bytes are unreadable"),
        (
            WING_NAME,
            u32::MAX,
            -6,
            "unsupported wing-name read failure code",
        ),
        (WING_NAME, u32::MAX, i64::MAX, "not recorded in full"),
        (WING_NAME, 0, 0, "duplicate, extra or out of order"),
    ] {
        f.frame = frame.clone();
        if let TableView::Complete(TableRows::RaidTooltipOperands(rows)) =
            &mut *f.frame.tables[TableId::RaidTooltipOperands.index()]
        {
            if let Some(row) = rows
                .iter_mut()
                .find(|row| row.kind == kind && row.key == key)
            {
                row.raw = raw;
            } else {
                rows.push(RaidTooltipOperandRow {
                    raid: RAID,
                    ordinal: rows.len() as u32,
                    kind,
                    key,
                    raw,
                });
            }
        }
        f.with_renderer(&mut fresh(), Content::default(), |renderer| {
            let why = renderer.unit_name(WING).unwrap_err();
            assert!(why.contains(reason), "{kind}:{key:#x}={raw}: {why}");
        });
    }
    set_wing_name(&mut f, b"Long wing name");
    let frame = f.frame.clone();
    for (fault, reason) in [
        (0, "duplicated"),
        (1, "not recorded in full"),
        (2, "duplicate, extra or out of order"),
        (3, "is not recorded"),
    ] {
        f.frame = frame.clone();
        if let TableView::Complete(TableRows::RaidTooltipOperands(rows)) =
            &mut *f.frame.tables[TableId::RaidTooltipOperands.index()]
        {
            match fault {
                0 => {
                    let mut duplicate = rows[4].clone();
                    duplicate.ordinal = 20;
                    rows.push(duplicate);
                }
                1 => {
                    rows.pop();
                }
                2 => rows.swap(5, 6),
                _ => rows.retain(|row| row.kind != WING_NAME),
            }
        }
        f.with_renderer(&mut fresh(), Content::default(), |renderer| {
            let why = renderer.unit_name(WING).unwrap_err();
            assert!(why.contains(reason), "{fault}: {why}");
        });
    }
}

#[test]
fn unit_name_refuses_other_wings_ended_raids_and_unread_tables() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut f, _folder) = wing_fixture(b"Stored wing");
    f.with_renderer(&mut fresh(), Content::default(), |renderer| {
        assert!(
            renderer
                .unit_name(WING + (1 << 32))
                .unwrap_err()
                .contains("outside selected active raids is not recorded")
        );
    });
    if let TableView::Complete(TableRows::Raids(rows)) =
        &mut *f.frame.tables[TableId::Raids.index()]
    {
        rows[0].phase = engine_native::craid_instance::PHASE_ENDED;
    }
    f.with_renderer(&mut fresh(), Content::default(), |renderer| {
        assert!(
            renderer
                .unit_name(WING)
                .unwrap_err()
                .contains("outside selected active raids")
        );
    });
    for table in [TableId::Raids, TableId::RaidTooltipOperands] {
        let (mut f, _folder) = wing_fixture(b"Stored wing");
        *f.frame.tables[table.index()] = TableView::Unavailable {
            reason: "synthetic naming fault".into(),
        };
        f.with_renderer(&mut fresh(), Content::default(), |renderer| {
            assert!(
                renderer
                    .unit_name(WING)
                    .unwrap_err()
                    .contains("synthetic naming fault")
            );
        });
    }
}

#[test]
fn active_raids_must_agree_on_one_wing_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (mut f, _folder) = wing_fixture(b"Wing");
    let frame = f.frame.clone();
    for same in [true, false] {
        f.frame = frame.clone();
        let other = RAID + (1 << 32);
        if let TableView::Complete(TableRows::Raids(rows)) =
            &mut *f.frame.tables[TableId::Raids.index()]
        {
            let mut raid = rows[0].clone();
            raid.id += 1;
            rows.push(raid);
        }
        if let TableView::Complete(TableRows::RaidTooltipOperands(rows)) =
            &mut *f.frame.tables[TableId::RaidTooltipOperands.index()]
        {
            let more: Vec<_> = rows
                .iter()
                .cloned()
                .map(|mut row| {
                    row.raid = other;
                    if !same
                        && row.kind == engine_native::raid_tooltip_operands::WING_NAME
                        && row.key == 0
                    {
                        row.raw = i64::from_le_bytes(*b"Else\0\0\0\0");
                    }
                    row
                })
                .collect();
            rows.extend(more);
        }
        f.with_renderer(&mut fresh(), Content::default(), |renderer| {
            if same {
                assert_eq!(renderer.unit_name(WING).as_deref(), Ok("Wing"));
            } else {
                assert!(
                    renderer
                        .unit_name(WING)
                        .unwrap_err()
                        .contains("different names for the same wing")
                );
            }
        });
    }
}

#[test]
fn active_raid_wing_names_have_no_country_or_wing_cap() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLEPLAN_GAME_DIR must name the supported installation for runtime table tests");
    let count = 24;
    let (mut f, _folder) = wing_fixture(b"Wing");
    let countries = crate::effect_description::tests::synthetic_countries(count);
    *f.frame.tables[TableId::CountryLinks.index()] = TableView::Complete(TableRows::CountryLinks(
        countries
            .iter()
            .map(|row| CountryLinkRow {
                id: row.id,
                canonical_slot: row.canonical_slot,
            })
            .collect(),
    ));
    *f.frame.tables[TableId::Countries.index()] =
        TableView::Complete(TableRows::Countries(countries));
    let prototype = match &*f.frame.tables[TableId::RaidTooltipOperands.index()] {
        TableView::Complete(TableRows::RaidTooltipOperands(rows)) => rows.clone(),
        _ => unreachable!(),
    };
    *f.frame.tables[TableId::Raids.index()] = TableView::Complete(TableRows::Raids(
        (0..count)
            .map(|index| RaidRow {
                id: (RAID >> 32) as u32 + index,
                reference_type: RAID as u32,
                air_wing: WING + ((index as u64) << 32),
                phase: 4,
                ..Default::default()
            })
            .collect(),
    ));
    *f.frame.tables[TableId::RaidTooltipOperands.index()] =
        TableView::Complete(TableRows::RaidTooltipOperands(
            (0..count)
                .flat_map(|index| {
                    prototype.iter().cloned().map(move |mut row| {
                        row.raid = RAID + ((index as u64) << 32);
                        if row.kind == engine_native::raid_tooltip_operands::REFERENCES
                            && row.key == 1
                        {
                            row.raw = (WING + ((index as u64) << 32)) as i64;
                        }
                        row
                    })
                })
                .collect(),
        ));
    f.with_renderer(&mut fresh(), Content::default(), |renderer| {
        assert_eq!(renderer.unit_name(WING).as_deref(), Ok("Wing"));
        assert_eq!(
            renderer
                .unit_name(WING + (((count - 1) as u64) << 32))
                .as_deref(),
            Ok("Wing")
        );
    });
}
