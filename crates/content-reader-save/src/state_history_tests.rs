use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Content(std::path::PathBuf);

impl Content {
    fn new(history: &[(&str, &str)]) -> Self {
        content_reader_script::test_game::enter();
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let content = Self(std::env::temp_dir().join(format!(
            "hoi4_content_reader_save_history_{}_{stamp}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )));
        for (path, bytes) in [
            (
                "map/default.map",
                "definitions=\"definition.csv\" provinces=\"unused.bmp\" terrain=\"unused.bmp\" \
                 rivers=\"unused.bmp\" heightmap=\"unused.bmp\" tree_definition=\"unused.csv\"",
            ),
            (
                "map/definition.csv",
                "0;0;0;0;land;false;test\n1;1;0;0;land;false;test\n2;2;0;0;land;false;test\n\
                 3;3;0;0;land;false;test\n4;4;0;0;land;false;test\n",
            ),
            (
                "common/defines/00.lua",
                "NDefines={NBuildings={MAX_BUILDING_LEVELS=20}}",
            ),
        ]
        .into_iter()
        .chain(history.iter().copied())
        {
            let path = content.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let mut bmp = vec![0_u8; 58];
        bmp[..2].copy_from_slice(b"BM");
        for (at, value) in [(2, 58_u32), (10, 54), (14, 40), (18, 1), (22, 1)] {
            bmp[at..at + 4].copy_from_slice(&value.to_le_bytes());
        }
        bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
        bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
        bmp[54..57].copy_from_slice(&[0, 0, 1]);
        std::fs::write(content.0.join("map/unused.bmp"), bmp).unwrap();
        content
    }

    fn layers(&self) -> ResolvedLayers {
        ResolvedLayers::from_folders(&[(&self.0, &[])]).unwrap()
    }
}

impl Drop for Content {
    fn drop(&mut self) {
        assert!(self.0.is_absolute());
        assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn one_unported_history_excludes_its_file_suffix_and_leaves_other_files_known() {
    let content = Content::new(&[
        (
            "history/states/bad.txt",
            "state={id=1 provinces={1} history={IF={limit={} \
             free_operative={operative=a} free_operative={operative=b}}}} \
             state={id=4 provinces={4}}",
        ),
        (
            "history/states/clean.txt",
            "state={id=2 provinces={2}} state={id=3 provinces={3}}",
        ),
    ]);
    let layers = content.layers();
    let membership = read_partial_map_membership(&layers).unwrap();
    assert_eq!(membership.known_states, BTreeSet::from([2, 3]));
    assert_eq!(membership.excluded_states.len(), 2);
    let excluded = &membership.excluded_states[&1];
    assert_eq!(excluded.file, "history/states/bad.txt");
    assert!(excluded.reasons[0].contains("Deferred effect `free_operative`"));
    assert!(excluded.reasons[0].contains("count: 2"));
    assert_eq!(
        membership.provinces[&1],
        ProvinceMembership::Unknown(BTreeSet::from([1]))
    );
    assert_eq!(membership.provinces[&2], ProvinceMembership::Known(Some(2)));
    assert_eq!(membership.provinces[&3], ProvinceMembership::Known(Some(3)));
    let later = &membership.excluded_states[&4];
    assert_eq!(later.file, excluded.file);
    assert_eq!(
        later.reasons,
        [format!(
            "after state 1's unported reader: {}",
            excluded.reasons[0]
        )]
    );
    assert_eq!(
        membership.provinces[&4],
        ProvinceMembership::Unknown(BTreeSet::from([4]))
    );
    assert!(membership.province_is_unknown(1));
}

#[test]
fn partial_base_resources_keeps_clean_rows_and_explicitly_excludes_bad_states() {
    let content = Content::new(&[
        (
            "common/resources/synthetic.txt",
            "resources={synthetic_resource={}}",
        ),
        (
            "history/states/bad.txt",
            "state={id=1 provinces={1} resources={synthetic_resource=9} \
             history={free_operative={operative=a}}}",
        ),
        (
            "history/states/clean.txt",
            "state={id=2 provinces={2} resources={synthetic_resource=3}}",
        ),
    ]);
    let layers = content.layers();
    let (membership, partial) = read_partial_state_history(&layers).unwrap();
    let partial = partial.unwrap();
    assert_eq!(partial.known.rows.len(), 1);
    assert_eq!(
        partial.known.rows[&(2, b"synthetic_resource".to_vec())],
        (0, 300_000)
    );
    assert!(membership.state_is_excluded(1));
    assert_eq!(
        membership.excluded_states[&1].file,
        "history/states/bad.txt"
    );
    assert!(membership.excluded_states[&1].reasons[0].contains("free_operative"));
    assert!(membership.province_is_unknown(1));
}

#[test]
fn an_unrepresentable_resource_declaration_does_not_discard_membership() {
    let content = Content::new(&[
        (
            "common/resources/synthetic.txt",
            "resources={synthetic_resource={}}",
        ),
        (
            "history/states/clean.txt",
            "state={id=1 provinces={1} resources={synthetic_resource=2 synthetic_resource=7}}",
        ),
    ]);
    let (membership, resources) = read_partial_state_history(&content.layers()).unwrap();
    assert!(membership.excluded_states.is_empty());
    assert_eq!(membership.provinces[&1], ProvinceMembership::Known(Some(1)));
    assert!(
        resources
            .unwrap_err()
            .to_string()
            .contains("recorded (state, resource) key")
    );
}

#[test]
fn a_diagnostic_inside_one_block_does_not_exclude_another_state_in_the_file() {
    let content = Content::new(&[(
        "history/states/two.txt",
        "state={id=1 provinces={1} impassable=maybe} state={id=2 provinces={2}}",
    )]);
    let membership = read_partial_map_membership(&content.layers()).unwrap();
    assert_eq!(
        membership
            .excluded_states
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [1]
    );
    assert!(membership.excluded_states[&1].reasons[0].contains("not yes or no"));
    assert_eq!(membership.provinces[&2], ProvinceMembership::Known(Some(2)));
}

#[test]
fn an_unattributable_diagnostic_excludes_every_state_in_its_file() {
    let content = Content::new(&[
        (
            "history/states/two.txt",
            "@a=1 @a=2 state={id=1 provinces={1}} state={id=2 provinces={2}}",
        ),
        ("history/states/clean.txt", "state={id=3 provinces={3}}"),
    ]);
    let membership = read_partial_map_membership(&content.layers()).unwrap();
    assert_eq!(
        membership
            .excluded_states
            .keys()
            .copied()
            .collect::<Vec<_>>(),
        [1, 2]
    );
    for id in [1, 2] {
        assert_eq!(
            membership.excluded_states[&id].file,
            "history/states/two.txt"
        );
        assert!(
            membership.excluded_states[&id].reasons[0].contains("variable `@a` is already taken")
        );
        assert_eq!(
            membership.provinces[&id],
            ProvinceMembership::Unknown(BTreeSet::from([id]))
        );
    }
    assert_eq!(membership.provinces[&3], ProvinceMembership::Known(Some(3)));
}

#[test]
fn a_file_whose_reading_cannot_reach_its_end_still_refuses_membership_globally() {
    let content = Content::new(&[
        (
            "history/states/unreadable.txt",
            "state={id=1 state_startup_effect={free_operative={x @y=1}} provinces={1}} \
             state={id=2 provinces={2}}",
        ),
        ("history/states/clean.txt", "state={id=3 provinces={3}}"),
    ]);
    let error = read_partial_map_membership(&content.layers())
        .unwrap_err()
        .to_string();
    assert!(error.contains("history/states/unreadable.txt"), "{error}");
    assert!(error.contains("state inventory is unavailable"), "{error}");
    assert!(
        error.contains("ids and candidate provinces cannot be determined"),
        "{error}"
    );
}

#[test]
fn a_clean_first_definition_is_exact_despite_a_later_state_id_conflict() {
    let content = Content::new(&[
        ("history/states/first.txt", "state={id=1 provinces={1}}"),
        ("history/states/second.txt", "state={id=1 provinces={2}}"),
    ]);
    let layers = content.layers();
    let membership = read_partial_map_membership(&layers).unwrap();
    assert!(membership.excluded_states.is_empty());
    assert_eq!(membership.provinces[&1], ProvinceMembership::Known(Some(1)));
    assert_eq!(membership.provinces[&2], ProvinceMembership::Known(None));
    assert_eq!(membership.notes.len(), 1);
    assert!(membership.notes[0].contains("the game keeps the first"));
}

#[test]
fn state_control_compares_excluded_states_while_province_control_keeps_explicit_controllers() {
    let content = Content::new(&[(
        "history/states/two.txt",
        "state={id=1 provinces={1} impassable=maybe} state={id=2 provinces={2}}",
    )]);
    let membership = read_partial_map_membership(&content.layers()).unwrap();
    let mut saved = parse(
        br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={AAA={}} states={1={owner=AAA} 2={owner=AAA}} provinces={1={controller=AAA}}
"#,
    )
    .unwrap();
    let rows = saved.known_state_control(&membership).unwrap();
    assert_eq!(rows.iter().map(|row| row.id).collect::<Vec<_>>(), [1, 2]);
    let rows = saved.province_control(&membership).unwrap();
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        [1, 2, 3, 4]
    );
    assert_eq!(rows[0].owner, None);
    assert!(!rows[0].membership_known);
    assert_eq!(rows[0].state, None);
    assert_eq!(rows[0].controller.as_deref(), Some(b"AAA".as_slice()));
    assert!(rows[0].controller_explicit);
    assert_eq!(rows[1].owner.as_deref(), Some(b"AAA".as_slice()));
    assert!(rows[1].membership_known);
    assert_eq!(rows[1].state, Some(2));
    assert!(rows[2].membership_known);
    assert_eq!(rows[2].state, None);
    assert_eq!(rows[2].owner, None);
    saved.provinces.get_mut(&1).unwrap().owner = Some(b"AAA".to_vec());
    assert!(
        saved
            .province_control(&membership)
            .unwrap_err()
            .to_string()
            .contains("unsupported per-province owner")
    );
    saved.provinces.get_mut(&1).unwrap().owner = None;
    saved.provinces.get_mut(&1).unwrap().controller = Some(b"missing".to_vec());
    assert!(
        saved
            .province_control(&membership)
            .unwrap_err()
            .to_string()
            .contains("province 1 references unknown saved country missing")
    );
    saved.provinces.get_mut(&1).unwrap().controller = None;
    let rows = saved.province_control(&membership).unwrap();
    assert_eq!(rows[0].controller, None);
    assert!(!rows[0].controller_explicit);
    saved.states.get_mut(&1).unwrap().owner = None;
    assert!(
        saved
            .known_state_control(&membership)
            .unwrap_err()
            .to_string()
            .contains("state 1 lacks its owner")
    );
    assert!(saved.province_control(&membership).is_ok());
    saved.states.get_mut(&1).unwrap().owner = Some(b"AAA".to_vec());
    saved.states.get_mut(&2).unwrap().owner = None;
    assert!(
        saved
            .known_state_control(&membership)
            .unwrap_err()
            .to_string()
            .contains("state 2 lacks its owner")
    );
    let rows = saved.province_control(&membership).unwrap();
    assert_eq!(rows[1].owner, None);
    assert_eq!(rows[1].controller, None);
}

#[test]
fn an_unknown_shared_province_does_not_erase_the_clean_state_inventory() {
    let content = Content::new(&[(
        "history/states/two.txt",
        "state={id=1 provinces={1} impassable=maybe} state={id=2 provinces={1}}",
    )]);
    let membership = read_partial_map_membership(&content.layers()).unwrap();
    assert_eq!(membership.known_states, BTreeSet::from([2]));
    assert!(membership.province_is_unknown(1));
    assert_eq!(
        membership.provinces[&1],
        ProvinceMembership::Unknown(BTreeSet::from([1]))
    );
}

#[test]
fn clean_overlaps_are_refused_even_when_the_province_has_an_excluded_candidate() {
    let content = Content::new(&[(
        "history/states/overlap.txt",
        "state={id=1 provinces={1} impassable=maybe} \
         state={id=2 provinces={1}} state={id=3 provinces={1 2}}",
    )]);
    let error = read_partial_map_membership(&content.layers())
        .unwrap_err()
        .to_string();
    assert!(error.contains("already assigned province 1"), "{error}");

    let content = Content::new(&[(
        "history/states/overlap.txt",
        "state={id=1 provinces={1} impassable=maybe} \
         state={id=2 provinces={1 2}} state={id=3 provinces={2}}",
    )]);
    let error = read_partial_map_membership(&content.layers())
        .unwrap_err()
        .to_string();
    assert!(error.contains("already assigned province 2"), "{error}");
}

#[test]
fn static_membership_rejects_missing_and_duplicate_province_assignments() {
    let mut map = MapMembership::from_definition(b"0;0;0;0\n1;1;2;3\n2;3;2;1\n3;4;5;6\n").unwrap();
    map.add_state_history(&history_fixture(b"state={id=5 provinces={1 2}}"))
        .unwrap();
    assert_eq!(map.provinces[&0], None);
    assert_eq!(map.provinces[&1], Some(5));
    assert_eq!(map.provinces[&3], None);
    let original = map.clone();
    for bad in [
        "state={id=6 provinces={2}}",
        "state={id=6 provinces={4}}",
        "state={id=5 provinces={3}}",
    ] {
        assert!(
            map.add_state_history(&history_fixture(bad.as_bytes()))
                .is_err()
        );
        assert_eq!(map, original);
    }
}

#[test]
fn full_province_control_uses_only_proven_defaults_and_refuses_missing_ownership() {
    let mut save = parse(
        br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={ GER={} D01={} } states={1={owner=GER}} provinces={2={controller=D01}}
equipments={convoy_1={id={id=1 type=70}} convoy_1={id={id=2 type=70}}}
"#,
    )
    .unwrap();
    let mut map = MapMembership::from_definition(b"0;0;0;0\n1;1;2;3\n2;3;2;1\n3;4;5;6\n").unwrap();
    map.add_state_history(&history_fixture(b"state={id=1 provinces={1 2}}"))
        .unwrap();
    let map = PartialMapMembership::from(map);
    let rows = save.province_control(&map).unwrap();
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row.membership_known));
    assert_eq!(rows[0].controller.as_deref(), Some(b"GER".as_slice()));
    assert!(!rows[0].controller_explicit);
    assert_eq!(rows[1].controller.as_deref(), Some(b"D01".as_slice()));
    assert!(rows[1].controller_explicit);
    assert_eq!(rows[2].owner, None);
    assert_eq!(rows[2].controller, None);
    assert_eq!(save.equipment_definitions.len(), 2);
    save.states.get_mut(&1).unwrap().owner = None;
    let rows = save.province_control(&map).unwrap();
    assert_eq!(rows[0].owner, None);
    assert_eq!(rows[0].controller, None);
    assert_eq!(rows[1].controller.as_deref(), Some(b"D01".as_slice()));
    save.states.remove(&1);
    assert!(
        save.province_control(&map)
            .unwrap_err()
            .to_string()
            .contains("lacks state")
    );
}

#[test]
fn synthesized_unclaimed_bmp_province_is_accepted_by_state_history_and_save_control() {
    let content = Content::new(&[(
        "history/states/unclaimed.txt",
        "state={id=1 provinces={1 5}}",
    )]);
    let mut bmp = vec![0_u8; 62];
    bmp[..2].copy_from_slice(b"BM");
    for (at, value) in [(2, 62_u32), (10, 54), (14, 40), (18, 2), (22, 1)] {
        bmp[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    bmp[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bmp[54..60].copy_from_slice(&[0, 0, 1, 7, 8, 9]);
    std::fs::write(content.0.join("map/unused.bmp"), bmp).unwrap();

    let membership = read_partial_map_membership(&content.layers()).unwrap();
    assert_eq!(membership.known_states, BTreeSet::from([1]));
    assert_eq!(membership.provinces[&5], ProvinceMembership::Known(Some(1)));

    let save = parse(
        br#"HOI4txt
date=1936.1.1.1 game_unique_id=c version=b game_unique_seed=0 debug_current_ref_id=1 unit=2 order_index=3 front_index=4 theatre_index=5 theater_group_index=6 military_deployment_line_index=7 military_deployment_conveyor_index=8 equipment_variant_index=9 country_leader_index=10 navy_id=11 land_combat_id=12 multiplayer_random_count=-13
countries={ GER={} D01={} } states={1={owner=GER}} provinces={5={controller=D01}}
"#,
    )
    .unwrap();
    let rows = save.province_control(&membership).unwrap();
    let synthesized = rows.iter().find(|row| row.id == 5).unwrap();
    assert!(synthesized.membership_known);
    assert_eq!(synthesized.owner.as_deref(), Some(b"GER".as_slice()));
    assert_eq!(synthesized.controller.as_deref(), Some(b"D01".as_slice()));
    assert!(synthesized.controller_explicit);
}
