
use super::*;
use content_reader_layers::ResolvedLayers;
use content_reader_gui::flags::FlagSet;
use content_reader_map::map::MapData;
use replay_data::rows::TableRows;
use replay_data::rows::{TableId, tables};
use viewer_foundation::test_support::{
    assert_reads_recorded,
    complete_frame,
    map_scoped,
    put_sample_campaign,
    test_put,
};
use viewer_native_gui::lazy_icons::flag_set;
use viewer_runtime::memo::Keyed;
use viewer_foundation::perspective::Vision;

const PRESENTED_ABSENT: [TableId; 4] = [
    TableId::DivisionStatus,
    TableId::DivisionPanelStats,
    TableId::UnitIntel,
    TableId::CountryVisibility,
];

#[test]
fn map_battle_windows_read_only_recorded_tables() {
    let (root, assets, lazy) = assets::tests::synthetic_assets("capture_scope", "", |_| {});
    let layers = Arc::new(ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap());
    let flags = flag_set(&layers, FlagSet::Normal, [1, 1], |_| {}, &lazy).unwrap();
    let content = StaticContent {
        air_context: None,
        attrition: Default::default(),
        medals: Default::default(),
        assets: Arc::new(assets),
        localisation: Default::default(),
        recorded_language: "english".into(),
        recorded_labels: Arc::new(naval_labels::RecordedLabels::from_localisation(
            &Default::default(),
        )),
        scripted: Arc::new(Err("synthetic content has no scripted text".into())),
        map: Arc::new(MapData {
            width: 4,
            height: 1,
            province_ids: vec![1, 2, 3, 4],
            definitions: Default::default(),
            country_colors: Default::default(),
        }),
        strategic_regions: Default::default(),
        battle_name_provinces: Default::default(),
        state_name_keys: Default::default(),
        province_geometry: Default::default(),
        region_of: Default::default(),
        terrain: Default::default(),
        scores: viewer_engagements::land_combat_math::ScoreDefines {
            avoid_with_defence: 90_000,
            avoid_without_defence: 60_000,
            organisation_die: 2,
        },
        fog: Arc::new(Err("synthetic content has no fog layer".into())),
        vision_memo: Arc::new(Keyed::new(
            viewer_foundation::perspective::VISION_TABLES,
        )),
        eta_cache: Default::default(),
        modifier_names: Arc::new(modifier_text::name_index_cache()),
        movement: None,
        window_cache: Arc::new(viewer_native_gui::native_windows::WindowCache::new(
            Arc::clone(&layers),
        )),
        small_flags: Arc::clone(&flags),
        large_flags: flags,
        lazy,
        trigger_content: Arc::new(
            viewer_unit_reads::movement_reads::test_support::unnamed_assets().trigger_content,
        ),
        occupation: Arc::new(Err("not loaded".into())),
    };
    let mut frame = complete_frame(11, |table| {
        replay_data::rows::sample_extended_table(table, 0)
    });
    put_sample_campaign(&mut frame);
    let mut details = frame
        .complete::<tables::LandCombatDetails>()
        .unwrap()
        .to_vec();
    let combat = details[0].combat;
    let mut divisions = frame
        .complete::<tables::LandCombatDivisions>()
        .unwrap()
        .to_vec();
    for (at, row) in divisions.iter_mut().enumerate() {
        row.combat = combat;
        row.side = (at % 2) as u8;
        row.ordinal = at as u32;
    }
    divisions.sort_by_key(|row| (row.combat, row.side, row.ordinal));
    for (side, details) in details[0].sides.iter_mut().enumerate() {
        details.committed = divisions
            .iter()
            .filter(|row| usize::from(row.side) == side)
            .map(|row| row.division)
            .collect();
    }
    test_put(&mut frame, TableRows::LandCombatDivisions(divisions));
    test_put(&mut frame, TableRows::LandCombatDetails(details));
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
    for selection in [
        Selection::Land(combat),
        Selection::Naval(200),
        Selection::NavalResult(300),
    ] {
        let bindings = assert_reads_recorded(&frame, &PRESENTED_ABSENT, || {
            let vision =
                Vision::read(&frame, &links, &tags, &Choice::default(), Err("no fog")).unwrap();
            let engaged = combat_flags(&frame, vision.ours(), None).unwrap();
            Bindings::read(&content, &frame, selection, &vision, &engaged, true, false, &Err("no dominance sources in this test".into()), None)
                .unwrap_or_else(|error| panic!("{selection:?}: {error}"))
        });
        if let Selection::Land(_) = selection {
            assert!(
                bindings.entries[..2].iter().all(|side| !side.is_empty()),
                "both sides' committed entries bind"
            );
            assert!(
                bindings.warnings.iter().any(|warning| warning
                    .contains("has no maximum HP in DivisionPanelStats")),
                "{:?}",
                bindings.warnings
            );
            assert!(
                bindings
                    .warnings
                    .iter()
                    .all(|warning| !warning.contains("division history:")),
                "{:?}",
                bindings.warnings
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
