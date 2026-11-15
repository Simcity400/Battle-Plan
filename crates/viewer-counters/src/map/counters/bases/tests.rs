use super::*;
use viewer_foundation::map::province_map::ProvinceMap;

/// Every slot admitted, for tests about the templates rather than the loader.
fn admitted(_: &SpawnPoint, _: &positions::BuildingPosition, _: Option<u32>) -> SlotAdmission {
    SlotAdmission::Admitted
}

fn slot(kind: &str, state: u32, x: f32) -> positions::BuildingPosition {
    positions::BuildingPosition {
        state,
        kind: kind.into(),
        x,
        y: 2.0,
        z: 0.5,
        rotation: 0.0,
        adjacent_sea: 0,
    }
}

/// A frame whose only table is `LexicalTokens`, holding these tokens in order.
fn token_frame(literals: &[(u32, &str)]) -> DecodedFrame {
    use replay_data::{rows::LexicalTokenRow, rows::TableId, rows::TableRows};
    use replay_frame::{frame::Metrics, frame::TableView};
    let mut frame = DecodedFrame {
        branch: 0,
        hour: 7,
        tables: std::array::from_fn(|_| {
            TableView::Unavailable {
                reason: "not needed".into(),
            }
            .into()
        }),
        metrics: Metrics::default(),
    };
    *frame.tables[TableId::LexicalTokens.index()] = TableView::Complete(TableRows::LexicalTokens(
        literals
            .iter()
            .map(|&(token, literal)| LexicalTokenRow {
                token,
                literal: literal.as_bytes().to_vec(),
            })
            .collect(),
    ));
    frame
}

/// The tokens carried across copies of LexicalTokens equal the full pass over each copy: an equal
/// new copy, a grown table (a name resolved anew, a second token for a resolved name), a rewritten
/// text, a fallen count and a branch change.
#[test]
fn building_tokens_carried_across_copies_equal_the_full_pass() {
    let registry = ["naval_hq", "facility", "dock", "absent"].map(String::from);
    let keys = TokenKeys::new(&registry);
    let needed = BTreeSet::from(["naval_hq", "facility", "dock", "absent"]);
    let carry = TokenCarry::default();
    let copies: [(replay_data::BranchId, &[(u32, &str)]); 7] = [
        (1, &[(3, "Naval_HQ"), (5, "facility"), (9, "other")]),
        (1, &[(3, "Naval_HQ"), (5, "facility"), (9, "other")]),
        (1, &[(3, "Naval_HQ"), (4, "dock"), (5, "facility"), (9, "other"), (12, "FACILITY")]),
        (1, &[(3, "Naval_HQ"), (4, "dock"), (5, "facility"), (9, "renamed"), (12, "FACILITY")]),
        (1, &[(3, "Naval_HQ"), (5, "facility"), (9, "renamed")]),
        (2, &[(3, "dock"), (5, "facility")]),
        (2, &[(3, "dock"), (5, "facility"), (7, "absent")]),
    ];
    for (at, (branch, literals)) in copies.into_iter().enumerate() {
        let mut frame = token_frame(literals);
        frame.branch = branch;
        let whole = building_tokens(&frame, &keys, &needed).unwrap();
        let carried = carry.tokens(&frame, &keys, &needed).unwrap();
        assert_eq!(*carried, whole, "copy {at}");
    }
}

#[test]
fn building_tokens_fold_like_the_lexer_and_refuse_ambiguity() {
    let frame = token_frame(&[(3, "Naval_HQ"), (5, "facility"), (8, "FACILITY")]);
    let registry = ["naval_hq", "facility", "absent"].map(String::from);
    let (found, refused) = building_tokens(
        &frame,
        &TokenKeys::new(&registry),
        &BTreeSet::from(["naval_hq", "facility", "absent"]),
    )
    .unwrap();
    assert_eq!(found, BTreeMap::from([("naval_hq".to_owned(), 3)]));
    assert_eq!(refused.len(), 1, "{refused:?}");
    assert!(refused.iter().next().unwrap().contains("facility"));
}

