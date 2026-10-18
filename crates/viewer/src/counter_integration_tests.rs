//! Engagement orchestration composed with counter marker reads.
use replay_data::{rows::TableId, rows::TableRows};
use replay_frame::{frame::DecodedFrame, frame::TableView};
use std::collections::{BTreeMap, BTreeSet};
use viewer_counters::map::counters::test_support::{LandMarkers, kept_frames_at};
use viewer_foundation::{
    engaged::Engaged,
    engagement_history::{LAND_ICON, Provenance, kept_frames},
    map::perspective::Country,
};

/// One hour of a branch for kept-frame tests. Countries 1..=4 (slot = id,
/// AAA to DDD, player 40 on DDD); AAA views, and `allied` puts AAA and BBB in
/// one faction. `land` holds (combat, attacker units, BBB in the attacking
/// country list, indicator byte), with null map entities and CCC defending.
/// `naval` holds (combat, attacking owner): that owner's convoy against a CCC
/// cruiser, which draws as the convoy variant.
fn kept_frame_hour(
    hour: replay_data::Hour,
    allied: bool,
    land: &[(u64, Vec<u64>, bool, u8)],
    naval: &[(u64, u32)],
) -> DecodedFrame {
    use replay_data::{
        rows::CountryLinkRow, rows::CountryRow, rows::DivisionMovementRow, rows::FactionMemberRow,
        rows::HumanControllerRow, rows::LandCombatRow, rows::LandCombatSideRow,
        rows::NavalCombatDetailsRow, rows::NavalCombatParticipantRow, rows::NavalCombatRow,
        rows::PlayerRow, rows::ShipRow,
    };
    let mut frame = DecodedFrame {
        branch: 1,
        hour,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not captured".into(),
            }
            .into()
        }),
        metrics: Default::default(),
    };
    let mut set = |table: TableId, rows: TableRows| {
        *frame.tables[table.index()] = TableView::Complete(rows);
    };
    set(
        TableId::Countries,
        TableRows::Countries(
            (1..=4)
                .zip(["AAA", "BBB", "CCC", "DDD"])
                .map(|(id, tag)| CountryRow {
                    id,
                    canonical_slot: id,
                    tag: tag.as_bytes().to_vec(),
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::CountryLinks,
        TableRows::CountryLinks(
            (1..=4)
                .map(|id| CountryLinkRow {
                    id,
                    canonical_slot: id,
                })
                .collect(),
        ),
    );
    set(
        TableId::FactionMembers,
        TableRows::FactionMembers(if allied {
            [1, 2]
                .map(|country| FactionMemberRow {
                    country,
                    faction: 9,
                    ordinal: 0,
                })
                .to_vec()
        } else {
            Vec::new()
        }),
    );
    for table in [TableId::WarRelations, TableId::Subjects] {
        set(table, TableRows::empty(table));
    }
    set(
        TableId::Players,
        TableRows::Players(vec![PlayerRow {
            id: 40,
            country: 4,
            ..Default::default()
        }]),
    );
    set(
        TableId::HumanControllers,
        TableRows::HumanControllers(vec![HumanControllerRow {
            country: 4,
            count: 1,
        }]),
    );
    set(
        TableId::LandCombats,
        TableRows::LandCombats(
            land.iter()
                .map(|(id, attackers, _, _)| LandCombatRow {
                    id: *id,
                    attackers: attackers.clone(),
                    defenders: vec![*id + 1000],
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::LandCombatSides,
        TableRows::LandCombatSides(
            land.iter()
                .map(|&(combat, _, listed, indicator)| LandCombatSideRow {
                    combat,
                    indicator,
                    attacker_primary: if listed { vec![2] } else { Vec::new() },
                    defender_primary: vec![3],
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::DivisionMovement,
        TableRows::DivisionMovement(
            land.iter()
                .flat_map(|(_, attackers, _, _)| attackers.iter().copied())
                .map(|division| DivisionMovementRow {
                    division,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::NavalCombats,
        TableRows::NavalCombats(
            naval
                .iter()
                .map(|&(id, _)| NavalCombatRow {
                    id,
                    province: 500,
                    ..Default::default()
                })
                .collect(),
        ),
    );
    set(
        TableId::NavalCombatParticipants,
        TableRows::NavalCombatParticipants(
            naval
                .iter()
                .flat_map(|&(combat, owner)| {
                    [(0, owner, 0), (1, 3, 11)].map(|(side, owner, ship)| {
                        NavalCombatParticipantRow {
                            combat,
                            side,
                            owner,
                            ship,
                            ..Default::default()
                        }
                    })
                })
                .collect(),
        ),
    );
    set(
        TableId::Ships,
        TableRows::Ships(vec![ShipRow {
            id: 11,
            category: 3,
            ..Default::default()
        }]),
    );
    set(
        TableId::NavalCombatDetails,
        TableRows::NavalCombatDetails(
            naval
                .iter()
                .map(|&(combat, owner)| {
                    let mut row = NavalCombatDetailsRow {
                        combat,
                        province: 500,
                        ..Default::default()
                    };
                    row.sides[0].summary.countries = vec![owner];
                    row.sides[1].summary.countries = vec![3];
                    row
                })
                .collect(),
        ),
    );
    frame
}

/// An unseen combat icon keeps the frame
/// its last seen hour set, carried by the engaged index and named per
/// combat under the camera assumption; a new instance with no seen hour (a new
/// combat, or a side emptied then refilled) uses the authored initial frame,
/// counted as not from history. A seek reads the same
/// frame. Land combat 62 is seen at hours 0
/// and 1 (AAA allied with the attacking BBB until hour 1, the flag
/// lagging; indicator 1) and unseen from hour 2, when a joining unit
/// rebuilds the side. The naval icons' carried frame is
/// `naval_markers`' and the engaged index's.
#[test]
fn an_unseen_combat_icon_keeps_its_last_seen_frame_through_the_index() {
    use viewer_foundation::icon_frames::{CARRIED, Kept, LAND_NO_HISTORY};
    let hours = [
        kept_frame_hour(0, true, &[(62, vec![7], true, 1)], &[(70, 2)]),
        // The alliance ends; the land flag lags until a side rebuilds.
        kept_frame_hour(1, false, &[(62, vec![7], true, 1)], &[(70, 2)]),
        kept_frame_hour(
            2,
            false,
            &[(62, vec![7, 8], true, 0), (63, vec![9], true, 1)],
            &[(70, 2), (71, 3)],
        ),
        // BBB leaves combat 62's attacking list: the game erases the icon.
        kept_frame_hour(3, false, &[(62, vec![7, 8], false, 0)], &[(70, 2)]),
        // Refilled: a new instance, never seen.
        kept_frame_hour(4, false, &[(62, vec![7, 8], true, 0)], &[(70, 2)]),
    ];
    let index =
        crate::engaged_index::EngagedIndex::folded(&hours, Country::Tag("AAA".into()), Some(0));
    let marker = |land: &LandMarkers, id| {
        land.markers
            .iter()
            .find(|marker| marker.id == id)
            .map(|marker| (marker.seen, marker.frame))
    };
    // Seen: the indicator's frame, nothing carried.
    let land = kept_frames_at(index.shared.as_ref(), &hours[0]);
    assert_eq!(marker(&land, 62), Some((true, Some(1))));
    assert!(!land.limits.contains_key(CARRIED));
    let land = kept_frames_at(index.shared.as_ref(), &hours[1]);
    assert_eq!(marker(&land, 62), Some((true, Some(1))), "{:?}", land.gaps);
    // Unseen: the last seen frame, counted under the assumption; a new
    // combat created while unseen uses the authored initial frame.
    let land = kept_frames_at(index.shared.as_ref(), &hours[2]);
    assert_eq!(marker(&land, 62), Some((false, Some(1))), "{:?}", land.gaps);
    assert_eq!(marker(&land, 63), Some((false, None)));
    assert_eq!(land.limits.get(CARRIED), Some(&BTreeSet::from([62])));
    assert_eq!(
        land.limits.get(&Kept::Fresh.limitation(LAND_NO_HISTORY)),
        Some(&BTreeSet::from([63]))
    );
    // An empty side: no icon at all, as the game draws none.
    let land = kept_frames_at(index.shared.as_ref(), &hours[3]);
    assert_eq!(marker(&land, 62), None);
    assert!(land.gaps.is_empty(), "{:?}", land.gaps);
    // Refilled: a new instance with no seen hour.
    let land = kept_frames_at(index.shared.as_ref(), &hours[4]);
    assert_eq!(marker(&land, 62), Some((false, None)), "{:?}", land.gaps);
    assert_eq!(
        land.limits.get(&Kept::Fresh.limitation(LAND_NO_HISTORY)),
        Some(&BTreeSet::from([62]))
    );
    // A seek back reads the same kept frame from the index.
    let land = kept_frames_at(index.shared.as_ref(), &hours[2]);
    assert_eq!(marker(&land, 62), Some((false, Some(1))));
    // Without the index an unseen icon's history is unknown, by name.
    let (kept, provenance) = kept_frames(
        &hours[2],
        LAND_ICON,
        1,
        &BTreeMap::from([(62, Engaged::No)]),
        None,
    )
    .unwrap();
    assert_eq!(provenance, Provenance::Unindexed);
    assert_eq!(kept[&62], Kept::Unindexed);
}
