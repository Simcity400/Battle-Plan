use super::*;
use content_reader_parser::parser;

fn fuel_status(
    scalars: &str,
    consumers: &[(&str, &str, &str)],
    history: impl Fn(i32) -> String,
) -> String {
    let mut text = scalars.to_owned();
    for (index, (priority, received, requested)) in consumers.iter().enumerate() {
        text += &format!(
            " fuel_consumer_data={{ index={index} priority={priority} received={received} requested={requested} }}"
        );
    }
    text += " history_index=13 remaining_hours=1";
    for index in -1..FUEL_HISTORY_RING as i32 {
        text += &format!(" history={{ index={index} {} }}", history(index));
    }
    text
}

fn zero_history(_: i32) -> String {
    "fuel=0.00000 consumed=0.00000 fuel_gain=0 other=0.00000 requested={ 0 0 0 } received={ 0 0 0 }"
        .to_owned()
}

const SCALARS: &str = "fuel=489236.30682 max_fuel=627500.00000 fuel_gain=2 fuel_cost=0 fuel_gain_per_oil=3.6 fuel_gain_from_states=64.4 fuel_gain_from_lend_lease=0.00000 fuel_consumption_from_lend_lease=0.00000";

fn parse_fuel_text(body: &str) -> io::Result<Fuel> {
    let input = format!("countries={{ENG={{fuel_status={{{body}}}}}}}");
    let pack = parse(&parser::parse(input.as_bytes()).unwrap().items)?;
    Ok(pack.countries[b"ENG".as_slice()].fuel.clone().unwrap())
}

#[test]
fn fuel_preserves_dormant_literal_and_signed_modifier_precision() {
    let body = fuel_status(
        "fuel=-1.00000 max_fuel=371500.00000 fuel_gain=-0.00001 fuel_cost=90071992547.40993 fuel_gain_per_oil=2.2 fuel_gain_from_states=0 fuel_gain_from_lend_lease=-0.00003 fuel_consumption_from_lend_lease=281474976710655.99996",
        &[("1", "0", "0")],
        zero_history,
    );
    let document =
        parser::parse(format!("countries={{GER={{fuel_status={{{body}}}}} D01={{}}}}").as_bytes())
            .unwrap();
    let pack = parse(&document.items).unwrap();
    let fuel = pack.countries[b"GER".as_slice()].fuel.as_ref().unwrap();
    assert_eq!(fuel.current.lexeme, b"-1.00000");
    assert_eq!(fuel.current.raw().unwrap(), -32_768);
    assert_eq!(fuel.gain.raw, -1);
    assert_eq!(fuel.cost.raw, 9_007_199_254_740_993);
    assert_eq!(fuel.gain_per_oil.raw, 220_000);
    assert_eq!(fuel.lend_lease_gain.raw().unwrap(), -1);
    assert_eq!(fuel.lend_lease_consumption.raw().unwrap(), i64::MAX);
    assert!(pack.countries[b"D01".as_slice()].fuel.is_none());
}

