use super::{
    AirBaseFields, Arc, BTreeMap, BTreeSet, BaseTables, BuildingIconContent, BuildingRow,
    BuildingTokens, DecodedFrame, IconTemplate, RegistrationFields, SlotAdmission, SpawnPoint,
    TokenCarry, TokenKeys, Vision, WingFields, WingInputs, building_tokens, country_list_contains,
    io, merge, naval_base_ships, positions, radar_states, site_icon_states, wings_of,
    wings_per_site,
};
use viewer_gpu::province_map::ProvinceMap;

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

#[test]
fn a_type_sharing_its_lexer_token_with_an_ordinary_type_is_refused() {
    content_reader_script::test_game::enter();
    let buildings = content_reader_key_names::buildings::parse(
        b"buildings={
            Dock={naval_headquarter=yes} dock={show_on_map=1}
            Yard={specialization={specialization_air}} yard={show_on_map=1}
            hq={naval_headquarter=yes} fac={specialization={specialization_naval}}
        }",
        &content_reader_script::key_names::KeyNames::default(),
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

    let unnamed: BTreeMap<String, u32> = [("hq".to_owned(), 3), ("fac".to_owned(), 4)].into();
    assert_eq!(
        icons.headquarter(&[&hq], &unnamed).unwrap_err(),
        "naval HQ building Dock has no single token this hour"
    );
    assert_eq!(icons.unnamed_facility(&unnamed), Some("Yard"));
}