/// Lexer-fold uniqueness is checked across the whole building registry: an ordinary type
/// equal to a naval-HQ or facility type under the fold shares its rows' token, so a row
/// carrying that token is refused by name instead of read as the icon's building.
#[test]
fn a_type_sharing_its_lexer_token_with_an_ordinary_type_is_refused() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let buildings = installed_content_core::buildings::parse(
        b"buildings={
            Dock={naval_headquarter=yes} dock={show_on_map=1}
            Yard={specialization={specialization_air}} yard={show_on_map=1}
            hq={naval_headquarter=yes} fac={specialization={specialization_naval}}
        }",
        &installed_content_core::key_names::KeyNames::default(),
    )
    .unwrap();
    let provinces = ProvinceMap {
        width: 1,
        height: 1,
        ids: &[7],
    };
    let icons = BuildingIconContent::load(&buildings, &[], &provinces, admitted, [[0.0; 3]; 2]);
    let hq_names: Vec<&str> = icons
        .headquarters
        .iter()
        .map(|template| template.name.as_str())
        .collect();
    assert_eq!(hq_names, ["hq"]);
    assert_eq!(icons.facilities.keys().collect::<Vec<_>>(), ["fac"]);
    assert_eq!(icons.names(), BTreeSet::from(["Dock", "Yard", "hq", "fac"]));

    // The ordinary types' tokens: the lexer gives `Dock` and `dock` one token.
    let frame = token_frame(&[(1, "dock"), (2, "YARD"), (3, "hq"), (4, "fac")]);
    let (tokens, refused) = building_tokens(&frame, &icons.keys, &icons.names()).unwrap();
    assert_eq!(
        tokens,
        BTreeMap::from(
            [("Dock", 1), ("Yard", 2), ("hq", 3), ("fac", 4)]
                .map(|(name, token)| (name.to_owned(), token))
        )
    );
    assert_eq!(
        refused,
        BTreeSet::from([
            "building token 1 (dock) names several building types: Dock, dock".to_owned(),
            "building token 2 (YARD) names several building types: Yard, yard".to_owned(),
        ])
    );

    // An ordinary `dock` row is not read as the HQ; the province refuses by name.
    let (dock, hq) = (building(7, 1, 0, 1), building(7, 3, 1, 1));
    assert_eq!(
        icons.headquarter(&[&dock], &tokens).unwrap_err(),
        "naval HQ building Dock shares its lexer token with building type dock"
    );
    assert_eq!(
        icons.headquarter(&[&dock, &hq], &tokens).unwrap_err(),
        "naval HQ building Dock shares its lexer token with building type dock"
    );
    let found = icons.headquarter(&[&hq], &tokens).unwrap().unwrap();
    assert_eq!(found.0.name, "hq");

    // The same for the facility: an ordinary `yard` row reached first refuses.
    let names: BTreeMap<u32, &str> = icons
        .facility_names()
        .filter_map(|name| Some((*tokens.get(name)?, name)))
        .collect();
    let (yard, fac) = (building(7, 2, 0, 1), building(7, 4, 1, 1));
    assert_eq!(
        icons.facility([&yard, &fac], &names).unwrap_err(),
        "facility building Yard shares its lexer token with building type yard"
    );
    let (row, (first, _)) = icons.facility([&fac, &yard], &names).unwrap().unwrap();
    assert_eq!(
        (row.building_type, first.as_str()),
        (4, "specialization_naval")
    );

    // Without the shared token, an unnamed row could be either refused type.
    let unnamed: BTreeMap<String, u32> = [("hq".to_owned(), 3), ("fac".to_owned(), 4)].into();
    assert_eq!(
        icons.headquarter(&[&hq], &unnamed).unwrap_err(),
        "naval HQ building Dock has no single token this hour"
    );
    assert_eq!(icons.unnamed_facility(&unnamed), Some("Yard"));
}

