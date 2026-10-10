use super::*;

const WIDTH: f32 = 5632.0;
const DEFINES: GroupDefines = GroupDefines::VANILLA;

fn icon(kind: u32, owner: u32, province: u32, state: Option<u32>, x: f32) -> Icon {
    Icon {
        kind,
        owner,
        province,
        state,
        region: Some(1),
        anchor: [x, 0.0, 0.0],
    }
}

fn extents(state: f32, region: f32) -> Extents {
    Extents::build(
        &[
            (1, [0.0, 0.0, state, state]),
            (2, [0.0, 0.0, region, region]),
        ]
        .into(),
        &[(1, 7)].into(),
        &[(2, 9)].into(),
        &DEFINES,
        WIDTH,
    )
}

fn all_friendly(_: u32, _: u32) -> bool {
    true
}

#[test]
fn the_camera_height_picks_the_level_the_defines_name() {
    assert_eq!(DEFINES.level(0.0), Level::None);
    assert_eq!(DEFINES.level(89.9), Level::None);
    assert_eq!(DEFINES.level(90.0), Level::Province);
    assert_eq!(DEFINES.level(179.9), Level::Province);
    assert_eq!(DEFINES.level(180.0), Level::State);
    assert_eq!(DEFINES.level(349.9), Level::State);
    assert_eq!(DEFINES.level(350.0), Level::Strategic);
    let modded = GroupDefines {
        state_distance: 400.0,
        strategic_distance: 800.0,
        ..DEFINES
    };
    assert_eq!(modded.level(350.0), Level::Province);
    assert_eq!(modded.grouping(350.0), Level::Province);
    // A mod that gives the pass no budget turns grouping off at every height.
    let off = GroupDefines {
        passes: 0,
        ..DEFINES
    };
    assert_eq!(off.grouping(350.0), Level::None);
    assert_eq!(off.level(350.0), Level::Strategic);
}

#[test]
fn a_huge_region_is_one_above_the_define_and_not_one_below() {
    let huge = extents(DEFINES.state_huge + 1.0, DEFINES.strategic_area_huge + 1.0);
    assert!(huge.state_is_huge(7));
    assert!(huge.region_is_huge(9));
    let small = extents(DEFINES.state_huge, DEFINES.strategic_area_huge);
    assert!(!small.state_is_huge(7));
    assert!(!small.region_is_huge(9));
    // An unknown region is not huge, matching the engine's null template.
    assert!(!small.state_is_huge(999));
    assert!(!small.region_is_huge(999));
}

#[test]
fn the_huge_defines_truncate_like_the_native_integer_getter() {
    let defines = Defines::new().unwrap();
    defines
        .load(
            "mod.lua",
            b"NDefines = { NGraphics = {
                MAP_ICONS_GROUP_CAM_DISTANCE = 90.5,
                MAP_ICONS_STATE_GROUP_CAM_DISTANCE = 180,
                MAP_ICONS_STRATEGIC_GROUP_CAM_DISTANCE = 350,
                MAP_ICONS_STATE_HUGE = 100.9,
                MAP_ICONS_STRATEGIC_AREA_HUGE = -220.7,
                MAPICON_GROUP_PASSES = 20.9,
            } }",
        )
        .unwrap();
    let read = GroupDefines::read(&defines).unwrap();
    assert_eq!(read.province_distance, 90.5);
    // A fixed5 reading would keep 100.9 and -220.7.
    assert_eq!(read.state_huge, 100.0);
    assert_eq!(read.strategic_area_huge, -220.0);
    assert_eq!(read.passes, 20);
    // So a state 100.5 wide is huge, which 100.9 would not call it.
    let extents = Extents::build(
        &[(1, [0.0, 0.0, 100.5, 1.0])].into(),
        &[(1, 7)].into(),
        &BTreeMap::new(),
        &read,
        WIDTH,
    );
    assert!(extents.state_is_huge(7));
}

#[test]
fn level_two_gathers_a_state_and_level_one_only_a_province() {
    let extents = extents(10.0, 10.0);
    let a = icon(0, 1, 100, Some(7), 0.0);
    let b = icon(0, 1, 200, Some(7), 10.0);
    assert!(may_group(&a, &b, Level::State, &extents, &all_friendly));
    assert!(!may_group(&a, &b, Level::Province, &extents, &all_friendly));
    assert!(!may_group(&a, &b, Level::None, &extents, &all_friendly));
    let same = icon(0, 1, 100, Some(7), 10.0);
    assert!(may_group(
        &a,
        &same,
        Level::Province,
        &extents,
        &all_friendly
    ));
}

