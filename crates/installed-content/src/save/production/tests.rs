use super::*;

#[test]
fn deployment_conveyor_requires_its_unconditional_name_key() {
    let parse = |field: &str| {
        let doc = crate::parser::parse(format!("military_deployment_conveyor={{id={{id=1 type=8}} division_template_id={{id=2 type=9}} amount=1 location=0 priority=1 {field}}}").as_bytes()).unwrap();
        parse_deployment(&doc.items)
    };
    assert!(parse("").unwrap_err().to_string().contains("name"));
    assert!(parse("name=\"\"").unwrap()[0].name.is_empty());
    assert_eq!(parse("name=Custom").unwrap()[0].name, b"Custom");
}

#[test]
fn deployment_name_ordered_retains_presence_and_refuses_malformed_flags() {
    for (flag, expected) in [("", None), ("is_name_ordered=no", Some(false))] {
        let text = format!("id={{id=17 type=63}} division_name={{type=0 {flag}}}");
        let fields = crate::parser::parse(text.as_bytes()).unwrap();
        assert_eq!(
            parse_deployment_line(&fields.items).unwrap().name_ordered,
            expected
        );
    }
    for flag in [
        "is_name_ordered={}",
        "is_name_ordered=no is_name_ordered=no",
        "is_name_ordered=unknown",
    ] {
        let text = format!("id={{id=17 type=63}} division_name={{type=0 {flag}}}");
        let fields = crate::parser::parse(text.as_bytes()).unwrap();
        assert!(parse_deployment_line(&fields.items).is_err());
    }
}

#[test]
fn equipment_rosters_keep_saved_order_repeats_and_absence() {
    let production = |text: &[u8]| {
        let document = crate::parser::parse(text).unwrap();
        parse(&document.items).unwrap().countries.unwrap()[b"GER".as_slice()]
            .production
            .clone()
            .unwrap()
    };
    let saved = production(
        br#"countries={GER={production={available_equipments={
            equipment={id=4294967295 type=70}
            equipment={id=0 type=70}
            equipment={id=4294967295 type=70}
        } foreign_lease_equipments={ equipment={id=0 type=70} }}}}"#,
    );
    assert_eq!(
        saved.available_variants,
        Some(vec![0xffff_ffff_0000_0046, 70, 0xffff_ffff_0000_0046])
    );
    assert_eq!(saved.foreign_lease_variants, [70]);
    let absent = production(b"countries={GER={production={}}}");
    assert_eq!(absent.available_variants, None);
    assert!(absent.foreign_lease_variants.is_empty());
    // The recalculation flag keeps its absence apart from `no`.
    assert_eq!(absent.dirty, None);
    assert_eq!(
        production(b"countries={GER={production={dirty=yes}}}").dirty,
        Some(true)
    );
    assert_eq!(
        production(b"countries={GER={production={dirty=no}}}").dirty,
        Some(false)
    );
}

#[test]
fn pool_preserves_order_duplicates_signed_extrema_and_zero() {
    let doc = crate::parser::parse(
        br#"
        equipment={id={id=4294967295 type=70} amount=-92233720368547.75808}
        equipment={id={id=0 type=70} amount=0.00000}
        equipment={id={id=4294967295 type=70} amount=92233720368547.75807}
        allow_zero_entries=yes
        "#,
    )
    .unwrap();
    let pool = parse_pool(&doc.items).unwrap();
    assert_eq!(pool.allow_zero_entries, Some(true));
    assert_eq!(pool.equipment.len(), 3);
    assert_eq!(pool.equipment[0].variant, 0xffff_ffff_0000_0046);
    assert_eq!(pool.equipment[1].variant, 70);
    assert_eq!(pool.equipment[2].variant, pool.equipment[0].variant);
    assert_eq!(pool.equipment[0].amount.raw, i64::MIN);
    assert_eq!(pool.equipment[1].amount.raw, 0);
    assert_eq!(pool.equipment[2].amount.raw, i64::MAX);
    assert_eq!(parse_pool(&[]).unwrap().allow_zero_entries, None);
}

#[test]
fn pool_refuses_conflicting_fields_null_handles_and_precision_loss() {
    for text in [
        "equipment={id={id=1 type=70} amount=1 amount=2}",
        "equipment={id={id=0 type=0} amount=1}",
        "equipment={id={id=1 type=70} amount=0.000001}",
        "equipment={id={id=1 type=70} amount=92233720368547.75808}",
        "allow_zero_entries=yes allow_zero_entries=no",
    ] {
        let doc = crate::parser::parse(text.as_bytes()).unwrap();
        assert!(parse_pool(&doc.items).is_err(), "{text}");
    }
}