/// Build the port buckets from every record, so a bucket only an earlier
/// declaration of a repeated port redirects into is still a port bucket, and ambiguous:
/// the HQ anchored there is refused before it can fall back to slot 0. The earlier
/// redirects are spelled in another case and still bind by token.
#[test]
fn a_bucket_a_repeated_declaration_reaches_refuses_before_the_fallback() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let buildings = installed_content_core::buildings::parse(
        b"buildings={
            port={is_port=yes spawn_point=SHARED}
            hq={naval_headquarter=yes spawn_point=shared}
            fac={specialization={specialization_air} spawn_point=facility_spawn}
            port={}
            yard={specialization={specialization_naval} spawn_point=Facility_Spawn}
            yard={specialization={specialization_naval}}
            hq_elsewhere={naval_headquarter=yes spawn_point=lone}
        }
        spawn_points={shared={} facility_spawn={} lone={}}",
        &installed_content_core::key_names::KeyNames::default(),
    )
    .unwrap();
    // The last `port` declaration neither is a port nor redirects.
    assert!(!buildings.types["port"].is_port);
    assert_eq!(buildings.types["port"].spawn_point, None);
    let provinces = ProvinceMap {
        width: 1,
        height: 1,
        ids: &[7],
    };
    let slots = [
        slot("shared", 1, 0.5),
        slot("facility_spawn", 1, 0.5),
        slot("lone", 1, 0.5),
    ];
    let icons = BuildingIconContent::load(&buildings, &slots, &provinces, admitted, [[0.0; 3]; 2]);
    let hq = icons
        .headquarters
        .iter()
        .find(|template| template.name == "hq")
        .unwrap();
    let slot0 = Some([9.0; 3]);
    assert_eq!(
        icons.anchor(7, hq, slot0).unwrap_err(),
        "spawn bucket shared of hq is reached by a building or bucket declared more than \
         once, differently"
    );
    // The same through the specialization buckets: `yard` is refused by name and its
    // earlier redirect makes the facility's shared bucket ambiguous.
    assert!(icons.refused_facilities.contains_key("yard"));
    let (_, fac) = &icons.facilities["fac"];
    assert_eq!(
        icons.anchor(7, fac, slot0).unwrap_err(),
        "spawn bucket facility_spawn of fac is reached by a building or bucket declared \
         more than once, differently"
    );
    // No port declaration reaches `lone`: that HQ takes slot 0, as native does.
    let elsewhere = icons
        .headquarters
        .iter()
        .find(|template| template.name == "hq_elsewhere")
        .unwrap();
    assert_eq!(icons.anchor(7, elsewhere, slot0), Ok([9.0; 3]));
}

#[test]
fn ships_need_a_seen_occupant_or_port_detail() {
    assert_eq!(naval_base_ships(&[], || None, |_| None), Some(false));
    let seen = |unit| Some(if unit == 2 { 0x96 } else { 0 });
    assert_eq!(naval_base_ships(&[1, 2], || None, seen), Some(true));
    assert_eq!(naval_base_ships(&[1], || Some(2), |_| Some(0)), Some(true));
    assert_eq!(
        naval_base_ships(&[1], || Some(1), |_| Some(0x95)),
        Some(false)
    );
    assert_eq!(naval_base_ships(&[1], || Some(1), |_| None), None);
    assert_eq!(naval_base_ships(&[1], || None, |_| Some(0)), None);
    // A truncated `UnitIntel` omitting a spotted foreign occupant (unit 2): its byte is
    // unknown, so a recorded unseen occupant and port detail below 2 decide nothing...
    let truncated = |unit| (unit == 1).then_some(0x95);
    assert_eq!(naval_base_ships(&[1, 2], || Some(1), truncated), None);
    // ...while a seen occupant or port detail 2 still proves the ships glyph.
    let spotted = |unit| (unit == 1).then_some(0x96);
    assert_eq!(naval_base_ships(&[1, 2], || Some(0), spotted), Some(true));
    assert_eq!(naval_base_ships(&[1, 2], || Some(2), truncated), Some(true));
}

fn building(location: u32, building_type: u32, ordinal: u32, level: i16) -> BuildingRow {
    BuildingRow {
        location,
        building_type,
        ordinal,
        level,
        ..Default::default()
    }
}