#[test]
fn fuel_keeps_consumers_cursor_and_every_history_entry_in_writer_order() {
    let fuel = parse_fuel_text(&fuel_status(
        SCALARS,
        &[
            ("1", "83.62571", "83.62571"),
            ("-2", "0", "8.1459"),
            ("1", "216.145", "216.145"),
        ],
        |index| match index {
            0 => "fuel=490422.48471 consumed=265.14755 fuel_gain=390.4 other=0.00000 requested={ 40.85669 8.1459 216.145 } received={ 40.85669 8.1459 216.145 }".to_owned(),
            1 => "fuel=490547.73712 consumed=265.14755 fuel_gain=390.4 other=-0.00003 requested={ 0 -0.00001 1 } received={ 1 2 3 }".to_owned(),
            _ => zero_history(index),
        },
    ))
    .unwrap();
    assert_eq!(fuel.gain_per_oil.raw, 360_000);
    assert_eq!(fuel.gain_from_states.raw, 6_440_000);
    assert_eq!(fuel.lend_lease_gain.raw().unwrap(), 0);
    assert_eq!((fuel.history_index, fuel.remaining_hours), (13, 1));
    assert_eq!(fuel.consumers.len(), 3);
    assert_eq!(fuel.consumers[1].index, 1);
    assert_eq!(fuel.consumers[1].priority, -2);
    assert_eq!(fuel.consumers[1].received.raw, 0);
    assert_eq!(fuel.consumers[1].requested.raw, 814_590);
    assert_eq!(fuel.history.len(), 25);
    assert_eq!(
        fuel.history
            .iter()
            .map(|entry| entry.index)
            .collect::<Vec<_>>(),
        (-1..24).collect::<Vec<_>>()
    );
    let entry = &fuel.history[1];
    assert_eq!(entry.fuel.lexeme, b"490422.48471");
    assert_eq!(entry.consumed.lexeme, b"265.14755");
    assert_eq!(entry.gain.raw, 39_040_000);
    assert_eq!(
        entry.requested.each_ref().map(|value| value.raw),
        [4_085_669, 814_590, 21_614_500]
    );
    let entry = &fuel.history[2];
    assert_eq!(entry.other.raw().unwrap(), -1);
    assert_eq!(
        entry.requested.each_ref().map(|value| value.raw),
        [0, -1, 100_000]
    );
    assert_eq!(
        entry.received.each_ref().map(|value| value.raw),
        [100_000, 200_000, 300_000]
    );
    assert!(
        parse_fuel_text(&fuel_status(SCALARS, &[], zero_history))
            .unwrap()
            .consumers
            .is_empty()
    );
}

#[test]
fn fuel_refuses_layouts_its_writer_cannot_produce() {
    let consumers = [("1", "0", "0"); 3];
    let valid = fuel_status(SCALARS, &consumers, zero_history);
    assert!(parse_fuel_text(&valid).is_ok());
    let mut cases = vec![
        valid.replacen("fuel_cost=0 ", "", 1),
        valid.replacen(
            "fuel_gain_per_oil=3.6 fuel_gain_from_states=64.4",
            "fuel_gain_from_states=64.4 fuel_gain_per_oil=3.6",
            1,
        ),
        valid.replacen("history_index=13", "history_index=13 history_index=13", 1),
        format!("{valid} fuel=1.00000"),
        valid.replacen("index=1 priority", "index=2 priority", 1),
        valid.replacen("received=0 requested=0", "requested=0 received=0", 1),
        valid.replacen("index=5 ", "index=6 ", 1),
        valid.replacen("history={ index=23 ", "history_extra={ index=23 ", 1),
        format!("{valid} history={{ index=24 {} }}", zero_history(24)),
        valid.replacen("requested={ 0 0 0 }", "requested={ 0 0 }", 1),
        valid.replacen("received={ 0 0 0 }", "received={ a=0 0 0 }", 1),
        valid.replacen("fuel_gain=2 ", "fuel_gain=0.000001 ", 1),
    ];
    cases.push(fuel_status(SCALARS, &consumers, |index| {
        if index == 3 {
            zero_history(index).replace(
                "consumed=0.00000 fuel_gain=0",
                "fuel_gain=0 consumed=0.00000",
            )
        } else {
            zero_history(index)
        }
    }));
    for case in cases {
        assert!(parse_fuel_text(&case).is_err(), "{case}");
    }
    assert!(parse(&parser::parse(b"countries={GER={} GER={}}").unwrap().items).is_err());
}

#[test]
fn fuel_text_preserves_full_quantity_range_without_raw_bit_claim() {
    let quantity = FuelQuantity::parse(b"281474976710656.00000").unwrap();
    assert_eq!(quantity.decimal_numerator, 28_147_497_671_065_600_000);
    assert_eq!(
        FuelQuantity::parse(b"-0.00003").unwrap().decimal_numerator,
        -3
    );
    assert_eq!(FuelQuantity::parse(b"-0.00003").unwrap().raw().unwrap(), -1);
    assert!(quantity.raw().is_err());
    assert!(FuelQuantity::parse(b"0.00001").unwrap().raw().is_err());
    for raw in [i64::MIN + 1, -32_768, -1, 0, 1, 32_767, i64::MAX] {
        let numerator = i128::from(raw).abs() * 100_000 / 32_768;
        let literal = format!(
            "{}{}.{:05}",
            if raw < 0 { "-" } else { "" },
            numerator / 100_000,
            numerator % 100_000
        );
        assert_eq!(
            FuelQuantity::parse(literal.as_bytes())
                .unwrap()
                .raw()
                .unwrap(),
            raw
        );
    }
    assert!(FuelQuantity::parse(b"1.000001").is_err());
    assert!(FuelQuantity::parse(b"1e2").is_err());
}