#[test]
fn registry_keeps_full_handles_and_distinguishes_absence_from_empty() {
    assert!(parse(&[]).unwrap().variants.is_none());
    let doc = crate::parser::parse(b"equipments={}").unwrap();
    assert!(parse(&doc.items).unwrap().variants.unwrap().is_empty());
    let doc =
        crate::parser::parse(b"equipments={same={id={id=1 type=70}} same={id={id=1 type=71}}}")
            .unwrap();
    let rows = parse(&doc.items).unwrap().variants.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[&0x1_0000_0046].definition, b"same");
    let doc =
        crate::parser::parse(b"equipments={first={id={id=1 type=70}} second={id={id=1 type=70}}}")
            .unwrap();
    assert!(parse(&doc.items).is_err());
}

#[test]
fn variant_version_keeps_raw_byte_and_native_zero_omission() {
    let variant = |text: &str| {
        let document = crate::parser::parse(
            format!("equipments={{synthetic={{id={{id=1 type=70}} {text}}}}}").as_bytes(),
        )
        .unwrap();
        parse(&document.items).map(|pack| pack.variants.unwrap().remove(&0x1_0000_0046).unwrap())
    };
    assert_eq!(variant("version=255").unwrap().version, Some(u8::MAX));
    assert_eq!(variant("version=1").unwrap().version, Some(1));
    assert_eq!(variant("version=0").unwrap().version, Some(0));
    assert_eq!(variant("").unwrap().version, None);
    for text in [
        "version=-1",
        "version=256",
        "version={}",
        "version=1 version=2",
    ] {
        assert!(variant(text).is_err(), "{text}");
    }
}

#[test]
fn division_bank_wrapper_is_distinct_from_other_country_equipment() {
    let doc = crate::parser::parse(
        b"countries={GER={equipment={invalid=yes} units={
          division={id={id=7 type=51} id={id=7 type=51} equipment={
            equipment={id={id=3 type=70} amount=0.00001}}}
          division={id={id=8 type=51}}
        }}}",
    )
    .unwrap();
    let divisions = parse(&doc.items).unwrap().divisions.unwrap();
    assert_eq!(divisions.len(), 2);
    assert_eq!(
        divisions[&0x7_0000_0033]
            .equipment
            .as_ref()
            .unwrap()
            .equipment[0]
            .amount
            .raw,
        1
    );
    assert!(divisions[&0x8_0000_0033].equipment.is_none());
}

#[test]
fn production_base_preserves_native_widths_and_writer_omissions() {
    let doc = crate::parser::parse(
        b"id={id=4294967295 type=56} cost=92233720368547.75807
          speed=-92233720368547.75808 produced=-0.00001
          active_factories=-2147483648 priority=2147483647 amount=-1",
    )
    .unwrap();
    let line = parse_line_base(&doc.items).unwrap();
    assert_eq!(line.id, 0xffff_ffff_0000_0038);
    assert_eq!(line.cost.raw, i64::MAX);
    assert_eq!(line.speed.unwrap().raw, i64::MIN);
    assert_eq!(line.produced.unwrap().raw, -1);
    assert_eq!(line.active_factories, Some(i32::MIN));
    assert_eq!(line.priority, i32::MAX);
    assert_eq!(line.amount, -1);
    let doc = crate::parser::parse(b"id={id=0 type=56} cost=0 priority=0 amount=1").unwrap();
    let line = parse_line_base(&doc.items).unwrap();
    assert_eq!(line.speed, None);
    assert_eq!(line.produced, None);
    assert_eq!(line.active_factories, None);
}

#[test]
fn delivery_progress_exists_only_for_active_status_and_keeps_defaults_absent() {
    let doc = crate::parser::parse(b"produced={}").unwrap();
    let delivery = parse_delivery(&doc.items).unwrap();
    assert_eq!(delivery.status, None);
    assert_eq!(delivery.progress, None);
    assert_eq!(delivery.total_progress, None);
    let doc = crate::parser::parse(
        b"status=1 progress=-0.00001 total_progress=92233720368547.75807 produced={}",
    )
    .unwrap();
    let delivery = parse_delivery(&doc.items).unwrap();
    assert_eq!(delivery.progress.unwrap().raw, -1);
    assert_eq!(delivery.total_progress.unwrap().raw, i64::MAX);
    for text in [
        "status=1 produced={}",
        "status=2 progress=0 produced={}",
        "status=1 progress=0",
    ] {
        let doc = crate::parser::parse(text.as_bytes()).unwrap();
        assert!(parse_delivery(&doc.items).is_err(), "{text}");
    }
}

