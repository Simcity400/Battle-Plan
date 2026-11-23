use super::*;

fn parse_text(bytes: &[u8]) -> io::Result<Pack> {
    parse(&content_reader_parser::parser::parse(bytes)?.items)
}

const ORDER_OF_BATTLE: &[u8] = br#"countries={GER={theatres={theatre={
        id={ id=2 type=67 }
        orders_group={
            id={ id=57 type=53 }
            name="Army 1"
            member={ unit={ id=12456 type=4713 } }
            member={ unit={ id=111 type=51 } }
            leader_unit={ id=12456 type=4713 }
            leader={ id=279 type=4713 }
            field_marshal_group=no
            motorization_level=0
        }
        field_marshal_group={
            orders_group={ id=57 type=53 }
            id={ id=431 type=53 }
            name="Army Group 1"
            field_marshal_group=yes
            motorization_level=-1
        }
        theater_group={
            id={ id=45 type=68 }
            priority=1
            name="British Theater 1"
            orders_group={ id=57 type=53 }
            orders_group={ id=431 type=53 }
        }
    }}}}"#;

#[test]
fn orders_group_definitions_keep_saved_order_kind_and_conditional_pointers() {
    let pack = parse_text(ORDER_OF_BATTLE).unwrap();
    let theatre = &pack.theatre_countries.as_ref().unwrap()[b"GER".as_slice()].land[0];
    assert_eq!(theatre.definitions.len(), 2);
    let army = &theatre.definitions[0];
    assert_eq!(army.id, (57 << 32) | 53);
    assert_eq!(army.name.as_deref(), Some(b"Army 1".as_slice()));
    assert!(!army.is_army_group);
    assert_eq!(army.motorization_level, 0);
    assert_eq!(army.leader, Some((279 << 32) | 4713));
    assert_eq!(army.leader_unit, Some((12456 << 32) | 4713));
    assert_eq!(army.members, [(12456 << 32) | 4713, (111 << 32) | 51]);
    assert!(army.children.is_empty());
    let group = &theatre.definitions[1];
    assert_eq!(group.id, (431 << 32) | 53);
    assert!(group.is_army_group);
    assert_eq!(group.motorization_level, -1);
    assert_eq!((group.leader, group.leader_unit), (None, None));
    assert!(group.members.is_empty());
    assert_eq!(group.children, [(57 << 32) | 53]);
    assert_eq!(
        theatre.groups[0].members,
        [(57 << 32) | 53, (431 << 32) | 53]
    );
}

#[test]
fn orders_group_names_keep_saved_presence_and_reject_malformed_fields() {
    let original = String::from_utf8(ORDER_OF_BATTLE.to_vec()).unwrap();
    for (saved, expected) in [
        ("", None),
        ("name=\"\"", Some(b"".as_slice())),
        (
            "name=\"Renamed command\"",
            Some(b"Renamed command".as_slice()),
        ),
    ] {
        let text = original.replace("name=\"Army 1\"", saved);
        let pack = parse_text(text.as_bytes()).unwrap();
        let army =
            &pack.theatre_countries.as_ref().unwrap()[b"GER".as_slice()].land[0].definitions[0];
        assert_eq!(army.name.as_deref(), expected);
        assert_eq!(army.id, (57 << 32) | 53);
    }
    for saved in ["name=A name=B", "name=\"\" name=\"\"", "name={bad=x}"] {
        let text = original.replace("name=\"Army 1\"", saved);
        assert!(parse_text(text.as_bytes()).is_err(), "{saved}");
    }
}

#[test]
fn an_orders_group_without_its_unconditional_fields_is_refused() {
    for removed in ["field_marshal_group=no", "motorization_level=0"] {
        let text = String::from_utf8(ORDER_OF_BATTLE.to_vec())
            .unwrap()
            .replace(removed, "");
        assert!(parse_text(text.as_bytes()).is_err(), "{removed}");
    }
    let text = String::from_utf8(ORDER_OF_BATTLE.to_vec())
        .unwrap()
        .replace("member={ unit={ id=111 type=51 } }", "member={ }");
    assert!(parse_text(text.as_bytes()).is_err());
}

#[test]
fn orders_group_distance_requires_nonnegative_saved_values() {
    let original = String::from_utf8(ORDER_OF_BATTLE.to_vec()).unwrap();
    for value in [0, 1, i32::MAX] {
        let text = original.replace(
            "motorization_level=0",
            &format!("motorization_level=0 distance={value}"),
        );
        let pack = parse_text(text.as_bytes()).unwrap();
        let groups =
            &pack.theatre_countries.as_ref().unwrap()[b"GER".as_slice()].land[0].definitions;
        assert_eq!(groups[0].distance, Some(value));
        assert_eq!(groups[1].distance, None);
    }
    for value in [i32::MIN, -7, -1] {
        let text = original.replace(
            "motorization_level=0",
            &format!("motorization_level=0 distance={value}"),
        );
        assert!(parse_text(text.as_bytes()).is_err());
    }
}