#[test]
fn experience_keeps_all_nine_fixed15_channels_and_fractional_divisor() {
    let document = parser::parse(
        br#"countries={GER={experience_status={
        army_experience=281474976710655.99996
        army_experience_daily=-0.00003
        army_experience_daily_training=0.00006
        navy_experience=-281474976710655.99996
        navy_experience_daily=0.00009
        navy_experience_daily_training=-0.00009
        air_experience=0.00003 air_experience_daily=0.00000
        num_armies_for_training=32.70385
    }}}"#,
    )
    .unwrap();
    let pack = parse(&document.items).unwrap();
    let experience = pack.countries[b"GER".as_slice()]
        .experience
        .as_ref()
        .unwrap();
    assert_eq!(experience.army_raw, Some(i64::MAX));
    assert_eq!(experience.army_daily_raw, Some(-1));
    assert_eq!(experience.army_daily_training_raw, Some(2));
    assert_eq!(experience.navy_raw, Some(i64::MIN + 1));
    assert_eq!(experience.navy_daily_raw, Some(3));
    assert_eq!(experience.navy_daily_training_raw, Some(-3));
    assert_eq!(experience.air_raw, Some(1));
    assert_eq!(experience.air_daily_raw, Some(0));
    assert_eq!(experience.armies_for_training_raw, 1_071_640);
}

#[test]
fn experience_keeps_zero_omissions_separate_from_missing_status() {
    let document = parser::parse(
        b"countries={GER={experience_status={num_armies_for_training=0.00000}} D01={}}",
    )
    .unwrap();
    let pack = parse(&document.items).unwrap();
    let experience = pack.countries[b"GER".as_slice()]
        .experience
        .as_ref()
        .unwrap();
    assert_eq!(
        [
            experience.army_raw,
            experience.army_daily_raw,
            experience.army_daily_training_raw,
            experience.navy_raw,
            experience.navy_daily_raw,
            experience.navy_daily_training_raw,
            experience.air_raw,
            experience.air_daily_raw,
        ],
        [None; 8]
    );
    assert_eq!(experience.armies_for_training_raw, 0);
    assert!(pack.countries[b"D01".as_slice()].experience.is_none());
    assert!(
        parse(
            &parser::parse(b"countries={GER={experience_status={}}}")
                .unwrap()
                .items
        )
        .is_err()
    );
}

#[test]
fn experience_refuses_unwritable_decimals_and_duplicate_channels() {
    for fields in [
        "num_armies_for_training=0 army_experience=0.00001",
        "num_armies_for_training=0.00001",
        "num_armies_for_training=0 army_experience_daily_training=1 navy_experience_daily=0.000001",
        "num_armies_for_training=0 air_experience=0 air_experience=0",
        "num_armies_for_training=0 navy_experience=--281474976710656.00000",
    ] {
        let input = format!("countries={{GER={{experience_status={{{fields}}}}}}}");
        assert!(
            parse(&parser::parse(input.as_bytes()).unwrap().items).is_err(),
            "{fields}"
        );
    }
}