#[test]
fn a_huge_state_falls_back_to_the_province() {
    let extents = extents(DEFINES.state_huge + 1.0, 10.0);
    let a = icon(0, 1, 100, Some(7), 0.0);
    let b = icon(0, 1, 200, Some(7), 10.0);
    assert!(!may_group(&a, &b, Level::State, &extents, &all_friendly));
    let same = icon(0, 1, 100, Some(7), 10.0);
    assert!(may_group(&a, &same, Level::State, &extents, &all_friendly));
}

#[test]
fn a_huge_strategic_region_falls_back_to_the_state() {
    let extents = extents(10.0, DEFINES.strategic_area_huge + 1.0);
    let mut a = icon(0, 1, 100, Some(7), 0.0);
    let mut b = icon(0, 1, 200, Some(7), 10.0);
    a.region = Some(9);
    b.region = Some(9);
    assert!(may_group(&a, &b, Level::Strategic, &extents, &all_friendly));
    b.state = Some(8);
    assert!(!may_group(
        &a,
        &b,
        Level::Strategic,
        &extents,
        &all_friendly
    ));
}

#[test]
fn sea_icons_gather_by_strategic_region_at_every_level() {
    let extents = extents(10.0, 10.0);
    let mut a = icon(1, 1, 100, None, 0.0);
    let mut b = icon(1, 1, 200, None, 10.0);
    a.region = Some(9);
    b.region = Some(9);
    for at in [Level::Province, Level::State, Level::Strategic] {
        assert!(may_group(&a, &b, at, &extents, &all_friendly), "{at:?}");
    }
    let land = icon(1, 1, 300, Some(7), 20.0);
    assert!(!may_group(&a, &land, Level::State, &extents, &all_friendly));
}

#[test]
fn the_kind_the_owner_and_the_relation_all_gate_a_group() {
    let extents = extents(10.0, 10.0);
    let a = icon(0, 1, 100, Some(7), 0.0);
    let mut other = icon(1, 1, 200, Some(7), 10.0);
    assert!(!may_group(
        &a,
        &other,
        Level::State,
        &extents,
        &all_friendly
    ));
    other.kind = 0;
    other.owner = 2;
    assert!(may_group(&a, &other, Level::State, &extents, &all_friendly));
    assert!(!may_group(&a, &other, Level::State, &extents, &|_, _| {
        false
    }));
    // The native read is directed: the other owner's relation toward the first.
    let toward_first = |first, other| (other, first) == (2, 1);
    assert!(may_group(&a, &other, Level::State, &extents, &toward_first));
    assert!(!may_group(
        &other,
        &a,
        Level::State,
        &extents,
        &toward_first
    ));
    // Level 3 never consults the relation.
    assert!(may_group(
        &a,
        &other,
        Level::Strategic,
        &extents,
        &|_, _| false
    ));
    let ownerless = icon(0, 0, 100, Some(7), 0.0);
    assert!(!may_group(
        &ownerless,
        &other,
        Level::State,
        &extents,
        &all_friendly
    ));
}

#[test]
fn the_relation_is_asked_only_after_the_geography_accepts() {
    let extents = extents(10.0, 10.0);
    let asked = std::cell::Cell::new(0);
    let counting = |_: u32, _: u32| {
        asked.set(asked.get() + 1);
        true
    };
    let a = icon(0, 1, 100, Some(7), 0.0);
    let far = icon(0, 2, 200, Some(8), 10.0);
    assert!(!may_group(&a, &far, Level::State, &extents, &counting));
    assert_eq!(asked.get(), 0);
    let near = icon(0, 2, 200, Some(7), 10.0);
    assert!(may_group(&a, &near, Level::State, &extents, &counting));
    assert_eq!(asked.get(), 1);
}

#[test]
fn a_group_sits_on_the_mean_of_its_members_and_a_lone_icon_does_not_move() {
    let extents = extents(10.0, 10.0);
    let icons = [
        icon(0, 1, 100, Some(7), 0.0),
        icon(0, 1, 200, Some(7), 30.0),
        icon(0, 1, 300, Some(8), 90.0),
    ];
    let groups = group(&icons, Level::State, &extents, &all_friendly);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].members, vec![0, 1]);
    assert_eq!(groups[0].anchor, [15.0, 0.0, 0.0]);
    assert_eq!(groups[1].members, vec![2]);
    assert_eq!(groups[1].anchor, icons[2].anchor);
}