#[test]
fn invasion_source_distance_has_its_own_nonnegative_save_field() {
    let original = String::from_utf8(ORDER_OF_BATTLE.to_vec()).unwrap();
    for value in [0, 17, i32::MAX] {
        let text = original.replace(
            "motorization_level=0",
            &format!(
                "motorization_level=0 distance=31 hq_distance_to_naval_invasion_source={value}"
            ),
        );
        let pack = parse_text(text.as_bytes()).unwrap();
        let groups =
            &pack.theatre_countries.as_ref().unwrap()[b"GER".as_slice()].land[0].definitions;
        assert_eq!(groups[0].invasion_source_distance, Some(value));
        assert_eq!(groups[0].distance, Some(31));
        assert_eq!(groups[1].invasion_source_distance, None);
    }
    for value in [i32::MIN, -7, -1] {
        let text = original.replace(
            "motorization_level=0",
            &format!("motorization_level=0 hq_distance_to_naval_invasion_source={value}"),
        );
        assert!(parse_text(text.as_bytes()).is_err());
    }
}

fn adjacency_content() -> (Vec<u8>, Vec<u8>) {
    let definitions =
        b"0;0;0;0;land\n1;1;0;0;land\n2;2;0;0;sea\n3;3;0;0;lake\n4;4;0;0;land\n5;5;0;0;land\n"
            .to_vec();
    let pixels = [0, 0, 1, 0, 0, 2, 0, 0, 0, 0, 3, 0, 0, 4, 0, 0];
    let mut bitmap = vec![0; 54];
    bitmap[..2].copy_from_slice(b"BM");
    bitmap[2..6].copy_from_slice(&70_u32.to_le_bytes());
    bitmap[10..14].copy_from_slice(&54_u32.to_le_bytes());
    bitmap[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bitmap[18..22].copy_from_slice(&2_i32.to_le_bytes());
    bitmap[22..26].copy_from_slice(&2_i32.to_le_bytes());
    bitmap[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bitmap[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bitmap.extend_from_slice(&pixels);
    (definitions, bitmap)
}

#[test]
fn railway_adjacency_keeps_game_raster_diagonal_and_authored_order() {
    let (definitions, bitmap) = adjacency_content();
    let authored = b"From;To;Type;Through;x;y;x;y;rule;comment\n1;5;sea;-1;-1;-1;-1;-1;CANAL;new\n2;2;impassable;-1;-1;-1;-1;-1;;self\n2;1;river;-1;-1;-1;-1;-1;;existing\n-1;-1;;-1;-1;-1;-1;-1;-1\n1;999;sea;-1;-1;-1;-1;-1;;ignored\n";
    let adjacency = RailwayAdjacency::parse(&definitions, &bitmap, Some(authored)).unwrap();
    assert_eq!(adjacency.neighbors(1).unwrap(), [2, 3, 4, 5]);
    assert_eq!(adjacency.neighbors(2).unwrap(), [1, 4, 3, 2]);
    assert_eq!(adjacency.neighbors(3).unwrap(), [1, 2, 4]);
    assert_eq!(adjacency.neighbors(4).unwrap(), [1, 2, 3]);
    assert_eq!(adjacency.neighbors(5).unwrap(), [1]);
    assert!(adjacency.neighbors(0).is_none());
    assert!(adjacency.neighbors(6).is_none());
}

#[test]
fn railway_adjacency_refuses_content_whose_game_identity_or_storage_differs() {
    let (definitions, bitmap) = adjacency_content();
    assert!(
        RailwayAdjacency::parse(&definitions, &bitmap, None)
            .unwrap()
            .neighbors(5)
            .unwrap()
            .is_empty()
    );
    let reordered =
        b"0;0;0;0;land\n2;2;0;0;sea\n1;1;0;0;land\n3;3;0;0;lake\n4;4;0;0;land\n5;5;0;0;land\n";
    assert!(RailwayAdjacency::parse(reordered, &bitmap, None).is_err());
    let expected = RailwayAdjacency::parse(&definitions, &bitmap, None).unwrap();
    for (offset, value) in [(10, 70_i32), (14, 56)] {
        let mut altered = bitmap.clone();
        altered[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        assert_eq!(
            RailwayAdjacency::parse(&definitions, &altered, None).unwrap(),
            expected
        );
    }
    let mut top_down = bitmap.clone();
    top_down[22..26].copy_from_slice(&(-2_i32).to_le_bytes());
    assert!(RailwayAdjacency::parse(&definitions, &top_down, None).is_err());
    let mut sentinel = bitmap.clone();
    sentinel[54..57].fill(0);
    let allocated = RailwayAdjacency::parse(&definitions, &sentinel, None).unwrap();
    assert_eq!(allocated.neighbors(0), None);
    assert_eq!(allocated.neighbors(1).unwrap(), &[] as &[u32]);
    assert_eq!(allocated.neighbors(6).unwrap(), [2, 3, 4]);
    assert_eq!(allocated.neighbors(7), None);
    assert!(
        RailwayAdjacency::parse(
            &definitions,
            &bitmap,
            Some(b"header\n1;6;sea;-1;-1;-1;-1;-1;;bad\n")
        )
        .is_err()
    );
}

#[test]
fn stationed_units_and_area_representatives_keep_save_order() {
    let pack = parse_text(
        br#"countries={GER={theatres={
                theatre={id={id=5 type=67} volunteers_theatre=SOV
                    unit={id=9 type=4919} unit={id=9 type=4919}
                    area={ 21 22 }}
                theatre={id={id=6 type=67} area={ }}}}}"#,
    )
    .unwrap();
    let land = &pack.theatre_countries.unwrap()[b"GER".as_slice()].land;
    assert_eq!(land[0].units, [(9_u64 << 32) | 4919; 2]);
    assert_eq!(land[0].area_representatives, [21, 22]);
    assert_eq!(
        land[0].volunteers_country.as_deref(),
        Some(b"SOV".as_slice())
    );
    assert!(land[1].units.is_empty() && land[1].area_representatives.is_empty());
}

#[test]
fn theatre_area_arrays_append_in_order_and_refuse_keyed_or_scalar_forms() {
    let pack = parse_text(
        br#"countries={GER={theatres={theatre={id={id=5 type=67}
                area={ 7 } area={ 8 9 }}}}}"#,
    )
    .unwrap();
    let land = &pack.theatre_countries.unwrap()[b"GER".as_slice()].land;
    assert_eq!(land[0].area_representatives, [7, 8, 9]);
    let keyed =
        parse_text(br#"countries={GER={theatres={theatre={id={id=5 type=67} area={ x=7 }}}}}"#)
            .unwrap_err()
            .to_string();
    assert!(
        keyed.contains("unkeyed representative-province list"),
        "{keyed}"
    );
    let scalar = parse_text(br#"countries={GER={theatres={theatre={id={id=5 type=67} area=7}}}}"#)
        .unwrap_err()
        .to_string();
    assert!(
        scalar.contains("expected an ordinary save block"),
        "{scalar}"
    );
}

#[test]
fn rail_info_and_theatre_trees_preserve_scopes_order_and_signed_fields() {
    let pack = parse_text(
        br#"rail_way={rail_way={1={rail_way={0 -1 2147483647} cooldown=1} 2={}}}
            countries={GER={
                theatres={theatre={id={id=4294967295 type=67}
                    theater_group={id={id=0 type=68} priority=-2147483648
                        orders_group={id=4294967295 type=53} orders_group={id=0 type=53}}}}
                air_theatre={id={id=4294967295 type=77} country=GER
                    air_group={id={id=0 type=78} air_theatre={id=4294967295 type=77}
                        name="" air_wing={{id=4294967295 type=69} {id=0 type=69}}}}
            }}"#,
    )
    .unwrap();
    let rails = pack.railways.unwrap();
    assert_eq!(rails[&1].levels, [0, -1, i32::MAX]);
    assert_eq!(rails[&1].cooldown, Some(1));
    assert!(rails[&2].levels.is_empty());
    assert_eq!(rails[&2].cooldown, None);
    let countries = pack.theatre_countries.unwrap();
    let country = &countries[b"GER".as_slice()];
    assert_eq!(country.land[0].volunteers_country, None);
    assert_eq!(country.land[0].groups[0].priority, i32::MIN);
    assert_eq!(country.land[0].groups[0].name, None);
    assert_eq!(
        country.land[0].groups[0].members,
        [(u64::from(u32::MAX) << 32) | 53, 53]
    );
    assert_eq!(country.air[0].name, None);
    assert_eq!(country.air[0].groups[0].name, b"");
    assert_eq!(
        country.air[0].groups[0].wings,
        [(u64::from(u32::MAX) << 32) | 69, 69]
    );
    assert!(parse_text(b"rail_way={rail_way={1={cooldown=0}}}").is_err());
    assert!(parse_text(b"rail_way={rail_way={1={} 1={}}}").is_err());
}

#[test]
fn supremacy_pairs_keep_fixed5_extrema_and_distinguish_missing_scopes() {
    let pack = parse_text(br#"region={
            1={dominance={countries={GER} values={
                { GER {current=90071992547.40993 base_target=-92233720368547.75808 target=92233720368547.75807 individual_ratio=-0.00001 previous=0 decline_from=1.00001} }
                { GER {current=0 base_target=0 target=0 individual_ratio=0 previous=0 decline_from=0} }
            }}}
            4={dominance={countries={}}}
            8={}
        }"#).unwrap();
    let regions = pack.supremacy_regions.unwrap();
    let rows = regions[&1].values.as_ref().unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].current.raw, 9_007_199_254_740_993);
    assert_eq!(rows[0].base_target.raw, i64::MIN);
    assert_eq!(rows[0].target.raw, i64::MAX);
    assert_eq!(rows[0].individual_ratio.raw, -1);
    assert_eq!(regions[&4].values, Some(Vec::new()));
    assert_eq!(regions[&8].values, None);
    assert_eq!(
        parse_text(b"region={}").unwrap().supremacy_regions,
        Some(BTreeMap::new())
    );
    assert_eq!(parse_text(b"").unwrap().supremacy_regions, None);
    assert!(parse_text(b"region={1={dominance={values={{GER {current=0}}}}}}").is_err());
    assert!(parse_text(b"region={1={} 1={}}").is_err());
}