#[test]
fn supply_preserves_saved_word_bits_order_alias_tags_and_zero_levels() {
    let document = parser::parse(
        br#"
        settings={
            node={id={-2147483648 -1} data={disabled=no motorization_level={GER=0 GER=255}}}
            node={id={1 2} data={disabled=yes motorization_level={}}}
            node={id={3 4} data={disabled=no motorization_level={D01=1}}}
        }
        disrupted_supply={node={id={-1 -2147483648} value=-90071992547.40993}}
    "#,
    )
    .unwrap();
    let supply = parse_supply(&document.items).unwrap();
    assert_eq!(supply.motorization.len(), 3);
    assert_eq!(supply.motorization[0].province, i32::MIN);
    assert_eq!(supply.motorization[0].level, 0);
    assert_eq!(supply.motorization[1].level, 255);
    assert_eq!(supply.motorization[2].node_index, 2);
    assert_eq!(supply.motorization[1].map_entry_index, 1);
    assert_eq!(supply.disrupted[0].province as u32, u32::MAX);
    assert_eq!(supply.disrupted[0].value.raw, -9_007_199_254_740_993);
    assert_eq!(parse_supply(&[]).unwrap(), Supply::default());
    let document = parser::parse(b"countries={} supply_system_2={countries={GER={}}}").unwrap();
    let pack = parse(&document.items).unwrap();
    assert!(
        pack.supply.unwrap()[b"GER".as_slice()]
            .motorization
            .is_empty()
    );
}

#[test]
fn temp_nodes_omitted_and_empty_preserve_supply_root_presence() {
    let pack = parse(&parser::parse(b"countries={}").unwrap().items).unwrap();
    assert!(pack.supply.is_none());
    for fields in ["", "temp_nodes={}"] {
        let input = format!("countries={{}} supply_system_2={{countries={{TEST={{{fields}}}}}}}");
        let pack = parse(&parser::parse(input.as_bytes()).unwrap().items).unwrap();
        assert_eq!(pack.supply.unwrap()[b"TEST".as_slice()], Supply::default());
    }
}

#[test]
fn temp_nodes_preserve_signed_provinces_duplicates_order_and_boolean_base() {
    let document = parser::parse(
        b"temp_nodes={
            {province=-2147483648 base=no}
            {province=-1 base=yes}
            {province=-1 base=no}
            {province=0 base=no}
            {province=2147483647 base=yes}
        }",
    )
    .unwrap();
    let supply = parse_supply(&document.items).unwrap();
    let expected: Vec<_> = [
        (i32::MIN, false),
        (-1, true),
        (-1, false),
        (0, false),
        (i32::MAX, true),
    ]
    .into_iter()
    .enumerate()
    .map(|(ordinal, (province, base))| TemporaryNode {
        ordinal: ordinal as u32,
        province,
        base,
    })
    .collect();
    assert_eq!(supply.temp_nodes, expected);
}

#[test]
fn temp_nodes_require_anonymous_blocks_and_mandatory_typed_fields() {
    for input in [
        "temp_nodes=yes",
        "temp_nodes={1}",
        "temp_nodes={node={province=1 base=yes}}",
        "temp_nodes={{base=no}}",
        "temp_nodes={{province=1}}",
        "temp_nodes={{province=1 province=2 base=yes}}",
        "temp_nodes={{province=1 base=yes base=no}}",
        "temp_nodes={{province=1 base=2}}",
        "temp_nodes={{province=1 base={}}}",
        "temp_nodes={{province=2147483648 base=yes}}",
        "temp_nodes={{province=@variable base=yes}}",
        "temp_nodes={{province={} base=yes}}",
        "temp_nodes={} temp_nodes={}",
    ] {
        assert!(
            parse_supply(&parser::parse(input.as_bytes()).unwrap().items).is_err(),
            "{input}"
        );
    }
}