#[test]
fn a_bucket_a_repeated_declaration_reaches_refuses_before_the_fallback() {
    content_reader_script::test_game::enter();
    let buildings = content_reader_key_names::buildings::parse(
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
        &content_reader_script::key_names::KeyNames::default(),
    )
    .unwrap();
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
    assert!(icons.refused_facilities.contains_key("yard"));
    let (_, fac) = &icons.facilities["fac"];
    assert_eq!(
        icons.anchor(7, fac, slot0).unwrap_err(),
        "spawn bucket facility_spawn of fac is reached by a building or bucket declared \
         more than once, differently"
    );
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
    let truncated = |unit| (unit == 1).then_some(0x95);
    assert_eq!(naval_base_ships(&[1, 2], || Some(1), truncated), None);
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

#[test]
fn repeated_icon_templates_are_refused_by_name() {
    content_reader_script::test_game::enter();
    let buildings = content_reader_key_names::buildings::parse(
        b"buildings={
            hq={naval_headquarter=yes} later_hq={naval_headquarter=yes}
            dock={specialization={specialization_naval}} yard={specialization={specialization_air}}
            hq={show_on_map=1}
            yard={specialization={specialization_air} show_on_map=2}
        }",
        &content_reader_script::key_names::KeyNames::default(),
    )
    .unwrap();
    let provinces = ProvinceMap {
        width: 1,
        height: 1,
        ids: &[7],
    };
    let icons = BuildingIconContent::load(&buildings, &[], &provinces, admitted, [[0.0; 3]; 2]);
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
    let (hq, later) = (building(7, 1, 0, 1), building(7, 2, 1, 1));
    assert_eq!(
        icons.headquarter(&[&hq, &later], &tokens).unwrap_err(),
        "naval HQ building hq is declared more than once, differently"
    );
    let found = icons.headquarter(&[&later], &tokens).unwrap().unwrap();
    assert_eq!(found.0.name, "later_hq");
    let razed = building(7, 1, 0, 0);
    assert!(
        icons
            .headquarter(&[&razed, &later], &tokens)
            .unwrap()
            .is_some()
    );
    let unnamed: BTreeMap<String, u32> = [("later_hq".to_owned(), 2)].into();
    assert_eq!(
        icons.headquarter(&[&later], &unnamed).unwrap_err(),
        "naval HQ building hq has no single token this hour"
    );
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

#[test]
fn icons_anchor_only_at_buckets_their_kind_redirects_into() {
    content_reader_script::test_game::enter();
    let buildings = content_reader_key_names::buildings::parse(
        b"buildings={
            port={is_port=yes spawn_point=Port_Spawn}
            hq_at_port={naval_headquarter=yes only_costal=yes spawn_point=port_spawn}
            hq={naval_headquarter=yes only_costal=yes}
            facility={specialization={specialization_air second} spawn_point=FACILITY_SPAWN}
        }
        spawn_points={port_spawn={} Facility_Spawn={}}",
        &content_reader_script::key_names::KeyNames::default(),
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
    assert_eq!(
        icons.anchor(7, &icons.headquarters[0], slot0),
        Ok([1.5, 2.0, 1.5])
    );
    assert_eq!(icons.anchor(8, &icons.headquarters[0], slot0), Ok([9.0; 3]));
    assert_eq!(icons.anchor(7, &icons.headquarters[1], slot0), Ok([9.0; 3]));
    assert!(icons.anchor(7, &icons.headquarters[1], None).is_err());
    let (first, facility) = &icons.facilities["facility"];
    assert_eq!(first, "specialization_air");
    assert_eq!(icons.anchor(8, facility, slot0), Ok([1.5, 2.0, 0.5]));
}

#[test]
fn icons_anchor_only_at_records_the_loader_admits() {
    content_reader_script::test_game::enter();
    use content_reader_map::map::{CountryColors, MapData, ProvinceDefinition, ProvinceKind};
    use content_reader_mesh::mesh::Roster;
    use content_reader_states::states::StateDefinition;
    let buildings = content_reader_key_names::buildings::parse(
        b"buildings={
            port={is_port=yes spawn_point=port_spawn}
            hq={naval_headquarter=yes spawn_point=port_spawn}
            facility={specialization={specialization_air} spawn_point=facility_spawn}
            lone_hq={naval_headquarter=yes spawn_point=lone_spawn}
            lone_port={is_port=yes spawn_point=lone_spawn}
        }
        spawn_points={port_spawn={} facility_spawn={} lone_spawn={}}",
        &content_reader_script::key_names::KeyNames::default(),
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
    let roster = Roster {
        descriptors: content_reader_mesh::mesh::Descriptors {
            entities: ["building_port_spawn", "building_facility_spawn", "building_lone_spawn"]
                .into_iter().map(|name| (name.into(), Default::default())).collect(),
            ..Default::default()
        }.into(),
        ..Default::default()
    };
    let provinces = ProvinceMap {
        width: map.width,
        height: map.height,
        ids: &map.province_ids,
    };
    let slots = [
        slot("port_spawn", 99, 0.1),
        slot("PORT_SPAWN", 3, 0.2),
        slot("facility_spawn", 4, 0.3),
        slot("facility_spawn", 3, 0.4),
        slot("lone_spawn", 99, 0.5),
        slot("lone_spawn", 3, 1.5),
    ];
    let icons = BuildingIconContent::load(
        &buildings,
        &slots,
        &provinces,
        |spawn, slot, province| {
            viewer_foundation::building_facts::admit_slot(
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
    let spawn = &buildings.spawn_points["port_spawn"];
    let admit = |slot| {
        viewer_foundation::building_facts::admit_slot(
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
    content_reader_script::test_game::enter();
    use replay_data::rows::{AirBaseRow, ProvinceRow, TableRows};
    use viewer_foundation::test_support::{test_frame, test_put};
    viewer_counter_kit::fixtures::with_counter_inputs_fixture(
        viewer_counter_kit::fixtures::action_gui(false),
        |_| {},
        |_| {},
        |_| {
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
            let cached = BaseTables::read(&frame, &vision, &Err("unused in fixture".into())).unwrap();
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
        assert_eq!(refusal, "lexical tokens are not in order", "hour {hour}");
    }
}


#[test]
fn merging_state_air_bases_preserves_unknown_occupancy_unless_known_positive() {
    let link = |raw: u32| Some(raw);
    let single_empty = wings_of(&WingInputs {
        flags: &[],
        flags_complete: true,
        link: &link,
        wings: &[],
        wings_complete: true,
        registrations: &[],
        registrations_complete: true,
        registrations_truncated: false,
        bases: &[AirBaseFields {
            id: 1,
            state: Some(7),
            kind: 0,
            capacity: 200,
        }],
        ours: 1,
    });
    assert_eq!(single_empty[&(7, 0)].occupied, Some(false));
    assert_eq!(single_empty[&(7, 0)].stationed, Some(false));
    assert_eq!(single_empty[&(7, 0)].foreign, Some(false));

    let single_unknown = wings_of(&WingInputs {
        flags: &[],
        flags_complete: true,
        link: &link,
        wings: &[],
        wings_complete: false,
        registrations: &[RegistrationFields {
            base: 1,
            slot: 2,
            capacity_raw: 200,
            wings: vec![99],
        }],
        registrations_complete: true,
        registrations_truncated: false,
        bases: &[AirBaseFields {
            id: 1,
            state: Some(7),
            kind: 0,
            capacity: 200,
        }],
        ours: 1,
    });
    assert_eq!(single_unknown[&(7, 0)].occupied, None);

    for (first_id, second_id) in [(1, 2), (2, 1)] {
        let empty_and_unknown = wings_of(&WingInputs {
            flags: &[],
            flags_complete: true,
            link: &link,
            wings: &[],
            wings_complete: false,
            registrations: &[RegistrationFields {
                base: first_id,
                slot: 2,
                capacity_raw: 200,
                wings: vec![99],
            }],
            registrations_complete: true,
            registrations_truncated: false,
            bases: &[
                AirBaseFields {
                    id: first_id,
                    state: Some(7),
                    kind: 0,
                    capacity: 200,
                },
                AirBaseFields {
                    id: second_id,
                    state: Some(7),
                    kind: 0,
                    capacity: 200,
                },
            ],
            ours: 1,
        });
        assert_eq!(
            empty_and_unknown[&(7, 0)].occupied,
            None,
            "unknown occupancy must not collapse to Some(false) for ({first_id}, {second_id})"
        );

        let positive_and_unknown = wings_of(&WingInputs {
            flags: &[],
            flags_complete: true,
            link: &link,
            wings: &[WingFields {
                id: 50,
                owner: 2,
                country_override: 0,
                air_base: second_id,
                base_override: 0,
                planes: 10,
            }],
            wings_complete: false,
            registrations: &[
                RegistrationFields {
                    base: first_id,
                    slot: 2,
                    capacity_raw: 200,
                    wings: vec![99],
                },
                RegistrationFields {
                    base: second_id,
                    slot: 2,
                    capacity_raw: 200,
                    wings: vec![50],
                },
            ],
            registrations_complete: true,
            registrations_truncated: false,
            bases: &[
                AirBaseFields {
                    id: first_id,
                    state: Some(7),
                    kind: 0,
                    capacity: 200,
                },
                AirBaseFields {
                    id: second_id,
                    state: Some(7),
                    kind: 0,
                    capacity: 200,
                },
            ],
            ours: 1,
        });
        assert_eq!(positive_and_unknown[&(7, 0)].occupied, Some(true));
    }

    assert_eq!(merge(Some(true), None), Some(true));
    assert_eq!(merge(None, Some(true)), Some(true));
    assert_eq!(merge(Some(true), Some(false)), Some(true));
    assert_eq!(merge(Some(false), Some(true)), Some(true));
    assert_eq!(merge(Some(false), Some(false)), Some(false));
    assert_eq!(merge(Some(false), None), None);
    assert_eq!(merge(None, Some(false)), None);
    assert_eq!(merge(None, None), None);
}