/// A naval-HQ or facility type declared twice differently, without a `spawn_point`, is
/// refused by name wherever it may decide the icon, even when its last declaration drops
/// the classification; provinces it cannot reach keep their decision.
#[test]
fn repeated_icon_templates_are_refused_by_name() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let buildings = installed_content_core::buildings::parse(
        b"buildings={
            hq={naval_headquarter=yes} later_hq={naval_headquarter=yes}
            dock={specialization={specialization_naval}} yard={specialization={specialization_air}}
            hq={show_on_map=1}
            yard={specialization={specialization_air} show_on_map=2}
        }",
        &installed_content_core::key_names::KeyNames::default(),
    )
    .unwrap();
    let provinces = ProvinceMap {
        width: 1,
        height: 1,
        ids: &[7],
    };
    let icons = BuildingIconContent::load(&buildings, &[], &provinces, admitted, [[0.0; 3]; 2]);
    // The repeats are out of both lists but still looked up by token.
    assert_eq!(
        icons
            .headquarters
            .iter()
            .map(|template| template.name.as_str())
            .collect::<Vec<_>>(),
        ["later_hq"]
    );
    assert_eq!(icons.facilities.keys().collect::<Vec<_>>(), ["dock"]);
    assert_eq!(
        icons.names(),
        BTreeSet::from(["hq", "later_hq", "dock", "yard"])
    );
    let tokens: BTreeMap<String, u32> = [("hq", 1), ("later_hq", 2), ("dock", 3), ("yard", 4)]
        .map(|(name, token)| (name.to_owned(), token))
        .into();
    // `hq`'s last declaration is no HQ, but its first was: a province holding it refuses.
    let (hq, later) = (building(7, 1, 0, 1), building(7, 2, 1, 1));
    assert_eq!(
        icons.headquarter(&[&hq, &later], &tokens).unwrap_err(),
        "naval HQ building hq is declared more than once, differently"
    );
    let found = icons.headquarter(&[&later], &tokens).unwrap().unwrap();
    assert_eq!(found.0.name, "later_hq");
    // An inactive repeat decides nothing.
    let razed = building(7, 1, 0, 0);
    assert!(
        icons
            .headquarter(&[&razed, &later], &tokens)
            .unwrap()
            .is_some()
    );
    // Without a token for the repeat, an unnamed row could be it.
    let unnamed: BTreeMap<String, u32> = [("later_hq".to_owned(), 2)].into();
    assert_eq!(
        icons.headquarter(&[&later], &unnamed).unwrap_err(),
        "naval HQ building hq has no single token this hour"
    );
    // The facility is the first active specialized building in recorded building order.
    let names: BTreeMap<u32, &str> = tokens
        .iter()
        .map(|(name, token)| (*token, name.as_str()))
        .collect();
    let (yard, dock) = (building(7, 4, 0, 1), building(7, 3, 1, 1));
    assert_eq!(
        icons.facility([&yard, &dock], &names).unwrap_err(),
        "facility building yard is declared more than once, differently"
    );
    let (row, (first, _)) = icons.facility([&dock, &yard], &names).unwrap().unwrap();
    assert_eq!(
        (row.building_type, first.as_str()),
        (3, "specialization_naval")
    );
    assert_eq!(icons.unnamed_facility(&tokens), None);
    assert_eq!(icons.unnamed_facility(&unnamed), Some("dock"));
}

