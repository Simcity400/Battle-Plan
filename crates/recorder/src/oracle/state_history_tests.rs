//! History effects are read natively; comparisons use their saved results.

use super::*;

pub(super) fn membership(history: &str, province_count: u32) -> save::MapMembership {
    read_membership(history, province_count).unwrap()
}

fn read_membership(history: &str, province_count: u32) -> io::Result<save::MapMembership> {
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
    ] {
        let path = content.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }
    let layers = installed_content::ResolvedLayers::from_folders(&[(&content.0, &[])]).unwrap();
    save::read_map_membership(&layers)
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
fn an_unported_history_reader_still_refuses_membership_by_name_and_count() {
    let error = read_membership(
        "state={id=1 provinces={1} history={IF={limit={} \
         set_country_flag=first set_country_flag=second}}}",
        2,
    )
    .unwrap_err()
    .to_string();
    assert!(
        error.contains("Deferred effect `set_country_flag`"),
        "{error}"
    );
    assert!(error.contains("count: 2"), "{error}");
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