#[test]
fn production_country_wrapper_and_variant_children_preserve_native_keys_and_order() {
    let doc = crate::parser::parse(
        br#"
        countries={GER={production={
            naval_lines={id={id=4294967295 type=57} cost=0 priority=1 amount=-1
                requested_factories=-2 equipment_variant_index={id=3 type=70}
                industrial_manufacturer={id=0 type=79}}
            military_lines={id={id=0 type=56} cost=0 priority=0 amount=1
                requested_factories=2 equipment_variant_index={id=3 type=70}}
            equipments={allow_zero_entries=no}
        }}}
        equipments={plane={id={id=3 type=70} name="name" position="II"
            show_position=no obsolete=yes creator="GER" origin="---"
            modules={slot=first slot=second}
            upgrades={upgrades={{"engine" 255} {"engine" 0}}}
        }}
    "#,
    )
    .unwrap();
    let pack = parse(&doc.items).unwrap();
    let country = &pack.countries.as_ref().unwrap()[b"GER".as_slice()];
    let production = country.production.as_ref().unwrap();
    assert_eq!(production.lines[0].kind, 1);
    assert_eq!(production.lines[1].kind, 0);
    assert_eq!(production.lines[0].base.id, 0xffff_ffff_0000_0039);
    assert_eq!(production.lines[0].mio, Some(79));
    assert_eq!(production.lines[0].requested_factories, -2);
    assert_eq!(production.lines[1].mio, None);
    let variant = &pack.variants.as_ref().unwrap()[&0x3_0000_0046];
    assert_eq!(variant.variant_name.as_deref(), Some(b"II".as_slice()));
    assert_eq!(variant.append_variant_name, Some(false));
    let modules = variant.modules.as_ref().unwrap();
    assert_eq!(modules[0].slot, modules[1].slot);
    assert_eq!(modules[0].definition, b"first");
    assert_eq!(modules[1].definition, b"second");
    let upgrades = variant.upgrades.as_ref().unwrap();
    assert_eq!(upgrades[0].level, 255);
    assert_eq!(upgrades[1].level, 0);
    for level in ["-1", "256"] {
        let text = "equipments={plane={id={id=3 type=70} upgrades={upgrades={{engine LEVEL}}}}}"
            .replace("LEVEL", level);
        let doc = crate::parser::parse(text.as_bytes()).unwrap();
        assert!(parse(&doc.items).is_err(), "{level}");
    }
}

#[test]
fn selected_variant_organisation_marker_keeps_signed_values_and_omission() {
    for (suffix, expected) in [
        ("", None),
        ("number_of_design_team_traits=0", Some(0)),
        ("number_of_design_team_traits=-2147483648", Some(i32::MIN)),
        ("number_of_design_team_traits=2147483647", Some(i32::MAX)),
    ] {
        let doc = crate::parser::parse(
            format!("equipments={{type={{id={{id=1 type=70}} {suffix}}}}}").as_bytes(),
        )
        .unwrap();
        let pack = parse(&doc.items).unwrap();
        assert_eq!(
            pack.variants.as_ref().unwrap()[&((1 << 32) | 70)].organisation_marker,
            expected
        );
    }
    for suffix in [
        "number_of_design_team_traits=2147483648",
        "number_of_design_team_traits=-2147483649",
        "number_of_design_team_traits={}",
        "number_of_design_team_traits=1 number_of_design_team_traits=1",
    ] {
        let doc = crate::parser::parse(
            format!("equipments={{type={{id={{id=1 type=70}} {suffix}}}}}").as_bytes(),
        )
        .unwrap();
        assert!(parse(&doc.items).is_err(), "{suffix}");
    }
}