/// Bucket names are lexer tokens: a redirect and a slot name spelled in another case
/// reach the declared bucket, for the HQ and the facility.
#[test]
fn icons_anchor_only_at_buckets_their_kind_redirects_into() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    let buildings = installed_content_core::buildings::parse(
        b"buildings={
            port={is_port=yes spawn_point=Port_Spawn}
            hq_at_port={naval_headquarter=yes only_costal=yes spawn_point=port_spawn}
            hq={naval_headquarter=yes only_costal=yes}
            facility={specialization={specialization_air second} spawn_point=FACILITY_SPAWN}
        }
        spawn_points={port_spawn={} Facility_Spawn={}}",
        &installed_content_core::key_names::KeyNames::default(),
    )
    .unwrap();
    let ids = [7, 8];
    let provinces = ProvinceMap {
        width: 2,
        height: 1,
        ids: &ids,
    };
    let slots = [
        slot("facility_spawn", 1, 1.5),
        slot("PORT_SPAWN", 1, 0.5),
        slot("port_spawn", 1, 0.7),
    ];
    let icons = BuildingIconContent::load(
        &buildings,
        &slots,
        &provinces,
        admitted,
        [[1.0, 0.0, 1.0], [-1.0, 0.0, 1.0]],
    );
    let names: Vec<&str> = icons
        .headquarters
        .iter()
        .map(|template| template.name.as_str())
        .collect();
    assert_eq!(names, ["hq_at_port", "hq"]);
    let slot0 = Some([9.0; 3]);
    // The first record of the redirected bucket, with the coastal offset.
    assert_eq!(
        icons.anchor(7, &icons.headquarters[0], slot0),
        Ok([1.5, 2.0, 1.5])
    );
    assert_eq!(icons.anchor(8, &icons.headquarters[0], slot0), Ok([9.0; 3]));
    assert_eq!(icons.anchor(7, &icons.headquarters[1], slot0), Ok([9.0; 3]));
    assert!(icons.anchor(7, &icons.headquarters[1], None).is_err());
    let (first, facility) = &icons.facilities["facility"];
    assert_eq!(first, "specialization_air");
    // Not coastal: no offset.
    assert_eq!(icons.anchor(8, facility, slot0), Ok([1.5, 2.0, 0.5]));
}

/// The anchors search only the records the game admits, by
/// the building layer's own rules: a rejected first record gives way to the next
/// admitted one, and a bucket with none left falls back to slot 0.
#[test]
fn icons_anchor_only_at_records_the_loader_admits() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use installed_content_core::{
        map::CountryColors, map::MapData, map::ProvinceDefinition, map::ProvinceKind, mesh::Roster,
    };
    use installed_content_states::states::StateDefinition;
    let buildings = installed_content_core::buildings::parse(
        b"buildings={
            port={is_port=yes spawn_point=port_spawn}
            hq={naval_headquarter=yes spawn_point=port_spawn}
            facility={specialization={specialization_air} spawn_point=facility_spawn}
            lone_hq={naval_headquarter=yes spawn_point=lone_spawn}
            lone_port={is_port=yes spawn_point=lone_spawn}
        }
        spawn_points={port_spawn={} facility_spawn={} lone_spawn={}}",
        &installed_content_core::key_names::KeyNames::default(),
    )
    .unwrap();
    let definition = |id, kind| ProvinceDefinition {
        id,
        rgb: [0, 0, 0],
        kind,
        continent: 0,
    };
    let map = MapData {
        width: 2,
        height: 1,
        province_ids: vec![7, 8],
        definitions: BTreeMap::from([
            (7, definition(7, ProvinceKind::Land)),
            (8, definition(8, ProvinceKind::Sea)),
        ]),
        country_colors: CountryColors::default(),
    };
    let states = BTreeMap::from([(
        3,
        StateDefinition {
            id: 3,
            provinces: vec![7],
            ..Default::default()
        },
    )]);
    let mut roster = Roster::default();
    for name in [
        "building_port_spawn",
        "building_facility_spawn",
        "building_lone_spawn",
    ] {
        roster
            .descriptors
            .entities
            .insert(name.into(), Default::default());
    }
    let provinces = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    let slots = [
        // Rejected: state 99 does not exist.
        slot("port_spawn", 99, 0.1),
        slot("PORT_SPAWN", 3, 0.2),
        // Rejected: province 7 is not in state 4, which does not exist either.
        slot("facility_spawn", 4, 0.3),
        slot("facility_spawn", 3, 0.4),
        // Every `lone_spawn` record is rejected: the first names no state, the second
        // stands on the sea province 8.
        slot("lone_spawn", 99, 0.5),
        slot("lone_spawn", 3, 1.5),
    ];
    let icons = BuildingIconContent::load(
        &buildings,
        &slots,
        &provinces,
        |spawn, slot, province| {
            viewer_foundation::map::building_facts::admit_slot(
                spawn, slot, province, &map, &states, &roster,
            )
        },
        [[0.0; 3]; 2],
    );
    let template = |name: &str| {
        icons
            .headquarters
            .iter()
            .find(|template| template.name == name)
            .unwrap()
    };
    let slot0 = Some([9.0; 3]);
    assert_eq!(icons.anchor(7, template("hq"), slot0), Ok([0.2, 2.0, 0.5]));
    let (_, facility) = &icons.facilities["facility"];
    assert_eq!(icons.anchor(7, facility, slot0), Ok([0.4, 2.0, 0.5]));
    assert_eq!(icons.anchor(7, template("lone_hq"), slot0), Ok([9.0; 3]));
    assert_eq!(icons.anchor(8, template("lone_hq"), slot0), Ok([9.0; 3]));
    // A slot the position test rejects is rejected, its bucket registered or not.
    let spawn = &buildings.spawn_points["port_spawn"];
    let admit = |slot| {
        viewer_foundation::map::building_facts::admit_slot(
            spawn,
            slot,
            Some(7),
            &map,
            &states,
            &roster,
        )
    };
    assert_eq!(admit(&slots[0]), SlotAdmission::Outside);
    assert_eq!(admit(&slots[1]), SlotAdmission::Admitted);
}