#[test]
fn purchase_parser_preserves_full_typed_id_and_rejects_duplicate_identity() {
    let contract = br#"purchase_contract={
        id={id=4294967295 type=81}
        contract_definition={contract_draft={seller=GER buyer=D01 speed=-1 equipments={} subsidies={}}
            prices={} price_levels={levels={}}}
        delivery_route_handler={
            convoy_client={length=4 type=2 equipments={} id={id=90649 type=4713}
                convoys_subscriber={convoys=1 total=2} country="INS"
                efficiency=1 efficiency_due_to_lost_convoys=0.96728 request=1}
            efficiency=-0.00001 sunk_convoys=2.88504 days=-2147483648}
        contract_delivery_state={factory_cic_progress=90071992547.40993 equipment_transfer_progress=0 collected=-1}
        days=1 variables={}
        contract_meta={completed=3 cic=0 convoys=0 efficiency_due_to_lost_convoys=1
            history={factory_cic_progress=0 equipment_transfer_progress=0 collected=0}}
    }"#;
    let mut input = b"countries={} equipment_market={contracts={".to_vec();
    input.extend_from_slice(contract);
    input.extend_from_slice(b"} requests={0}}");
    let parsed = parse(&parser::parse(&input).unwrap().items).unwrap();
    let purchases = parsed.purchases.unwrap();
    let id = (u64::from(u32::MAX) << 32) | 81;
    assert_eq!(purchases[&id].route_efficiency_sum.raw, -1);
    let client = &purchases[&id].client;
    assert_eq!(client.id, (90649_u64 << 32) | 4713);
    assert_eq!(client.efficiency.raw, 100_000);
    assert_eq!(client.efficiency_due_to_lost_convoys.raw, 96_728);
    assert_eq!(client.request, Some(1));
    assert_eq!(
        client.convoys,
        ConvoyCounts {
            assigned: 1,
            required: 2
        }
    );
    assert_eq!(purchases[&id].route_days, i32::MIN);
    assert_eq!(purchases[&id].completed_deliveries, 3);
    input.truncate(input.len() - b"} requests={0}}".len());
    input.extend_from_slice(contract);
    input.extend_from_slice(b"} requests={0}}");
    assert!(parse(&parser::parse(&input).unwrap().items).is_err());
}

#[test]
fn foreign_homebases_keep_duplicate_provinces_and_inner_identity() {
    let row = br#"{7 {supply=-0.00001 start=1 penalty=2 add=" A\\B " duration=72 province=8 hours=0 decay=1 base=yes}}"#;
    let mut input = b"foreign_homebase_nodes={".to_vec();
    input.extend_from_slice(row);
    input.extend_from_slice(row);
    input.extend_from_slice(b"}");
    let supply = parse_supply(&parser::parse(&input).unwrap().items).unwrap();
    assert_eq!(supply.foreign_homebases.len(), 2);
    assert_eq!(supply.foreign_homebases[0].province, 7);
    assert_eq!(supply.foreign_homebases[0].inner_province, 8);
    assert_eq!(supply.foreign_homebases[0].source, b" A\\B ");
    assert_eq!(supply.foreign_homebases[0].supply.raw, -1);
    assert_eq!(supply.foreign_homebases[1].ordinal, 1);
}

#[test]
fn foreign_homebase_add_is_literal_and_empty_is_distinct_from_missing() {
    let input = br#"foreign_homebase_nodes={
        {10007 {supply=0.32 start=3.3 penalty=0.84 add="Fleet Docking Rights Supply"
            duration=72 province=10007 hours=0 base=yes decay=1}}
        {7 {supply=0 start=0 penalty=0 add="" duration=0 province=7 hours=0 base=no decay=0}}
    }"#;
    let supply = parse_supply(&parser::parse(input).unwrap().items).unwrap();
    assert_eq!(
        supply.foreign_homebases[0].source,
        b"Fleet Docking Rights Supply"
    );
    assert!(supply.foreign_homebases[1].source.is_empty());
    for field in ["", "source=\"unsupported guessed key\""] {
        let input = format!(
            "foreign_homebase_nodes={{{{7 {{supply=0 start=0 penalty=0 {field} duration=0 province=7 hours=0 base=no decay=0}}}}}}"
        );
        assert!(parse_supply(&parser::parse(input.as_bytes()).unwrap().items).is_err());
    }
}

