//! A Map recording holds every table its counters, markers and selections read.

use super::*;
use replay_data::rows::TableId;
use viewer_foundation::map::test_support::{
    assert_reads_recorded, complete_frame, map_scoped, put_sample_campaign,
};

/// Bars are withheld without DivisionStatus; fog is off without its inputs.
const PRESENTED_ABSENT: [TableId; 3] = [
    TableId::DivisionStatus,
    TableId::UnitIntel,
    TableId::CountryVisibility,
];

#[test]
fn map_counters_markers_and_selections_read_only_recorded_tables() {
    engine_executable::executable_tables::initialize_for_tests().unwrap();
    with_counter_inputs_fixture(
        action_gui(false),
        |_| {},
        |_| {},
        |inputs| {
            let mut frame = complete_frame(11, |table| {
                replay_data::rows::sample_extended_table(table, 0)
            });
            put_sample_campaign(&mut frame);
            let frame = map_scoped(frame);
            let links: BTreeMap<u32, u32> = frame
                .complete::<tables::CountryLinks>()
                .unwrap()
                .iter()
                .map(|link| (link.id, link.canonical_slot))
                .collect();
            let tags: BTreeMap<u32, String> = frame
                .complete::<tables::Countries>()
                .unwrap()
                .iter()
                .map(|country| {
                    let tag = String::from_utf8_lossy(&country.tag).into_owned();
                    (country.canonical_slot, tag)
                })
                .collect();
            let fleets: Vec<u64> = frame
                .complete::<tables::Fleets>()
                .unwrap()
                .iter()
                .map(|fleet| fleet.id)
                .collect();
            assert_reads_recorded(&frame, &PRESENTED_ABSENT, || {
                let vision = Vision::read(
                    &frame,
                    &links,
                    &tags,
                    &Choice::default(),
                    Err("synthetic content has no fog layer"),
                )
                .unwrap();
                let viewer = vision.ours();
                let moves = viewer_foundation::map::movement_reads::test_support::unnamed_assets();
                let tables = hour_tables(
                    &frame,
                    inputs.map,
                    Arc::new(vision),
                    inputs.content,
                    inputs.assets,
                    inputs.equipment,
                    inputs.localisation,
                    &Default::default(),
                    None,
                    None,
                    &type_tint::Memo::new(type_tint::TABLES, 1),
                    &MissionMemo::new(1),
                    moves.movement_reads(),
                    Err("unreached synthetic naval topology".into()),
                )
                .unwrap();
                prepare_cached(&inputs, &frame, &tables).unwrap();
                // The sample rows may refuse a selection, but never for a table Map leaves out.
                let scoped = |what: &str, result: std::io::Result<()>| {
                    if let Err(error) = result {
                        assert!(
                            !error.to_string().contains("not recorded"),
                            "{what}: {error}"
                        );
                    }
                };
                scoped(
                    "fleet choices",
                    crate::map::naval_selection::fleet_choices(&frame, viewer).map(drop),
                );
                scoped(
                    "fleet home bases",
                    crate::map::naval_selection::home_bases(&frame, viewer, &fleets).map(drop),
                );
                scoped(
                    "naval mission members",
                    crate::map::naval_selection::mission_members(&frame, viewer, 1, 1).map(drop),
                );
                scoped(
                    "air base target",
                    crate::map::air_selection::base_target(&frame, viewer, 1, 1).map(drop),
                );
                scoped(
                    "air mission members",
                    crate::map::air_selection::mission_members(&frame, viewer, 1, 1).map(drop),
                );
            });
        },
    );
}