#[test]
fn base_prohibition_lists_keep_raw_zero_and_nonzero_country_aliases() {
    let links = BTreeMap::from([(11, 1), (99, 1), (22, 2)]);
    let slot_of = |raw| links.get(&raw).copied();
    assert_eq!(country_list_contains(&[0], 0, slot_of), Ok(true));
    assert_eq!(country_list_contains(&[11], 0, slot_of), Ok(false));
    assert_eq!(country_list_contains(&[99], 11, slot_of), Ok(true));
    assert_eq!(country_list_contains(&[22], 11, slot_of), Ok(false));
    assert!(country_list_contains(&[88], 11, slot_of).is_err());
}

#[test]
fn cached_base_joins_equal_fresh_hour_derivations() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use replay_data::rows::{AirBaseRow, ProvinceRow, TableRows};
    use viewer_foundation::map::test_support::{test_frame, test_put};
    super::super::tests::with_counter_inputs_fixture(
        super::super::tests::action_gui(false),
        |_| {},
        |_| {},
        |inputs| {
            let mut frame = test_frame(1);
            test_put(&mut frame, TableRows::CountryLinks(Vec::new()));
            test_put(
                &mut frame,
                TableRows::Provinces(vec![ProvinceRow {
                    id: 9,
                    controller: 5,
                    ..Default::default()
                }]),
            );
            test_put(
                &mut frame,
                TableRows::AirBases(
                    [0, 1, 2, 0]
                        .map(|kind| AirBaseRow {
                            id: kind as u64 + 1,
                            state: Some(7),
                            kind,
                            capacity: 100,
                            ..Default::default()
                        })
                        .into(),
                ),
            );
            let vision = Vision::default();
            let cached = BaseTables::read(&frame, &vision, inputs.content).unwrap();
            let cached = &cached.values;
            assert_eq!(cached.wings, wings_per_site(&frame, &vision).unwrap().into_iter().collect());
            let states = site_icon_states(&frame).unwrap().0;
            assert!(cached.complete);
            for kind in 0..3 {
                assert!(cached.wings.iter().filter(|((_, of), _)| *of == kind).map(|((state, _), _)| *state).eq(states[kind].iter().copied()), "a site has a count exactly while its state holds a base of its kind");
            }
            let radar = radar_states(&frame, vision.ours()).unwrap();
            assert!(cached.radar.iter().map(|(state, _)| *state).eq(radar.shown.iter().copied()));
            assert_eq!(*cached.radar_notes, radar.notes);
            assert_eq!(cached.controlled.get(&9), Some(&true));
            assert_eq!(cached.controlled.get(&10), None);
            assert_eq!(
                cached.site_bases[&(7, 0)],
                [1, 1],
                "repeated base rows stay repeated"
            );
        },
    );
}