#[test]
fn exchanges_preserve_full_ids_negative_deliveries_and_optional_presence() {
    let document = parser::parse(
        br#"countries={GER={resources={
        export={id={id=4294967295 type=4713} receiver=D01 resource=steel
            efficiency=1 efficiency_due_to_lost_convoys=-0.00001 delivered=-90071992547.40993
            required_cic=-1 lended_cic=2}
        export={id={id=1 type=4713} efficiency=0 efficiency_due_to_lost_convoys=0 delivered=0
            required_cic=0 lended_cic=0 convoys_subscriber={convoys=0 total=3}}
        produced={} transfer_overlord_subject={} imported={} to_use={ {} {} {} } to_export={} base_export={} exported={}
    }} D01={}}"#,
    )
    .unwrap();
    let parsed = parse(&document.items).unwrap();
    let exports = parsed.countries[b"GER".as_slice()]
        .exports
        .as_ref()
        .unwrap();
    assert_eq!(exports[0].id, (u64::from(u32::MAX) << 32) | 4713);
    assert_eq!(exports[0].delivered.raw, -9_007_199_254_740_993);
    assert_eq!(exports[0].convoys, ConvoyCounts::default());
    assert_eq!(
        exports[1].convoys,
        ConvoyCounts {
            assigned: 0,
            required: 3
        }
    );
    assert!(exports[1].receiver.is_none());
    assert!(exports[1].resource.is_none());
    assert!(parsed.countries[b"D01".as_slice()].exports.is_none());
}

fn queue(data: &str) -> String {
    format!(
        "history_queue={{ 0={{ max_elements=2 offset=1 is_full=yes {data} }} 1={{ max_elements=12 offset=0 is_full=no }} 2={{ max_elements=7 offset=0 is_full=no }} }}"
    )
}

#[test]
fn lend_lease_keeps_each_exchanges_delivered_and_sunk_pools_in_save_order() {
    let parsed = parse(
        &parser::parse(
            br#"countries={GER={resources={
                export={id={id=1 type=4713} receiver=D01 resource=test_resource
                    efficiency=1 efficiency_due_to_lost_convoys=0 delivered=2
                    required_cic=1 lended_cic=0}
                lend_lease={ receiver="SOV" id={ id=97730 type=4713 }
                    country="SOV" efficiency=1 efficiency_due_to_lost_convoys=0.5 request=0
                    convoys_subscriber={ convoys=1 total=4 }
                    total_delivered={ equipment={ id={ id=1723 type=70 } amount=18 }
                        equipment={ id={ id=1722 type=70 } amount=0.5 } allow_zero_entries=no }
                    equipment_sunk={ allow_zero_entries=no } }
                lend_lease={ receiver="D01" id={ id=97731 type=4713 }
                    country="D01" efficiency=0.25 efficiency_due_to_lost_convoys=1 request=0
                    total_delivered={ allow_zero_entries=no }
                    equipment_sunk={ equipment={ id={ id=5157 type=70 } amount=3 } allow_zero_entries=no } }
                produced={test_resource=-0.00001} transfer_overlord_subject={} imported={} to_use={ {} {} {} } to_export={} base_export={} exported={}
            }} ENG={resources={produced={} transfer_overlord_subject={} imported={} to_use={ {} {} {} } to_export={} base_export={} exported={}}} ITA={}}"#,
        )
        .unwrap()
        .items,
    )
    .unwrap();
    let ger = parsed.countries[b"GER".as_slice()]
        .lend_lease
        .as_ref()
        .unwrap();
    assert_eq!(ger.len(), 2);
    assert_eq!(ger[0].id, (97730_u64 << 32) | 4713);
    assert_eq!(ger[0].receiver, b"SOV");
    assert_eq!(
        [
            ger[0].efficiency.raw,
            ger[0].efficiency_due_to_lost_convoys.raw
        ],
        [100_000, 50_000]
    );
    assert_eq!(
        ger[0].convoys,
        ConvoyCounts {
            assigned: 1,
            required: 4
        }
    );
    assert_eq!(ger[1].efficiency.raw, 25_000);
    assert_eq!([ger[0].request, ger[1].request], [Some(0), Some(0)]);
    assert_eq!(ger[1].convoys, ConvoyCounts::default());
    assert_eq!(ger[0].delivered.equipment[1].amount.raw, 50_000);
    assert!(ger[0].sunk.equipment.is_empty());
    assert_eq!(ger[1].receiver, b"D01");
    assert_eq!(ger[1].sunk.equipment[0].amount.raw, 300_000);
    let country = &parsed.countries[b"GER".as_slice()];
    assert_eq!(country.exports.as_ref().unwrap()[0].delivered.raw, 200_000);
    assert_eq!(country.exports.as_ref().unwrap()[0].request, None);
    assert_eq!(country.pools.as_ref().unwrap().produced[0].1.raw, -1);
    assert_eq!(
        parsed.countries[b"ENG".as_slice()].lend_lease,
        Some(Vec::new())
    );
    assert!(parsed.countries[b"ITA".as_slice()].lend_lease.is_none());
    assert!(parsed.countries[b"ITA".as_slice()].exports.is_none());
    assert!(parsed.countries[b"ITA".as_slice()].pools.is_none());
    assert!(
        parse(
            &parser::parse(br#"countries={GER={resources={lend_lease={ receiver="SOV" id={ id=1 type=4713 } efficiency=1 efficiency_due_to_lost_convoys=1 } produced={} transfer_overlord_subject={} imported={} to_use={ {} {} {} } to_export={} base_export={} exported={}}}}"#)
                .unwrap()
                .items
        )
        .is_err()
    );
}