#[test]
fn a_nontransitive_relation_joins_every_accepting_group() {
    let extents = extents(10.0, 10.0);
    let icons = [
        icon(0, 1, 100, Some(7), 0.0),
        icon(0, 2, 200, Some(7), 40.0),
        icon(0, 3, 300, Some(7), 20.0),
    ];
    // 2 is not friendly toward 1, so it leads its own group; 3 is friendly
    // toward both leaders, and the native scan appends it to both.
    let directed = |first, other| matches!((other, first), (3, 1) | (3, 2));
    let groups = group(&icons, Level::State, &extents, &directed);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].members, vec![0, 2]);
    assert_eq!(groups[1].members, vec![1, 2]);
    assert_eq!(groups[0].anchor, [10.0, 0.0, 0.0]);
    assert_eq!(groups[1].anchor, [30.0, 0.0, 0.0]);
}

#[test]
fn the_indexed_rebuild_matches_the_native_scan_over_every_group() {
    // Brute force: offer each icon to every group, as the game does.
    fn scan(
        icons: &[Icon],
        at: Level,
        extents: &Extents,
        friendly: &impl Fn(u32, u32) -> bool,
    ) -> Vec<Vec<usize>> {
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for (index, icon) in icons.iter().enumerate() {
            let mut joined = false;
            for group in &mut groups {
                if may_group(&icons[group[0]], icon, at, extents, friendly) {
                    group.push(index);
                    joined = true;
                }
            }
            if !joined {
                groups.push(vec![index]);
            }
        }
        groups
    }
    // A directed nontransitive relation, everyone allied and no one allied.
    let relations: [fn(u32, u32) -> bool; 3] = [
        |a, b| !(a + 2 * b).is_multiple_of(3),
        |_, _| true,
        |_, _| false,
    ];
    // The brute force must reach every branch the index narrows on.
    let mut several_groups = false;
    let mut sea_below_level_three = false;
    let mut huge_region_by_state = false;
    let mut allied_below_level_three = false;
    for start in [7_u32, 11, 2026] {
        let mut seed = start;
        let mut next = |modulo: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 16) % modulo
        };
        // Place metadata is drawn per icon, so one province can also carry
        // mixed land/sea, state and region facts.
        let icons: Vec<Icon> = (0..300)
            .map(|_| {
                let sea = next(5) == 0;
                Icon {
                    kind: next(4),
                    owner: next(6),
                    province: next(40),
                    state: (!sea).then(|| next(8)),
                    region: (next(5) != 0).then(|| next(4)),
                    anchor: [next(100) as f32, 0.0, 0.0],
                }
            })
            .collect();
        for friendly in relations {
            for huge_states in [false, true] {
                for huge_regions in [false, true] {
                    // State 7 and region 3 have no extent: never huge.
                    let mut extents = Extents {
                        state_huge: DEFINES.state_huge,
                        strategic_area_huge: DEFINES.strategic_area_huge,
                        map_width: WIDTH,
                        ..Extents::default()
                    };
                    for state in 0..7 {
                        let huge = huge_states && state % 2 == 0;
                        let extent = if huge { DEFINES.state_huge + 1.0 } else { 10.0 };
                        extents.states.insert(state, extent);
                    }
                    for region in 0..3 {
                        let huge = huge_regions && region % 2 == 0;
                        let extent = if huge {
                            DEFINES.strategic_area_huge + 1.0
                        } else {
                            10.0
                        };
                        extents.regions.insert(region, extent);
                    }
                    for at in [Level::None, Level::Province, Level::State, Level::Strategic] {
                        let expected = scan(&icons, at, &extents, &friendly);
                        let fast: Vec<Vec<usize>> = group(&icons, at, &extents, &friendly)
                            .into_iter()
                            .map(|group| group.members)
                            .collect();
                        assert_eq!(
                            fast, expected,
                            "seed {start}, {at:?}, huge states={huge_states}, huge regions={huge_regions}"
                        );
                        let mut joined = vec![0; icons.len()];
                        for members in &expected {
                            let first = &icons[members[0]];
                            for &member in members {
                                joined[member] += 1;
                                let other = &icons[member];
                                let below_three = at < Level::Strategic;
                                allied_below_level_three |=
                                    below_three && other.owner != first.owner;
                                if other.province == first.province {
                                    continue;
                                }
                                sea_below_level_three |= below_three && first.state.is_none();
                                huge_region_by_state |= at == Level::Strategic
                                    && first.state.is_some()
                                    && first.region.is_some_and(|r| extents.region_is_huge(r));
                            }
                        }
                        several_groups |= joined.iter().any(|&count| count > 1);
                    }
                }
            }
        }
    }
    assert!(several_groups);
    assert!(sea_below_level_three);
    assert!(huge_region_by_state);
    assert!(allied_below_level_three);
}

