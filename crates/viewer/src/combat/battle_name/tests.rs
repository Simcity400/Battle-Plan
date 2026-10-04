use super::*;
use crate::combat::binding::NoIcons;

#[test]
fn the_native_root_gives_fixed5_pixel_distances() {
    assert_eq!(integer_root(0), 0);
    // 1424DAE00 stops once the square is within a hundredth of the input
    // (value * 1000 / 100000), so a root sits near, not at, the true one.
    let close = |value: i64, expected: i64| (value - expected).abs() * 10_000 <= expected;
    assert!(close(integer_root(25), 500_000), "{}", integer_root(25));
    assert!(close(integer_root(1), 100_000), "{}", integer_root(1));
    // x 95 and 5 on a 100-wide map are 10 apart across the wrap.
    let wrapped = wrapped_distance(100, (1, [95, 0]), (2, [5, 4]));
    assert!(close(wrapped, 1_077_033), "{wrapped}");
    let plain = wrapped_distance(100, (1, [0, 0]), (2, [3, 4]));
    assert!(close(plain, 500_000), "{plain}");
    assert_eq!(wrapped_distance(100, (7, [0, 0]), (7, [3, 4])), 0);
    // Past 2^21 the input is shifted ten bits and the root five.
    let large = integer_root(4_000_000);
    assert!((199_900_000..=200_100_000).contains(&large), "{large}");
}

#[test]
fn the_first_candidate_is_taken_and_only_a_greater_score_replaces_it() {
    let positions = BTreeMap::from([(1, [0, 0]), (10, [3, 4]), (11, [6, 8]), (12, [0, 1])]);
    let points = BTreeMap::from([(10, 1), (11, 1), (12, 0)]);
    let position = |id: u32| positions.get(&id).copied().unwrap_or([0, 0]);
    let victory_points = |id: u32| {
        points
            .get(&id)
            .copied()
            .ok_or_else(|| format!("province {id} has no Provinces row"))
    };
    let picker = Picker {
        width: 1_000,
        factor: 10_000_000,
        cutoff: 100_000,
        position: &position,
        victory_points: &victory_points,
    };
    // 1 VP at distance 5 scores 20.0; at 10 it scores 10.0.
    assert_eq!(picker.pick(1, &[11, 10]), Ok(Some(10)));
    // No victory points score zero, below the cutoff: the region's name.
    assert_eq!(picker.pick(1, &[12]), Ok(None));
    assert_eq!(picker.pick(1, &[]), Ok(None));
    // The battle province itself is at distance zero and scores 0xFFFFFFFF.
    let own = BTreeMap::from([(1, 0)]);
    let own_points = |id: u32| Ok(own.get(&id).copied().unwrap_or(1));
    let picker_own = Picker {
        victory_points: &own_points,
        ..picker
    };
    assert_eq!(picker_own.pick(1, &[10, 1]), Ok(Some(1)));
    assert_eq!(
        picker.pick(1, &[99]),
        Err("province 99 has no Provinces row".into())
    );
}

#[test]
fn candidates_are_the_regions_states_in_first_appearance_order() {
    let region = |id, provinces: &[u32]| StrategicRegion {
        id,
        provinces: provinces.to_vec(),
        ..Default::default()
    };
    let state = |id, provinces: &[u32]| StateDefinition {
        id,
        provinces: provinces.to_vec(),
        ..Default::default()
    };
    let regions = BTreeMap::from([(1, region(1, &[30, 20, 21, 5, 404]))]);
    let states = BTreeMap::from([
        (7, state(7, &[20, 21, 22, 21, 404])),
        (8, state(8, &[31, 30])),
    ]);
    let valid = |province: u32| province != 404;
    // Sea province 5 has no state and 404 is no map province; state 8
    // comes first through province 30 and brings 31, which lies outside
    // the region; state 7's repeat of 21 is dropped.
    assert_eq!(
        battle_name_candidates(&regions, &states, &valid)[&1],
        vec![31, 30, 20, 21, 22]
    );
    // Province 20 listed by states 7 and 9 belongs to state 9, the later
    // one initialised, yet stays in both lists.
    let states = BTreeMap::from([(7, state(7, &[20, 21])), (9, state(9, &[20, 23]))]);
    let regions = BTreeMap::from([(1, region(1, &[20, 21]))]);
    assert_eq!(
        battle_name_candidates(&regions, &states, &valid)[&1],
        vec![20, 23, 20, 21]
    );
}

