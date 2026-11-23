use super::*;

fn kinds() -> BTreeMap<u32, ProvinceKind> {
    BTreeMap::from([
        (10, ProvinceKind::Sea),
        (11, ProvinceKind::Sea),
        (12, ProvinceKind::Land),
        (13, ProvinceKind::Lake),
    ])
}

#[test]
fn direct_order_and_duplicates_are_separate_from_global_sea_spillover() {
    let combats = [
        Combat {
            id: 8,
            home: 10,
            registry_ordinal: 2,
        },
        Combat {
            id: 7,
            home: 12,
            registry_ordinal: 1,
        },
        Combat {
            id: 9,
            home: 11,
            registry_ordinal: 0,
        },
    ];
    let direct = [Direct {
        province: 10,
        combats: vec![8, 8, 7],
    }];
    let transfers = [
        Transfers {
            combat: 7,
            transfers: vec![100, 100, 0],
        },
        Transfers {
            combat: 9,
            transfers: vec![101],
        },
    ];
    let units = [
        Units {
            transfer: 100,
            units: vec![200, 201, 202, 203],
        },
        Units {
            transfer: 101,
            units: vec![200],
        },
    ];
    let locations = BTreeMap::from([(200, 10), (201, 11), (202, 12), (203, 13)]);
    let kinds = kinds();
    let kind = |province| kinds.get(&province).copied();
    let out = derive(
        Sources {
            combats: &combats,
            combats_complete: true,
            direct: &direct,
            direct_complete: true,
            transfers: Some(&transfers),
            units: Some(&units),
            unit_locations: Some(&locations),
            province_kind: &kind,
        },
        &|_| Some(true),
        &|_| Some(true),
    );
    assert_eq!(
        out.by_province[&10]
            .iter()
            .map(|row| row.combat)
            .collect::<Vec<_>>(),
        [8, 7, 9]
    );
    assert_eq!(
        out.by_province[&11]
            .iter()
            .map(|row| row.combat)
            .collect::<Vec<_>>(),
        [7]
    );
    assert_eq!(
        out.by_province[&10]
            .iter()
            .map(|row| row.home)
            .collect::<Vec<_>>(),
        [true, true, false]
    );
    assert_eq!(
        out.by_province[&10]
            .iter()
            .map(|row| (row.anchor_province, row.ordinal))
            .collect::<Vec<_>>(),
        [(10, Some(1)), (12, Some(4)), (10, Some(3))],
        "direct P!=H keeps H after a later global dedup hit, while every admitted call consumes a slot"
    );
    assert!(!out.by_province[&11][0].home);
    assert_eq!(out.by_province[&11][0].ordinal, Some(0));
    assert!(!out.by_province.contains_key(&12));
    assert!(!out.by_province.contains_key(&13));
    assert_eq!(out.spillover_gap, None);
}

#[test]
fn absent_transfer_sources_keep_direct_presence_but_not_sea_offset() {
    let combats = [Combat {
        id: 8,
        home: 10,
        registry_ordinal: 0,
    }];
    let direct = [Direct {
        province: 10,
        combats: vec![8],
    }];
    let kinds = kinds();
    let kind = |province| kinds.get(&province).copied();
    let out = derive(
        Sources {
            combats: &combats,
            combats_complete: true,
            direct: &direct,
            direct_complete: true,
            transfers: None,
            units: None,
            unit_locations: None,
            province_kind: &kind,
        },
        &|_| Some(true),
        &|_| Some(true),
    );
    assert_eq!(
        out.by_province[&10],
        [Occurrence {
            province: 10,
            anchor_province: 10,
            combat: 8,
            home: true,
            ordinal: None,
        }]
    );
    assert_eq!(
        out.spillover_gap,
        Some("naval-combat transfer census is incomplete")
    );
}

#[test]
fn stale_transfer_ref_is_skipped_from_a_complete_unit_table() {
    let combats = [Combat {
        id: 8,
        home: 10,
        registry_ordinal: 0,
    }];
    let direct = [Direct {
        province: 10,
        combats: vec![8],
    }];
    let transfers = [Transfers {
        combat: 8,
        transfers: vec![100],
    }];
    let kinds = kinds();
    let kind = |province| kinds.get(&province).copied();
    let out = derive(
        Sources {
            combats: &combats,
            combats_complete: true,
            direct: &direct,
            direct_complete: true,
            transfers: Some(&transfers),
            units: Some(&[]),
            unit_locations: Some(&BTreeMap::new()),
            province_kind: &kind,
        },
        &|_| Some(true),
        &|_| Some(true),
    );
    assert_eq!(out.by_province[&10].len(), 1);
    assert_eq!(out.by_province[&10][0].ordinal, Some(0));
    assert_eq!(out.spillover_gap, None);
}

