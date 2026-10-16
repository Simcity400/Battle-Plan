use super::*;
use viewer_foundation::tooltip_text::NoIcons;

/// Victory-point keys use the state owner's original tag or own tag
/// from its independent exact suffix; missing rows are refused.
#[test]
fn the_victory_point_tag_is_the_state_owners_original_tag() {
    use replay::{
        frame::TableView,
        rows::{
            CountryAppearanceRow, CountryLinkRow, CountryRow, Recorded, ReservedCountryNamingRow,
            StateRow, TableId, TableRows,
        },
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
            .contains("CountryAppearance is incomplete this hour")
    );
    *frame.tables[TableId::CountryAppearance.index()] =
        TableView::Complete(TableRows::CountryAppearance(vec![
            CountryAppearanceRow {
                country: 70,
                original_tag_reference: Recorded::Value(Some(1)),
                original_tag: Recorded::Value(Some(b"CHI".to_vec())),
                ..Default::default()
            },
            CountryAppearanceRow {
                country: 80,
                original_tag_reference: Recorded::Value(Some(0)),
                original_tag: Recorded::Value(Some(Vec::new())),
                ..Default::default()
            },
        ]));
    assert_eq!(vp_tag(&frame, 7, &states, true).unwrap(), Ok("CHI".into()));
    assert_eq!(vp_tag(&frame, 8, &states, true).unwrap(), Ok("JAP".into()));
    assert_eq!(
        vp_tag(&frame, 9, &states, true).unwrap(),
        Err("state 9 has no States row".into())
    );
    // No state and a null owner link resolve the separate reserved zero object.
    let states = [
        states[0].clone(),
        StateRow {
            id: 5,
            owner: 0,
            ..Default::default()
        },
    ];
    assert!(
        vp_tag(&frame, 0, &states, true)
            .unwrap()
            .unwrap_err()
            .contains("ReservedCountryNaming")
    );
    *frame.tables[TableId::ReservedCountryNaming.index()] =
        TableView::Complete(TableRows::ReservedCountryNaming(vec![
            ReservedCountryNamingRow {
                tag: b"ZERO".to_vec(),
                original_tag_reference: Recorded::Value(Some(1)),
                original_tag: Recorded::Value(Some(b"REB".to_vec())),
                ..Default::default()
            },
        ]));
    assert_eq!(vp_tag(&frame, 0, &states, true).unwrap(), Ok("REB".into()));
    assert_eq!(vp_tag(&frame, 5, &states, true).unwrap(), Ok("REB".into()));
    if let TableView::Complete(TableRows::CountryAppearance(rows)) =
        &mut *frame.tables[TableId::CountryAppearance.index()]
    {
        rows[0].original_tag = Recorded::NotRecorded;
    }
    assert!(
        vp_tag(&frame, 7, &states, true)
            .unwrap()
            .unwrap_err()
            .contains("CountryAppearance.original_tag not recorded")
    );
    // Reserved success stays independent of unavailable positive-country rows.
    *frame.tables[TableId::Countries.index()] = TableView::Unavailable {
        reason: "unrelated positive roster".into(),
    };
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
    // the game picks the qualified key once: its empty text goes to the
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
            Err("province 10's recorded rename is unknown: the ProvinceNames table is unavailable this hour (x)".into())
        );
    // State display naming: the rename, else the owner name,
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
        Err("state 7: the custom name is not UTF-8".into())
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