#[test]
fn army_upgrade_parent_priority_and_nested_requests_are_not_conflated() {
    let doc = crate::parser::parse(
        b"countries={GER={units={division={
        id={id=1 type=51} requests={upgrades={priority=-1
            request={request=-0.00001 total=92233720368547.75807
                equipment_variant_index={id=4294967295 type=70}}
            request={request=0 total=1}
            delivery={status=-2147483648 produced={allow_zero_entries=yes
                equipment={id={id=0 type=70} amount=-92233720368547.75808}}}
        }}
    }}}}",
    )
    .unwrap();
    let pack = parse(&doc.items).unwrap();
    let group = pack.divisions.as_ref().unwrap()[&0x1_0000_0033]
        .upgrades
        .as_ref()
        .unwrap();
    assert_eq!(group.priority, Some(u32::MAX));
    assert_eq!(group.requests.len(), 2);
    assert_eq!(group.requests[0].equipment, Some(0xffff_ffff_0000_0046));
    assert_eq!(group.requests[1].equipment, None);
    assert_eq!(group.deliveries[0].status, Some(0x8000_0000));
    assert_eq!(group.deliveries[0].progress, None);
    assert_eq!(
        group.deliveries[0].produced.equipment[0].amount.raw,
        i64::MIN
    );
}

fn pack(text: &[u8]) -> io::Result<Pack> {
    parse(&crate::parser::parse(text)?.items)
}

const QUEUE: &[u8] = br#"countries={
GER={
production={
production_licenses={
    available={ id=5061 type=70 }
    available={ id=5061 type=70 }
    owned_license={ lended_cic=-1 required_cic=2147483647 owner="LAT" giver="ENG"
                    start_date="1939.5.10.1" parent={ id=1441 type=70 }
                    equipment={ id=5061 type=70 } equipment={ id=5061 type=70 } }
}
}
deployment={
unlocked_subunits={ { infantry yes} }
military_deployment_conveyor={
    id={ id=433 type=64 }
    division_template_id={ id=2526 type=52 }
    name="Medium Tank 1942 C"
    amount=-1
    location=6103
    priority=1
    role=armor
    closed=no
    military_deployment_line={
        id={ id=7067 type=63 }
        division_name={ type=0 override="1. Light Tank" override_set_programmatically=yes }
        amount=1
        military_deployment={
            army_manpower_value={ }
            army_manpower_need={ }
            halting_reason=1
            priority=3
        }
    }
    military_deployment_line={
        id={ id=7066 type=63 }
        division_name={ type=0 name_order=4 }
        amount=1
        military_deployment={
            training=0.00001
            max_training=0.98799
            army_manpower_value={ value={ tag="ENG" value=10620 } value={ tag="GER" value=1 } }
            army_manpower_need={ }
            equipment={
                equipment={ id={ id=5553 type=70 } amount=16 }
                allow_zero_entries=no
            }
            max_manpower=10620
        }
    }
}
military_deployment_conveyor={
    id={ id=434 type=64 }
    division_template_id={ id=1 type=52 }
    name=""
    amount=0
    location=0
    priority=0
    role=infantry
    closed=yes
    government_in_exile_tag="ETH"
}
}
}
}"#;