fn parse_logistics_text(body: &str) -> io::Result<Pack> {
    parse(
        &parser::parse(
            format!("countries={{GER={{logistics={{{body}}}}} ENG={{logistics={{}}}} D01={{}}}}")
                .as_bytes(),
        )
        .unwrap()
        .items,
    )
}

#[test]
fn logistics_keeps_each_series_window_and_exact_record_values() {
    let body = format!(
        "history={{ 15={{ {} }} 16={{ {} }} }}",
        queue("data={ 0 1 2 145075 2017.5 3423 }"),
        queue("data={ 0 0 0 -112 -0.00001 90071992547.40993 }"),
    );
    let parsed = parse_logistics_text(&body).unwrap();
    let Some(Logistics::History(series)) = &parsed.countries[b"GER".as_slice()].logistics else {
        panic!("saved history");
    };
    assert_eq!(series.keys().copied().collect::<Vec<_>>(), [15, 16]);
    let daily = &series[&16].windows[0];
    assert_eq!(
        (daily.max_elements, daily.offset, daily.is_full),
        (2, 1, true)
    );
    assert_eq!(
        daily.record(1, 3).unwrap(),
        [-11_200_000, -1, 9_007_199_254_740_993]
    );
    assert_eq!(
        series[&15].windows[0].record(1, 3).unwrap(),
        [14_507_500_000, 201_750_000, 342_300_000]
    );
    assert_eq!(series[&15].windows[1].record(11, 3).unwrap(), [0, 0, 0]);
    assert!(daily.record(1, 4).is_err());
    assert!(daily.record(2, 3).is_err());
    assert_eq!(
        parsed.countries[b"ENG".as_slice()].logistics,
        Some(Logistics::Empty)
    );
    assert!(parsed.countries[b"D01".as_slice()].logistics.is_none());
}

#[test]
fn logistics_refuses_layouts_its_writer_cannot_produce() {
    for body in [
        format!(
            "history={{ 16={{ {} }} 15={{ {} }} }}",
            queue(""),
            queue("")
        ),
        format!("history={{ 15={{ {} }} }} other=1", queue("")),
        format!("history={{ 15={{ {} extra=1 }} }}", queue("")),
        format!("history={{ 15={{ {} }} }}", queue("data={ 1 a=2 }")),
        format!("history={{ 15={{ {} }} }}", queue("data={ 1.000001 }")),
        "history={ 15={ history_queue={ 0={ max_elements=2 offset=1 } } } }".to_owned(),
        "history={ 15={ history_queue={ 1={ max_elements=2 offset=1 is_full=no } } } }".to_owned(),
        format!(
            "history={{ 15={{ {} }} }}",
            queue("").replace(
                "is_full=no } }",
                "is_full=no } 3={ max_elements=1 offset=0 is_full=no } }"
            )
        ),
        format!(
            "history={{ 15={{ {} }} }}",
            queue("").replace(
                "is_full=no } }",
                "is_full=no } 0={ max_elements=2 offset=0 is_full=no } }"
            )
        ),
    ] {
        assert!(parse_logistics_text(&body).is_err(), "{body}");
    }
}

