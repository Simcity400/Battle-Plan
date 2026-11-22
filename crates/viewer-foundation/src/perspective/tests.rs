use super::*;
use crate::visibility::tests::fixture;
use replay_data::{
    rows::CountryIntelRow, rows::CountryVisibilityRow, rows::DivisionRow, rows::FactionMemberRow,
    rows::TableId, rows::UnitIntelRow, rows::WarRelationRow,
};
use replay_frame::frame::TableView;

fn set<T: replay_data::rows::TableSchema>(frame: &mut DecodedFrame, rows: Vec<T::Row>) {
    *frame.tables[T::ID.index()] = TableView::Complete(T::wrap(rows));
}

fn hour() -> (
    DecodedFrame,
    BTreeMap<u32, u32>,
    BTreeMap<u32, String>,
    FogContent,
) {
    let (mut frame, links, content) = fixture();
    set::<tables::LocalPlayer>(
        &mut frame,
        vec![replay_data::rows::LocalPlayerRow {
            country: 101,
            observer_country: 0,
        }],
    );
    set::<tables::Divisions>(
        &mut frame,
        vec![DivisionRow {
            id: 7,
            roster_owner: 102,
            logical_owner: 102,
            province: 2,
            ..Default::default()
        }],
    );
    let tags = [(1, "GER"), (2, "ENG"), (3, "SOV")]
        .into_iter()
        .map(|(slot, tag)| (slot, tag.to_owned()))
        .collect();
    (frame, links, tags, content)
}

fn view(
    frame: &DecodedFrame,
    links: &BTreeMap<u32, u32>,
    tags: &BTreeMap<u32, String>,
    content: &FogContent,
    country: Country,
    fog: bool,
) -> Vision {
    Vision::read(frame, links, tags, &Choice { country, fog }, Ok(content)).unwrap()
}

#[test]
fn the_vision_tables_are_all_a_vision_reads() {
    let (frame, links, tags, content) = hour();
    let projected = viewer_runtime::memo::project(&frame, VISION_TABLES);
    for (country, fog) in [
        (Country::Recorded, true),
        (Country::Tag("ENG".into()), true),
        (Country::Tag("SOV".into()), false),
    ] {
        assert_eq!(
            format!(
                "{:?}",
                view(&frame, &links, &tags, &content, country.clone(), fog)
            ),
            format!(
                "{:?}",
                view(&projected, &links, &tags, &content, country, fog)
            ),
        );
    }
}

#[test]
fn any_country_and_either_fog_setting_rebuild_from_the_same_hour() {
    let (frame, links, tags, content) = hour();
    let german = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(
        (german.ours(), german.resolution()),
        (1, &Resolution::Recorded)
    );
    assert_eq!(german.province_byte(2), Some(150));
    assert_eq!(german.sees_land(7, 2, 2), Sight::Partial);
    let english = view(
        &frame,
        &links,
        &tags,
        &content,
        Country::Tag("ENG".into()),
        true,
    );
    assert_eq!(english.sees_land(7, 2, 2), Sight::Full);
    let soviet = view(
        &frame,
        &links,
        &tags,
        &content,
        Country::Tag("SOV".into()),
        true,
    );
    assert_eq!(soviet.province_byte(2), Some(0));
    assert_eq!(soviet.sees_land(7, 2, 2), Sight::None);
    let clear = view(
        &frame,
        &links,
        &tags,
        &content,
        Country::Tag("SOV".into()),
        false,
    );
    assert_eq!(clear.province_fog(), None);
    assert_eq!(clear.province_byte(2), Some(255));
    assert_eq!(clear.sees_land(7, 2, 2), Sight::Full);
    let absent = view(
        &frame,
        &links,
        &tags,
        &content,
        Country::Tag("XXX".into()),
        true,
    );
    assert_eq!(absent.resolution(), &Resolution::Absent("XXX".into()));
    let by_slot = view(&frame, &links, &tags, &content, Country::Slot { slot: 2, tag: "OLD".into() }, true);
    assert_eq!((by_slot.ours(), by_slot.resolution()), (2, &Resolution::Requested));
    assert_eq!(by_slot.sees_land(7, 2, 2), english.sees_land(7, 2, 2));
    let gone = view(&frame, &links, &tags, &content, Country::Slot { slot: 9, tag: "D01".into() }, true);
    assert_eq!((gone.ours(), gone.resolution()), (1, &Resolution::Absent("D01".into())));
}

#[test]
fn one_missing_input_is_scoped_to_what_reads_it() {
    let (mut frame, links, tags, content) = hour();
    *frame.tables[TableId::RadarCoverage.index()] = TableView::Unavailable {
        reason: "read fault".into(),
    };
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.visibility_gap(), Some("RadarCoverage"));
    assert_eq!(vision.province_fog(), None);
    assert_eq!(vision.undecided(), None, "unit sight keeps its own inputs");
    assert_eq!(vision.sees_land(7, 2, 2), Sight::Full);
    let unloaded = Vision::read(
        &frame,
        &links,
        &tags,
        &Choice::default(),
        Err("visibility content unavailable"),
    )
    .unwrap();
    assert_eq!(
        unloaded.visibility_gap(),
        Some("visibility content unavailable")
    );
    *frame.tables[TableId::FactionMembers.index()] = TableView::Unavailable {
        reason: "read fault".into(),
    };
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.undecided(), Some("FactionMembers"));
    assert_eq!(vision.sees_land(7, 2, 2), Sight::Full);
}