#[test]
fn licences_and_the_deployment_queue_keep_order_duplicates_and_signed_words() {
    let saved = pack(QUEUE).unwrap();
    let country = &saved.countries.unwrap()[b"GER".as_slice()];
    let licenses = country
        .production
        .as_ref()
        .unwrap()
        .licenses
        .clone()
        .unwrap();
    assert_eq!(licenses.len(), 2);
    assert_eq!(licenses[0], (5061_u64 << 32) | 70);
    assert_eq!(licenses[0], licenses[1]);

    let records = country
        .production
        .as_ref()
        .unwrap()
        .license_records
        .clone()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].lended_cic, -1);
    assert_eq!(records[0].required_cic, i32::MAX);
    assert_eq!(records[0].owner, b"LAT");
    assert_eq!(records[0].giver, b"ENG");
    assert_eq!(records[0].parent, (1441_u64 << 32) | 70);
    assert_eq!(records[0].equipment, vec![(5061_u64 << 32) | 70; 2]);
    assert_eq!(records[0].start_date.year, 1939);

    let queue = country.deployment.clone().unwrap();
    assert_eq!(queue.len(), 2);
    assert_eq!(
        queue[1].government_in_exile_tag.as_deref(),
        Some(b"ETH".as_slice())
    );
    // A custom-named line omits name_order and carries an override name;
    // an untrained deployment omits training, max_training and equipment.
    let custom = &queue[0].lines[0];
    assert_eq!(custom.name_order, None);
    assert_eq!(
        custom.name_override.as_deref(),
        Some(b"1. Light Tank".as_slice())
    );
    assert_eq!(custom.name_override_programmatic, Some(true));
    let untrained = custom.deployment.as_ref().unwrap();
    assert_eq!(untrained.training, None);
    assert_eq!(untrained.max_training, None);
    assert_eq!(untrained.equipment, None);
    assert_eq!(untrained.max_manpower, None);
    assert_eq!(untrained.halting_reason, Some(1));
    assert_eq!(untrained.priority, Some(3));
    assert_eq!(queue[0].id, (433_u64 << 32) | 64);
    assert_eq!(queue[0].template, (2526_u64 << 32) | 52);
    assert_eq!(queue[0].name, b"Medium Tank 1942 C");
    assert_eq!(queue[0].amount, -1);
    assert_eq!(queue[0].location, 6103);
    assert_eq!(queue[0].priority, 1);
    assert_eq!(queue[0].role.as_deref(), Some(b"armor".as_slice()));
    assert_eq!(queue[0].closed, Some(false));
    assert_eq!(queue[1].closed, Some(true));
    assert!(queue[1].lines.is_empty());
    assert_eq!(queue[1].name, b"");

    let line = &queue[0].lines[1];
    assert_eq!(line.id, (7066_u64 << 32) | 63);
    assert_eq!(line.name_order, Some(4));
    assert_eq!(line.name_type, Some(0));
    assert_eq!(line.amount, Some(1));
    let deployment = line.deployment.as_ref().unwrap();
    assert_eq!(deployment.training.as_ref().unwrap().raw, 1);
    assert_eq!(deployment.max_training.as_ref().unwrap().raw, 98_799);
    assert_eq!(deployment.max_manpower, Some(10_620));
    assert_eq!(
        deployment.manpower_have.as_ref().unwrap(),
        &[
            ManpowerValue {
                tag: b"ENG".to_vec(),
                value: 10_620
            },
            ManpowerValue {
                tag: b"GER".to_vec(),
                value: 1
            },
        ]
    );
    assert_eq!(deployment.manpower_need.as_ref().unwrap(), &Vec::new());
    assert_eq!(deployment.priority, None);
    let pool = deployment.equipment.as_ref().unwrap();
    assert_eq!(pool.allow_zero_entries, Some(false));
    assert_eq!(pool.equipment[0].variant, (5553_u64 << 32) | 70);
    assert_eq!(pool.equipment[0].amount.raw, 1_600_000);
}

#[test]
fn absent_containers_stay_absent_and_foreign_keys_are_refused() {
    let saved = pack(b"countries={GER={production={}}}").unwrap();
    let country = &saved.countries.unwrap()[b"GER".as_slice()];
    assert_eq!(country.production.as_ref().unwrap().licenses, None);
    assert_eq!(country.production.as_ref().unwrap().license_records, None);
    assert_eq!(country.deployment, None);
    let empty = pack(b"countries={GER={production={production_licenses={}} deployment={}}}")
        .unwrap()
        .countries
        .unwrap();
    let country = &empty[b"GER".as_slice()];
    assert_eq!(
        country.production.as_ref().unwrap().licenses,
        Some(Vec::new())
    );
    assert_eq!(
        country.production.as_ref().unwrap().license_records,
        Some(Vec::new())
    );
    assert_eq!(country.deployment, Some(Vec::new()));
    for text in [
        "countries={GER={production={production_licenses={ owned={ id=1 type=70 } }}}}",
        r#"countries={GER={production={production_licenses={ owned_license={
            required_cic=0 owner="A" giver="B" start_date="1936.1.1.1"
            parent={ id=1 type=70 } }}}}}"#,
        r#"countries={GER={production={production_licenses={ owned_license={
            lended_cic=0 required_cic=0 owner="A" giver="B" start_date="1936.1.1.1" }}}}}"#,
        "countries={GER={deployment={military_deployment_conveyor={ name=x }}}}",
        r#"countries={GER={deployment={military_deployment_conveyor={
            id={ id=1 type=64 } name=x amount=0 location=0 priority=0 }}}}"#,
        r#"countries={GER={deployment={military_deployment_conveyor={
            id={ id=1 type=64 } division_template_id={ id=1 type=52 }
            name=x amount=0 location=0 priority=0
            military_deployment_line={ id={ id=2 type=63 } military_deployment={
                army_manpower_value={ tag={ tag="ENG" value=1 } } } } }}}}"#,
    ] {
        assert!(pack(text.as_bytes()).is_err(), "{text}");
    }
}
