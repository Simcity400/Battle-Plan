use super::*;
use replay_data::rows::*;
use replay_frame::frame::TableView;

/// GER (raw 11) orders template 5 twice over two orders; another country one more.
fn frame() -> DecodedFrame {
    let mut f = dashboard_base::test_fixtures::source(1, 60_000_008, None);
    *f.tables[TableId::DeploymentConveyors.index()] = TableView::Complete(TableRows::DeploymentConveyors(vec![
        DeploymentConveyorRow { handle: 100, country: 11, ordinal: 1, template: 5, amount: 3, priority: 7, location: 4_242, ..Default::default() },
        DeploymentConveyorRow { handle: 101, country: 9001, ordinal: 0, template: 5, amount: 1, closed: 1, ..Default::default() },
        DeploymentConveyorRow { handle: 102, country: 77, ordinal: 0, template: 4, amount: 9, ..Default::default() },
    ]));
    *f.tables[TableId::DeploymentLines.index()] = TableView::Complete(TableRows::DeploymentLines(vec![
        DeploymentLineRow { handle: 200, conveyor: 100, country: 11, ordinal: 1, training_raw: 50_000, max_training_raw: 100_000, max_manpower: 10_000,
            custom_name: b"Second".to_vec(), halting_reason_present: 1, halting_reason: 3, ..Default::default() },
        DeploymentLineRow { handle: 201, conveyor: 100, country: 11, ordinal: 0, training_raw: 0, max_training_raw: 100_000,
            custom_name: b"First".to_vec(), ..Default::default() },
        DeploymentLineRow { handle: 202, conveyor: 102, country: 77, ordinal: 0, ..Default::default() },
    ]));
    *f.tables[TableId::DeploymentManpower.index()] = TableView::Complete(TableRows::DeploymentManpower(vec![
        DeploymentManpowerRow { line: 200, side: 0, ordinal: 0, source_country: 11, amount: i32::MAX },
        DeploymentManpowerRow { line: 200, side: 0, ordinal: 1, source_country: 11, amount: 2 },
        DeploymentManpowerRow { line: 200, side: 1, ordinal: 0, source_country: 11, amount: 7_000 },
    ]));
    *f.tables[TableId::DeploymentEquipment.index()] = TableView::Complete(TableRows::DeploymentEquipment(vec![
        DeploymentEquipmentRow { line: 200, ordinal: 0, variant_handle: 3 << 32 | 70, amount_raw: -50_000 },
    ]));
    f
}

fn reply(frame: &DecodedFrame, tag: &str) -> TrainingQueueReply {
    let need = super::super::division_needs::Needs::read(None, frame);
    reply_with(frame, frame, tag, &need, None)
}

/// The queue of `read` for the country holding `tag` in `playback`.
fn reply_with(
    read: &DecodedFrame,
    playback: &DecodedFrame,
    tag: &str,
    need: &super::super::division_needs::Needs<'_>,
    cat: Option<&Arc<Catalogue>>,
) -> TrainingQueueReply {
    let designs = army_content::Designs::new(None, None);
    let mut refusals = Refusals::default();
    let mut reply = build(read, playback, tag, &Context { cat, designs: &designs, names: None, icon: &|_| Err("no artwork".into()), need },
        &mut refusals, &AtomicBool::new(false)).unwrap();
    reply.warnings = refusals.lines();
    reply
}

/// The countries of `frame` with GER's row changed.
fn with_country(frame: &DecodedFrame, change: impl Fn(&mut CountryRow)) -> DecodedFrame {
    let mut f = frame.clone();
    let TableView::Complete(TableRows::Countries(rows)) = &mut *f.tables[TableId::Countries.index()] else { unreachable!() };
    change(rows.iter_mut().find(|row| row.tag == b"GER").unwrap());
    f
}

#[test]
fn orders_and_their_divisions_keep_the_games_order_and_never_borrow_another_order() {
    let r = reply(&frame(), "GER");
    assert_eq!(r.read_hour, Some(60_000_008));
    assert_eq!(r.orders.iter().map(|o| o.order.as_str()).collect::<Vec<_>>(), ["101", "100"], "queue position, not handle");
    let first = &r.orders[1];
    assert_eq!(first.amount, 3, "an amount ordered never becomes invented divisions");
    assert_eq!(first.divisions.iter().map(|d| d.name.as_deref().unwrap()).collect::<Vec<_>>(), ["First", "Second"]);
    assert!(r.orders[0].divisions.is_empty() && r.orders[0].closed);
}