#[test]
fn detection_takes_the_first_entry_and_the_viewer_modifier_scales_it() {
    let (mut frame, links, tags, content) = hour();
    set::<tables::UnitIntel>(
        &mut frame,
        [(1, 3), (0, 0)]
            .into_iter()
            .map(|(ordinal, level)| UnitIntelRow {
                unit: 9,
                ordinal,
                viewer: 1,
                level,
            })
            .collect(),
    );
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(
        vision.sees_navy(9, 3),
        Sight::None,
        "first entry is level 0"
    );
    set::<tables::CountryVisibility>(
        &mut frame,
        vec![CountryVisibilityRow {
            country: 101,
            unit_modifier: -50_000,
        }],
    );
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.sees_land(7, 2, 2), Sight::None, "150 halves to 75");
}

#[test]
fn a_truncated_intel_census_leaves_an_unlisted_foreign_task_force_unknown() {
    use replay_data::rows::TableSchema;
    let (mut frame, links, tags, content) = hour();
    *frame.tables[tables::UnitIntel::ID.index()] = TableView::Truncated {
        rows: tables::UnitIntel::wrap(vec![UnitIntelRow {
            unit: 9,
            ordinal: 0,
            viewer: 1,
            level: 3,
        }]),
        missing: 1,
    };
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.navy_byte_known(9, 3), Some(vision.navy_byte(9, 3)));
    assert_eq!(vision.navy_byte_known(8, 3), None);
    assert_eq!(vision.navy_byte(8, 3), 255);
    assert_eq!(vision.sees_land(8, 3, 2), Sight::Full);
    assert_eq!(vision.navy_byte_known(8, 1), Some(255));
    set::<tables::UnitIntel>(&mut frame, Vec::new());
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.navy_byte_known(8, 3), Some(vision.navy_byte(8, 3)));
}

#[test]
fn a_war_breaks_shared_vision_with_a_faction_partner() {
    let (mut frame, links, tags, content) = hour();
    set::<tables::FactionMembers>(
        &mut frame,
        [101, 102]
            .into_iter()
            .map(|country| FactionMemberRow {
                country,
                faction: 4,
                ordinal: 0,
            })
            .collect(),
    );
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.shares_vision_proven(2), Ok(true));
    assert_eq!(vision.sees_combat([Some(2)].into_iter(), None), Ok(true));
    set::<tables::WarRelations>(
        &mut frame,
        vec![WarRelationRow {
            first_country: 102,
            second_country: 101,
            ..Default::default()
        }],
    );
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.shares_vision_proven(2), Ok(false));
}

#[test]
fn air_base_icons_need_access_intel_or_a_clear_byte() {
    let (mut frame, links, tags, content) = hour();
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.sees_air_base(3, 3), Some(false));
    set::<tables::RelationLinks>(
        &mut frame,
        vec![RelationLinkRow {
            country: 101,
            target_index: 3,
            links: RelationLinkRow::AIR_BASE_ACCESS,
            transport_subject: Default::default(),
        }],
    );
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.sees_air_base(3, 3), Some(true));
    set::<tables::RelationLinks>(&mut frame, vec![]);
    set::<tables::CountryIntel>(
        &mut frame,
        vec![CountryIntelRow {
            owner_country: 101,
            target_index: 3,
            air: 3_000_000,
            ..Default::default()
        }],
    );
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.sees_air_base(3, 3), Some(true));
    assert_eq!(vision.sees_air_port_detail(3), Some(true));
    assert_eq!(vision.air_port_level(3), Some(1));
    *frame.tables[TableId::RelationLinks.index()] = TableView::Unavailable {
        reason: "read fault".into(),
    };
    set::<tables::CountryIntel>(&mut frame, vec![]);
    let vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    assert_eq!(vision.sees_air_base(3, 3), None);
    assert_eq!(vision.province_fog(), None, "attaches feed the fog rebuild");
}

#[test]
fn carrier_country_intel_uses_fog_locally_and_keeps_relation_failures_named() {
    let mut vision = Vision::sharing(1, [2]);
    vision.relations_gap = Some("FactionMembers");
    assert_eq!(vision.shares_vision_or_fog(1), Ok(true));
    assert_eq!(vision.shares_vision_or_fog(2), Ok(true));
    assert_eq!(
        vision.shares_vision_or_fog(3),
        Err(SharingUnknown::Relations("FactionMembers"))
    );
    vision.fog = false;
    assert_eq!(vision.shares_vision_or_fog(3), Ok(true));
    assert_eq!(
        vision.shares_vision_proven(3),
        Err(SharingUnknown::Relations("FactionMembers")),
        "the local fog setting does not change ordinary shared vision"
    );
}

#[test]
fn province_objects_hide_below_fifty_unless_always_shown() {
    let (frame, links, tags, content) = hour();
    let mut vision = view(&frame, &links, &tags, &content, Country::Recorded, true);
    vision.set_visibility([(7, 49), (8, 50)]);
    assert_eq!(vision.sees_province_object(7, false), Some(false));
    assert_eq!(vision.sees_province_object(8, false), Some(true));
    assert_eq!(vision.sees_province_object(7, true), Some(true));
    assert_eq!(vision.sees_province_object(999, false), None);
}

#[test]
fn the_choice_parses_and_prints_the_words_the_cli_takes() {
    let gone = Country::parse("omniscient").unwrap_err().to_string();
    assert!(gone.contains("--fog off"), "{gone}");
    assert_eq!(Country::parse(" ger ").unwrap(), Country::Tag("GER".into()));
    assert!(Country::parse("  ").is_err());
    assert!(!Choice::parse_fog("off").unwrap());
    assert!(Choice::parse_fog("maybe").is_err());
    assert_eq!(
        Choice {
            country: Country::Recorded,
            fog: false
        }
        .to_string(),
        "recorded, fog of war off"
    );
}