#[test]
fn memoized_building_tokens_equal_a_fresh_build_hour_by_hour() {
    use replay_data::{rows::LexicalTokenRow, rows::TableRows};
    use replay_frame::frame::TableView;
    let registry = ["naval_hq", "facility", "dock"].map(String::from);
    let template = |name: &str| IconTemplate {
        name: name.into(),
        anchor: None,
        ambiguous: false,
    };
    let icons = BuildingIconContent {
        headquarters: vec![template("naval_hq")],
        facilities: BTreeMap::from([(
            "facility".to_owned(),
            ("specialization_naval".to_owned(), template("facility")),
        )]),
        keys: TokenKeys::new(&registry),
        ..Default::default()
    };
    let at = |frame: &DecodedFrame, hour| DecodedFrame {
        hour,
        ..frame.clone()
    };
    let rows = |literals: &[(u32, &str)]| {
        TableRows::LexicalTokens(
            literals
                .iter()
                .map(|&(token, literal)| LexicalTokenRow {
                    token,
                    literal: literal.as_bytes().to_vec(),
                })
                .collect(),
        )
    };
    let with = |view: TableView| {
        let mut frame = token_frame(&[]);
        *frame.tables[replay_data::rows::TableId::LexicalTokens.index()] = view;
        frame
    };
    let first = token_frame(&[(3, "Naval_HQ"), (5, "facility"), (8, "FACILITY")]);
    let unsorted = token_frame(&[(8, "FACILITY"), (3, "Naval_HQ")]);
    let hours = [
        at(&first, 1),
        at(&first, 2),
        token_frame(&[(3, "Naval_HQ"), (5, "facility"), (8, "FACILITY")]),
        token_frame(&[(3, "Naval_HQ"), (5, "facility")]),
        at(&unsorted, 5),
        at(&unsorted, 6),
        with(TableView::Unavailable {
            reason: "not read this hour".into(),
        }),
        with(TableView::Truncated {
            rows: rows(&[(3, "naval_hq")]),
            missing: 2,
        }),
        at(&first, 9),
    ];
    let answer = |built: &io::Result<Arc<BuildingTokens>>| match built {
        Ok(built) => Ok(BuildingTokens::clone(built)),
        Err(error) => Err(error.to_string()),
    };
    let mut kept: Vec<Arc<BuildingTokens>> = Vec::new();
    for (index, frame) in hours.iter().enumerate() {
        let fresh = building_tokens(frame, &icons.keys, &icons.names()).map(Arc::new);
        let memo = icons.tokens(frame);
        assert_eq!(answer(&memo), answer(&fresh), "hour {index}");
        if let Ok(memo) = memo {
            kept.push(memo);
        }
    }
    assert!(Arc::ptr_eq(&kept[0], &kept[1]));
    assert!(!Arc::ptr_eq(&kept[0], &kept[2]));
    assert_eq!(kept[0], kept[2]);
    assert_eq!(
        kept[0].0,
        BTreeMap::from([("naval_hq".to_owned(), 3)]),
        "the refused facility has no token"
    );
    assert_eq!(kept[3].0.get("facility"), Some(&5));
    for hour in [5, 6] {
        let refusal = answer(&icons.tokens(&hours[hour - 1])).unwrap_err();
        assert!(refusal.contains(&format!("hour {hour} ")), "{refusal}");
    }
}