#[test]
fn training_manpower_and_equipment_keep_exact_signed_values() {
    let r = reply(&frame(), "GER");
    let second = &r.orders[1].divisions[1];
    assert_eq!((second.training_raw.as_str(), second.max_training_raw.as_str()), ("50000", "100000"));
    assert_eq!(second.manpower, i32::MAX.wrapping_add(2), "available entries add as the game adds them");
    assert_eq!((second.manpower_needed, second.max_manpower), (7_000, 10_000));
    assert_eq!(second.contributors.len(), 3);
    assert_eq!(second.equipment[0].amount_raw, "-50000");
    assert!(second.halted, "a stored halting reason is reported as present, never named");
    assert!(second.need.is_empty() && second.need_missing.is_some(), "unknown need is never zero need");
    let first = &r.orders[1].divisions[0];
    assert_eq!((first.manpower, first.manpower_needed), (0, 0));
    assert!(first.contributors.is_empty() && first.equipment.is_empty());
}

#[test]
fn an_unread_queue_and_an_absent_country_are_named() {
    let mut f = frame();
    *f.tables[TableId::DeploymentManpower.index()] = TableView::Unavailable { reason: "slice gap".into() };
    let r = reply(&f, "GER");
    assert!(r.missing.is_some() && r.orders.is_empty());
    assert!(reply(&frame(), "FRA").absent);
}

#[test]
fn priority_and_location_keep_their_stored_values_and_name_what_they_cannot() {
    let r = reply(&frame(), "GER");
    let order = &r.orders[1];
    assert_eq!((order.priority, order.location), (7, 4_242));
    assert!(order.priority_name.is_none() && order.priority_missing.as_deref().is_some_and(|why| why.contains("no proved name")),
        "a priority outside the game's three controls is refused by value");
    assert!(order.location_name.is_none() && order.location_missing.is_some());
    assert_eq!(r.orders[0].priority_missing.as_deref(), Some("Names need the recorded content stack"));
}

const LOCAL_DOCUMENT: &[u8] = br#"
    sub_units = { test_line = { type = infantry need = { widgets = 100 } } }
    equipments = { widgets = { is_archetype = yes type = infantry } }
"#;

/// Template 5 holds one local clone of a line needing 100 widgets, which GER's
/// clone raises to 1.23456 apiece.
fn local_clone_frame() -> DecodedFrame {
    let mut f = frame();
    *f.tables[TableId::LexicalTokens.index()] =
        TableView::Complete(TableRows::LexicalTokens(vec![
            LexicalTokenRow {
                token: 1,
                literal: b"test_line".to_vec(),
            },
            LexicalTokenRow {
                token: 2,
                literal: b"widgets".to_vec(),
            },
        ]));
    *f.tables[TableId::DivisionTemplateIcons.index()] =
        TableView::Complete(TableRows::DivisionTemplateIcons(vec![
            DivisionTemplateIconRow {
                template: 5,
                country: 9001,
                subunits: vec![TemplateSubunit {
                    token: 1,
                    local: 1,
                    count: 2,
                }],
                ..Default::default()
            },
        ]));
    *f.tables[TableId::CountryLocalSubunits.index()] =
        TableView::Complete(TableRows::CountryLocalSubunits(vec![
            CountryLocalSubunitRow {
                country: 11,
                token: 1,
                requirements: vec![LocalSubunitRequirement {
                    archetype_token: 2,
                    amount_raw: 123_456,
                }],
            },
        ]));
    f
}

#[test]
fn the_training_queue_uses_canonical_template_needs_with_recorded_local_clones() {
    let f = local_clone_frame();
    let needs = super::super::division_needs::Needs::from_documents(&[LOCAL_DOCUMENT], &f);
    let r = reply_with(&f, &f, "GER", &needs, None);
    let need = &r.orders[1].divisions[1].need;
    assert_eq!(
        (
            need.len(),
            need[0].archetype.as_str(),
            need[0].need_raw.as_str()
        ),
        (1, "widgets", "246912")
    );
    assert!(r.orders[1].divisions[1].need_missing.is_none());
}

#[test]
fn a_held_design_of_unresolved_type_makes_the_comparison_unknown_not_empty() {
    let f = local_clone_frame();
    let needs = super::super::division_needs::Needs::from_documents(&[LOCAL_DOCUMENT], &f);
    let r = reply_with(&f, &f, "GER", &needs, None);
    let division = &r.orders[1].divisions[1];
    assert!(division.equipment[0].archetype.is_none(), "no design definitions were read");
    assert!(division.comparison_missing.is_some(), "what the unresolved design fills is unknown, never 0%");
    assert_eq!((division.need.len(), division.need[0].need_raw.as_str(), division.need[0].held_raw.as_str()), (1, "246912", "0"),
        "the known need and known holdings stay");
    let empty = &r.orders[1].divisions[0];
    assert!(empty.comparison_missing.is_none() && empty.equipment.is_empty(), "a division holding nothing compares as known");
}