#[test]
fn unengaged_combat_needs_no_transfer_resolution() {
    let combats = [Combat {
        id: 8,
        home: 10,
        registry_ordinal: 0,
    }];
    let transfers = [Transfers {
        combat: 8,
        transfers: vec![100],
    }];
    let kinds = kinds();
    let kind = |province| kinds.get(&province).copied();
    let out = derive(
        Sources {
            combats: &combats,
            combats_complete: true,
            direct: &[],
            direct_complete: true,
            transfers: Some(&transfers),
            units: Some(&[]),
            unit_locations: Some(&BTreeMap::new()),
            province_kind: &kind,
        },
        &|_| Some(true),
        &|_| Some(false),
    );
    assert!(out.by_province.is_empty());
    assert_eq!(out.spillover_gap, None);
}

#[test]
fn global_spillovers_follow_registry_order_and_unknown_engagement_keeps_direct() {
    let combats = [
        Combat {
            id: 8,
            home: 11,
            registry_ordinal: 9,
        },
        Combat {
            id: 7,
            home: 11,
            registry_ordinal: 2,
        },
    ];
    let direct = [Direct {
        province: 11,
        combats: vec![8],
    }];
    let transfers = [
        Transfers {
            combat: 8,
            transfers: vec![108],
        },
        Transfers {
            combat: 7,
            transfers: vec![107],
        },
    ];
    let units = [
        Units {
            transfer: 108,
            units: vec![208],
        },
        Units {
            transfer: 107,
            units: vec![207],
        },
    ];
    let locations = BTreeMap::from([(207, 10), (208, 10)]);
    let kinds = kinds();
    let kind = |province| kinds.get(&province).copied();
    let sources = || Sources {
        combats: &combats,
        combats_complete: true,
        direct: &direct,
        direct_complete: true,
        transfers: Some(&transfers),
        units: Some(&units),
        unit_locations: Some(&locations),
        province_kind: &kind,
    };
    let complete = derive(sources(), &|_| Some(true), &|_| Some(true));
    assert_eq!(
        complete.by_province[&10]
            .iter()
            .map(|row| row.combat)
            .collect::<Vec<_>>(),
        [7, 8]
    );
    let unknown = derive(sources(), &|_| Some(true), &|id| (id == 8).then_some(true));
    assert_eq!(
        unknown.by_province[&11],
        [Occurrence {
            province: 11,
            anchor_province: 11,
            combat: 8,
            home: true,
            ordinal: None,
        }]
    );
    assert!(!unknown.by_province.contains_key(&10));
    assert_eq!(
        unknown.spillover_gap,
        Some("naval-combat country engagement is unknown")
    );
}

#[test]
fn missing_unit_location_is_not_a_stale_transfer_and_poison_sea_offsets() {
    let combats = [Combat {
        id: 8,
        home: 10,
        registry_ordinal: 0,
    }];
    let direct = [Direct {
        province: 10,
        combats: vec![8],
    }];
    let transfers = [Transfers {
        combat: 8,
        transfers: vec![100],
    }];
    let units = [Units {
        transfer: 100,
        units: vec![200],
    }];
    let kinds = kinds();
    let kind = |province| kinds.get(&province).copied();
    let out = derive(
        Sources {
            combats: &combats,
            combats_complete: true,
            direct: &direct,
            direct_complete: true,
            transfers: Some(&transfers),
            units: Some(&units),
            unit_locations: Some(&BTreeMap::new()),
            province_kind: &kind,
        },
        &|_| Some(true),
        &|_| Some(true),
    );
    assert_eq!(out.by_province[&10][0].ordinal, None);
    assert_eq!(
        out.spillover_gap,
        Some("a naval-transfer unit location is unresolved")
    );
}