#[test]
fn level_none_leaves_every_icon_where_it_was() {
    let extents = extents(10.0, 10.0);
    let icons = [
        icon(0, 1, 100, Some(7), 0.0),
        icon(0, 1, 100, Some(7), 30.0),
    ];
    let groups = group(&icons, Level::None, &extents, &all_friendly);
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0].anchor, icons[0].anchor);
    assert_eq!(groups[1].anchor, icons[1].anchor);
}

#[test]
#[ignore = "release unit scaling benchmark"]
fn unit_scale_grouping() {
    for count in [50_000, 100_000] {
        // Unique plates distributed among 10,000 synthetic provinces,
        // 100 states and 20 regions. No country is allied to another.
        let icons: Vec<_> = (0..count)
            .map(|i| Icon {
                kind: 0,
                owner: (i % 1_500 + 1) as u32,
                province: (i % 10_000) as u32,
                state: Some((i % 10_000 / 100) as u32),
                region: Some((i % 10_000 / 500) as u32),
                anchor: [(i % 1000) as f32, 0.0, (i / 1000) as f32],
            })
            .collect();
        let extents = extents(10.0, 10.0);
        for level in [Level::None, Level::Province, Level::State, Level::Strategic] {
            let mut times = Vec::new();
            let mut retained = 0;
            let mut group_count = 0;
            for _ in 0..5 {
                let started = std::time::Instant::now();
                let groups = group(&icons, level, &extents, &|_, _| false);
                times.push(started.elapsed().as_secs_f64() * 1e3);
                assert_eq!(groups.iter().map(|g| g.members.len()).sum::<usize>(), count);
                retained = groups.capacity() * std::mem::size_of::<Group>()
                    + groups
                        .iter()
                        .map(|g| g.members.capacity() * std::mem::size_of::<usize>())
                        .sum::<usize>();
                group_count = groups.len();
            }
            times.sort_by(f64::total_cmp);
            println!(
                "UNIT_SCALE name=CounterGrouping count={count} level={level:?} groups={group_count} p50_ms={:.6} max_ms={:.6} retained_bytes={retained}",
                times[2], times[4]
            );
        }
    }
}

#[test]
fn unwrapping_moves_x_onto_the_nearest_copy_of_the_reference() {
    assert_eq!(unwrap_x(WIDTH, 0.0, 0.0), 0.0);
    assert_eq!(unwrap_x(WIDTH, 3010.0, 2956.37), 2956.37);
    assert_eq!(unwrap_x(WIDTH, 5600.0, 32.0), 32.0 + WIDTH);
    assert_eq!(unwrap_x(WIDTH, 32.0, 5600.0), 5600.0 - WIDTH);
    assert_eq!(unwrap_x(WIDTH, -40.0, 5620.0), 5620.0 - WIDTH);
    assert!(unwrap_x(WIDTH, f32::NAN, 12.0) == 12.0);
    assert!(unwrap_x(WIDTH, 12.0, f32::NAN).is_nan());
}

#[test]
fn a_group_astride_the_seam_means_across_it_rather_than_through_the_map() {
    let extents = extents(10.0, 10.0);
    let icons = [
        icon(0, 1, 100, Some(7), WIDTH - 32.0),
        icon(0, 1, 100, Some(7), 32.0),
    ];
    let groups = group(&icons, Level::State, &extents, &all_friendly);
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].anchor[0], 0.0);
    let mirrored = [
        icon(0, 1, 100, Some(7), 32.0),
        icon(0, 1, 100, Some(7), WIDTH - 32.0),
    ];
    let groups = group(&mirrored, Level::State, &extents, &all_friendly);
    assert_eq!(groups[0].anchor[0], 0.0);
}