#[test]
fn the_queue_follows_the_country_holding_the_tag_at_the_hour() {
    let read = frame();
    // Renamed since the read: the same country under its new tag keeps its queue.
    let renamed = with_country(&read, |row| row.tag = b"DEU".to_vec());
    let need = super::super::division_needs::Needs::read(None, &read);
    let r = reply_with(&read, &renamed, "DEU", &need, None);
    assert!(!r.absent && r.missing.is_none());
    assert_eq!(r.orders.len(), 2);
    assert!(reply_with(&read, &renamed, "GER", &need, None).absent, "the old tag names no country at the hour");
    // Reused since the read: the queue read when another country held the slot is never lent.
    let before = with_country(&read, |row| row.id = 4_242);
    let r = reply_with(&before, &read, "GER", &need, None);
    assert!(r.orders.is_empty() && !r.absent);
    assert!(r.missing.as_deref().is_some_and(|why| why.contains("another country identity")), "{:?}", r.missing);
}

/// The queue's read carries every operand the scripted text interpreter reads, so a
/// priority button's `[ROOT.GetName]` resolves from the recorded naming rows.
#[test]
fn the_queue_read_carries_the_operands_of_scripted_priority_names() {
    let tables = read_tables();
    for table in QUEUE.iter().chain(&NAMING).chain(dashboard_text::scripted_text::TABLES) {
        assert!(tables.contains(table), "{table:?}");
    }
    for table in [TableId::CountryAppearance, TableId::Politics, TableId::PoliticalParties, TableId::Autonomy] {
        assert!(tables.contains(&table), "{table:?} names a country");
    }
}

#[test]
fn priority_names_are_the_effective_interface_tooltips_of_the_priority_buttons() {
    // The native GUI reader needs the executable's metadata.
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let (cat, _folder) = dashboard_text::catalogue::tests::folder_catalogue(&[
        (
            "interface/countrydeploymentview.gui",
            "guiTypes = { containerWindowType = { name = \"military_deployment_conveyor_view\"
                buttonType = { name = \"low_prio\" pdx_tooltip = \"MOD_LOW_PRIORITY\" }
                buttonType = { name = \"medium_prio\" pdx_tooltip = \"MOD_MEDIUM_PRIORITY\" }
                buttonType = { name = \"high_prio\" pdx_tooltip = \"MOD_HIGH_PRIORITY\" }
            } }",
        ),
        (
            "localisation/english/test_l_english.yml",
            "\u{feff}l_english:\n MOD_LOW_PRIORITY:0 \"Rebound low\"\n DEPLOYMENT_LOW_PRIORITY_DESC:0 \"Installed low\"\n MOD_HIGH_PRIORITY:0 \"[ROOT.NoSuchScopedFunction] high\"\n",
        ),
    ]);
    let cat = Arc::new(cat);
    let mut refusals = Refusals::default();
    let read = frame();
    let priorities = army_content::DeploymentPriorities::read(Some(&cat), &read, 7, &mut refusals);
    assert_eq!(priorities.name(0).as_deref(), Ok("Rebound low"), "a mod's rebinding, not the installed key");
    assert!(priorities.name(1).is_err_and(|why| why.contains("MOD_MEDIUM_PRIORITY")), "a rebound key without text is refused by name");
    assert!(priorities.name(2).is_err_and(|why| why.contains("refused")),
        "a scoped expression the interpreter cannot evaluate is refused, never shown as written");
    assert!(priorities.name(3).is_err_and(|why| why.contains("no proved name")));
    let need = super::super::division_needs::Needs::read(None, &read);
    let r = reply_with(&read, &read, "GER", &need, Some(&cat));
    assert_eq!(r.orders[0].priority_name.as_deref(), Some("Rebound low"), "each order takes its stored priority's button");
}

#[test]
fn a_queue_left_to_the_native_capture_is_read_from_its_recording_never_refused_as_unrecorded() {
    let mut cut = frame();
    // Full leaves DivisionModelInputs out, so a frame in its mode marks it so.
    for table in [TableId::DeploymentLines, TableId::DivisionModelInputs] {
        *cut.tables[table.index()] = TableView::NotRecorded { mode: replay_data::scope::CaptureMode::Full };
    }
    let (source, root) = dashboard_base::test_fixtures::file_source(dashboard_base::test_fixtures::replay(vec![cut]));
    let error = TrainingQueue::load(
        &source,
        &TrainingQueueParams { branch_id: "1".into(), hour: 60_000_008, tag: "GER".into() },
        &LoadContext { content: None },
        &AtomicBool::new(false),
    )
    .unwrap_err()
    .to_string();
    drop(source);
    std::fs::remove_dir_all(root).ok();
    assert_eq!(
        error,
        "session 1 is still being recorded: its hours are read once its native recording lands"
    );
}