#[test]
fn a_new_bases_product_lays_out_the_changed_base_alone_and_is_held_back_while_none_is_drawn() {
    engine_executable::executable_tables::initialize_for_tests()
        .expect("BATTLE_PLAN_GAME_DIR must name the supported installation for runtime table tests");
    use crate::map::counters::preparation::plan_probe;
    use building_facts::{AIR_BASE, ICON_KINDS, IconAnchor, RADAR, SiteIcon};
    use replay_data::rows::{TableId, TableRows};
    use replay_frame::frame::TableView;
    const STATES: u32 = 200;
    super::super::tests::with_counter_inputs_fixture(
        super::super::tests::action_gui(false),
        |_| {},
        |content| {
            let mut icons: [BTreeMap<u32, SiteIcon>; ICON_KINDS] = Default::default();
            for state in 1..=STATES {
                icons[AIR_BASE].insert(state * 10, SiteIcon { state, anchor: Ok(IconAnchor::Record([state as f32 / 100., 9.5, 2.])) });
                icons[RADAR].insert(state * 10 + 1, SiteIcon { state, anchor: Ok(IconAnchor::Record([state as f32 / 100., 9.5, 3.])) });
            }
            content.site_icons = Arc::new(icons);
        },
        |mut inputs| {
            let frame = DecodedFrame { branch: 1, hour: 1, metrics: Default::default(), tables: TableId::ALL.map(|table| TableView::Complete(TableRows::empty(table)).into()) };
            let moves = viewer_foundation::map::movement_reads::test_support::unnamed_assets();
            let tables = hour_tables(
                &frame,
                inputs.map,
                Arc::new(Vision::sharing(1, [])),
                inputs.content,
                inputs.assets,
                inputs.equipment,
                inputs.localisation,
                &Default::default(),
                None,
                None,
                None,
                &type_tint::Memo::new(type_tint::TABLES),
                &MissionMemo::new(),
                moves.movement_reads(),
                Err("unreached synthetic naval topology".into()),
            )
            .unwrap();
            let count = |planes| AirBaseCount { capacity: 10, planes, shown: true, stationed: Some(true), ..Default::default() };
            let old = Arc::new(BaseValues {
                wings: (1..=STATES).map(|state| ((state, AIR_BASE), count(5))).collect(),
                complete: true,
                aircraft_complete: true,
                controlled: (1..=STATES).map(|state| (state * 10, true)).collect(),
                ..Default::default()
            });
            let mut one_base = (*old).clone();
            one_base.wings.insert((77, AIR_BASE), AirBaseCount { shown: false, ..count(9) });
            let one_base = Arc::new(one_base);
            let mut one_province = (*one_base).clone();
            one_province.controlled.insert(1230, false);
            let one_province = Arc::new(one_province);
            let mut one_radar = (*one_province).clone();
            one_radar.radar.insert(5, ());
            let one_radar = Arc::new(one_radar);
            let rules = [true; 3];
            let (layout, counts) = plan_probe::collect(|| BaseLayout::new(rules, &inputs, &frame, &tables, Some(Arc::clone(&old))).unwrap());
            assert_eq!(counts.site_icons, 2 * u64::from(STATES), "a first layout lays out every air base and radar icon");
            let mut layout = layout;
            for (values, what) in [(&one_base, "one base's wings"), (&one_province, "one province's controller"), (&one_radar, "one state's radar")] {
                let ((), counts) = plan_probe::collect(|| layout.update(&inputs, &frame, &tables, Some(values)).unwrap());
                assert_eq!(counts.site_icons, 1, "{what} lays out that base's icon alone");
                let whole = BaseLayout::new(rules, &inputs, &frame, &tables, Some(Arc::clone(values))).unwrap();
                assert!(layout.offers.offers().eq(whole.offers.offers()), "{what}: the offers are a whole layout's");
                assert_eq!(layout.report, whole.report, "{what}: and so is the report");
                assert!(layout.laid_from(Some(values)));
            }
            assert_eq!(layout.offers.offers().count(), STATES as usize, "the uncontrolled province's icon is gone, one radar icon came");

            // The hand-over: above every base cutoff, a new product changes nothing drawn and is held back.
            let mut view = Preparation::default();
            let published = |values: &Arc<BaseValues>| Layered { bases: Some(Arc::clone(values)), ..Layered::default() };
            inputs.camera.height = 5000.;
            view.set_layered(published(&old), Kinds::BASES, Kinds::default());
            view.bind(&inputs, &tables, 0);
            view.bases(&inputs, &frame, &tables, &mut CounterFrame::default()).unwrap();
            let identity = view.layered().identity();
            view.set_layered(published(&one_base), Kinds::BASES, Kinds::default());
            assert_eq!(view.layered().identity(), identity, "no base icon reads it: the window lays nothing out again");
            assert_eq!(view.drawing(), Kinds::BASES);
            inputs.camera.height = 100.;
            let ((), counts) = plan_probe::collect(|| view.bases(&inputs, &frame, &tables, &mut CounterFrame::default()).unwrap());
            assert!(Arc::ptr_eq(view.layered().bases.as_ref().unwrap(), &one_base), "a view that draws the icons takes the newest");
            assert_eq!(counts.site_icons, 2 * u64::from(STATES), "laid out at the new rules");
        },
    );
}