#[test]
fn country_resource_pools_keep_every_written_pool() {
    let document = parser::parse(
        br#"countries={GER={resources={
        dirty=no
        produced={ oil=32 steel=301.00001 }
        transfer_overlord_subject={ steel=-2.1 }
        imported={ }
        to_use={ { oil=24 } { } { steel=-30 } }
        to_export={ oil=8 } base_export={ oil=8 } exported={ coal=64 }
    }} D01={}}"#,
    )
    .unwrap();
    let parsed = parse(&document.items).unwrap();
    let pools = parsed.countries[b"GER".as_slice()].pools.as_ref().unwrap();
    let raw = |pool: &ResourcePool| {
        pool.iter()
            .map(|(name, amount)| (String::from_utf8_lossy(name).into_owned(), amount.raw))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        raw(&pools.produced),
        [("oil".into(), 3_200_000), ("steel".into(), 30_100_001)]
    );
    assert_eq!(
        raw(&pools.transfer_overlord_subject),
        [("steel".into(), -210_000)]
    );
    assert!(pools.imported.is_empty());
    assert_eq!(raw(&pools.to_use[0]), [("oil".into(), 2_400_000)]);
    assert!(pools.to_use[1].is_empty());
    assert_eq!(raw(&pools.to_use[2]), [("steel".into(), -3_000_000)]);
    assert_eq!(raw(&pools.exported), [("coal".into(), 6_400_000)]);
    assert!(parsed.countries[b"D01".as_slice()].pools.is_none());
    for broken in [
        "produced={ 1 }",
        "to_use={ { } { } }",
        "to_use={ a={ } { } { } }",
    ] {
        let text = format!("countries={{GER={{resources={{{broken}}}}}}}");
        assert!(
            parse(&parser::parse(text.as_bytes()).unwrap().items).is_err(),
            "{broken}"
        );
    }
}

#[test]
fn repeated_resource_names_keep_every_occurrence_in_written_order() {
    let document = parser::parse(
        br#"countries={GER={resources={
        produced={ steel=3 coal=1 steel=-5 }
        transfer_overlord_subject={ } imported={ } to_use={ { } { steel=2 } { } }
        to_export={ } base_export={ } exported={ }
    }}}"#,
    )
    .unwrap();
    let parsed = parse(&document.items).unwrap();
    let pools = parsed.countries[b"GER".as_slice()].pools.as_ref().unwrap();
    assert_eq!(
        pools
            .produced
            .iter()
            .map(|(name, amount)| (name.as_slice(), amount.raw))
            .collect::<Vec<_>>(),
        [
            (b"steel".as_slice(), 300_000),
            (b"coal".as_slice(), 100_000),
            (b"steel".as_slice(), -500_000),
        ]
    );
    assert_eq!(pools.to_use[1].len(), 1);
}

#[test]
fn a_convoy_subscriber_needs_both_counts() {
    let exchange = |subscriber: &str| {
        let text = format!(
            "countries={{GER={{resources={{export={{id={{id=1 type=4713}} efficiency=1 efficiency_due_to_lost_convoys=1 delivered=0 required_cic=0 lended_cic=0 {subscriber}}} produced={{}} transfer_overlord_subject={{}} imported={{}} to_use={{ {{}} {{}} {{}} }} to_export={{}} base_export={{}} exported={{}}}}}}}}"
        );
        parse(&parser::parse(text.as_bytes()).unwrap().items)
            .map(|pack| pack.countries[b"GER".as_slice()].exports.as_ref().unwrap()[0].convoys)
    };
    assert_eq!(
        exchange("convoys_subscriber={convoys=2 total=2}").unwrap(),
        ConvoyCounts {
            assigned: 2,
            required: 2
        }
    );
    for subscriber in [
        "convoys_subscriber={}",
        "convoys_subscriber={convoys=1}",
        "convoys_subscriber={total=1}",
        "convoys_subscriber={convoys=1 total=1} convoys_subscriber={convoys=1 total=1}",
    ] {
        assert!(exchange(subscriber).is_err(), "{subscriber}");
    }
}
