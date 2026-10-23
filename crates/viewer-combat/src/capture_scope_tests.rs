//! A Map recording holds every table its battle windows read.

use super::*;
use installed_content_core::{ResolvedLayers, flags::FlagSet, map::MapData};
use replay_data::rows::TableRows;
use replay_data::rows::{TableId, tables};
use viewer_foundation::map::test_support::{
    assert_reads_recorded, complete_frame, map_scoped, put_sample_campaign, test_put,
};
use viewer_foundation::map::{lazy_icons::flag_set, memo::Keyed, perspective::Vision};

/// Bars and division names are withheld without DivisionStatus; fog is off without its inputs.
const PRESENTED_ABSENT: [TableId; 3] = [
    TableId::DivisionStatus,
    TableId::UnitIntel,
    TableId::CountryVisibility,
];

#[test]
fn map_battle_windows_read_only_recorded_tables() {
    let (root, assets, lazy) = assets::tests::synthetic_assets("capture_scope", "", |_| {});
    let layers = Arc::new(ResolvedLayers::from_folders(&[(&root, &[][..])]).unwrap());
    let flags = flag_set(&layers, FlagSet::Normal, [1, 1], |_| {}, &lazy).unwrap();
    let content = StaticContent {
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
        scores: viewer_foundation::land_combat_math::ScoreDefines {
            avoid_with_defence: 90_000,
            avoid_without_defence: 60_000,
            organisation_die: 2,
        },
        fog: Arc::new(Err("synthetic content has no fog layer".into())),
        vision_memo: Arc::new(Keyed::new(
            viewer_foundation::map::perspective::VISION_TABLES,
            1,
        )),
        eta_cache: Default::default(),
        modifier_names: Arc::new(modifier_text::name_index_cache()),
        movement: None,
        window_cache: Arc::new(viewer_foundation::map::native_windows::WindowCache::new(
            Arc::clone(&layers),
        )),
        small_flags: Arc::clone(&flags),
        large_flags: flags,
        lazy,
        trigger_content: Arc::new(
            viewer_foundation::map::movement_reads::test_support::unnamed_assets().trigger_content,
        ),
    };
    let mut frame = complete_frame(11, |table| {
        replay_data::rows::sample_extended_table(table, 0)
    });
    put_sample_campaign(&mut frame);
    // A land combat with committed divisions on each side, so its entries and their hovers bind.
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
            Bindings::read(&content, &frame, selection, &vision, &engaged, true, false)
                .unwrap_or_else(|error| panic!("{selection:?}: {error}"))
        });
        if let Selection::Land(_) = selection {
            assert!(
                bindings.entries[..2].iter().all(|side| !side.is_empty()),
                "both sides' committed entries bind"
            );
            // Division hovers name what they omit in the window's warnings.
            assert!(
                bindings.warnings.iter().any(|warning| warning.contains(
                    "division hover parts are omitted: division history: DivisionHistory not recorded in this recording"
                )),
                "{:?}",
                bindings.warnings
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
