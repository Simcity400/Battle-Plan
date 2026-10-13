//! History effects are read natively; comparisons use their saved results.

use super::*;

pub(super) fn membership(history: &str, province_count: u32) -> save::PartialMapMembership {
    read_membership(history, province_count).unwrap()
}

fn read_membership(history: &str, province_count: u32) -> io::Result<save::PartialMapMembership> {
    with_content(history, province_count, save::read_partial_map_membership)
}

fn with_content<T>(
    history: &str,
    province_count: u32,
    read: impl FnOnce(&installed_content::ResolvedLayers) -> io::Result<T>,
) -> io::Result<T> {
    with_content_files(history, province_count, &[], read)
}

fn with_content_files<T>(
    history: &str,
    province_count: u32,
    extra_files: &[(&str, &str)],
    read: impl FnOnce(&installed_content::ResolvedLayers) -> io::Result<T>,
) -> io::Result<T> {
    engine::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    struct Content(std::path::PathBuf);
    impl Drop for Content {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let content = Content(std::env::temp_dir().join(format!(
        "hoi4_oracle_state_history_{}_{:?}",
        std::process::id(),
        std::thread::current().id()
    )));
    let definitions = (0..=province_count)
        .map(|id| format!("{id};{id};0;0;land;false;test\n"))
        .collect::<String>();
    for (path, bytes) in [
        (
            "map/default.map",
            "definitions=\"definition.csv\" provinces=\"unused.bmp\" terrain=\"unused.bmp\" \
             rivers=\"unused.bmp\" heightmap=\"unused.bmp\" tree_definition=\"unused.csv\"",
        ),
        ("map/definition.csv", definitions.as_str()),
        (
            "common/defines/00.lua",
            "NDefines={NBuildings={MAX_BUILDING_LEVELS=20}}",
        ),
        (
            "common/buildings/00.txt",
            "buildings={civ={general_production=1}}",
        ),
        ("history/states/test.txt", history),
        ("common/resources/00.txt", "resources={synthetic={}}"),
    ]
    .into_iter()
    .chain(extra_files.iter().copied())
    {
        let path = content.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let layers = installed_content::ResolvedLayers::from_folders(&[(&content.0, &[])]).unwrap();
    read(&layers)
}

#[test]
fn file_notes_without_kept_states_leave_partial_coverage_complete() {
    let membership = with_content_files(
        "state={id=1 provinces={1}}",
        2,
        &[
            ("history/states/z-empty.txt", "@a bad orphan = yes"),
            (
                "history/states/z-duplicate.txt",
                "@a bad state={id=1 provinces={2}}",
            ),
        ],
        save::read_partial_map_membership,
    )
    .unwrap();
    assert!(membership.excluded_states.is_empty());
    assert_eq!(membership.notes.len(), 3);
    let (frame, saved, _, equipment) = super::tests::fixture();
    let report = compare_core(
        &frame,
        SampleBoundary::EndOfHour,
        &saved,
        Ok(&membership),
        Ok(&equipment),
        None,
    )
    .unwrap();
    assert!(report.coverage_line().contains("coverage: complete"));
    assert!(report.outcome().is_ok());
    assert_eq!(report.limitations["state-history reader notes"].len(), 3);
}

#[test]
fn core_reports_missing_rows_even_when_their_membership_is_excluded() {
    let membership = membership(
        "state={id=1 provinces={1}} \
         state={id=2 provinces={2} history={free_operative={operative=a}}}",
        2,
    );
    let (mut frame, mut saved, _, _) = super::tests::fixture();
    let mut state = saved.states[&1].clone();
    state.id = 2;
    saved.states.insert(2, state);
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    {
        rows.retain(|row| row.id != 2);
    }
    let context = country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_provinces(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    compare_states(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
    assert!(
        report
            .differences
            .iter()
            .any(|row| row.contains("province 2"))
    );
    assert!(report.differences.iter().any(|row| row.contains("state 2")));
}

#[test]
fn base_resources_skip_excluded_states_and_compare_clean_declarations() {
    let (membership, content) = with_content(
        "state={id=2 provinces={2} resources={synthetic=3}} \
         state={id=1 provinces={1} resources={synthetic=4} history={free_operative={operative=a}}}",
        2,
        save::read_partial_state_history,
    )
    .unwrap();
    let content = content.unwrap();
    let rows = [
        replay::rows::StateBaseResourceRow {
            state: 1,
            resource: 91,
            ordinal: 0,
            amount: 999,
        },
        replay::rows::StateBaseResourceRow {
            state: 2,
            resource: 91,
            ordinal: 0,
            amount: 400_000,
        },
    ];
    let mut report = Report::default();
    construction::partial_base_resources(
        &rows,
        &content,
        &membership,
        &BTreeMap::from([(91, b"synthetic".as_slice())]),
        &mut report,
    )
    .unwrap();
    assert_eq!(
        report.compared_tables["StateBaseResources (verified content)"],
        1
    );
    assert_eq!(report.membership_skips["StateBaseResources"], 1);
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("state base resource.2.synthetic.amount"));
    assert!(report.coverage_line().contains("incomplete"));
}

#[test]
fn a_previously_deferred_if_without_map_effect_runs_the_dependent_comparisons() {
    let membership = membership(
        "state={id=1 provinces={1} history={owner=OTHER \
         1936.1.1={IF={limit={} log=authored_message}}}}",
        2,
    );
    let (frame, saved, _, equipment) = super::tests::fixture();
    let report = compare_core(
        &frame,
        SampleBoundary::EndOfHour,
        &saved,
        Ok(&membership),
        Ok(&equipment),
        None,
    )
    .unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert!(
        !report
            .failed_comparisons
            .contains_key("content map membership")
    );
    assert_eq!((report.provinces, report.states), (2, 1));
}

#[test]
fn an_unported_history_reader_excludes_its_state_and_later_states_by_name_and_count() {
    let membership = membership(
        "state={id=1 provinces={1} history={IF={limit={} \
         free_operative={operative=a} free_operative={operative=b}}}} \
         state={id=2 provinces={2}}",
        2,
    );
    let excluded = &membership.excluded_states[&1];
    assert_eq!(excluded.file, "history/states/test.txt");
    let reason = excluded.reasons.join("; ");
    assert!(
        reason.contains("Deferred effect `free_operative`"),
        "{reason}"
    );
    assert!(reason.contains("count: 2"), "{reason}");
    assert!(membership.province_is_unknown(1));
    assert_eq!(
        membership.provinces[&2],
        save::ProvinceMembership::Unknown(BTreeSet::from([2]))
    );
    let later = &membership.excluded_states[&2];
    assert_eq!(later.file, excluded.file);
    assert_eq!(
        later.reasons,
        [format!(
            "after state 1's unported reader: {}",
            excluded.reasons[0]
        )]
    );
}

#[test]
fn core_skips_only_membership_fields_and_keeps_saved_value_differences() {
    let membership = membership(
        "state={id=1 provinces={1}} \
         state={id=2 provinces={2} history={free_operative={operative=a}}}",
        2,
    );
    let (mut frame, mut saved, _, equipment) = super::tests::fixture();
    let mut state = saved.states[&1].clone();
    state.id = 2;
    state.manpower_available = Some(42);
    saved.states.insert(2, state);
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    {
        rows[0].victory_points = 7;
        rows[1].state = 999;
        rows[1].owner = u32::MAX;
        rows[1].controller = 0;
        rows[1].victory_points = 99;
    }
    if let TableView::Complete(TableRows::States(rows)) =
        &mut *frame.tables[TableId::States.index()]
    {
        rows[0].custom_name = b"clean difference".to_vec();
        let mut row = rows[0].clone();
        row.id = 2;
        row.owner = 0;
        row.controller = 0;
        row.manpower_available = 43;
        row.custom_name = b"excluded difference".to_vec();
        rows.push(row);
    }
    let context = country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_provinces(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    compare_states(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!((report.provinces, report.states), (2, 2));
    assert_eq!(report.compared_tables["Provinces.victory_points"], 2);
    assert_eq!(report.membership_skips["Provinces"], 1);
    assert!(!report.membership_skips.contains_key("States"));
    assert_eq!(report.differences.len(), 8, "{:?}", report.differences);
    for field in [
        "province 1.victory_points",
        "province 2.victory_points",
        "province 2.controller",
        "state 1.custom_name",
        "state 2.custom_name",
        "state 2.owner",
        "state 2.controller",
        "state 2.manpower_available",
    ] {
        assert!(report.differences.iter().any(|row| row.contains(field)));
    }
    let lines = report
        .limitations
        .values()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    for text in [
        "Provinces: 1 rows with membership-dependent comparisons skipped",
        "state 2",
        "history/states/test.txt",
        "free_operative",
    ] {
        assert!(lines.contains(text), "{lines}");
    }
    assert!(report.coverage_line().contains("incomplete"));

    // The exclusion itself makes coverage incomplete, even without differences
    // or a dependent row present in the recording.
    let mut empty_report = Report::default();
    empty_report.membership_content(&membership);
    assert!(empty_report.coverage_line().contains("incomplete"));
    assert!(empty_report.outcome().is_err());
    let report = compare_core(
        &frame,
        SampleBoundary::EndOfHour,
        &saved,
        Ok(&membership),
        Ok(&equipment),
        None,
    )
    .unwrap();
    assert!(
        !report
            .failed_comparisons
            .contains_key("content map membership")
    );
    assert!(report.coverage_line().contains("incomplete"));

    // An omitted controller still depends on the excluded state's owner.
    saved.provinces.get_mut(&2).unwrap().controller = None;
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    {
        rows[1].controller = u32::MAX;
    }
    let context = country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_provinces(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 2, "{:?}", report.differences);
}

#[test]
fn an_unattributed_diagnostic_excludes_every_state_in_its_file() {
    let membership = membership(
        "state={id=1 provinces={1}} orphan tail state={id=2 provinces={2}}",
        2,
    );
    assert_eq!(membership.excluded_states.len(), 2);
    assert!(membership.province_is_unknown(1));
    assert!(membership.province_is_unknown(2));
}

#[test]
fn an_unreadable_tail_still_refuses_the_whole_membership_inventory() {
    let error = read_membership(
        "state={id=1 provinces={1}} state={id=2 provinces={2} history={free_operative={x @y=1}}} state={id=3 provinces={3}}",
        3,
    ).unwrap_err().to_string();
    assert!(error.contains("inventory"), "{error}");
    assert!(error.contains("history/states/test.txt"), "{error}");
}

#[test]
fn add_victory_points_compares_the_saved_result_and_keeps_a_difference() {
    let membership = membership(
        "state={id=1 provinces={1} history={victory_points={1 2} \
         1936.1.1={IF={limit={} add_victory_points={province=1 value=3}}}}}",
        2,
    );
    let (mut frame, mut saved, _, _) = super::tests::fixture();
    saved.provinces.insert(
        1,
        save::ProvinceOverride {
            id: 1,
            name: None,
            owner: None,
            controller: None,
            victory_points: Some(5),
        },
    );
    if let TableView::Complete(TableRows::Provinces(rows)) =
        &mut *frame.tables[TableId::Provinces.index()]
    {
        rows[0].victory_points = 5;
    }
    let context = country_context(&frame).unwrap();
    let mut report = Report::default();
    compare_provinces(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert!(report.differences.is_empty(), "{:?}", report.differences);
    assert_eq!(report.compared_tables["Provinces.victory_points"], 2);
    saved.provinces.get_mut(&1).unwrap().victory_points = Some(4);
    let mut report = Report::default();
    compare_provinces(&frame, &saved, &membership, &context.links, &mut report).unwrap();
    assert_eq!(report.differences.len(), 1);
    assert!(report.differences[0].contains("province 1.victory_points"));
}
