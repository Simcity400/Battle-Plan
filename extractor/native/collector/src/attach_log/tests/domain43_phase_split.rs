use super::*;

const D43_PHASE_UNITS: [&str; 11] = [
    "d43_phase_census_us",
    "d43_phase_contract_ptrs_us",
    "d43_phase_contract_rows_us",
    "d43_phase_detail_rows_us",
    "d43_phase_descriptors_us",
    "d43_phase_resource_flow_us",
    "d43_phase_factory_header_us",
    "d43_phase_resource_rows_us",
    "d43_phase_classic_deals_us",
    "d43_phase_stockpile_us",
    "d43_phase_total_us",
];

const D43_PHASE_VALUES: [u64; 11] = [11, 22, 33, 44, 55, 66, 77, 88, 99, 110, 121];

fn read_with_phase(status_flags: u32) -> CompactMarketTradeRead {
    CompactMarketTradeRead {
        rows: Vec::new(),
        phase: Domain43PhaseTimes {
            census_us: 11,
            contract_ptrs_us: 22,
            contract_rows_us: 33,
            detail_rows_us: 44,
            descriptors_us: 55,
            resource_flow_us: 66,
            factory_header_us: 77,
            resource_rows_us: 88,
            classic_deals_us: 99,
            stockpile_us: 110,
            total_us: 121,
        },
        detail_rows: Vec::new(),
        resource_flow_rows: Vec::new(),
        resource_flow_present: false,
        factory_header_rows: Vec::new(),
        factory_header_present: false,
        resource_rows: Vec::new(),
        resource_rows_present: false,
        resource_row_resource_count: 0,
        resource_tokens: Vec::new(),
        classic_deal_rows: Vec::new(),
        classic_deal_country_witnesses: Vec::new(),
        classic_deal_incomplete_owners: Vec::new(),
        classic_deals_present: false,
        activation: None,
        stockpile_countries: Vec::new(),
        stockpile_items: Vec::new(),
        stockpile_expected_country_count: 0,
        stockpile_present: false,
        detail_skipped_rows: 0,
        read_time_us: 0,
        changed_rows: 0,
        status_flags,
        observed_contract_rows: 0,
        emitted_contract_rows: 0,
        buyer_ptr_valid_rows: 0,
        variant_valid_rows: 0,
        amount_valid_rows: 0,
        partial_field_rows: 0,
    }
}

#[test]
fn d43_phase_units_are_emitted_on_a_clean_status() {
    let units = compact_market_trade_diagnostic_units(&read_with_phase(0));
    let names: Vec<&str> = units.iter().map(|unit| unit.name).collect();
    assert_eq!(
        names,
        D43_PHASE_UNITS.to_vec(),
        "a clean d43 frame must carry exactly the phase split and no \
         failure-context word"
    );
    let values: Vec<u64> = units.iter().map(|unit| unit.value).collect();
    assert_eq!(values, D43_PHASE_VALUES.to_vec(), "phase unit wiring");
}

#[test]
fn d43_failure_context_precedes_the_phase_split() {
    let units = compact_market_trade_diagnostic_units(&read_with_phase(
        compact_contract::COMPACT_STATUS_SOURCE_UNAVAILABLE,
    ));
    let names: Vec<&str> = units.iter().map(|unit| unit.name).collect();
    assert!(
        names.len() > D43_PHASE_UNITS.len(),
        "a failing frame must carry failure context as well as the split"
    );
    assert!(
        !names[0].starts_with("d43_phase_"),
        "failure context must fill the per-offender floor before the phase \
         timers; first unit was {:?}",
        names[0]
    );
    assert_eq!(
        &names[names.len() - D43_PHASE_UNITS.len()..],
        &D43_PHASE_UNITS[..],
        "the phase split is appended last"
    );
}