#[test]
fn unknown_engagement_without_sea_candidate_does_not_poison_other_spillovers() {
    let combats = [
        Combat {
            id: 7,
            home: 11,
            registry_ordinal: 0,
        },
        Combat {
            id: 8,
            home: 11,
            registry_ordinal: 1,
        },
    ];
    let transfers = [Transfers {
        combat: 8,
        transfers: vec![108],
    }];
    let units = [Units {
        transfer: 108,
        units: vec![208],
    }];
    let locations = BTreeMap::from([(208, 10)]);
    let kinds = kinds();
    let kind = |province| kinds.get(&province).copied();
    let out = derive(
        Sources {
            combats: &combats,
            combats_complete: true,
            direct: &[],
            direct_complete: true,
            transfers: Some(&transfers),
            units: Some(&units),
            unit_locations: Some(&locations),
            province_kind: &kind,
        },
        &|_| Some(true),
        &|id| (id == 8).then_some(true),
    );
    assert_eq!(out.spillover_gap, None);
    assert_eq!(out.by_province[&10][0].combat, 8);
    assert_eq!(out.by_province[&10][0].ordinal, Some(0));
}

#[test]
fn unknown_direct_admission_makes_later_offsets_unknown_without_removing_earlier_icon() {
    let combats = [
        Combat {
            id: 6,
            home: 10,
            registry_ordinal: 0,
        },
        Combat {
            id: 7,
            home: 10,
            registry_ordinal: 1,
        },
        Combat {
            id: 8,
            home: 10,
            registry_ordinal: 2,
        },
    ];
    let direct = [Direct {
        province: 10,
        combats: vec![6, 7, 8],
    }];
    let kinds = kinds();
    let kind = |province| kinds.get(&province).copied();
    let locations = BTreeMap::new();
    let out = derive(
        Sources {
            combats: &combats,
            combats_complete: true,
            direct: &direct,
            direct_complete: true,
            transfers: Some(&[]),
            units: Some(&[]),
            unit_locations: Some(&locations),
            province_kind: &kind,
        },
        &|id| (id != 7).then_some(true),
        &|_| Some(true),
    );
    assert_eq!(out.spillover_gap, None);
    assert_eq!(out.admission_gaps, [(10, 7)]);
    assert_eq!(
        out.by_province[&10]
            .iter()
            .map(|row| row.combat)
            .collect::<Vec<_>>(),
        [6, 8]
    );
    assert_eq!(
        out.by_province[&10]
            .iter()
            .map(|row| row.ordinal)
            .collect::<Vec<_>>(),
        [Some(0), None]
    );
}

#[test]
fn an_unengaged_combat_with_unresolved_units_leaves_other_icons_placed() {
    let combats = [
        Combat {
            id: 7,
            home: 10,
            registry_ordinal: 0,
        },
        Combat {
            id: 8,
            home: 11,
            registry_ordinal: 1,
        },
    ];
    let direct = [Direct {
        province: 10,
        combats: vec![7],
    }];
    let transfers = [
        Transfers {
            combat: 7,
            transfers: vec![107],
        },
        Transfers {
            combat: 8,
            transfers: vec![108],
        },
    ];
    let units = [
        Units {
            transfer: 107,
            units: vec![],
        },
        Units {
            transfer: 108,
            units: vec![208],
        },
    ];
    let kinds = kinds();
    let kind = |province| kinds.get(&province).copied();
    let locations = BTreeMap::new();
    let out = derive(
        Sources {
            combats: &combats,
            combats_complete: true,
            direct: &direct,
            direct_complete: true,
            transfers: Some(&transfers),
            units: Some(&units),
            unit_locations: Some(&locations),
            province_kind: &kind,
        },
        &|_| Some(true),
        &|id| Some(id == 7),
    );
    assert_eq!(out.spillover_gap, None);
    assert_eq!(out.by_province[&10][0].combat, 7);
    assert_eq!(out.by_province[&10][0].ordinal, Some(0));
    assert_eq!(out.by_province.len(), 1);
}

#[test]
fn authored_integer_gui_height_sets_positive_screen_stack_offset() {
    assert_eq!(screen_y_offset(0, 32.0), Some(0.0));
    assert_eq!(screen_y_offset(2, 32.0), Some(64.0));
    assert_eq!(screen_y_offset(2, 32.5), None);
    assert_eq!(screen_y_offset(u32::MAX, 32.0), None);
}