/// 140E68E30 qualifies with the state owner's original tag or tag
/// (1406DF830), from CountryAppearance.name_tag, never the current tag of
/// a dynamic country; missing rows are refused by table.
#[test]
fn the_victory_point_tag_is_the_state_owners_original_tag() {
    use replay::{
        frame::TableView,
        rows::{CountryAppearanceRow, CountryLinkRow, CountryRow, StateRow, TableId, TableRows},
    };
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 1,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "missing".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    let states = [
        StateRow {
            id: 7,
            owner: 70,
            ..Default::default()
        },
        StateRow {
            id: 8,
            owner: 80,
            ..Default::default()
        },
    ];
    assert_eq!(
            vp_tag(&frame, 7, &states, true).unwrap(),
            Err("the CountryLinks table is incomplete this hour, so state 7's owner link 70 is unresolved".into())
        );
    *frame.tables[TableId::CountryLinks.index()] =
        TableView::Complete(TableRows::CountryLinks(vec![
            CountryLinkRow {
                id: 70,
                canonical_slot: 700,
            },
            CountryLinkRow {
                id: 80,
                canonical_slot: 800,
            },
        ]));
    *frame.tables[TableId::Countries.index()] = TableView::Complete(TableRows::Countries(vec![
        CountryRow {
            id: 70,
            canonical_slot: 700,
            tag: b"D01".to_vec(),
            ..Default::default()
        },
        CountryRow {
            id: 80,
            canonical_slot: 800,
            tag: b"JAP".to_vec(),
            ..Default::default()
        },
    ]));
    assert!(
        vp_tag(&frame, 7, &states, true)
            .unwrap()
            .unwrap_err()
            .starts_with("the CountryAppearance table is incomplete this hour")
    );
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            CountryAppearanceRow {
                country: 70,
                name_tag: b"CHI".to_vec(),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 80,
                ..Default::default()
            },
        ]));
    assert_eq!(vp_tag(&frame, 7, &states, true).unwrap(), Ok("CHI".into()));
    assert_eq!(vp_tag(&frame, 8, &states, true).unwrap(), Ok("JAP".into()));
    assert_eq!(
        vp_tag(&frame, 9, &states, true).unwrap(),
        Err("state 9 has no States row".into())
    );
    // No state: the country at index 0 (140BA4BB0), here without an
    // original tag; a null owner link resolves there too (140BA56A0).
    let states = [
        states[0].clone(),
        StateRow {
            id: 5,
            owner: 0,
            ..Default::default()
        },
    ];
    assert_eq!(
            vp_tag(&frame, 0, &states, true).unwrap(),
            Err("the country at index 0, which a province without a state qualifies with (140BA4BB0) has no Countries row".into())
        );
    let TableView::Complete(TableRows::Countries(countries)) =
        &mut *frame.tables[TableId::Countries.index()]
    else {
        unreachable!()
    };
    countries.push(CountryRow {
        id: 1,
        canonical_slot: 0,
        tag: b"REB".to_vec(),
        ..Default::default()
    });
    let TableView::Complete(TableRows::CountryAppearance(appearance)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    else {
        unreachable!()
    };
    appearance.push(CountryAppearanceRow {
        country: 1,
        ..Default::default()
    });
    assert_eq!(vp_tag(&frame, 0, &states, true).unwrap(), Ok("REB".into()));
    assert_eq!(vp_tag(&frame, 5, &states, true).unwrap(), Ok("REB".into()));
}