#[test]
fn convoy_history_preserves_signed_fields_country_aliases_and_order() {
    let pack = parse_text(
        br#"sunk_convoys_history={
            sunk_convoy={month=-2147483648 convoys=-1 killer_country=--- owner=GER}
            sunk_convoy={month=2147483647 convoys=2147483647 killer_country=SPR owner=D01}
        }"#,
    )
    .unwrap();
    assert_eq!(pack.sunk_convoys[0].month, i32::MIN);
    assert_eq!(pack.sunk_convoys[0].convoys, -1);
    assert_eq!(pack.sunk_convoys[1].owner_country, b"D01");
    assert!(
        parse_text(b"sunk_convoys_history={sunk_convoy={month=1 convoys=0 owner=GER}}").is_err()
    );
}

#[test]
fn anonymous_history_preserves_duplicates_optional_references_and_null_battle() {
    let entry = br#"sunk_ship={name="" country=GER killer_country=--- level=-1
            definition=destroyer killer_definition=none date=1939.1.1.1
            equipment_variant={id=4294967295 type=70}
            air_wing={id=0 type=54} battle={id=0 type=0} convoy=no}"#;
    let mut text = b"history={".to_vec();
    text.extend_from_slice(entry);
    text.extend_from_slice(entry);
    text.extend_from_slice(b"}");
    let pack = parse_text(&text).unwrap();
    assert_eq!(pack.sunk_ships.len(), 2);
    assert_eq!(pack.sunk_ships[0], pack.sunk_ships[1]);
    let row = &pack.sunk_ships[0];
    assert_eq!(row.ship_name, Some(Vec::new()));
    assert_eq!(row.attribution_name, None);
    assert_eq!(
        row.victim_equipment_variant,
        Some((u64::from(u32::MAX) << 32) | 70)
    );
    assert_eq!(row.damage_source_reference, Some(54));
    assert_eq!(row.province, None);
    assert_eq!(row.battle, 0);
    assert!(!row.assisted_kill);
    assert_eq!(parse_text(b"").unwrap(), Pack::default());
}