#[test]
fn a_picked_province_is_named_through_the_native_ladder() {
    let loc: Localisation = [
        ("VICTORY_POINTS_10", "Midway"),
        ("JAP_VICTORY_POINTS_10", "Midowei"),
        ("RENAMED", "New Name"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let province = |rename: Option<&'static [u8]>, vp_tag| Province {
        id: 10,
        rename: Ok(rename),
        vp_tag,
        state: None,
        region_name_key: Some("STRATEGICREGION_5"),
    };
    assert_eq!(
        province_name(&NoIcons, &loc, &province(None, Ok("USA"))).as_deref(),
        Ok("Midway")
    );
    assert_eq!(
        province_name(&NoIcons, &loc, &province(None, Ok("JAP"))).as_deref(),
        Ok("Midowei")
    );
    assert_eq!(
        province_name(&NoIcons, &loc, &province(Some(b"RENAMED"), Ok("JAP"))).as_deref(),
        Ok("New Name")
    );
    assert_eq!(
        province_name(&NoIcons, &loc, &province(Some(b"Literal"), Err("x".into()))).as_deref(),
        Ok("Literal")
    );
    // Without a victory point name and without a state, the region name.
    assert_eq!(
        province_name(
            &NoIcons,
            &Localisation::default(),
            &province(None, Ok("USA"))
        )
        .as_deref(),
        Ok("STRATEGICREGION_5")
    );
    // 140E68E30 picks the qualified key once: its empty text goes to the
    // state ladder, never to the generic key.
    let empty: Localisation = [
        ("VICTORY_POINTS_10", "Midway"),
        ("JAP_VICTORY_POINTS_10", ""),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    assert_eq!(
        province_name(&NoIcons, &empty, &province(None, Ok("JAP"))).as_deref(),
        Ok("STRATEGICREGION_5")
    );
    // A rename the hour cannot rule out is refused by province and cause.
    let unknown = Province {
        rename: Err("the ProvinceNames table is unavailable this hour (x)".into()),
        ..province(None, Ok("USA"))
    };
    assert_eq!(
            province_name(&NoIcons, &loc, &unknown),
            Err("province 10's recorded rename (CProvince+0x40) is unknown: the ProvinceNames table is unavailable this hour (x)".into())
        );
    // CState::display_name (1409CC380): the rename, else the owner name,
    // else the definition's localised name key.
    let mut loc: Localisation = [("STATE_7", "Bohemia")]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    loc.language = "english".into();
    let named = |custom_name: &'static [u8], owner_name: &'static [u8]| Province {
        id: 10,
        rename: Ok(None),
        vp_tag: Ok("USA"),
        state: Some(Ok(StateNames {
            id: 7,
            custom_name,
            owner_name,
            recorded_language: "english",
            name_key: Ok("STATE_7"),
        })),
        region_name_key: None,
    };
    assert_eq!(
        province_name(&NoIcons, &loc, &named(b"Sudetenland", b"Bohmen")).as_deref(),
        Ok("Sudetenland")
    );
    assert_eq!(
        province_name(&NoIcons, &loc, &named(b"", b"Bohmen")).as_deref(),
        Ok("Bohmen")
    );
    assert_eq!(
        province_name(&NoIcons, &loc, &named(b"", b"")).as_deref(),
        Ok("Bohemia")
    );
    assert_eq!(
        province_name(&NoIcons, &Localisation::default(), &named(b"", b"")).as_deref(),
        Ok("STATE_7")
    );
    assert_eq!(
        province_name(&NoIcons, &loc, &named(b"\xff", b"")),
        Err("state 7: the custom name (CState+0x38) is not UTF-8".into())
    );
    // Stored without its key, a name is refused in another language; the
    // template's is not.
    let mut german = loc.clone();
    german.language = "german".into();
    assert!(
        province_name(&NoIcons, &german, &named(b"", b"Bohmen"))
            .unwrap_err()
            .contains("stored in english without its key")
    );
    assert_eq!(
        province_name(&NoIcons, &german, &named(b"", b"")).as_deref(),
        Ok("Bohemia")
    );
    let unrecorded = Province {
        state: Some(Err("state 7 has no States row".into())),
        ..named(b"", b"")
    };
    assert!(
        province_name(&NoIcons, &loc, &unrecorded)
            .unwrap_err()
            .contains("state 7 has no States row")
    );
}