#[test]
fn result_handles_and_country_membership_keep_full_words_and_list_order() {
    let text = br#"naval_combat_result={id={id=4294967295 type=61}
            location=-1 date=1939.1.1.1 to_discard_date=1939.1.2.1
            importance=-2147483648 port_strike=yes naval_strike=no
            attacker={ship={unique_id={id=0 type=69} cached_info={sunk_by="cause"}} countries={GER GER ---}}
            defender={} shown_to_countries={SOV GER}}
        "#;
    let pack = parse_text(text).unwrap();
    let row = pack.results.values().next().unwrap();
    assert_eq!(row.id, (u64::from(u32::MAX) << 32) | 61);
    assert_eq!(row.province, u32::MAX);
    assert_eq!(row.category, i32::MIN as u32);
    assert_eq!(row.sides[0].members[0].id, 69);
    assert_eq!(
        row.sides[0].members[0].sunk_by.as_deref(),
        Some(b"cause".as_slice())
    );
    assert_eq!(
        row.sides[0].countries,
        [b"GER".to_vec(), b"GER".to_vec(), b"---".to_vec()]
    );
    assert!(row.sides[1].members.is_empty());
    let duplicated = [text.as_slice(), text.as_slice()].concat();
    assert!(parse_text(&duplicated).is_err());
}
